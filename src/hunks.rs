//! git's own hunks of one file (`git diff -U3`), so a single change can be
//! staged, unstaged or discarded with `git apply`. The side-by-side view is
//! computed separately (`diff.rs`); its toolbars find their hunk by the line
//! number of the hunk's first changed line.

/// Where a hunk's first change is: a removed line of the old side, or an
/// added line of the new side (1-based, as `diff::Side::number`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    Old(usize),
    New(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    /// The `@@ -a,b +c,d @@ heading` line, without its line break.
    pub header: String,
    /// The hunk as git printed it, header included (line breaks kept).
    text: String,
    pub anchor: Anchor,
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

/// `-a,b` or `+c,d` (the count defaults to 1): the start line.
fn range_start(s: &str) -> Option<usize> {
    s.get(1..)?.split(',').next()?.parse().ok()
}

/// Parse `git diff` output for one file. `None` for anything that is not a
/// plain text diff of one file (binary, combined conflict diff, no hunks).
pub fn parse(diff: &str, staged: bool) -> Option<Patch> {
    let mut header = String::new();
    let mut hunks: Vec<Hunk> = Vec::new();
    // Line counters of the current hunk, and whether its first change was seen.
    let (mut old, mut new, mut anchored) = (0usize, 0usize, false);
    let mut seen_files = false;
    for line in diff.split_inclusive('\n') {
        if line.starts_with("@@ ") {
            if !seen_files {
                return None;
            }
            let mut parts = line.split(' ');
            let (_, a, b) = (parts.next(), parts.next()?, parts.next()?);
            old = range_start(a)?;
            new = range_start(b)?;
            anchored = false;
            hunks.push(Hunk {
                header: line.trim_end_matches(['\n', '\r']).to_string(),
                text: line.to_string(),
                anchor: Anchor::Old(old),
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
        match line.as_bytes().first() {
            Some(b'-') => {
                if !anchored {
                    hunk.anchor = Anchor::Old(old);
                    anchored = true;
                }
                old += 1;
            }
            Some(b'+') => {
                if !anchored {
                    hunk.anchor = Anchor::New(new);
                    anchored = true;
                }
                new += 1;
            }
            // `\ No newline at end of file` counts on neither side.
            Some(b'\\') => {}
            _ => {
                old += 1;
                new += 1;
            }
        }
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
    fn splits_hunks_and_finds_their_first_change() {
        let p = parse(TWO, false).unwrap();
        assert_eq!(p.hunks.len(), 2);
        assert_eq!(p.hunks[0].anchor, Anchor::Old(2));
        assert_eq!(p.hunks[1].anchor, Anchor::New(12));
        assert_eq!(p.hunks[1].header, "@@ -10,3 +10,4 @@ fn main() {");
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
        assert_eq!(p.hunks[0].anchor, Anchor::Old(3));
        assert!(p.single(0).unwrap().ends_with("-x\r\n+y\r\n"));
        // A new file: the hunk starts at 0 on the old side, its lines at 1.
        let new = "diff --git a/n b/n\nnew file mode 100644\n--- /dev/null\n+++ b/n\n@@ -0,0 +1,2 @@\n+a\n+b\n";
        assert_eq!(parse(new, true).unwrap().hunks[0].anchor, Anchor::New(1));
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
