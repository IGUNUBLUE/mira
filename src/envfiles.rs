//! `.env` files: discovery, parsing, masking and safe edits.
//!
//! Values are secrets: they are masked everywhere by default, never logged and
//! never sent to agents. Diffs of env files show a short fingerprint per value
//! so changes stay visible without revealing them.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub key: String,
    pub value: String,
    /// 0-based line index in the file.
    pub line: usize,
}

#[derive(Debug, Clone)]
pub struct EnvFile {
    pub path: PathBuf,
    pub rel: String,
    pub entries: Vec<Entry>,
    pub example: bool,
    /// Tracked by git (a secrets file being tracked is a red flag).
    pub tracked: bool,
    /// Ignored by git.
    pub ignored: bool,
    /// Keys present in the sibling example file but missing here.
    pub missing: Vec<String>,
    /// Keys defined more than once.
    pub duplicates: Vec<String>,
}

impl EnvFile {
    pub fn risky(&self) -> bool {
        !self.example && (self.tracked || !self.ignored)
    }
}

/// Per-key change since the last review.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyChange {
    Added,
    Changed,
    Removed,
}

const SKIP_DIRS: &[&str] = &[
    ".git", "node_modules", "target", "dist", "build", ".venv", "venv", "vendor", ".next", ".cache",
    "__pycache__",
];

pub fn is_env_name(name: &str) -> bool {
    name == ".env" || name.starts_with(".env.") || (name.ends_with(".env") && name.len() > 4)
}

pub fn is_example_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    ["example", "sample", "template", "dist", "defaults"].iter().any(|w| n.contains(w))
}

pub fn discover(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let walker = ignore::WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(false) // .env files are usually ignored; we want them.
        .git_global(false)
        .git_exclude(false)
        .ignore(false)
        .max_depth(Some(5))
        .filter_entry(|e| {
            !(e.file_type().is_some_and(|t| t.is_dir())
                && SKIP_DIRS.contains(&e.file_name().to_string_lossy().as_ref()))
        })
        .build();
    for entry in walker.filter_map(Result::ok) {
        if entry.file_type().is_some_and(|t| t.is_file())
            && is_env_name(&entry.file_name().to_string_lossy())
        {
            found.push(entry.into_path());
        }
        if found.len() >= 200 {
            break;
        }
    }
    found.sort();
    found
}

fn valid_key(k: &str) -> bool {
    let mut chars = k.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// Split a line into `(key, value)` if it is an assignment.
fn parse_line(line: &str) -> Option<(String, String)> {
    let t = line.trim();
    if t.is_empty() || t.starts_with('#') {
        return None;
    }
    let t = t.strip_prefix("export ").unwrap_or(t);
    let (key, raw) = t.split_once('=')?;
    let key = key.trim();
    if !valid_key(key) {
        return None;
    }
    let raw = raw.trim();
    let value = match raw.chars().next() {
        Some(q @ ('"' | '\'')) => {
            let mut out = String::new();
            let mut escaped = false;
            for c in raw[1..].chars() {
                if escaped {
                    out.push(match c {
                        'n' if q == '"' => '\n',
                        c => c,
                    });
                    escaped = false;
                } else if c == '\\' && q == '"' {
                    escaped = true;
                } else if c == q {
                    break;
                } else {
                    out.push(c);
                }
            }
            out
        }
        _ => match raw.find(" #") {
            Some(i) => raw[..i].trim_end().to_string(),
            None => raw.to_string(),
        },
    };
    Some((key.to_string(), value))
}

pub fn parse(text: &str) -> Vec<Entry> {
    text.lines()
        .enumerate()
        .filter_map(|(line, l)| parse_line(l).map(|(key, value)| Entry { key, value, line }))
        .collect()
}

fn quote(value: &str) -> String {
    let plain = !value.is_empty()
        && value
            .chars()
            .all(|c| !c.is_whitespace() && !matches!(c, '"' | '\'' | '#' | '\\' | '$' | '`'));
    if plain {
        value.to_string()
    } else {
        let escaped = value.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n");
        format!("\"{escaped}\"")
    }
}

/// Set `key` to `value`, preserving every other line (and an `export ` prefix).
/// Appends the key if it does not exist.
pub fn set_value(text: &str, key: &str, value: &str) -> String {
    let mut lines: Vec<String> = text.lines().map(String::from).collect();
    let mut done = false;
    for line in &mut lines {
        if let Some((k, _)) = parse_line(line)
            && k == key && !done {
                let export = line.trim_start().starts_with("export ");
                *line = format!("{}{key}={}", if export { "export " } else { "" }, quote(value));
                done = true;
            }
    }
    if !done {
        lines.push(format!("{key}={}", quote(value)));
    }
    let mut out = lines.join("\n");
    if text.ends_with('\n') || !done || text.is_empty() {
        out.push('\n');
    }
    out
}

pub fn fingerprint(value: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut h);
    h.finish()
}

/// Masked representation of a value: length + 4 hex of a fingerprint.
pub fn mask(value: &str) -> String {
    if value.is_empty() {
        return "(empty)".into();
    }
    format!("•••••• ‹{:04x}› {} chars", fingerprint(value) & 0xffff, value.chars().count())
}

/// Mask every value in a whole env file (used for diffs).
pub fn mask_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        match parse_line(line) {
            Some((key, value)) => {
                let export = if line.trim_start().starts_with("export ") { "export " } else { "" };
                out.push_str(&format!("{export}{key}={}", mask(&value)));
            }
            None => out.push_str(line),
        }
        out.push('\n');
    }
    out
}

pub fn key_map(entries: &[Entry]) -> HashMap<String, u64> {
    entries.iter().map(|e| (e.key.clone(), fingerprint(&e.value))).collect()
}

pub fn changes(reviewed: &HashMap<String, u64>, now: &[Entry]) -> BTreeMap<String, KeyChange> {
    let current = key_map(now);
    let mut out = BTreeMap::new();
    for (k, v) in &current {
        match reviewed.get(k) {
            None => {
                out.insert(k.clone(), KeyChange::Added);
            }
            Some(old) if old != v => {
                out.insert(k.clone(), KeyChange::Changed);
            }
            _ => {}
        }
    }
    for k in reviewed.keys() {
        if !current.contains_key(k) {
            out.insert(k.clone(), KeyChange::Removed);
        }
    }
    out
}

/// Load and analyze every env file under `root`.
pub fn load(root: &Path) -> Vec<EnvFile> {
    let paths = discover(root);
    let rels: Vec<String> = paths
        .iter()
        .map(|p| p.strip_prefix(root).unwrap_or(p).to_string_lossy().into_owned())
        .collect();
    let (tracked, ignored) = crate::git::tracked_and_ignored(root, &rels);

    let mut files: Vec<EnvFile> = paths
        .into_iter()
        .zip(rels)
        .map(|(path, rel)| {
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            let entries = parse(&text);
            let mut seen = HashSet::new();
            let mut duplicates: Vec<String> =
                entries.iter().filter(|e| !seen.insert(e.key.clone())).map(|e| e.key.clone()).collect();
            duplicates.dedup();
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            EnvFile {
                example: is_example_name(&name),
                tracked: tracked.contains(&rel),
                ignored: ignored.contains(&rel),
                path,
                rel,
                entries,
                missing: Vec::new(),
                duplicates,
            }
        })
        .collect();

    // Compare each real file with an example file in the same directory.
    let examples: Vec<(PathBuf, HashSet<String>)> = files
        .iter()
        .filter(|f| f.example)
        .map(|f| (f.path.parent().map(Path::to_path_buf).unwrap_or_default(), f.entries.iter().map(|e| e.key.clone()).collect()))
        .collect();
    for f in files.iter_mut().filter(|f| !f.example) {
        let dir = f.path.parent().map(Path::to_path_buf).unwrap_or_default();
        if let Some((_, keys)) = examples.iter().find(|(d, _)| *d == dir) {
            let have: HashSet<&str> = f.entries.iter().map(|e| e.key.as_str()).collect();
            let mut missing: Vec<String> = keys.iter().filter(|k| !have.contains(k.as_str())).cloned().collect();
            missing.sort();
            f.missing = missing;
        }
    }
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_forms() {
        let text = "# c\nA=1\nexport B=\"two words\" \nC='x # y'\nD=plain # comment\n1BAD=x\nE=\n";
        let e = parse(text);
        let kv: Vec<(&str, &str)> = e.iter().map(|e| (e.key.as_str(), e.value.as_str())).collect();
        assert_eq!(kv, vec![("A", "1"), ("B", "two words"), ("C", "x # y"), ("D", "plain"), ("E", "")]);
    }

    #[test]
    fn set_value_preserves_other_lines() {
        let text = "# keep\nexport A=1\nB=2\n";
        assert_eq!(set_value(text, "A", "new value"), "# keep\nexport A=\"new value\"\nB=2\n");
        assert_eq!(set_value(text, "C", "3"), "# keep\nexport A=1\nB=2\nC=3\n");
    }

    #[test]
    fn mask_hides_values_but_shows_changes() {
        let a = mask_text("TOKEN=secret1\n");
        let b = mask_text("TOKEN=secret2\n");
        assert!(!a.contains("secret"));
        assert_ne!(a, b);
    }

    #[test]
    fn detects_key_changes() {
        let before = key_map(&parse("A=1\nB=2\n"));
        let c = changes(&before, &parse("A=1\nB=3\nC=4\n"));
        assert_eq!(c.get("B"), Some(&KeyChange::Changed));
        assert_eq!(c.get("C"), Some(&KeyChange::Added));
        let c = changes(&before, &parse("A=1\n"));
        assert_eq!(c.get("B"), Some(&KeyChange::Removed));
    }
}
