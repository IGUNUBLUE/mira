//! Find in the open file (Ctrl+F): literal matches line by line, with the
//! byte columns iced's editor uses (cosmic-text indexes a line by bytes), and
//! which match to go to from the cursor.

use regex::RegexBuilder;

/// Matches kept per file; the count shows `+` past it.
pub const MAX_MATCHES: usize = 10_000;

/// One occurrence: `line` (0-based) and its byte range `start..end` in that line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Match {
    pub line: usize,
    pub start: usize,
    pub end: usize,
}

/// Every non-overlapping occurrence of `query` (literal, case-insensitive
/// unless `case_sensitive`) in `lines`, in order, at most [`MAX_MATCHES`].
/// Returns the matches and whether they stopped at the cap. A match never
/// spans lines, so a query with a line break finds nothing.
pub fn find<S: AsRef<str>>(
    lines: impl IntoIterator<Item = S>,
    query: &str,
    case_sensitive: bool,
) -> (Vec<Match>, bool) {
    let mut out = Vec::new();
    if query.is_empty() {
        return (out, false);
    }
    // Escaped, so it can only fail on a query too large for a regex: nothing to find then.
    let Ok(re) = RegexBuilder::new(&regex::escape(query)).case_insensitive(!case_sensitive).build() else {
        return (out, false);
    };
    for (line, text) in lines.into_iter().enumerate() {
        for m in re.find_iter(text.as_ref()) {
            if out.len() >= MAX_MATCHES {
                return (out, true);
            }
            out.push(Match { line, start: m.start(), end: m.end() });
        }
    }
    (out, false)
}

/// The match to go to from `at` (line, byte column), wrapping around.
/// Forward: the first match starting at `at` or after it (strictly after
/// with `skip_at`, so the selected match moves on). Backward: the last one
/// starting before `at`.
pub fn pick(matches: &[Match], at: (usize, usize), forward: bool, skip_at: bool) -> Option<usize> {
    if matches.is_empty() {
        return None;
    }
    let key = |m: &Match| (m.line, m.start);
    if forward {
        let i =
            if skip_at { matches.partition_point(|m| key(m) <= at) } else { matches.partition_point(|m| key(m) < at) };
        Some(if i == matches.len() { 0 } else { i })
    } else {
        let i = matches.partition_point(|m| key(m) < at);
        Some(if i == 0 { matches.len() - 1 } else { i - 1 })
    }
}

/// The match that is exactly the selection `line`, `start..end`, if any.
pub fn selected(matches: &[Match], line: usize, start: usize, end: usize) -> Option<usize> {
    matches.binary_search(&Match { line, start, end }).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(line: usize, start: usize, end: usize) -> Match {
        Match { line, start, end }
    }

    #[test]
    fn literal_and_case_insensitive_by_default() {
        let lines = ["Foo foo", "a.b axb", "FOO"];
        let (found, capped) = find(lines, "foo", false);
        assert!(!capped);
        assert_eq!(found, [m(0, 0, 3), m(0, 4, 7), m(2, 0, 3)]);
        assert_eq!(find(lines, "foo", true).0, [m(0, 4, 7)]);
        // Literal: the dot is a dot.
        assert_eq!(find(lines, "a.b", false).0, [m(1, 0, 3)]);
        assert!(find(lines, "", false).0.is_empty());
        assert!(find(lines, "foo\nfoo", false).0.is_empty());
    }

    #[test]
    fn columns_are_bytes_within_the_line() {
        // "é" is two bytes: the editor's columns count bytes too.
        let (found, _) = find(["café café"], "CAFÉ", false);
        assert_eq!(found, [m(0, 0, 5), m(0, 6, 11)]);
        assert_eq!(&"café café"[6..11], "café");
    }

    #[test]
    fn stops_at_the_cap() {
        let line = "a".repeat(MAX_MATCHES + 5);
        let (found, capped) = find([line.as_str()], "a", true);
        assert_eq!((found.len(), capped), (MAX_MATCHES, true));
    }

    #[test]
    fn next_and_previous_start_from_the_cursor_and_wrap() {
        let ms = [m(0, 4, 7), m(2, 0, 3), m(2, 8, 11), m(5, 1, 4)];
        // From a plain cursor: the match at it, else the next one.
        assert_eq!(pick(&ms, (2, 0), true, false), Some(1));
        assert_eq!(pick(&ms, (1, 9), true, false), Some(1));
        // The selected match moves on.
        assert_eq!(pick(&ms, (2, 0), true, true), Some(2));
        // Past the last one: wrap to the first.
        assert_eq!(pick(&ms, (5, 1), true, true), Some(0));
        assert_eq!(pick(&ms, (9, 0), true, false), Some(0));
        // Backward: the last one before the cursor, wrapping to the end.
        assert_eq!(pick(&ms, (2, 8), false, false), Some(1));
        assert_eq!(pick(&ms, (0, 4), false, false), Some(3));
        assert_eq!(pick(&[], (0, 0), true, false), None);
    }

    #[test]
    fn finds_the_selected_match() {
        let ms = [m(0, 4, 7), m(2, 0, 3)];
        assert_eq!(selected(&ms, 2, 0, 3), Some(1));
        assert_eq!(selected(&ms, 2, 0, 2), None);
    }
}
