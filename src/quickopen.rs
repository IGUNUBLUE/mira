//! Quick open (Ctrl+P): the folder's files and a fuzzy match on their paths.

use std::path::Path;

/// Files listed when the palette opens; a larger tree is cut here.
pub const MAX_FILES: usize = 20_000;

/// Paths of the files under `root`, relative and with `/`, sorted; the same
/// walk as Search (.gitignore respected, `.git` skipped). `true` when the
/// list stopped at [`MAX_FILES`].
pub fn list_files(root: &Path) -> (Vec<String>, bool) {
    let mut files = Vec::new();
    let mut capped = false;
    for entry in crate::search::walk(root) {
        if files.len() >= MAX_FILES {
            capped = true;
            break;
        }
        let path = entry.path();
        let rel = path.strip_prefix(root).unwrap_or(path).to_string_lossy();
        files.push(rel.replace(std::path::MAIN_SEPARATOR, "/"));
    }
    files.sort_unstable();
    (files, capped)
}

fn fold(c: char) -> char {
    if c.is_ascii() { c.to_ascii_lowercase() } else { c.to_lowercase().next().unwrap_or(c) }
}

/// A match right after a separator or at a camelCase hump.
fn boundary(before: Option<char>, c: char) -> bool {
    match before {
        None => true,
        Some(b) => matches!(b, '/' | '_' | '-' | '.' | ' ') || (b.is_lowercase() && c.is_uppercase()),
    }
}

/// Greedy subsequence match of `query` in `path[from..]`, case-insensitive;
/// `None` if some query character is missing. Spaces in the query are ignored.
fn subsequence(query: &str, path: &str, from: usize) -> Option<i64> {
    let name_at = path.rfind('/').map_or(0, |i| i + 1);
    let mut chars = path[from..].char_indices().map(|(i, c)| (i + from, c));
    let mut before = path[..from].chars().next_back();
    let mut prev_end = None;
    let mut score = 0;
    for q in query.chars().filter(|c| !c.is_whitespace()).map(fold) {
        loop {
            let (i, c) = chars.next()?;
            let b = before.replace(c);
            if fold(c) != q {
                continue;
            }
            score += 1;
            if prev_end == Some(i) {
                score += 5;
            }
            if i == name_at {
                score += 12;
            } else if boundary(b, c) {
                score += 4;
            }
            prev_end = Some(i + c.len_utf8());
            break;
        }
    }
    Some(score)
}

/// How well `query` matches `path` (higher is better), or `None` when it is
/// not a subsequence of it. Consecutive characters, the start of the file
/// name and word starts score; a match inside the file name beats one spread
/// over the directories; longer paths lose a little.
pub fn score(query: &str, path: &str) -> Option<i64> {
    let name_at = path.rfind('/').map_or(0, |i| i + 1);
    let matched = match subsequence(query, path, name_at) {
        Some(s) => s + 10,
        None => subsequence(query, path, 0)?,
    };
    Some(matched - (path.len() / 8) as i64)
}

/// Indices of the best `limit` matches of `query` in `paths`: by score, then
/// shorter path, then name.
pub fn rank(query: &str, paths: &[String], limit: usize) -> Vec<usize> {
    let mut hits: Vec<(i64, usize)> =
        paths.iter().enumerate().filter_map(|(i, p)| score(query, p).map(|s| (s, i))).collect();
    let order = |a: &(i64, usize), b: &(i64, usize)| {
        b.0.cmp(&a.0).then(paths[a.1].len().cmp(&paths[b.1].len())).then(paths[a.1].cmp(&paths[b.1]))
    };
    if hits.len() > limit {
        hits.select_nth_unstable_by(limit, order);
        hits.truncate(limit);
    }
    hits.sort_unstable_by(order);
    hits.into_iter().map(|(_, i)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ranked<'a>(query: &str, paths: &'a [String]) -> Vec<&'a str> {
        rank(query, paths, 10).into_iter().map(|i| paths[i].as_str()).collect()
    }

    fn paths(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn subsequence_case_insensitive_or_nothing() {
        assert!(score("mrs", "src/main.rs").is_some());
        assert!(score("MAIN", "src/main.rs").is_some());
        assert!(score("main rs", "src/main.rs").is_some());
        assert!(score("xyz", "src/main.rs").is_none());
        // Order matters.
        assert!(score("sniam", "src/main.rs").is_none());
    }

    #[test]
    fn consecutive_characters_beat_scattered_ones() {
        let p = paths(&["src/a_b_c_d.rs", "src/abcd.rs"]);
        assert_eq!(ranked("abcd", &p)[0], "src/abcd.rs");
    }

    #[test]
    fn file_name_start_beats_a_directory_match() {
        let p = paths(&["view/old.rs", "src/view.rs"]);
        assert_eq!(ranked("view", &p)[0], "src/view.rs");
        // Even when the directory match comes first in the path.
        let p = paths(&["diff/src/a.rs", "src/app/diff.rs"]);
        assert_eq!(ranked("diff", &p)[0], "src/app/diff.rs");
    }

    #[test]
    fn shorter_paths_first() {
        let p = paths(&["docs/deep/nested/folder/mod.rs", "src/mod.rs", "mod.rs"]);
        assert_eq!(ranked("mod", &p), ["mod.rs", "src/mod.rs", "docs/deep/nested/folder/mod.rs"]);
        // An empty query lists everything, shortest first.
        assert_eq!(ranked("", &p)[0], "mod.rs");
    }

    #[test]
    fn keeps_the_best_within_the_limit() {
        let p: Vec<String> = (0..50).map(|i| format!("{}/file{i}.rs", "d".repeat(i % 7))).collect();
        let top = rank("file", &p, 5);
        assert_eq!(top.len(), 5);
        let all = rank("file", &p, 100);
        assert_eq!(top, all[..5]);
    }

    #[test]
    fn lists_files_like_search() {
        let dir = crate::files::test_dir("quickopen");
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::create_dir_all(dir.join("target")).unwrap();
        std::fs::write(dir.join("src/main.rs"), "").unwrap();
        std::fs::write(dir.join(".env"), "").unwrap();
        std::fs::write(dir.join("target/out.bin"), "").unwrap();
        std::fs::write(dir.join(".gitignore"), "target/\n").unwrap();
        // .gitignore applies inside a repository.
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        let (files, capped) = list_files(&dir);
        assert!(!capped);
        assert_eq!(files, [".env", ".gitignore", "src/main.rs"]);
        let _ = std::fs::remove_dir_all(dir);
    }
}
