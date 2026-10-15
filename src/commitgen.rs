//! Commit message generation. Same prompt, excludes and size budget as the
//! `chatgpt-commit-gen` VSCodium extension, streamed live into the commit box.
//!
//! Providers (`commit_provider` in `~/.config/mira/config`, default `auto`):
//! - `chatgpt`: your ChatGPT subscription, signed in from mira (OAuth);
//! - `openai`: an API key from `OPENAI_API_KEY` or `~/.config/mira/openai_key`;
//! - `command`: `commit_command`, any shell command that reads the prompt on
//!   stdin and prints the message (e.g. a local model), streamed as it prints.
//!
//! `auto` uses the ChatGPT session if there is one, else an API key, else
//! `commit_command`, else asks you to sign in with ChatGPT.
//! Only the diff of the next commit and its `--stat` leave the machine.

use crate::git::git;
use crate::openai::{self, Auth, StreamError};
use std::borrow::Cow;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

pub const DEFAULT_MODEL: &str = "gpt-6-luna";
const MAX_DIFF_CHARS: usize = 12_000;
const MAX_STAT_CHARS: usize = 4_000;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(180);

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

/// Progress of a generation, sent to the UI as it happens.
#[derive(Debug, Clone)]
pub enum Event {
    Status(String),
    /// More text of the message.
    Delta(String),
    /// The ChatGPT account after the run: `Some(email)` (maybe empty) or signed out.
    Account(Option<String>),
    Done(Result<String, String>),
}

/// `(stat, diff)` of what the next commit will contain: the index, or — when
/// nothing is staged, as "Commit All" does — every change in the working tree.
/// `root` is the repository root; only paths under `prefix` (the opened
/// folder, `""` for the whole repository) count, as "Commit All" stages only those.
pub fn changes(root: &Path, staged: bool, prefix: &str) -> Result<(String, String), String> {
    let scope = if prefix.is_empty() { "." } else { prefix };
    let mut common = vec!["diff", "--no-ext-diff", "--no-color"];
    if staged {
        common.push("--cached");
    } else if git(root, &["rev-parse", "--verify", "-q", "HEAD"]).is_ok() {
        common.push("HEAD");
    }
    let excludes: Vec<String> = EXCLUDES.iter().map(|g| format!(":(exclude,glob){g}")).collect();
    let mut stat_args = common.clone();
    stat_args.extend(["--stat=120", "--", scope]);
    let mut stat = git(root, &stat_args)?;
    let mut diff_args = common.clone();
    diff_args.extend(["--", scope]);
    diff_args.extend(excludes.iter().map(String::as_str));
    let diff = git(root, &diff_args)?;
    if !staged {
        let untracked = git(root, &["ls-files", "--others", "--exclude-standard", "--", scope])?;
        for f in untracked.lines().filter(|l| !l.is_empty()) {
            stat.push_str(&format!(" {f} (new, untracked)\n"));
        }
    }
    if stat.trim().is_empty() {
        return Err("Nothing to commit".into());
    }
    Ok((stat, diff))
}

/// `stat` cut to about [`MAX_STAT_CHARS`] on a line boundary, ending with how
/// many files were left out. A repo with thousands of new files would otherwise
/// send every name.
fn cap_stat(stat: &str) -> Cow<'_, str> {
    if stat.len() <= MAX_STAT_CHARS {
        return Cow::Borrowed(stat);
    }
    let mut kept = String::new();
    let mut lines = stat.lines();
    for line in lines.by_ref() {
        if kept.len() + line.len() + 1 > MAX_STAT_CHARS {
            // This line and the rest are left out (the summary line is not a file).
            let rest = std::iter::once(line).chain(lines).filter(|l| !is_stat_summary(l)).count();
            kept.push_str(&format!("… {rest} more files\n"));
            return Cow::Owned(kept);
        }
        kept.push_str(line);
        kept.push('\n');
    }
    Cow::Owned(kept)
}

/// `git diff --stat`'s last line, " 3 files changed, 10 insertions(+)".
fn is_stat_summary(line: &str) -> bool {
    line.contains(" file changed") || line.contains(" files changed")
}

/// The user turn: stat plus the (capped) diff. Instructions go separately.
pub fn user_text(stat: &str, diff: &str) -> String {
    let stat = cap_stat(stat);
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
        "Generate a commit message for these staged changes.\n\n\
         Files changed (git diff --stat, includes every staged file):\n{}\n\n\
         Diff (lockfiles and generated files may be omitted):\n{body}",
        stat.trim_end()
    )
}

/// Instructions and user turn in one text, for `commit_command`.
pub fn build_prompt(stat: &str, diff: &str) -> String {
    format!("{SYSTEM_PROMPT}\n\n{}", user_text(stat, diff))
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

/// How the model is reached.
#[derive(Debug, PartialEq)]
enum Provider {
    ChatGpt,
    ApiKey(String),
    Command(String),
}

/// Pick the provider from `commit_provider` and what is available.
fn choose(
    setting: &str,
    signed_in: bool,
    api_key: impl FnOnce() -> Result<Option<String>, String>,
    command: Option<String>,
) -> Result<Provider, String> {
    let no_key = "no OpenAI API key: set OPENAI_API_KEY or put it in ~/.config/mira/openai_key (chmod 600)";
    match setting.trim().to_ascii_lowercase().as_str() {
        "chatgpt" => Ok(Provider::ChatGpt),
        "openai" | "api" => api_key()?.map(Provider::ApiKey).ok_or_else(|| no_key.into()),
        "command" => {
            command.map(Provider::Command).ok_or_else(|| "commit_provider = command needs commit_command".into())
        }
        "" | "auto" => Ok(if signed_in {
            Provider::ChatGpt
        } else if let Some(key) = api_key()? {
            Provider::ApiKey(key)
        } else {
            command.map_or(Provider::ChatGpt, Provider::Command)
        }),
        other => Err(format!("unknown commit_provider {other:?}: use auto, chatgpt, openai or command")),
    }
}

fn config(key: &str) -> Option<String> {
    crate::persist::config_get(key).filter(|v| !v.trim().is_empty())
}

/// Generate a message for the next commit in the repository at `root`
/// (limited to `prefix`, see [`changes`]), sending progress to `emit`.
/// Blocking; stops early when `cancel` is set.
pub fn generate(
    root: &Path,
    staged: bool,
    prefix: &str,
    cancel: &AtomicBool,
    emit: &mut dyn FnMut(Event),
) -> Result<String, String> {
    let (stat, diff) = changes(root, staged, prefix)?;
    let model = config("commit_model").unwrap_or_else(|| DEFAULT_MODEL.into());
    let provider = choose(
        &config("commit_provider").unwrap_or_default(),
        openai::load_tokens().is_some(),
        openai::api_key,
        config("commit_command"),
    )?;
    let raw = match provider {
        Provider::ChatGpt => {
            let result = chatgpt(&model, &user_text(&stat, &diff), cancel, emit);
            emit(Event::Account(openai::account()));
            result?
        }
        Provider::ApiKey(key) => {
            openai::stream(&Auth::ApiKey(&key), &model, SYSTEM_PROMPT, &user_text(&stat, &diff), cancel, &mut |d| {
                emit(Event::Delta(d.to_string()))
            })
            .map_err(|e| match e {
                StreamError::Unauthorized => "OpenAI rejected the API key (401)".to_string(),
                StreamError::Other(e) => e,
            })?
        }
        Provider::Command(line) => run_command(&line, &build_prompt(&stat, &diff), cancel, emit)?,
    };
    let message = clean(&raw);
    if message.is_empty() { Err("The model returned an empty message".into()) } else { Ok(message) }
}

fn chatgpt(model: &str, input: &str, cancel: &AtomicBool, emit: &mut dyn FnMut(Event)) -> Result<String, String> {
    let tokens = match openai::load_tokens() {
        Some(t) => openai::fresh(t, cancel)?,
        None => {
            emit(Event::Status("Sign in with ChatGPT in your browser…".into()));
            let t = openai::login(cancel)?;
            emit(Event::Account(Some(t.email.clone().unwrap_or_default())));
            emit(Event::Status("Signed in — generating a commit message…".into()));
            t
        }
    };
    let mut tokens = tokens;
    let mut retried = false;
    loop {
        match openai::stream(&Auth::ChatGpt(&tokens), model, SYSTEM_PROMPT, input, cancel, &mut |d| {
            emit(Event::Delta(d.to_string()))
        }) {
            Ok(text) => return Ok(text),
            Err(StreamError::Unauthorized) if !retried => {
                retried = true;
                tokens = openai::refresh(&tokens, cancel)?;
            }
            Err(StreamError::Unauthorized) => {
                let _ = openai::sign_out();
                return Err("ChatGPT rejected the session — click ✦ to sign in again".into());
            }
            Err(StreamError::Other(e)) => return Err(e),
        }
    }
}

/// Run `commit_command` with the prompt on stdin, streaming its stdout.
fn run_command(line: &str, prompt: &str, cancel: &AtomicBool, emit: &mut dyn FnMut(Event)) -> Result<String, String> {
    let child = Command::new("sh")
        .args(["-c", line])
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not start commit_command: {e}"))?;
    let mut pending = Vec::new();
    let mut text = String::new();
    // A command that ignores stdin closes it early; that is fine (`stdin_error` is not checked).
    let done = crate::proc::stream(child, prompt.as_bytes().to_vec(), cancel, COMMAND_TIMEOUT, &mut |bytes| {
        pending.extend_from_slice(bytes);
        // Emit whole UTF-8 characters only; keep a split one for later.
        let valid = match std::str::from_utf8(&pending) {
            Ok(_) => pending.len(),
            Err(e) if e.error_len().is_none() => e.valid_up_to(),
            Err(_) => pending.len(),
        };
        let chunk = String::from_utf8_lossy(&pending[..valid]).into_owned();
        pending.drain(..valid);
        if !chunk.is_empty() {
            text.push_str(&chunk);
            emit(Event::Delta(chunk));
        }
    })?;
    text.push_str(&String::from_utf8_lossy(&pending));
    if !done.status.success() {
        let why = done.stderr.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("commit_command failed");
        return Err(why.trim().to_string());
    }
    Ok(text)
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
        assert!(!user_text(" a | 1 +", "d").contains("Conventional Commits"));
    }

    #[test]
    fn auto_prefers_session_then_key_then_command_then_sign_in() {
        let key = || Ok(Some("sk".to_string()));
        let none = || Ok(None);
        let cmd = || Some("ollama".to_string());
        assert_eq!(choose("", true, key, cmd()), Ok(Provider::ChatGpt));
        assert_eq!(choose("auto", false, key, cmd()), Ok(Provider::ApiKey("sk".into())));
        assert_eq!(choose("", false, none, cmd()), Ok(Provider::Command("ollama".into())));
        assert_eq!(choose("", false, none, None), Ok(Provider::ChatGpt));
        assert_eq!(choose("ChatGPT", false, key, cmd()), Ok(Provider::ChatGpt));
        assert_eq!(choose("command", true, key, cmd()), Ok(Provider::Command("ollama".into())));
        assert!(choose("openai", true, none, None).unwrap_err().contains("OPENAI_API_KEY"));
        assert!(choose("codex", true, key, None).is_err());
    }

    #[test]
    fn caps_the_stat_on_a_line_boundary() {
        let lines: Vec<String> = (0..1000).map(|i| format!(" src/file{i}.rs | 1 +")).collect();
        let stat = format!("{}\n 1000 files changed, 1000 insertions(+)\n", lines.join("\n"));
        let capped = cap_stat(&stat);
        assert!(capped.len() <= MAX_STAT_CHARS + 40, "{}", capped.len());
        let kept = capped.lines().filter(|l| l.contains(" | ")).count();
        assert!(capped.ends_with(&format!("… {} more files\n", 1000 - kept)), "{capped}");
        assert_eq!(cap_stat(" a | 1 +\n"), " a | 1 +\n");
    }

    #[test]
    fn changes_are_limited_to_the_opened_folder() {
        let dir = crate::files::test_dir("commitgen");
        git(&dir, &["init", "-q"]).unwrap();
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/in.txt"), "in\n").unwrap();
        std::fs::write(dir.join("out.txt"), "out\n").unwrap();
        let (stat, _) = changes(&dir, false, "sub/").unwrap();
        assert!(stat.contains("sub/in.txt") && !stat.contains("out.txt"), "{stat}");
        let (stat, _) = changes(&dir, false, "").unwrap();
        assert!(stat.contains("sub/in.txt") && stat.contains("out.txt"), "{stat}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn command_streams_stdout_in_utf8_pieces() {
        let mut deltas = Vec::new();
        let out =
            run_command("head -c 4; sleep 0.2; printf 'é\\n'", "fix: stdin", &AtomicBool::new(false), &mut |e| {
                if let Event::Delta(d) = e {
                    deltas.push(d)
                }
            });
        assert_eq!(out.unwrap(), "fix:é\n");
        assert_eq!(deltas, ["fix:", "é\n"]);
    }

    #[test]
    fn command_failure_and_cancel() {
        let none = &mut |_: Event| {};
        assert_eq!(run_command("echo boom >&2; exit 3", "", &AtomicBool::new(false), none).unwrap_err(), "boom");
        assert_eq!(run_command("sleep 30", "", &AtomicBool::new(true), none).unwrap_err(), "Cancelled");
    }
}
