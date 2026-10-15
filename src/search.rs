//! Workspace search & replace. Respects .gitignore via the `ignore` crate
//! (the same walker ripgrep uses).

use ignore::WalkBuilder;
use regex::{Captures, Regex, RegexBuilder};
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
    /// The line around its first match, at most [`MAX_LINE_CHARS`] bytes
    /// (a minified file's single line is never kept whole).
    pub preview: String,
    /// Byte ranges of the matches shown in `preview`.
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

/// Results are shown line by line, so `^`/`$` match at line starts and ends
/// (multi-line mode, `\r\n` aware) both when searching and when replacing.
pub fn build_regex(query: &str, opts: &Options) -> Result<Regex, String> {
    let mut pattern = if opts.regex { query.to_string() } else { regex::escape(query) };
    if opts.whole_word {
        pattern = format!(r"\b(?:{pattern})\b");
    }
    RegexBuilder::new(&pattern)
        .case_insensitive(!opts.case_sensitive)
        .multi_line(true)
        .crlf(true)
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
    crate::files::read_text(path, MAX_FILE_BYTES).ok()
}

/// Search the files under `root`. `cancelled` is checked before each file: a
/// newer search stops this one instead of both walking the tree.
pub fn search(
    root: &Path,
    query: &str,
    opts: &Options,
    cancelled: impl Fn() -> bool,
) -> Result<(Vec<FileMatches>, usize), String> {
    let re = build_regex(query, opts)?;
    let mut results = Vec::new();
    let mut total = 0;

    for path in files(root) {
        if cancelled() {
            return Err("cancelled".into());
        }
        let Some(content) = read_text(&path) else { continue };
        if !re.is_match(&content) {
            continue;
        }
        let mut lines = Vec::new();
        for (i, line) in content.lines().enumerate() {
            let ranges: Vec<(usize, usize)> =
                re.find_iter(line).filter(|m| m.end() > m.start()).map(|m| (m.start(), m.end())).collect();
            if ranges.is_empty() {
                continue;
            }
            total += ranges.len();
            let (preview, ranges) = preview(line, &ranges);
            lines.push(LineMatch { line: i, preview, ranges });
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
/// Works line by line, exactly like the search results: the preview shows what
/// gets replaced (no match spans lines, empty matches are left alone).
pub fn replace_all(files: &[PathBuf], query: &str, replacement: &str, opts: &Options) -> Result<usize, String> {
    let re = build_regex(query, opts)?;
    let mut changed = 0;
    for path in files {
        let Some(content) = read_text(path) else { continue };
        let new = replace_lines(&re, &content, replacement, opts.regex);
        if new != content {
            std::fs::write(path, new.as_bytes()).map_err(|e| format!("{}: {e}", path.display()))?;
            changed += 1;
        }
    }
    Ok(changed)
}

fn replace_lines(re: &Regex, content: &str, replacement: &str, expand: bool) -> String {
    let mut out = String::with_capacity(content.len());
    for line in content.split_inclusive('\n') {
        let body = line.strip_suffix('\n').map_or(line, |b| b.strip_suffix('\r').unwrap_or(b));
        let replaced = re.replace_all(body, |caps: &Captures<'_>| {
            let whole = caps.get(0).map_or("", |m| m.as_str());
            if whole.is_empty() {
                String::new()
            } else if expand {
                let mut s = String::new();
                caps.expand(replacement, &mut s);
                s
            } else {
                replacement.to_string()
            }
        });
        out.push_str(&replaced);
        out.push_str(&line[body.len()..]);
    }
    out
}

/// Trim a long line around the first match so results stay readable.
/// Returns `(text, ranges)` adjusted to the trimmed text.
fn preview(line: &str, ranges: &[(usize, usize)]) -> (String, Vec<(usize, usize)>) {
    let text = line.trim_end();
    let first = ranges.first().map(|r| r.0).unwrap_or(0);
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
    let ranges = ranges
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
    fn anchors_match_per_line_in_search_and_replace() {
        let dir = crate::files::test_dir("search");
        let file = dir.join("f.txt");
        std::fs::write(&file, "a\nx\r\nax\nx").unwrap();
        let opts = Options { regex: true, case_sensitive: true, ..Default::default() };
        let (found, total) = search(&dir, "^x$", &opts, || false).unwrap();
        assert_eq!(total, 2);
        assert_eq!(found[0].lines.iter().map(|l| l.line).collect::<Vec<_>>(), [1, 3]);
        assert_eq!(replace_all(std::slice::from_ref(&file), "^x$", "y", &opts), Ok(1));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "a\ny\r\nax\ny");
        // A match across lines is not in the results, so it is not replaced either.
        assert_eq!(replace_all(std::slice::from_ref(&file), "a\\ny", "z", &opts), Ok(0));
        // Capture groups expand only in regex mode.
        assert_eq!(replace_all(std::slice::from_ref(&file), "(a)x", "$1$1", &opts), Ok(1));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "a\ny\r\naa\ny");
        let literal = Options { case_sensitive: true, ..Default::default() };
        assert_eq!(replace_all(std::slice::from_ref(&file), "aa", "$1", &literal), Ok(1));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "a\ny\r\n$1\ny");
        // Trailing whitespace: what search shows per line is what gets replaced.
        std::fs::write(&file, "foo  \n\nbar\t\r\nbaz").unwrap();
        let (found, total) = search(&dir, "\\s+$", &opts, || false).unwrap();
        assert_eq!((total, found[0].lines.len()), (2, 2));
        assert_eq!(replace_all(std::slice::from_ref(&file), "\\s+$", "", &opts), Ok(1));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "foo\n\nbar\r\nbaz");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn preview_keeps_ranges_aligned() {
        let text = format!("{}needle end", "x".repeat(100));
        let (shown, ranges) = preview(&text, &[(100, 106)]);
        let (s, e) = ranges[0];
        assert_eq!(&shown[s..e], "needle");
    }

    #[test]
    fn results_keep_a_bounded_window_and_stop_when_cancelled() {
        let dir = crate::files::test_dir("search-window");
        std::fs::write(dir.join("min.js"), format!("{}needle{}", "a".repeat(10_000), "b".repeat(10_000))).unwrap();
        let (found, _) = search(&dir, "needle", &Options::default(), || false).unwrap();
        let m = &found[0].lines[0];
        assert!(m.preview.len() <= MAX_LINE_CHARS + "…".len(), "{} bytes", m.preview.len());
        let (s, e) = m.ranges[0];
        assert_eq!(&m.preview[s..e], "needle");
        assert!(search(&dir, "needle", &Options::default(), || true).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
