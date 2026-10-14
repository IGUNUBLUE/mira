//! Thin wrapper over the `git` CLI. Using the CLI (instead of libgit2) keeps
//! behavior identical to what the user and their agents see in the terminal,
//! and respects hooks, config, credentials helpers and signing.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    /// Path relative to the repo root.
    pub path: String,
    /// Status letter: M, A, D, R, C, T, U (untracked) or `!` (conflict).
    pub code: char,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Commit {
    pub hash: String,
    pub subject: String,
    pub author: String,
    pub when: String,
    pub refs: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Snapshot {
    pub root: Option<PathBuf>,
    pub branch: String,
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
    /// Status letter per relative path, for explorer decorations.
    pub fn decorations(&self) -> HashMap<String, char> {
        let mut map = HashMap::new();
        for c in self.staged.iter().chain(self.unstaged.iter()) {
            map.insert(c.path.clone(), c.code);
        }
        map
    }
}

fn git(cwd: &Path, args: &[&str]) -> Result<String, String> {
    git_env(cwd, args, &[])
}

fn git_env(cwd: &Path, args: &[&str], env: &[(&str, &str)]) -> Result<String, String> {
    let mut cmd = Command::new("git");
    cmd.args(args)
        .current_dir(cwd)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().map_err(|e| format!("git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if err.is_empty() {
            format!("git {} failed", args.join(" "))
        } else {
            err
        })
    }
}

pub fn repo_root(dir: &Path) -> Option<PathBuf> {
    git(dir, &["rev-parse", "--show-toplevel"])
        .ok()
        .map(|s| PathBuf::from(s.trim()))
}

pub fn snapshot(dir: &Path) -> Result<Snapshot, String> {
    let Some(root) = repo_root(dir) else {
        return Ok(Snapshot::default());
    };
    let mut snap = Snapshot {
        root: Some(root.clone()),
        ..Default::default()
    };

    // Scoped to the opened folder: opening a subfolder of a large repo stays fast.
    let status = git(dir, &["status", "--porcelain=v1", "-z", "-uall", "--branch", "--", "."])?;
    let mut parts = status.split('\0').filter(|s| !s.is_empty());
    while let Some(entry) = parts.next() {
        if let Some(branch) = entry.strip_prefix("## ") {
            parse_branch(branch, &mut snap);
            continue;
        }
        if entry.len() < 4 {
            continue;
        }
        let (x, y) = (entry.as_bytes()[0] as char, entry.as_bytes()[1] as char);
        let path = entry[3..].to_string();
        if x == 'R' || x == 'C' {
            parts.next(); // original path of a rename/copy
        }
        let conflict = x == 'U' || y == 'U' || (x == 'A' && y == 'A') || (x == 'D' && y == 'D');
        if conflict {
            snap.unstaged.push(Change { path, code: '!' });
            continue;
        }
        if x == '?' {
            snap.unstaged.push(Change { path, code: 'U' });
            continue;
        }
        if x != ' ' {
            snap.staged.push(Change { path: path.clone(), code: x });
        }
        if y != ' ' {
            snap.unstaged.push(Change { path, code: y });
        }
    }

    for list in [&mut snap.staged, &mut snap.unstaged] {
        if list.len() > MAX_CHANGES {
            list.truncate(MAX_CHANGES);
            snap.truncated = true;
        }
    }

    if let Ok(log) = git(
        &root,
        &["log", "-n", "40", "--pretty=format:%h%x1f%s%x1f%an%x1f%ar%x1f%D"],
    ) {
        snap.log = log
            .lines()
            .filter_map(|l| {
                let f: Vec<&str> = l.split('\x1f').collect();
                (f.len() == 5).then(|| Commit {
                    hash: f[0].into(),
                    subject: f[1].into(),
                    author: f[2].into(),
                    when: f[3].into(),
                    refs: f[4].into(),
                })
            })
            .collect();
    }

    if let Ok(ignored) = git(
        dir,
        &["ls-files", "--others", "--ignored", "--exclude-standard", "--directory", "--full-name", "-z", "--", "."],
    ) {
        snap.ignored = ignored
            .split('\0')
            .filter(|s| !s.is_empty())
            .map(String::from)
            .take(MAX_CHANGES)
            .collect();
    }

    Ok(snap)
}

fn parse_branch(line: &str, snap: &mut Snapshot) {
    // Examples: "main...origin/main [ahead 1, behind 2]", "No commits yet on main", "HEAD (no branch)"
    let line = line.strip_prefix("No commits yet on ").unwrap_or(line);
    let (name, rest) = match line.find("...") {
        Some(i) => (&line[..i], &line[i..]),
        None => (line.split(' ').next().unwrap_or(line), ""),
    };
    snap.branch = name.to_string();
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

pub fn stage(root: &Path, paths: &[String]) -> Result<String, String> {
    let mut args = vec!["add", "-A", "--"];
    args.extend(paths.iter().map(String::as_str));
    git(root, &args)
}

pub fn unstage(root: &Path, paths: &[String]) -> Result<String, String> {
    let mut args = vec!["restore", "--staged", "--"];
    args.extend(paths.iter().map(String::as_str));
    git(root, &args).or_else(|_| {
        // Repository without commits: `restore --staged` has no HEAD to restore from.
        let mut args = vec!["rm", "--cached", "-r", "-q", "--"];
        args.extend(paths.iter().map(String::as_str));
        git(root, &args)
    })
}

pub fn commit(root: &Path, message: &str) -> Result<String, String> {
    git(root, &["commit", "-m", message])
}

pub fn push(root: &Path) -> Result<String, String> {
    git(root, &["push"]).or_else(|e| {
        if e.contains("no upstream") || e.contains("set-upstream") {
            git(root, &["push", "-u", "origin", "HEAD"])
        } else {
            Err(e)
        }
    })
}

pub fn pull(root: &Path) -> Result<String, String> {
    git(root, &["pull", "--ff-only"])
}

// ------------------------------------------------------------------ snapshots
//
// A snapshot is a git *tree* of the whole working directory (tracked +
// untracked, honoring .gitignore), written through a throwaway index so the
// user's index, branches and history are never touched. Trees are cheap and
// content-addressed: unchanged files reuse existing blobs.

/// Snapshot the working tree of `root` and return the tree id.
pub fn snapshot_tree(root: &Path) -> Result<String, String> {
    let git_dir = git(root, &["rev-parse", "--absolute-git-dir"])?;
    let index = std::env::temp_dir().join(format!(
        "mira-index-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    // Seeding from the real index lets git reuse cached stat info (fast).
    let _ = std::fs::copy(PathBuf::from(git_dir.trim()).join("index"), &index);
    let index_str = index.to_string_lossy().into_owned();
    let env = [("GIT_INDEX_FILE", index_str.as_str())];
    let result = git_env(root, &["add", "-A", "--", "."], &env)
        .and_then(|_| git_env(root, &["write-tree"], &env))
        .map(|s| s.trim().to_string());
    let _ = std::fs::remove_file(&index);
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
        changes.push(Change { path: path.to_string(), code });
    }
    Ok(changes)
}

/// Restore `paths` in the working tree to their state in `tree`; paths that do
/// not exist in `tree` are deleted (they were added afterwards).
pub fn restore_from_tree(root: &Path, tree: &str, paths: &[String]) -> Result<String, String> {
    for path in paths {
        if git(root, &["cat-file", "-e", &format!("{tree}:{path}")]).is_ok() {
            git(root, &["restore", &format!("--source={tree}"), "--worktree", "--", path])?;
        } else {
            let file = root.join(path);
            if file.exists() {
                std::fs::remove_file(&file).map_err(|e| format!("{path}: {e}"))?;
            }
        }
    }
    Ok(format!("Reverted {} file(s)", paths.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_branch_line() {
        let mut s = Snapshot::default();
        parse_branch("main...origin/main [ahead 2, behind 1]", &mut s);
        assert_eq!((s.branch.as_str(), s.ahead, s.behind), ("main", 2, 1));

        let mut s = Snapshot::default();
        parse_branch("No commits yet on dev", &mut s);
        assert_eq!(s.branch, "dev");
    }
}


#[cfg(test)]
mod snapshot_tests {
    use super::*;

    #[test]
    fn snapshot_diff_and_restore_roundtrip() {
        let dir = std::env::temp_dir().join(format!("mira-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "-q"]).unwrap();
        std::fs::write(dir.join("a.txt"), "one\n").unwrap();
        let t1 = snapshot_tree(&dir).unwrap();

        std::fs::write(dir.join("a.txt"), "two\n").unwrap();
        std::fs::write(dir.join("b.txt"), "new\n").unwrap();
        let t2 = snapshot_tree(&dir).unwrap();

        let changes = diff_trees(&dir, &t1, &t2).unwrap();
        let summary: Vec<(char, &str)> = changes.iter().map(|c| (c.code, c.path.as_str())).collect();
        assert_eq!(summary, vec![('M', "a.txt"), ('A', "b.txt")]);
        // The user's index is untouched.
        assert!(git(&dir, &["ls-files"]).unwrap().is_empty());

        restore_from_tree(&dir, &t1, &["a.txt".into(), "b.txt".into()]).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "one\n");
        assert!(!dir.join("b.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
