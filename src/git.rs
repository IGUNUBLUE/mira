//! Thin wrapper over the `git` CLI. Using the CLI (instead of libgit2) keeps
//! behavior identical to what the user and their agents see in the terminal,
//! and respects hooks, config, credentials helpers and signing.

pub mod branches;
pub mod commits;
pub mod remote;

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    /// Path relative to the repo root.
    pub path: String,
    /// Status letter: M, A, D, R, C, T, U (untracked) or `!` (conflict).
    pub code: char,
    /// A rename's or copy's original path.
    pub orig: Option<String>,
}

impl Change {
    pub fn new(path: impl Into<String>, code: char) -> Self {
        Self { path: path.into(), code, orig: None }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Commit {
    pub hash: std::sync::Arc<str>,
    pub subject: String,
    pub author: String,
    pub when: String,
    /// Refs pointing here, without the `HEAD -> ` marker.
    pub refs: String,
    /// `hash  author · when`, for the tooltip.
    pub detail: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Snapshot {
    pub root: Option<PathBuf>,
    pub branch: String,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub staged: Vec<Change>,
    pub unstaged: Vec<Change>,
    pub log: Vec<Commit>,
    /// Paths (relative, with trailing `/` for directories) ignored by git.
    pub ignored: Vec<String>,
    /// The change list was cut at [`MAX_CHANGES`] entries.
    pub truncated: bool,
}

/// Upper bound for listed changes; keeps huge repos (e.g. a dotfiles repo at \$HOME) responsive.
pub const MAX_CHANGES: usize = 2_000;

impl Snapshot {
    /// Unmerged paths (code `!`); they are listed with the unstaged changes.
    pub fn conflicts(&self) -> usize {
        self.unstaged.iter().filter(|c| c.code == '!').count()
    }
}

/// What a refresh reuses from the previous one of the same folder (see [`snapshot`]).
#[derive(Debug, Clone, Default)]
pub struct Cache {
    repo: Option<Repo>,
    /// HEAD and refs as they were when `log` was read, and when that was.
    log_stamp: Option<(Stamp, Instant)>,
    log: Vec<Commit>,
}

/// The repository of the opened folder.
#[derive(Debug, Clone)]
struct Repo {
    root: PathBuf,
    /// `$GIT_DIR` and `$GIT_COMMON_DIR` (they differ in a linked worktree);
    /// unknown with a git too old for `--path-format`.
    dirs: Option<(PathBuf, PathBuf)>,
}

/// mtimes of HEAD and of everything a ref update rewrites.
type Stamp = Vec<(PathBuf, SystemTime)>;

/// `git log` runs again at least this often anyway: it shows relative dates.
const LOG_MAX_AGE: Duration = Duration::from_secs(60);

/// A stamp with an mtime this recent isn't trusted: two updates within the
/// filesystem's timestamp granularity would look like one.
const STAMP_SETTLE: Duration = Duration::from_secs(2);

/// Past this many refs directories, `log` just runs every time.
const MAX_STAMP_DIRS: usize = 256;

pub(crate) fn git(cwd: &Path, args: &[&str]) -> Result<String, String> {
    git_env(cwd, args, &[])
}

/// `program` (git, or `timeout` running git) for scripted use: no prompts, no
/// stdin, no optional locks (a refresh never blocks the user's git).
fn command(program: &str, cwd: &Path, env: &[(&str, &str)]) -> Command {
    let mut cmd = Command::new(program);
    cmd.current_dir(cwd).stdin(Stdio::null()).env("GIT_TERMINAL_PROMPT", "0").env("GIT_OPTIONAL_LOCKS", "0");
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd
}

/// stdout on success, else stderr (or which command failed when it is empty).
fn to_result(out: &Output, args: &[&str]) -> Result<String, String> {
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
    Err(if err.is_empty() { format!("git {} failed", args.join(" ")) } else { err })
}

fn git_env(cwd: &Path, args: &[&str], env: &[(&str, &str)]) -> Result<String, String> {
    let out = command("git", cwd, env).args(args).output().map_err(|e| format!("git: {e}"))?;
    to_result(&out, args)
}

pub fn repo_root(dir: &Path) -> Option<PathBuf> {
    git(dir, &["rev-parse", "--show-toplevel"]).ok().map(|s| PathBuf::from(s.trim()))
}

/// The git state of folder `dir`. `cache` carries what the previous refresh
/// of the same folder learned: the repository (not looked up again) and the
/// log (read again only when HEAD or a ref moved, or after [`LOG_MAX_AGE`]).
/// Ignored files are listed only `with_ignored` (they are shown in Files only).
pub fn snapshot(dir: &Path, cache: &mut Cache, with_ignored: bool) -> Result<Snapshot, String> {
    let repo = match cache.repo.take() {
        // A repository created inside the opened folder (`git init` by an
        // agent) takes over from the outer one: look it up again.
        Some(repo) if repo.root == dir || !dir.join(".git").exists() => repo,
        _ => match probe(dir) {
            Some(repo) => repo,
            None => {
                *cache = Cache::default();
                return Ok(Snapshot::default());
            }
        },
    };
    let mut snap = Snapshot { root: Some(repo.root.clone()), ..Default::default() };

    // Scoped to the opened folder: opening a subfolder of a large repo stays fast.
    let status = match git(dir, &["status", "--porcelain=v1", "-z", "-uall", "--branch", "--", "."]) {
        Ok(status) => status,
        Err(e) => {
            // The repository may be gone (or moved): find it again next time.
            *cache = Cache::default();
            return if probe(dir).is_some() { Err(e) } else { Ok(Snapshot::default()) };
        }
    };
    parse_status(&status, &mut snap);

    // Taken before `log` runs, so a commit made meanwhile shows next time.
    let stamp = repo.dirs.as_ref().and_then(|(git_dir, common)| refs_stamp(git_dir, common));
    let fresh = match (&stamp, &cache.log_stamp) {
        (Some(now), Some((then, at))) => now == then && at.elapsed() < LOG_MAX_AGE,
        _ => false,
    };
    if fresh {
        snap.log = cache.log.clone();
    } else {
        if let Ok(log) = git(&repo.root, &["log", "-n", "40", "--pretty=format:%h%x1f%s%x1f%an%x1f%ar%x1f%D"]) {
            snap.log = parse_log(&log);
        }
        cache.log = snap.log.clone();
        cache.log_stamp = stamp.map(|s| (s, Instant::now()));
    }
    cache.repo = Some(repo);

    if with_ignored
        && let Ok(ignored) = git(
            dir,
            &["ls-files", "--others", "--ignored", "--exclude-standard", "--directory", "--full-name", "-z", "--", "."],
        )
    {
        snap.ignored = ignored.split('\0').filter(|s| !s.is_empty()).map(String::from).take(MAX_CHANGES).collect();
    }

    Ok(snap)
}

/// The repository containing `dir`, if any.
fn probe(dir: &Path) -> Option<Repo> {
    let args = ["rev-parse", "--path-format=absolute", "--show-toplevel", "--absolute-git-dir", "--git-common-dir"];
    if let Ok(out) = git(dir, &args) {
        let mut lines = out.lines().map(|l| PathBuf::from(l.trim()));
        if let (Some(root), Some(git_dir), Some(common)) = (lines.next(), lines.next(), lines.next()) {
            return Some(Repo { root, dirs: Some((git_dir, common)) });
        }
    }
    // An older git without `--path-format`: no stamp, so `log` always runs.
    repo_root(dir).map(|root| Repo { root, dirs: None })
}

/// mtimes of HEAD, `packed-refs`, reftable's table list and every directory
/// under `refs/`: git updates a ref by renaming a lock file into its
/// directory (or rewriting one of those files), so any ref change moves one.
/// `None` when it can't be trusted (too many directories, a very recent change).
fn refs_stamp(git_dir: &Path, common: &Path) -> Option<Stamp> {
    let mtime = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    let mut stamp = Stamp::new();
    for f in [git_dir.join("HEAD"), common.join("packed-refs"), common.join("reftable/tables.list")] {
        if let Some(t) = mtime(&f) {
            stamp.push((f, t));
        }
    }
    let mut dirs = vec![common.join("refs")];
    if git_dir != common {
        // Per-worktree refs (bisect, worktree/…).
        dirs.push(git_dir.join("refs"));
    }
    let mut seen = 0;
    while let Some(dir) = dirs.pop() {
        let Some(t) = mtime(&dir) else { continue };
        seen += 1;
        if seen > MAX_STAMP_DIRS {
            return None;
        }
        stamp.push((dir.clone(), t));
        for entry in std::fs::read_dir(&dir).ok()?.filter_map(Result::ok) {
            if entry.file_type().is_ok_and(|k| k.is_dir()) {
                dirs.push(entry.path());
            }
        }
    }
    let now = SystemTime::now();
    if stamp.iter().any(|(_, t)| now.duration_since(*t).map_or(true, |age| age < STAMP_SETTLE)) {
        return None;
    }
    stamp.sort();
    Some(stamp)
}

fn parse_log(log: &str) -> Vec<Commit> {
    log.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\x1f').collect();
            (f.len() == 5).then(|| Commit {
                hash: f[0].into(),
                subject: f[1].into(),
                author: f[2].into(),
                when: f[3].into(),
                refs: f[4].replace("HEAD -> ", ""),
                detail: format!("{}  {} · {}", f[0], f[2], f[3]),
            })
        })
        .collect()
}

/// Fill `snap` from `git status --porcelain=v1 -z --branch` output.
fn parse_status(status: &str, snap: &mut Snapshot) {
    let mut parts = status.split('\0').filter(|s| !s.is_empty());
    while let Some(entry) = parts.next() {
        if let Some(branch) = entry.strip_prefix("## ") {
            parse_branch(branch, snap);
            continue;
        }
        let (Some(&[x, y]), Some(path)) = (entry.as_bytes().get(..2), entry.get(3..)) else { continue };
        let (x, y, path) = (x as char, y as char, path.to_string());
        if path.is_empty() {
            continue;
        }
        // A rename or copy, staged (X) or in the worktree (Y, e.g. after
        // `git add -N`), is followed by its original path.
        let orig = if matches!(x, 'R' | 'C') || matches!(y, 'R' | 'C') { parts.next().map(String::from) } else { None };
        let conflict = x == 'U' || y == 'U' || (x == 'A' && y == 'A') || (x == 'D' && y == 'D');
        if conflict {
            snap.unstaged.push(Change::new(path, '!'));
            continue;
        }
        if x == '?' {
            snap.unstaged.push(Change::new(path, 'U'));
            continue;
        }
        if x != ' ' {
            let orig = orig.clone().filter(|_| matches!(x, 'R' | 'C'));
            snap.staged.push(Change { path: path.clone(), code: x, orig });
        }
        if y != ' ' {
            let orig = orig.filter(|_| matches!(y, 'R' | 'C'));
            snap.unstaged.push(Change { path, code: y, orig });
        }
    }
    for list in [&mut snap.staged, &mut snap.unstaged] {
        if list.len() > MAX_CHANGES {
            list.truncate(MAX_CHANGES);
            snap.truncated = true;
        }
    }
}

fn parse_branch(line: &str, snap: &mut Snapshot) {
    // Examples: "main...origin/main [ahead 1, behind 2]", "No commits yet on main", "HEAD (no branch)"
    let line = line.strip_prefix("No commits yet on ").unwrap_or(line);
    let (name, rest) = match line.find("...") {
        Some(i) => (&line[..i], &line[i..]),
        None => (line.split(' ').next().unwrap_or(line), ""),
    };
    snap.branch = name.to_string();
    snap.upstream = rest.strip_prefix("...").and_then(|s| s.split_whitespace().next()).map(String::from);
    if let (Some(s), Some(e)) = (rest.find('['), rest.find(']')) {
        for item in rest[s + 1..e].split(", ") {
            if let Some(n) = item.strip_prefix("ahead ") {
                snap.ahead = n.parse().unwrap_or(0);
            } else if let Some(n) = item.strip_prefix("behind ") {
                snap.behind = n.parse().unwrap_or(0);
            }
        }
    }
}

/// Content of `path` at `rev` (`"HEAD"` or `""` for the index). Empty if absent.
pub fn show(root: &Path, rev: &str, path: &str) -> String {
    git(root, &["show", &format!("{rev}:{path}")]).unwrap_or_default()
}

/// git converts `path`'s line endings to CRLF on checkout (and back on add):
/// `core.autocrlf = true`, an `eol=crlf` attribute, or `text` with `core.eol = crlf`.
pub fn checks_out_crlf(root: &Path, path: &str) -> bool {
    let config = |key: &str| git(root, &["config", "--get", key]).map(|v| v.trim().to_ascii_lowercase()).ok();
    // Paths, not pathspecs: check-attr takes them literally.
    let attrs = git(root, &["check-attr", "text", "eol", "--", path]).unwrap_or_default();
    let attr = |name: &str| {
        attrs
            .lines()
            .find_map(|l| l.rsplit_once(": ").filter(|(head, _)| head.ends_with(&format!(": {name}"))))
            .map(|(_, v)| v.to_string())
    };
    let text = attr("text");
    if text.as_deref() == Some("unset") {
        return false;
    }
    if attr("eol").as_deref() == Some("crlf") {
        return true;
    }
    let text_set = matches!(text.as_deref(), Some("set" | "auto"));
    config("core.autocrlf").as_deref() == Some("true") || (text_set && config("core.eol").as_deref() == Some("crlf"))
}

/// Size in bytes of `path` at `rev` (as in [`show`]); `None` if absent.
pub fn blob_size(root: &Path, rev: &str, path: &str) -> Option<u64> {
    git(root, &["cat-file", "-s", &format!("{rev}:{path}")]).ok()?.trim().parse().ok()
}

/// Staged paths outside `prefix` (the opened folder; `""` is the whole
/// repository, so nothing is outside). `root` is the repository root.
pub fn staged_outside(root: &Path, prefix: &str) -> Result<Vec<String>, String> {
    if prefix.is_empty() {
        return Ok(Vec::new());
    }
    // No renames: a file moved in from outside also deletes something outside.
    let out = git(root, &["diff", "--cached", "--name-only", "--no-renames", "-z"])?;
    Ok(out.split('\0').filter(|p| !p.is_empty() && !p.starts_with(prefix)).map(String::from).collect())
}

/// `path` as a pathspec git matches exactly: without `:(literal)` it is also a
/// glob, so `app/[id]/page.tsx` would match `app/i/page.tsx` too (and `*`, `?`).
pub fn literal(path: &str) -> String {
    format!(":(literal){path}")
}

/// `git <args> -- <paths, literally>`.
fn git_paths<S: AsRef<str>>(root: &Path, args: &[&str], paths: &[S]) -> Result<String, String> {
    let specs: Vec<String> = paths.iter().map(|p| literal(p.as_ref())).collect();
    let mut all: Vec<&str> = args.to_vec();
    all.push("--");
    all.extend(specs.iter().map(String::as_str));
    git(root, &all)
}

pub fn stage(root: &Path, paths: &[String]) -> Result<String, String> {
    git_paths(root, &["add", "-A"], paths)
}

/// Discard unstaged changes, like VSCodium's Discard: tracked files go back to
/// the index (`git restore --worktree`), untracked files (`U`) are deleted.
/// Paths are relative to the repository root, with their status letter.
pub fn discard(root: &Path, files: &[(String, char)]) -> Result<String, String> {
    // Checked before touching anything: an untracked folder is a nested repository (git
    // lists it as `sub/`), and deleting it here would take its history with it.
    if let Some(path) = untracked_folders(root, files).first() {
        return Err(format!(
            "{path} is a folder (a nested repository?): nothing was discarded; remove it in a terminal"
        ));
    }
    // `A` in the worktree column is an intent-to-add entry (`git add -N`): restoring it
    // would empty the file. It goes like an untracked file, out of the index too.
    let tracked: Vec<&str> =
        files.iter().filter(|(_, code)| !matches!(code, 'U' | 'A')).map(|(p, _)| p.as_str()).collect();
    if !tracked.is_empty() {
        git_paths(root, &["restore", "--worktree"], &tracked)?;
    }
    let added: Vec<&str> = files.iter().filter(|(_, code)| *code == 'A').map(|(p, _)| p.as_str()).collect();
    if !added.is_empty() {
        git_paths(root, &["rm", "--cached", "-q", "--ignore-unmatch"], &added)?;
    }
    // Every untracked file is tried; the failures are reported together.
    let failed: Vec<String> = files
        .iter()
        .filter(|(_, code)| matches!(code, 'U' | 'A'))
        .filter_map(|(path, _)| match std::fs::remove_file(root.join(path)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Some(format!("{path}: {e}")),
            _ => None,
        })
        .collect();
    if !failed.is_empty() {
        return Err(failed.join("; "));
    }
    Ok(format!("Discarded {} file{}", files.len(), if files.len() == 1 { "" } else { "s" }))
}

/// The untracked entries of `files` that are folders (git lists a nested
/// repository as `sub/`). A symlink to a folder is a file here: removing it
/// leaves the folder alone.
pub fn untracked_folders(root: &Path, files: &[(String, char)]) -> Vec<String> {
    files
        .iter()
        .filter(|(p, c)| *c == 'U' && (p.ends_with('/') || root.join(p).symlink_metadata().is_ok_and(|m| m.is_dir())))
        .map(|(p, _)| p.clone())
        .collect()
}

/// git's diff of one file (`--cached`: HEAD ⇄ index), as `git apply` reads
/// it back: plain prefixes and no color, external diff or textconv, whatever
/// the user's config says. Patience, like mira's side-by-side view, so the
/// hunks line up with its rows.
pub fn file_patch(root: &Path, rel: &str, staged: bool) -> Result<String, String> {
    let mut args = vec![
        "diff",
        "--no-color",
        "--no-ext-diff",
        "--no-textconv",
        "--diff-algorithm=patience",
        "--src-prefix=a/",
        "--dst-prefix=b/",
        "-U3",
        // One hunk per change, whatever `diff.interHunkContext` says.
        "--inter-hunk-context=0",
    ];
    if staged {
        args.push("--cached");
    }
    let spec = literal(rel);
    args.extend(["--", spec.as_str()]);
    git(root, &args)
}

/// `git apply <flags>` with `patch` on stdin (`--cached`: to the index; `-R`:
/// in reverse). git checks that it applies cleanly before touching anything.
pub fn apply(root: &Path, patch: &str, flags: &[&str]) -> Result<String, String> {
    use std::io::Write;
    let mut args = vec!["apply", "--whitespace=nowarn"];
    args.extend_from_slice(flags);
    let mut child = command("git", root, &[])
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("git: {e}"))?;
    // Fed from a thread: git may answer before it read everything.
    let mut stdin = child.stdin.take().ok_or("git apply: no stdin")?;
    let input = patch.to_owned();
    let feed = std::thread::spawn(move || stdin.write_all(input.as_bytes()));
    let out = child.wait_with_output().map_err(|e| format!("git: {e}"))?;
    let _ = feed.join();
    to_result(&out, &args)
}

pub fn unstage(root: &Path, paths: &[String]) -> Result<String, String> {
    git_paths(root, &["restore", "--staged"], paths).or_else(|_| {
        // Repository without commits: `restore --staged` has no HEAD to restore from.
        git_paths(root, &["rm", "--cached", "-r", "-q"], paths)
    })
}

pub fn commit(root: &Path, message: &str) -> Result<String, String> {
    git(root, &["commit", "-m", message])
}

/// Make `root` a git repository.
pub fn init(root: &Path) -> Result<String, String> {
    git(root, &["init", "-q"]).map(|_| "Initialized a git repository".into())
}

/// The line of a git error that says what went wrong (git prints progress
/// and remote chatter first), plus the next step when it is a known case.
pub fn error_summary(stderr: &str) -> String {
    let lines: Vec<&str> = stderr.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let pick = ["error:", "fatal:", "! [", "hint:"]
        .iter()
        .find_map(|p| lines.iter().find(|l| l.starts_with(p)))
        .or(lines.first())
        .copied()
        .unwrap_or("git failed");
    let next = if stderr.contains("[rejected]") || stderr.contains("fetch first") || stderr.contains("non-fast-forward")
    {
        Some("Pull first")
    } else if stderr.contains("Not possible to fast-forward") || stderr.contains("divergent branches") {
        Some("the branches diverged: merge or rebase in the terminal")
    } else if stderr.contains("Please tell me who you are") {
        Some("set user.name and user.email in the terminal")
    } else {
        None
    };
    match next {
        Some(n) => format!("{pick} — {n}"),
        None => pick.to_string(),
    }
}

// ------------------------------------------------------------------ snapshots
//
// A snapshot is a git *tree* of the whole working directory (tracked +
// untracked, honoring .gitignore), written through a throwaway index so the
// user's index, branches and history are never touched. Trees are cheap and
// content-addressed: unchanged files reuse existing blobs.

/// Longest a snapshot step may run. A huge working tree (e.g. a whole home
/// directory under git) would otherwise keep `git add -A` busy for minutes.
pub const SNAPSHOT_TIMEOUT_SECS: u32 = 20;

/// Message prefix of a snapshot that hit [`SNAPSHOT_TIMEOUT_SECS`].
pub const SNAPSHOT_TIMED_OUT: &str = "snapshot timed out";

/// Why [`git_timed`] failed.
#[derive(Debug, PartialEq)]
enum TimedError {
    TimedOut,
    Failed(String),
}

/// `git` under coreutils `timeout`, so the limit holds even if mira exits
/// while git runs (a plain child would be orphaned and keep going).
fn git_timed(cwd: &Path, args: &[&str], env: &[(&str, &str)], secs: u32) -> Result<String, TimedError> {
    let limit = secs.to_string();
    let mut full = vec!["-k", "2", limit.as_str(), "git"];
    full.extend_from_slice(args);
    let out = match command("timeout", cwd, env).args(&full).output() {
        Ok(out) => out,
        // No coreutils `timeout`: run git directly.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return git_env(cwd, args, env).map_err(TimedError::Failed);
        }
        Err(e) => return Err(TimedError::Failed(format!("git: {e}"))),
    };
    match out.status.code() {
        // 124: timed out; 137: killed after the grace period.
        Some(124 | 137) | None => Err(TimedError::TimedOut),
        _ => to_result(&out, args).map_err(TimedError::Failed),
    }
}

/// A snapshot step under [`SNAPSHOT_TIMEOUT_SECS`].
fn git_snapshot(cwd: &Path, args: &[&str], env: &[(&str, &str)]) -> Result<String, String> {
    git_timed(cwd, args, env, SNAPSHOT_TIMEOUT_SECS).map_err(|e| match e {
        TimedError::TimedOut => {
            format!("{SNAPSHOT_TIMED_OUT} after {SNAPSHOT_TIMEOUT_SECS} s (working tree too large)")
        }
        TimedError::Failed(e) => e,
    })
}

/// Snapshot the working tree of `root` and return the tree id. The throwaway
/// index lives in mira's state directory (private to the user), not in /tmp.
pub fn snapshot_tree(root: &Path) -> Result<String, String> {
    snapshot_tree_in(root, &crate::persist::private_dir("index")?)
}

fn snapshot_tree_in(root: &Path, scratch: &Path) -> Result<String, String> {
    static N: AtomicU64 = AtomicU64::new(0);
    let git_dir = git(root, &["rev-parse", "--absolute-git-dir"])?;
    let index = scratch.join(format!("index-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    // Seeding from the real index lets git reuse cached stat info (fast).
    // Without one (a new repository) git starts from an empty index.
    let _ = std::fs::copy(PathBuf::from(git_dir.trim()).join("index"), &index);
    let index_str = index.to_string_lossy().into_owned();
    let env = [("GIT_INDEX_FILE", index_str.as_str())];
    let result = git_snapshot(root, &["add", "-A", "--", "."], &env)
        .and_then(|_| git_snapshot(root, &["write-tree"], &env))
        .map(|s| s.trim().to_string());
    // Missing files are fine; the lock is left behind when `timeout` kills git.
    let _ = std::fs::remove_file(&index);
    let _ = std::fs::remove_file(format!("{index_str}.lock"));
    result
}

/// Files that differ between two trees, as `Change`s (A/M/D).
pub fn diff_trees(root: &Path, from: &str, to: &str) -> Result<Vec<Change>, String> {
    if from == to {
        return Ok(Vec::new());
    }
    let out = git(root, &["diff-tree", "-r", "-z", "--no-renames", "--name-status", from, to])?;
    let mut parts = out.split('\0').filter(|s| !s.is_empty());
    let mut changes = Vec::new();
    while let (Some(status), Some(path)) = (parts.next(), parts.next()) {
        let code = status.chars().next().unwrap_or('M');
        changes.push(Change::new(path, code));
    }
    Ok(changes)
}

/// Restore `paths` in the working tree to their state in `tree`; paths that do
/// not exist in `tree` are deleted (they were added afterwards). Nothing is
/// touched when `tree` itself is missing (e.g. pruned by `git gc`): every path
/// would otherwise look added afterwards and be deleted.
pub fn restore_from_tree(root: &Path, tree: &str, paths: &[String]) -> Result<String, String> {
    if git(root, &["cat-file", "-e", &format!("{tree}^{{tree}}")]).is_err() {
        return Err(format!("the snapshot {tree} is no longer in the repository: nothing was reverted"));
    }
    // Every path is tried: stopping at the first failure would leave the rest unrestored.
    let mut failed = Vec::new();
    for path in paths {
        let result = if git(root, &["cat-file", "-e", &format!("{tree}:{path}")]).is_ok() {
            git_paths(root, &["restore", &format!("--source={tree}"), "--worktree"], &[path]).map(drop)
        } else {
            let file = root.join(path);
            if file.exists() { std::fs::remove_file(&file).map_err(|e| format!("{path}: {e}")) } else { Ok(()) }
        };
        if let Err(e) = result {
            failed.push(e);
        }
    }
    if !failed.is_empty() {
        return Err(failed.join("; "));
    }
    Ok(format!("Reverted {} file(s)", paths.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_summary_picks_the_reason_and_the_next_step() {
        let rejected = "To github.com:o/r.git\n ! [rejected]        main -> main (fetch first)\n\
                        error: failed to push some refs to 'github.com:o/r.git'\nhint: Updates were rejected";
        assert_eq!(error_summary(rejected), "error: failed to push some refs to 'github.com:o/r.git' — Pull first");
        let hook = "running checks…\nlint failed\nfatal: cannot do that";
        assert_eq!(error_summary(hook), "fatal: cannot do that");
        assert_eq!(error_summary("\n  just one line \n"), "just one line");
        assert_eq!(error_summary(""), "git failed");
    }

    #[test]
    fn parses_branch_line() {
        let mut s = Snapshot::default();
        parse_branch("main...origin/main [ahead 2, behind 1]", &mut s);
        assert_eq!((s.branch.as_str(), s.ahead, s.behind), ("main", 2, 1));
        assert_eq!(s.upstream.as_deref(), Some("origin/main"));

        let mut s = Snapshot::default();
        parse_branch("No commits yet on dev", &mut s);
        assert_eq!(s.branch, "dev");
        assert!(s.upstream.is_none());
    }

    #[test]
    fn parses_porcelain_status() {
        let mut s = Snapshot::default();
        let status = "## main...origin/main [ahead 1]\0R  new.txt\0old.txt\0UU both.txt\0AA added.txt\0\
                      ?? dir/u.txt\0MM m.txt\0 D gone.txt\0A  staged.txt\0x\0";
        parse_status(status, &mut s);
        assert_eq!((s.branch.as_str(), s.ahead), ("main", 1));
        let list = |l: &[Change]| l.iter().map(|c| (c.code, c.path.clone())).collect::<Vec<_>>();
        assert_eq!(list(&s.staged), [('R', "new.txt".into()), ('M', "m.txt".into()), ('A', "staged.txt".into())]);
        assert_eq!(
            list(&s.unstaged),
            [
                ('!', "both.txt".into()),
                ('!', "added.txt".into()),
                ('U', "dir/u.txt".into()),
                ('M', "m.txt".into()),
                ('D', "gone.txt".into())
            ]
        );
        assert!(!s.truncated);
        assert_eq!(s.staged[0].orig.as_deref(), Some("old.txt"), "a rename keeps where it came from");
    }

    #[test]
    fn worktree_renames_consume_their_original_path() {
        // `git add -N new.txt; mv old.txt new.txt` reports ` R new.txt\0old.txt\0`.
        let mut s = Snapshot::default();
        parse_status(" R new.txt\0old.txt\0 M other.txt\0", &mut s);
        let list = s.unstaged.iter().map(|c| (c.code, c.path.as_str(), c.orig.as_deref())).collect::<Vec<_>>();
        assert_eq!(list, [('R', "new.txt", Some("old.txt")), ('M', "other.txt", None)]);
        assert!(s.staged.is_empty());
    }
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;
    use crate::files::test_dir;

    fn repo() -> PathBuf {
        let dir = test_dir("git");
        git(&dir, &["init", "-q"]).unwrap();
        dir
    }

    #[test]
    fn snapshot_reuses_the_log_until_a_ref_moves() {
        let dir = repo();
        let commit = |msg: &str| {
            std::fs::write(dir.join("f"), msg).unwrap();
            git(&dir, &["add", "f"]).unwrap();
            let id = ["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"];
            git(&dir, &[&id[..], &["commit", "-qm", msg]].concat()).unwrap();
        };
        // Back-date HEAD and the refs tree, as if the last ref change were long ago.
        let age = || {
            let old = SystemTime::now() - Duration::from_secs(3600);
            let git_dir = dir.join(".git");
            let mut paths = vec![git_dir.join("HEAD")];
            let mut dirs = vec![git_dir.join("refs")];
            while let Some(d) = dirs.pop() {
                for e in std::fs::read_dir(&d).unwrap().filter_map(Result::ok) {
                    if e.file_type().unwrap().is_dir() {
                        dirs.push(e.path());
                    }
                }
                paths.push(d);
            }
            for p in paths {
                std::fs::File::open(p).unwrap().set_modified(old).unwrap();
            }
        };
        commit("one");
        age();
        let mut cache = Cache::default();
        assert_eq!(snapshot(&dir, &mut cache, false).unwrap().log[0].subject, "one");
        assert!(cache.log_stamp.is_some(), "settled refs are trusted");
        // Nothing moved: the cached log is reused (marked to tell it apart).
        cache.log[0].subject = "cached".into();
        let s = snapshot(&dir, &mut cache, false).unwrap();
        assert_eq!((s.log[0].subject.as_str(), s.ignored.len()), ("cached", 0));
        // A commit moves a ref: the log is read again.
        commit("two");
        assert_eq!(snapshot(&dir, &mut cache, true).unwrap().log[0].subject, "two");
        // A branch in a new refs subdirectory changes the stamp too.
        age();
        let (git_dir, common) = probe(&dir).and_then(|r| r.dirs).unwrap();
        let before = refs_stamp(&git_dir, &common).unwrap();
        git(&dir, &["branch", "feature/x"]).unwrap();
        age();
        assert_ne!(refs_stamp(&git_dir, &common).unwrap(), before);
        // A linked worktree has its own git dir and shares the common one.
        let wt = test_dir("git-wt");
        let _ = std::fs::remove_dir(&wt);
        git(&dir, &["worktree", "add", "-q", &wt.to_string_lossy(), "-b", "wt"]).unwrap();
        let (wt_git, wt_common) = probe(&wt).and_then(|r| r.dirs).unwrap();
        assert_ne!(wt_git, wt_common);
        assert_eq!(wt_common.canonicalize().unwrap(), common.canonicalize().unwrap());
        // The repository goes away: the cache resets and the folder reads as plain.
        let _ = std::fs::remove_dir_all(&wt);
        std::fs::remove_dir_all(dir.join(".git")).unwrap();
        assert_eq!(snapshot(&dir, &mut cache, false), Ok(Snapshot::default()));
        assert!(cache.repo.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn timed_git_reports_a_timeout() {
        let dir = std::env::temp_dir();
        // `git -c alias.zz='!sleep 5' zz` runs a shell command through git.
        assert_eq!(git_timed(&dir, &["-c", "alias.zz=!sleep 5", "zz"], &[], 1), Err(TimedError::TimedOut));
        assert_eq!(git_timed(&dir, &["--version"], &[], 5).map(|v| v.starts_with("git version")), Ok(true));
    }

    #[test]
    fn snapshot_diff_and_restore_roundtrip() {
        let dir = repo();
        let scratch = test_dir("git-index");
        std::fs::write(dir.join("a.txt"), "one\n").unwrap();
        let t1 = snapshot_tree_in(&dir, &scratch).unwrap();

        std::fs::write(dir.join("a.txt"), "two\n").unwrap();
        std::fs::write(dir.join("b.txt"), "new\n").unwrap();
        let t2 = snapshot_tree_in(&dir, &scratch).unwrap();
        assert_eq!(std::fs::read_dir(&scratch).unwrap().count(), 0, "temp index and lock removed");

        let changes = diff_trees(&dir, &t1, &t2).unwrap();
        let summary: Vec<(char, &str)> = changes.iter().map(|c| (c.code, c.path.as_str())).collect();
        assert_eq!(summary, vec![('M', "a.txt"), ('A', "b.txt")]);
        // The user's index is untouched.
        assert!(git(&dir, &["ls-files"]).unwrap().is_empty());

        restore_from_tree(&dir, &t1, &["a.txt".into(), "b.txt".into()]).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "one\n");
        assert!(!dir.join("b.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn discard_and_undo_with_the_snapshot() {
        let dir = repo();
        let scratch = test_dir("git-index");
        let id = ["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"];
        std::fs::write(dir.join("kept.txt"), "base\n").unwrap();
        std::fs::write(dir.join("gone.txt"), "base\n").unwrap();
        git(&dir, &["add", "-A"]).unwrap();
        git(&dir, &[&id[..], &["commit", "-qm", "base"]].concat()).unwrap();
        std::fs::write(dir.join("kept.txt"), "edited\n").unwrap();
        std::fs::remove_file(dir.join("gone.txt")).unwrap();
        std::fs::write(dir.join("new.txt"), "scratch\n").unwrap();
        let files = vec![('M', "kept.txt"), ('D', "gone.txt"), ('U', "new.txt")]
            .into_iter()
            .map(|(c, p)| (p.to_string(), c))
            .collect::<Vec<_>>();

        // What mira does: back up, then discard.
        let backup = snapshot_tree_in(&dir, &scratch).unwrap();
        assert_eq!(discard(&dir, &files), Ok("Discarded 3 files".into()));
        assert_eq!(std::fs::read_to_string(dir.join("kept.txt")).unwrap(), "base\n");
        assert_eq!(std::fs::read_to_string(dir.join("gone.txt")).unwrap(), "base\n");
        assert!(!dir.join("new.txt").exists());
        assert!(git(&dir, &["status", "--porcelain"]).unwrap().is_empty());

        // Undo: every file is back as it was, deletion included.
        let paths: Vec<String> = files.iter().map(|(p, _)| p.clone()).collect();
        restore_from_tree(&dir, &backup, &paths).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("kept.txt")).unwrap(), "edited\n");
        assert!(!dir.join("gone.txt").exists());
        assert_eq!(std::fs::read_to_string(dir.join("new.txt")).unwrap(), "scratch\n");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn stages_unstages_and_discards_single_hunks() {
        let dir = repo();
        let id = ["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"];
        let base: String = (1..=20).map(|i| format!("line {i}\n")).collect();
        std::fs::write(dir.join("f.txt"), &base).unwrap();
        git(&dir, &["add", "-A"]).unwrap();
        git(&dir, &[&id[..], &["commit", "-qm", "base"]].concat()).unwrap();
        // Two changes far apart: two hunks.
        let edited = base.replace("line 2\n", "line two\n").replace("line 18\n", "line 18\nline 18.5\n");
        std::fs::write(dir.join("f.txt"), &edited).unwrap();
        let patch = |staged| crate::hunks::parse(&file_patch(&dir, "f.txt", staged).unwrap(), staged);
        let unstaged = patch(false).unwrap();
        assert_eq!(unstaged.hunks.len(), 2);
        // Each hunk covers a changed row of mira's own diff (its toolbar goes there).
        let rows = crate::diff::compute(&base, &edited, Some(3));
        for h in &unstaged.hunks {
            let found = rows.items.iter().any(|item| match item {
                crate::diff::Item::Row { left, right } => h.covers(
                    left.number.filter(|_| left.kind == crate::diff::Kind::Delete),
                    right.number.filter(|_| right.kind == crate::diff::Kind::Insert),
                ),
                _ => false,
            });
            assert!(found, "{h:?}");
        }

        // Stage the first one only.
        apply(&dir, &unstaged.single(0).unwrap(), &["--cached"]).unwrap();
        let staged = patch(true).unwrap();
        assert_eq!(staged.hunks.len(), 1);
        assert_eq!(show(&dir, "", "f.txt"), base.replace("line 2\n", "line two\n"));
        assert_eq!(patch(false).unwrap().hunks.len(), 1, "the other one is still unstaged");

        // Unstage it again: the index is back to HEAD, the file untouched.
        apply(&dir, &staged.single(0).unwrap(), &["--cached", "-R"]).unwrap();
        assert!(patch(true).is_none());
        assert_eq!(std::fs::read_to_string(dir.join("f.txt")).unwrap(), edited);

        // Discard the second one in the working tree.
        let unstaged = patch(false).unwrap();
        apply(&dir, &unstaged.single(1).unwrap(), &["-R"]).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("f.txt")).unwrap(), base.replace("line 2\n", "line two\n"));
        // A stale patch (already applied) fails and changes nothing.
        assert!(apply(&dir, &unstaged.single(1).unwrap(), &["-R"]).is_err());
        assert_eq!(patch(false).unwrap().hunks.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn discard_refuses_nested_repositories_and_reports_every_failure() {
        let dir = repo();
        std::fs::create_dir_all(dir.join("sub/.git")).unwrap();
        std::fs::write(dir.join("loose.txt"), "x").unwrap();
        let err = discard(&dir, &[("loose.txt".into(), 'U'), ("sub/".into(), 'U')]).unwrap_err();
        assert!(err.contains("nested repository"), "{err}");
        assert!(dir.join("loose.txt").exists() && dir.join("sub/.git").exists(), "nothing was touched");
        // A file that can't be removed doesn't stop the others.
        std::fs::create_dir_all(dir.join("ro")).unwrap();
        std::fs::write(dir.join("ro/f.txt"), "x").unwrap();
        let mut perms = std::fs::metadata(dir.join("ro")).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o555);
        std::fs::set_permissions(dir.join("ro"), perms.clone()).unwrap();
        let err = discard(&dir, &[("ro/f.txt".into(), 'U'), ("loose.txt".into(), 'U')]);
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
        std::fs::set_permissions(dir.join("ro"), perms).unwrap();
        // (As root a read-only folder doesn't stop the removal.)
        if let Err(e) = err {
            assert!(e.contains("ro/f.txt"), "{e}");
        }
        assert!(!dir.join("loose.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn discard_removes_links_to_folders_and_intent_to_add_renames() {
        let dir = repo();
        let id = ["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"];
        std::fs::create_dir_all(dir.join("shared")).unwrap();
        std::fs::write(dir.join("shared/keep.txt"), "x").unwrap();
        std::fs::write(dir.join("old.txt"), "some text long enough to be a rename\n").unwrap();
        git(&dir, &["add", "-A"]).unwrap();
        git(&dir, &[&id[..], &["commit", "-qm", "base"]].concat()).unwrap();
        // A link to a folder is a file to git (`?? lib`): it goes, the folder stays.
        std::os::unix::fs::symlink("shared", dir.join("lib")).unwrap();
        assert!(untracked_folders(&dir, &[("lib".into(), 'U')]).is_empty());
        discard(&dir, &[("lib".into(), 'U')]).unwrap();
        assert!(dir.join("lib").symlink_metadata().is_err() && dir.join("shared/keep.txt").exists());
        // ` R new.txt <- old.txt`: the new name goes, the old file comes back whole.
        std::fs::rename(dir.join("old.txt"), dir.join("new.txt")).unwrap();
        git(&dir, &["add", "-N", "new.txt"]).unwrap();
        discard(&dir, &[("new.txt".into(), 'A'), ("old.txt".into(), 'D')]).unwrap();
        assert!(!dir.join("new.txt").exists());
        assert_eq!(std::fs::read_to_string(dir.join("old.txt")).unwrap(), "some text long enough to be a rename\n");
        assert!(git(&dir, &["status", "--porcelain"]).unwrap().trim().is_empty(), "clean again");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn paths_with_glob_characters_only_touch_themselves() {
        let dir = repo();
        let id = ["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"];
        for d in ["app/[id]", "app/i", "app/d"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
            std::fs::write(dir.join(d).join("page.tsx"), "base\n").unwrap();
        }
        git(&dir, &["add", "-A"]).unwrap();
        git(&dir, &[&id[..], &["commit", "-qm", "base"]].concat()).unwrap();
        for d in ["app/[id]", "app/i", "app/d"] {
            std::fs::write(dir.join(d).join("page.tsx"), "edited\n").unwrap();
        }
        let bracket = "app/[id]/page.tsx".to_string();
        let read = |d: &str| std::fs::read_to_string(dir.join(d).join("page.tsx")).unwrap();
        // Stage only the bracketed one: `[id]` as a glob would also match `i` and `d`.
        stage(&dir, std::slice::from_ref(&bracket)).unwrap();
        let staged = git(&dir, &["diff", "--cached", "--name-only"]).unwrap();
        assert_eq!(staged.trim(), "app/[id]/page.tsx");
        unstage(&dir, std::slice::from_ref(&bracket)).unwrap();
        assert!(git(&dir, &["diff", "--cached", "--name-only"]).unwrap().trim().is_empty());
        assert_eq!(
            crate::hunks::parse(&file_patch(&dir, &bracket, false).unwrap(), false).map(|p| p.hunks.len()),
            Some(1)
        );
        discard(&dir, &[(bracket.clone(), 'M')]).unwrap();
        assert_eq!(
            (read("app/[id]").as_str(), read("app/i").as_str(), read("app/d").as_str()),
            ("base\n", "edited\n", "edited\n")
        );
        let backup = snapshot_tree_in(&dir, &test_dir("git-index")).unwrap();
        std::fs::write(dir.join("app/[id]/page.tsx"), "again\n").unwrap();
        restore_from_tree(&dir, &backup, std::slice::from_ref(&bracket)).unwrap();
        assert_eq!(read("app/[id]"), "base\n");
        assert_eq!(read("app/i"), "edited\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_staged_rename_unstages_with_its_original_path() {
        let dir = repo();
        let id = ["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"];
        std::fs::write(dir.join("old.txt"), "some text long enough to be a rename\n").unwrap();
        git(&dir, &["add", "-A"]).unwrap();
        git(&dir, &[&id[..], &["commit", "-qm", "base"]].concat()).unwrap();
        git(&dir, &["mv", "old.txt", "new.txt"]).unwrap();
        let s = snapshot(&dir, &mut Cache::default(), false).unwrap();
        let rename = &s.staged[0];
        assert_eq!((rename.code, rename.path.as_str(), rename.orig.as_deref()), ('R', "new.txt", Some("old.txt")));
        // Both paths: only the new one would leave `D old.txt` staged.
        unstage(&dir, &["new.txt".to_string(), "old.txt".to_string()]).unwrap();
        assert!(git(&dir, &["diff", "--cached", "--name-only"]).unwrap().trim().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn restore_goes_on_past_a_failing_path() {
        let dir = repo();
        let scratch = test_dir("git-index");
        std::fs::write(dir.join("a.txt"), "one\n").unwrap();
        std::fs::write(dir.join("b.txt"), "one\n").unwrap();
        let before = snapshot_tree_in(&dir, &scratch).unwrap();
        std::fs::write(dir.join("a.txt"), "two\n").unwrap();
        std::fs::write(dir.join("b.txt"), "two\n").unwrap();
        // `new` was added later and is now a folder: removing it as a file fails.
        std::fs::create_dir_all(dir.join("new")).unwrap();
        let paths: Vec<String> = ["a.txt", "new", "b.txt"].map(String::from).to_vec();
        let err = restore_from_tree(&dir, &before, &paths).unwrap_err();
        assert!(err.contains("new"), "{err}");
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "one\n");
        assert_eq!(std::fs::read_to_string(dir.join("b.txt")).unwrap(), "one\n", "the path after the failure too");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn restore_from_a_missing_tree_deletes_nothing() {
        let dir = repo();
        std::fs::write(dir.join("a.txt"), "keep\n").unwrap();
        let gone = "0123456789abcdef0123456789abcdef01234567";
        let err = restore_from_tree(&dir, gone, &["a.txt".into()]).unwrap_err();
        assert!(err.contains("nothing was reverted"), "{err}");
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "keep\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn finds_staged_files_outside_the_folder() {
        let dir = repo();
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/in.txt"), "in\n").unwrap();
        std::fs::write(dir.join("out.txt"), "out\n").unwrap();
        stage(&dir, &["sub/in.txt".into()]).unwrap();
        assert!(staged_outside(&dir, "sub/").unwrap().is_empty());
        stage(&dir, &["out.txt".into()]).unwrap();
        assert_eq!(staged_outside(&dir, "sub/").unwrap(), ["out.txt"]);
        assert!(staged_outside(&dir, "").unwrap().is_empty());
        assert_eq!(blob_size(&dir, "", "out.txt"), Some(4));
        assert_eq!(blob_size(&dir, "", "missing.txt"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
