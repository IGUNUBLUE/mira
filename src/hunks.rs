//! git's own hunks of one file (`git diff -U3`), so a single change can be
//! staged, unstaged or discarded with `git apply`. The side-by-side view is
//! computed separately (`diff.rs`); its toolbars find their hunk by the
//! lines it spans (see [`Hunk::covers`]).

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    /// The `@@ -a,b +c,d @@ heading` line, without its line break.
    pub header: String,
    /// The hunk as git printed it, header included (line breaks kept).
    text: String,
    /// The lines it spans on each side (1-based, context included).
    pub old: std::ops::Range<usize>,
    pub new: std::ops::Range<usize>,
}

impl Hunk {
    /// A changed row (its old-side number if removed, new-side if added) is in
    /// this hunk. The side-by-side rows and git's hunks don't always pick the
    /// same lines (git's indent heuristic slides them): a range finds it anyway.
    pub fn covers(&self, removed: Option<usize>, added: Option<usize>) -> bool {
        removed.is_some_and(|n| self.old.contains(&n)) || added.is_some_and(|n| self.new.contains(&n))
    }
}

/// The hunks of one file's diff, with the file header they apply under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Patch {
    /// HEAD ⇄ index (`git diff --cached`), else index ⇄ working tree.
    pub staged: bool,
    /// `diff --git …`, mode lines, `--- a/…`, `+++ b/…`.
    header: String,
    pub hunks: Vec<Hunk>,
}

impl Patch {
    /// A patch with only hunk `i`, for `git apply`.
    pub fn single(&self, i: usize) -> Option<String> {
        self.hunks.get(i).map(|h| format!("{}{}", self.header, h.text))
    }
}

/// Which hunk's toolbar goes above each row: `rows` gives each row's removed
/// (old-side) and added (new-side) line numbers, `None` for unchanged sides.
/// Rows and hunks are both in file order, so one pass places them; a hunk
/// goes above the first changed row inside it.
pub fn place(rows: impl IntoIterator<Item = (Option<usize>, Option<usize>)>, hunks: &[Hunk]) -> Vec<Option<usize>> {
    let mut next = 0;
    rows.into_iter()
        .map(|(removed, added)| {
            if removed.is_none() && added.is_none() {
                return None;
            }
            // Hunks this row is already past (none of their rows changed in them) are skipped.
            while hunks.get(next).is_some_and(|h| {
                !h.covers(removed, added)
                    && (removed.is_some_and(|n| n >= h.old.end) || added.is_some_and(|n| n >= h.new.end))
            }) {
                next += 1;
            }
            let here = hunks.get(next).is_some_and(|h| h.covers(removed, added)).then_some(next);
            if here.is_some() {
                next += 1;
            }
            here
        })
        .collect()
}

/// `-a,b` or `+c,d` (the count defaults to 1): the lines it spans.
fn range(s: &str) -> Option<std::ops::Range<usize>> {
    let mut parts = s.get(1..)?.split(',');
    let start: usize = parts.next()?.parse().ok()?;
    let count: usize = parts.next().map_or(Some(1), |c| c.parse().ok())?;
    Some(start..start + count)
}

/// Parse `git diff` output for one file. `None` for anything that is not a
/// plain text diff of one file (binary, combined conflict diff, no hunks).
pub fn parse(diff: &str, staged: bool) -> Option<Patch> {
    let mut header = String::new();
    let mut hunks: Vec<Hunk> = Vec::new();
    let mut seen_files = false;
    for line in diff.split_inclusive('\n') {
        if line.starts_with("@@ ") {
            if !seen_files {
                return None;
            }
            let mut parts = line.split(' ');
            let (_, a, b) = (parts.next(), parts.next()?, parts.next()?);
            hunks.push(Hunk {
                header: line.trim_end_matches(['\n', '\r']).to_string(),
                text: line.to_string(),
                old: range(a)?,
                new: range(b)?,
            });
            continue;
        }
        let Some(hunk) = hunks.last_mut() else {
            // The file header: one file only, never a combined (`--cc`) diff.
            if line.starts_with("diff --cc") || line.starts_with("diff --combined") {
                return None;
            }
            if line.starts_with("diff --git") && !header.is_empty() {
                return None;
            }
            if line.starts_with("+++ ") {
                seen_files = true;
            }
            header.push_str(line);
            continue;
        };
        if line.starts_with("diff --git") {
            // A second file: not what was asked for.
            return None;
        }
        hunk.text.push_str(line);
    }
    (!hunks.is_empty()).then_some(Patch { staged, header, hunks })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWO: &str = "diff --git a/f.txt b/f.txt\n\
index 1111111..2222222 100644\n\
--- a/f.txt\n\
+++ b/f.txt\n\
@@ -1,4 +1,4 @@\n \
one\n\
-two\n\
+TWO\n \
three\n \
four\n\
@@ -10,3 +10,4 @@ fn main() {\n \
ten\n \
eleven\n\
+eleven and a half\n \
twelve\n\
\\ No newline at end of file\n";

    #[test]
    fn splits_hunks_and_knows_their_lines() {
        let p = parse(TWO, false).unwrap();
        assert_eq!(p.hunks.len(), 2);
        assert_eq!((p.hunks[0].old.clone(), p.hunks[0].new.clone()), (1..5, 1..5));
        assert_eq!((p.hunks[1].old.clone(), p.hunks[1].new.clone()), (10..13, 10..14));
        // A row removed at old line 2, or added at new line 12, is in its hunk.
        assert!(p.hunks[0].covers(Some(2), Some(2)) && !p.hunks[0].covers(Some(9), None));
        assert!(p.hunks[1].covers(None, Some(12)) && !p.hunks[1].covers(None, Some(14)));
        assert_eq!(p.hunks[1].header, "@@ -10,3 +10,4 @@ fn main() {");
    }

    #[test]
    fn toolbars_land_on_the_first_changed_row_inside_each_hunk() {
        let p = parse(TWO, false).unwrap();
        // The side-by-side rows put the second change a line lower than git does
        // (a slid insertion): still in its hunk, so it gets the toolbar.
        let rows = [(None, None), (Some(2), Some(2)), (None, None), (None, None), (None, Some(13)), (None, None)];
        assert_eq!(place(rows, &p.hunks), [None, Some(0), None, None, Some(1), None]);
        // A change git doesn't see (outside every hunk) gets none, and doesn't
        // take the next hunk's.
        let rows = [(Some(7), None), (None, Some(12))];
        assert_eq!(place(rows, &p.hunks), [None, Some(1)]);
    }

    #[test]
    fn a_single_hunk_patch_keeps_the_file_header() {
        let p = parse(TWO, true).unwrap();
        let one = p.single(1).unwrap();
        assert!(one.starts_with("diff --git a/f.txt b/f.txt\nindex"));
        assert!(!one.contains("-two"));
        assert!(one.ends_with("+eleven and a half\n twelve\n\\ No newline at end of file\n"));
        assert_eq!(p.single(2), None);
    }

    #[test]
    fn counts_default_to_one_and_crlf_is_kept() {
        let d = "diff --git a/w b/w\n--- a/w\n+++ b/w\n@@ -3 +3 @@\r\n-x\r\n+y\r\n";
        let p = parse(d, false).unwrap();
        assert_eq!(p.hunks[0].old, 3..4);
        assert!(p.single(0).unwrap().ends_with("-x\r\n+y\r\n"));
        // A new file: the hunk starts at 0 on the old side, its lines at 1.
        let new = "diff --git a/n b/n\nnew file mode 100644\n--- /dev/null\n+++ b/n\n@@ -0,0 +1,2 @@\n+a\n+b\n";
        let h = &parse(new, true).unwrap().hunks[0];
        assert_eq!((h.old.clone(), h.new.clone()), (0..0, 1..3));
    }

    #[test]
    fn nothing_for_binary_conflicts_or_several_files() {
        assert_eq!(parse("", false), None);
        assert_eq!(parse("diff --git a/b b/b\nBinary files a/b and b/b differ\n", false), None);
        assert_eq!(parse("diff --cc c\n--- a/c\n+++ b/c\n@@@ -1,1 -1,1 +1,1 @@@\n", false), None);
        let two_files = format!("{TWO}diff --git a/g b/g\n--- a/g\n+++ b/g\n@@ -1 +1 @@\n-a\n+b\n");
        assert_eq!(parse(&two_files, false), None);
    }
}
