//! Workspace search & replace. Respects .gitignore via the `ignore` crate
//! (the same walker ripgrep uses).

use ignore::WalkBuilder;
use regex::{NoExpand, Regex, RegexBuilder};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Options {
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub regex: bool,
}

#[derive(Debug, Clone)]
pub struct LineMatch {
    /// 0-based line index.
    pub line: usize,
    pub text: String,
    /// Byte ranges of matches within `text`.
    pub ranges: Vec<(usize, usize)>,
}

#[derive(Debug, Clone)]
pub struct FileMatches {
    pub path: PathBuf,
    pub rel: String,
    pub lines: Vec<LineMatch>,
}

pub const MAX_MATCHES: usize = 5_000;
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_LINE_CHARS: usize = 300;

pub fn build_regex(query: &str, opts: &Options) -> Result<Regex, String> {
    let mut pattern = if opts.regex { query.to_string() } else { regex::escape(query) };
    if opts.whole_word {
        pattern = format!(r"\b(?:{pattern})\b");
    }
    RegexBuilder::new(&pattern)
        .case_insensitive(!opts.case_sensitive)
        .build()
        .map_err(|e| e.to_string())
}

fn files(root: &Path) -> impl Iterator<Item = PathBuf> {
    WalkBuilder::new(root)
        .hidden(false)
        .filter_entry(|e| e.file_name() != ".git")
        .build()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .filter(|e| e.metadata().is_ok_and(|m| m.len() <= MAX_FILE_BYTES))
        .map(|e| e.into_path())
}

fn read_text(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.iter().take(8192).any(|&b| b == 0) {
        return None;
    }
    String::from_utf8(bytes).ok()
}

pub fn search(root: &Path, query: &str, opts: &Options) -> Result<(Vec<FileMatches>, usize), String> {
    let re = build_regex(query, opts)?;
    let mut results = Vec::new();
    let mut total = 0;

    for path in files(root) {
        let Some(content) = read_text(&path) else { continue };
        if !re.is_match(&content) {
            continue;
        }
        let mut lines = Vec::new();
        for (i, line) in content.lines().enumerate() {
            let ranges: Vec<(usize, usize)> = re
                .find_iter(line)
                .filter(|m| m.end() > m.start())
                .map(|m| (m.start(), m.end()))
                .collect();
            if ranges.is_empty() {
                continue;
            }
            total += ranges.len();
            lines.push(LineMatch { line: i, text: line.to_string(), ranges });
            if total >= MAX_MATCHES {
                break;
            }
        }
        if !lines.is_empty() {
            let rel = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().into_owned();
            results.push(FileMatches { path, rel, lines });
        }
        if total >= MAX_MATCHES {
            break;
        }
    }
    results.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok((results, total))
}

/// Replace every match in the given files. Returns the number of files changed.
pub fn replace_all(files: &[PathBuf], query: &str, replacement: &str, opts: &Options) -> Result<usize, String> {
    let re = build_regex(query, opts)?;
    let mut changed = 0;
    for path in files {
        let Some(content) = read_text(path) else { continue };
        let new = if opts.regex {
            re.replace_all(&content, replacement)
        } else {
            re.replace_all(&content, NoExpand(replacement))
        };
        if new != content {
            std::fs::write(path, new.as_bytes()).map_err(|e| format!("{}: {e}", path.display()))?;
            changed += 1;
        }
    }
    Ok(changed)
}

/// Trim a long line around the first match so results stay readable.
/// Returns `(text, ranges)` adjusted to the trimmed text.
pub fn preview(m: &LineMatch) -> (String, Vec<(usize, usize)>) {
    let text = m.text.trim_end();
    let first = m.ranges.first().map(|r| r.0).unwrap_or(0);
    let lead = text.len() - text.trim_start().len();
    let mut start = first.saturating_sub(40).max(lead.min(first));
    while !text.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = text.len().min(start + MAX_LINE_CHARS);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let prefix = if start > lead { "…" } else { "" };
    let shown = format!("{prefix}{}", &text[start..end]);
    let shift = prefix.len() as isize - start as isize;
    let ranges = m
        .ranges
        .iter()
        .filter(|(s, e)| *s >= start && *e <= end)
        .map(|(s, e)| ((*s as isize + shift) as usize, (*e as isize + shift) as usize))
        .collect();
    (shown, ranges)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_and_whole_word() {
        let opts = Options { whole_word: true, ..Default::default() };
        let re = build_regex("a.b", &opts).unwrap();
        assert!(re.is_match("x a.b y"));
        assert!(!re.is_match("axb"));
    }

    #[test]
    fn preview_keeps_ranges_aligned() {
        let text = format!("{}needle end", "x".repeat(100));
        let m = LineMatch { line: 0, ranges: vec![(100, 106)], text };
        let (shown, ranges) = preview(&m);
        let (s, e) = ranges[0];
        assert_eq!(&shown[s..e], "needle");
    }
}
