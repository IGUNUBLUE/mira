//! Commit message generation. Same prompt, excludes and size budget as the
//! `chatgpt-commit-gen` VSCodium extension, but mira makes no network calls:
//! the model is reached through an agent CLI the user already signed in to.
//!
//! - default: `codex exec` (ChatGPT subscription), read-only sandbox, in an
//!   empty temp dir so the agent sees only the prompt, never the repository;
//! - `commit_command` in `~/.config/mira/config`: any shell command that reads
//!   the prompt on stdin and prints the message (e.g. a local model).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub const DEFAULT_MODEL: &str = "gpt-6-luna";
const MAX_DIFF_CHARS: usize = 12_000;
const TIMEOUT: Duration = Duration::from_secs(180);

const EXCLUDES: &[&str] = &[
    "**/package-lock.json",
    "**/npm-shrinkwrap.json",
    "**/yarn.lock",
    "**/pnpm-lock.yaml",
    "**/bun.lock",
    "**/bun.lockb",
    "**/Cargo.lock",
    "**/poetry.lock",
    "**/uv.lock",
    "**/Pipfile.lock",
    "**/composer.lock",
    "**/Gemfile.lock",
    "**/go.sum",
    "**/*.min.js",
    "**/*.min.css",
    "**/*.map",
];

pub const SYSTEM_PROMPT: &str = r#"You generate Git commit messages from a diff, following the Conventional Commits 1.0.0 specification.

Format:
<type>[optional scope][optional !]: <description>

[optional body]

[optional footer(s)]

Rules:
- Reply with ONLY the commit message. No explanations, no markdown fences, no quotes.
- Types: feat, fix, docs, style, refactor, perf, test, build, ci, chore, revert.
- Scope is a noun describing a codebase section, in parenthesis; omit it when not obvious.
- Description: short summary after "type: ", imperative mood, lowercase, max 72 characters, no trailing period.
- A breaking API change MUST be marked with ! immediately before the colon (e.g. "feat(api)!: ...") and/or a "BREAKING CHANGE: <description>" footer.
- Body: optional, separated from the subject by one blank line, wrapped at 72 characters. Only add it when the change needs real context; most commits need only a subject.
- Footers: "Token: value" or "Token #value" pairs (git trailer style); tokens use hyphens instead of spaces, e.g. Refs: #123.
- Base the message strictly on the diff; never invent behavior."#;

fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(root)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// `(stat, diff)` of what the next commit will contain: the index, or — when
/// nothing is staged, as "Commit All" does — every change in the working tree.
pub fn changes(root: &Path, staged: bool) -> Result<(String, String), String> {
    let mut common = vec!["diff", "--no-ext-diff", "--no-color"];
    if staged {
        common.push("--cached");
    } else if git(root, &["rev-parse", "--verify", "-q", "HEAD"]).is_ok() {
        common.push("HEAD");
    }
    let excludes: Vec<String> = EXCLUDES.iter().map(|g| format!(":(exclude,glob){g}")).collect();
    let mut stat_args = common.clone();
    stat_args.push("--stat=120");
    let mut stat = git(root, &stat_args)?;
    let mut diff_args = common.clone();
    diff_args.extend(["--", "."]);
    diff_args.extend(excludes.iter().map(String::as_str));
    let diff = git(root, &diff_args)?;
    if !staged {
        let untracked = git(root, &["ls-files", "--others", "--exclude-standard"])?;
        for f in untracked.lines().filter(|l| !l.is_empty()) {
            stat.push_str(&format!(" {f} (new, untracked)\n"));
        }
    }
    if stat.trim().is_empty() {
        return Err("Nothing to commit".into());
    }
    Ok((stat, diff))
}

pub fn build_prompt(stat: &str, diff: &str) -> String {
    let body = if diff.len() > MAX_DIFF_CHARS {
        let mut cut = MAX_DIFF_CHARS;
        while !diff.is_char_boundary(cut) {
            cut -= 1;
        }
        format!("{}\n... (diff truncated) ...", &diff[..cut])
    } else if diff.trim().is_empty() {
        "(no diff content: only excluded or untracked files changed)".to_string()
    } else {
        diff.to_string()
    };
    format!(
        "{SYSTEM_PROMPT}\n\nGenerate a commit message for these staged changes.\n\n\
         Files changed (git diff --stat, includes every staged file):\n{}\n\n\
         Diff (lockfiles and generated files may be omitted):\n{body}",
        stat.trim_end()
    )
}

/// Strip fences and quotes a model may add despite the instructions.
pub fn clean(raw: &str) -> String {
    let mut s = raw.trim();
    if let Some(rest) = s.strip_prefix("```") {
        s = rest.split_once('\n').map(|(_, r)| r).unwrap_or("");
        s = s.trim_end().strip_suffix("```").unwrap_or(s);
    }
    let s = s.trim();
    let s = s
        .strip_prefix('"')
        .and_then(|r| r.strip_suffix('"'))
        .or_else(|| s.strip_prefix('\'').and_then(|r| r.strip_suffix('\'')))
        .unwrap_or(s);
    s.trim().to_string()
}

/// How the model is reached, from the config file.
pub enum Backend {
    Codex { model: String },
    Shell(String),
}

impl Backend {
    pub fn from_config() -> Self {
        match crate::persist::config_get("commit_command").filter(|c| !c.trim().is_empty()) {
            Some(cmd) => Self::Shell(cmd),
            None => Self::Codex {
                model: crate::persist::config_get("commit_model")
                    .filter(|m| !m.trim().is_empty())
                    .unwrap_or_else(|| DEFAULT_MODEL.into()),
            },
        }
    }
}

fn on_path(bin: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(bin).is_file()))
}

fn scratch_dir() -> Result<PathBuf, String> {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir =
        std::env::temp_dir().join(format!("mira-commit-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// Generate a message for the next commit in `root`. Blocking; returns early
/// with an error when `cancel` is set.
pub fn generate(root: &Path, staged: bool, cancel: Arc<AtomicBool>) -> Result<String, String> {
    let (stat, diff) = changes(root, staged)?;
    let prompt = build_prompt(&stat, &diff);
    let dir = scratch_dir()?;
    let result = run(&Backend::from_config(), &dir, &prompt, &cancel);
    let _ = std::fs::remove_dir_all(&dir);
    let message = clean(&result?);
    if message.is_empty() { Err("The model returned an empty message".into()) } else { Ok(message) }
}

fn run(backend: &Backend, dir: &Path, prompt: &str, cancel: &AtomicBool) -> Result<String, String> {
    let out_file = dir.join("message.txt");
    let mut cmd = match backend {
        Backend::Codex { model } => {
            if !on_path("codex") {
                return Err("codex not found — install it, or set commit_command in ~/.config/mira/config".into());
            }
            let mut c = Command::new("codex");
            c.args(["exec", "--ephemeral", "--skip-git-repo-check", "--sandbox", "read-only"])
                .args(["--ignore-user-config", "--ignore-rules", "--color", "never"])
                .args(["-m", model, "-c", "model_reasoning_effort=\"low\"", "-o"])
                .arg(&out_file)
                .arg("-");
            c
        }
        Backend::Shell(line) => {
            let mut c = Command::new("sh");
            c.args(["-c", line]);
            c
        }
    };
    let mut child = cmd
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not start the generator: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(prompt.as_bytes()).map_err(|e| e.to_string())?;
    }
    // Drain pipes on threads so a chatty process cannot block on a full pipe.
    let drain = |r: Option<Box<dyn std::io::Read + Send>>| {
        std::thread::spawn(move || {
            let mut buf = String::new();
            if let Some(mut r) = r {
                let _ = r.read_to_string(&mut buf);
            }
            buf
        })
    };
    let stdout = drain(child.stdout.take().map(|r| Box::new(r) as _));
    let stderr = drain(child.stderr.take().map(|r| Box::new(r) as _));
    let started = std::time::Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if cancel.load(Ordering::Relaxed) || started.elapsed() > TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return Err(if cancel.load(Ordering::Relaxed) { "Cancelled".into() } else { "Timed out".into() });
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let stdout = stdout.join().unwrap_or_default();
    let stderr = stderr.join().unwrap_or_default();
    if !status.success() {
        let why = stderr.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("generator failed");
        return Err(why.trim().to_string());
    }
    match backend {
        Backend::Codex { .. } => std::fs::read_to_string(&out_file).map_err(|_| "codex wrote no message".into()),
        Backend::Shell(_) => Ok(stdout),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_fences_and_quotes() {
        assert_eq!(clean("```\nfix: a\n```\n"), "fix: a");
        assert_eq!(clean("```text\nfeat(x): b\n\nbody\n```"), "feat(x): b\n\nbody");
        assert_eq!(clean("  \"docs: c\" "), "docs: c");
        assert_eq!(clean("chore: d"), "chore: d");
    }

    #[test]
    fn truncates_long_diffs_on_a_char_boundary() {
        let diff = "é".repeat(MAX_DIFF_CHARS);
        let p = build_prompt(" a | 1 +", &diff);
        assert!(p.contains("(diff truncated)"));
        assert!(p.starts_with(SYSTEM_PROMPT));
    }

    #[test]
    fn shell_backend_reads_prompt_on_stdin() {
        let dir = scratch_dir().unwrap();
        let out = run(&Backend::Shell("head -c 3".into()), &dir, "fix: stdin", &AtomicBool::new(false));
        let _ = std::fs::remove_dir_all(dir);
        assert_eq!(out.unwrap(), "fix");
    }

    #[test]
    fn cancel_kills_the_generator() {
        let dir = scratch_dir().unwrap();
        let out = run(&Backend::Shell("sleep 30".into()), &dir, "", &AtomicBool::new(true));
        let _ = std::fs::remove_dir_all(dir);
        assert_eq!(out.unwrap_err(), "Cancelled");
    }
}
