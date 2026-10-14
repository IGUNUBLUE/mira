//! Side-by-side diff model with intra-line emphasis.

use similar::{Algorithm, ChangeTag, TextDiff};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Equal,
    Delete,
    Insert,
    /// Padding on the side that has no counterpart line.
    Empty,
}

#[derive(Debug, Clone)]
pub struct Side {
    pub number: Option<usize>,
    pub kind: Kind,
    /// `(emphasized, text)` segments.
    pub segments: Vec<(bool, String)>,
}

impl Side {
    fn empty() -> Self {
        Self { number: None, kind: Kind::Empty, segments: Vec::new() }
    }
}

#[derive(Debug, Clone)]
pub enum Item {
    Row { left: Side, right: Side },
    /// Unchanged lines hidden in hunk mode.
    Gap(usize),
}

#[derive(Debug, Clone, Default)]
pub struct FileDiff {
    pub items: Vec<Item>,
    pub added: usize,
    pub removed: usize,
    pub binary: bool,
}

/// Lines in a file larger than this are diffed without inline emphasis.
const MAX_INLINE_BYTES: usize = 512 * 1024;

pub fn compute(old: &str, new: &str, context: Option<usize>) -> FileDiff {
    if old.contains('\0') || new.contains('\0') {
        return FileDiff { binary: true, ..Default::default() };
    }

    let diff = TextDiff::configure()
        .algorithm(Algorithm::Patience)
        .timeout(Duration::from_millis(800))
        .diff_lines(old, new);
    let inline = old.len() + new.len() < MAX_INLINE_BYTES;

    let groups = match context {
        Some(n) => diff.grouped_ops(n),
        None => vec![diff.ops().to_vec()],
    };
    let old_total = old.lines().count();

    let mut out = FileDiff::default();
    let mut old_cursor = 0;

    for group in &groups {
        let Some(first) = group.first() else { continue };
        let start = first.old_range().start;
        if start > old_cursor {
            out.items.push(Item::Gap(start - old_cursor));
        }

        for op in group {
            let mut dels = Vec::new();
            let mut ins = Vec::new();

            let mut push = |tag: ChangeTag, old_i: Option<usize>, new_i: Option<usize>, segs: Vec<(bool, String)>| {
                match tag {
                    ChangeTag::Equal => out.items.push(Item::Row {
                        left: Side { number: old_i.map(|i| i + 1), kind: Kind::Equal, segments: segs.clone() },
                        right: Side { number: new_i.map(|i| i + 1), kind: Kind::Equal, segments: segs },
                    }),
                    ChangeTag::Delete => dels.push(Side { number: old_i.map(|i| i + 1), kind: Kind::Delete, segments: segs }),
                    ChangeTag::Insert => ins.push(Side { number: new_i.map(|i| i + 1), kind: Kind::Insert, segments: segs }),
                }
            };

            if inline {
                for change in diff.iter_inline_changes(op) {
                    let segs = strip_newline(change.iter_strings_lossy().map(|(e, s)| (e, s.into_owned())).collect());
                    push(change.tag(), change.old_index(), change.new_index(), segs);
                }
            } else {
                for change in diff.iter_changes(op) {
                    let segs = strip_newline(vec![(false, change.to_string_lossy().into_owned())]);
                    push(change.tag(), change.old_index(), change.new_index(), segs);
                }
            }

            out.removed += dels.len();
            out.added += ins.len();
            let n = dels.len().max(ins.len());
            let mut dels = dels.into_iter();
            let mut ins = ins.into_iter();
            for _ in 0..n {
                out.items.push(Item::Row {
                    left: dels.next().unwrap_or_else(Side::empty),
                    right: ins.next().unwrap_or_else(Side::empty),
                });
            }
        }

        if let Some(last) = group.last() {
            old_cursor = last.old_range().end;
        }
    }

    if context.is_some() && old_cursor < old_total && !out.items.is_empty() {
        out.items.push(Item::Gap(old_total - old_cursor));
    }
    out
}

fn strip_newline(mut segs: Vec<(bool, String)>) -> Vec<(bool, String)> {
    while let Some((_, last)) = segs.last_mut() {
        let trimmed = last.trim_end_matches(['\n', '\r']).len();
        last.truncate(trimmed);
        if last.is_empty() && segs.len() > 1 {
            segs.pop();
        } else {
            break;
        }
    }
    // Tabs render inconsistently across fonts; expand them.
    for (_, s) in &mut segs {
        if s.contains('\t') {
            *s = s.replace('\t', "    ");
        }
    }
    segs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs_replacements_side_by_side() {
        let d = compute("a\nb\nc\n", "a\nB\nc\nd\n", None);
        assert_eq!((d.added, d.removed), (2, 1));
        let rows: Vec<_> = d
            .items
            .iter()
            .filter_map(|i| match i {
                Item::Row { left, right } => Some((left.kind, right.kind)),
                _ => None,
            })
            .collect();
        assert_eq!(
            rows,
            vec![
                (Kind::Equal, Kind::Equal),
                (Kind::Delete, Kind::Insert),
                (Kind::Equal, Kind::Equal),
                (Kind::Empty, Kind::Insert),
            ]
        );
    }

    #[test]
    fn hunk_mode_collapses_unchanged_lines() {
        let old: String = (0..50).map(|i| format!("{i}\n")).collect();
        let new = old.replace("25\n", "x\n");
        let d = compute(&old, &new, Some(2));
        assert!(matches!(d.items.first(), Some(Item::Gap(23))));
        assert!(matches!(d.items.last(), Some(Item::Gap(22))));
    }
}
