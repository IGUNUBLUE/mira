//! Read what the user asked an agent from the agent's own conversation log, to
//! title a run. Best effort and read-only; only kiro and codex are known.
//!
//! - kiro: `~/.kiro/sessions/<group>/sess_*/session.json` (`workspacePaths`,
//!   `lastModifiedAt`) and `messages.jsonl` (`payload.type == "user"`).
//! - codex: `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`; first line is the
//!   session meta with `cwd`; user turns are `event_msg` / `item_completed`
//!   with a `UserMessage` item.

use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// Slack around run boundaries: herdr is polled, logs are written async.
const SLACK: u64 = 5;
const MAX_PROMPT: usize = 200;

/// The last prompt `kind` received in `dir` before a run that went from
/// `started` to `finished` (unix seconds).
pub fn last_prompt(home: &Path, kind: &str, dir: &Path, started: u64, finished: u64) -> Option<String> {
    let until = finished + SLACK;
    let raw = match kind {
        "kiro" => kiro(home, dir, started, until),
        "codex" => codex(home, dir, started, until),
        _ => None,
    }?;
    tidy(&raw)
}

/// First non-empty line, whitespace collapsed, capped.
fn tidy(s: &str) -> Option<String> {
    let line = s.lines().map(str::trim).find(|l| !l.is_empty())?;
    let mut out: String = line.split_whitespace().collect::<Vec<_>>().join(" ");
    if out.chars().count() > MAX_PROMPT {
        out = out.chars().take(MAX_PROMPT - 1).collect::<String>() + "…";
    }
    Some(out)
}

fn modified(p: &Path) -> u64 {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs())
}

/// Lines of a JSONL file that contain `needle`, parsed.
fn json_lines(path: &Path, needle: &str) -> impl Iterator<Item = Value> {
    let file = std::fs::File::open(path).ok();
    let needle = needle.to_string();
    file.into_iter()
        .flat_map(|f| BufReader::new(f).lines().map_while(Result::ok))
        .filter(move |l| l.contains(&needle))
        .filter_map(|l| serde_json::from_str(&l).ok())
}

fn kiro(home: &Path, dir: &Path, started: u64, until: u64) -> Option<String> {
    let mut best: Option<(u64, PathBuf)> = None;
    for group in std::fs::read_dir(home.join(".kiro/sessions")).ok()?.flatten() {
        let Ok(sessions) = std::fs::read_dir(group.path()) else { continue };
        for s in sessions.flatten() {
            let Ok(text) = std::fs::read_to_string(s.path().join("session.json")) else { continue };
            let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
            let here = v["workspacePaths"]
                .as_array()
                .is_some_and(|ps| ps.iter().filter_map(Value::as_str).any(|p| Path::new(p) == dir));
            let when = v["lastModifiedAt"].as_str().and_then(parse_iso_utc).unwrap_or(0);
            if here && when + SLACK >= started && best.as_ref().is_none_or(|(t, _)| when > *t) {
                best = Some((when, s.path()));
            }
        }
    }
    let (_, session) = best?;
    let mut last = None;
    for v in json_lines(&session.join("messages.jsonl"), "\"user\"") {
        let p = &v["payload"];
        if p["type"] != "user" || v["timestamp"].as_str().and_then(parse_iso_utc).is_some_and(|t| t > until) {
            continue;
        }
        let text = match &p["content"] {
            Value::String(s) => s.clone(),
            Value::Array(parts) => parts.iter().filter_map(|x| x["text"].as_str()).collect::<Vec<_>>().join("\n"),
            _ => continue,
        };
        if !text.trim().is_empty() {
            last = Some(text);
        }
    }
    last
}

fn codex(home: &Path, dir: &Path, started: u64, until: u64) -> Option<String> {
    let mut best: Option<(u64, PathBuf)> = None;
    let mut stack = vec![(home.join(".codex/sessions"), 0)];
    while let Some((d, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            let path = e.path();
            if depth < 3 {
                if path.is_dir() {
                    stack.push((path, depth + 1));
                }
                continue;
            }
            let when = modified(&path);
            if when + SLACK < started || path.extension().is_none_or(|x| x != "jsonl") {
                continue;
            }
            let Some(Ok(first)) = std::fs::File::open(&path).ok().and_then(|f| BufReader::new(f).lines().next()) else {
                continue;
            };
            let here = serde_json::from_str::<Value>(&first)
                .ok()
                .is_some_and(|v| v["payload"]["cwd"].as_str().is_some_and(|c| Path::new(c) == dir));
            if here && best.as_ref().is_none_or(|(t, _)| when > *t) {
                best = Some((when, path));
            }
        }
    }
    let (_, log) = best?;
    let mut last = None;
    for v in json_lines(&log, "UserMessage") {
        let item = &v["payload"]["item"];
        if v["payload"]["type"] != "item_completed"
            || item["type"] != "UserMessage"
            || v["timestamp"].as_str().and_then(parse_iso_utc).is_some_and(|t| t > until)
        {
            continue;
        }
        let text: Vec<&str> = item["content"].as_array().into_iter().flatten().filter_map(|x| x["text"].as_str()).collect();
        let text = text.join("\n");
        if !text.trim().is_empty() {
            last = Some(text);
        }
    }
    last
}

/// `2026-10-14T17:13:27.934Z` → unix seconds (UTC only).
pub fn parse_iso_utc(s: &str) -> Option<u64> {
    let (date, time) = s.split_once('T')?;
    let mut d = date.split('-').map(|x| x.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let mut t = time.trim_end_matches('Z').split(':');
    let (h, min) = (t.next()?.parse::<i64>().ok()?, t.next()?.parse::<i64>().ok()?);
    let sec = t.next()?.split('.').next()?.parse::<i64>().ok()?;
    // Days from civil (Howard Hinnant).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days * 86_400 + h * 3600 + min * 60 + sec).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn temp_home() -> PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let d = std::env::temp_dir().join(format!("mira-agentlog-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).expect("temp home");
        d
    }

    #[test]
    fn parses_iso_timestamps() {
        assert_eq!(parse_iso_utc("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_iso_utc("2026-10-14T17:13:27.934Z"), Some(1_791_998_007));
        assert_eq!(parse_iso_utc("garbage"), None);
    }

    #[test]
    fn kiro_prompt_is_the_last_user_message_in_that_folder() {
        let home = temp_home();
        let s = home.join(".kiro/sessions/abc/sess_1");
        std::fs::create_dir_all(&s).expect("dirs");
        std::fs::write(
            s.join("session.json"),
            r#"{"id":"sess_1","workspacePaths":["/repo"],"lastModifiedAt":"2026-10-14T17:13:30.000Z"}"#,
        )
        .expect("write");
        std::fs::write(
            s.join("messages.jsonl"),
            concat!(
                r#"{"timestamp":"2026-10-14T17:00:00.000Z","payload":{"type":"user","content":"primero"}}"#, "\n",
                r#"{"timestamp":"2026-10-14T17:00:05.000Z","payload":{"type":"assistant","content":"ok"}}"#, "\n",
                r#"{"timestamp":"2026-10-14T17:13:20.000Z","payload":{"type":"user","content":"  agrega   tests\nal parser"}}"#, "\n",
                r#"{"timestamp":"2026-10-14T18:00:00.000Z","payload":{"type":"user","content":"después"}}"#, "\n",
            ),
        )
        .expect("write");
        let start = parse_iso_utc("2026-10-14T17:13:21Z").expect("ts");
        let end = parse_iso_utc("2026-10-14T17:13:29Z").expect("ts");
        assert_eq!(last_prompt(&home, "kiro", Path::new("/repo"), start, end).as_deref(), Some("agrega tests"));
        assert_eq!(last_prompt(&home, "kiro", Path::new("/other"), start, end), None);
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn codex_prompt_comes_from_user_message_items() {
        let home = temp_home();
        let d = home.join(".codex/sessions/2026/10/14");
        std::fs::create_dir_all(&d).expect("dirs");
        std::fs::write(
            d.join("rollout-x.jsonl"),
            concat!(
                r#"{"type":"session_meta","payload":{"id":"t1","cwd":"/repo"}}"#, "\n",
                r#"{"timestamp":"2026-10-14T17:13:20.000Z","type":"event_msg","payload":{"type":"item_completed","item":{"type":"UserMessage","content":[{"type":"text","text":"fix the add function\n"}]}}}"#, "\n",
            ),
        )
        .expect("write");
        let now = modified(&d.join("rollout-x.jsonl"));
        let prompt = last_prompt(&home, "codex", Path::new("/repo"), now, now + 60);
        assert_eq!(prompt.as_deref(), Some("fix the add function"));
        assert_eq!(last_prompt(&home, "claude", Path::new("/repo"), now, now), None);
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn tidy_caps_long_prompts() {
        let long = "x".repeat(500);
        let t = tidy(&long).expect("text");
        assert_eq!(t.chars().count(), MAX_PROMPT);
        assert!(t.ends_with('…'));
        assert_eq!(tidy("  \n "), None);
    }
}
