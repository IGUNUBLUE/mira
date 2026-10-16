//! Lazy file tree. Only expanded directories are read from disk, and a
//! directory is only read again when its mtime changes.
//!
//! Git decorations ([`Marks`]) are computed once per git snapshot and copied
//! onto the entries when either changes, so the view only reads fields.

use crate::git::Snapshot;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

#[derive(Debug, Clone)]
pub struct Entry {
    pub path: PathBuf,
    /// Path relative to the workspace root, `/`-separated.
    pub rel: String,
    pub name: String,
    pub is_dir: bool,
    pub depth: usize,
    pub expanded: bool,
    /// Git status letter: the file's, or the most important below a directory.
    pub status: Option<char>,
    /// Ignored by git (itself or a parent directory).
    pub ignored: bool,
}

/// Git decorations of the opened folder, keyed by paths relative to it.
#[derive(Debug, Default)]
pub struct Marks {
    files: HashMap<String, char>,
    /// Aggregate status per directory with changes below it.
    dirs: HashMap<String, char>,
    ignored_dirs: HashSet<String>,
    ignored_files: HashSet<String>,
}

impl Marks {
    /// Decorations from `snap`; `prefix` is the opened folder inside the
    /// repository (`""` at the top, else with a trailing `/`).
    pub fn new(snap: &Snapshot, prefix: &str) -> Self {
        let mut files = HashMap::new();
        // Unstaged after staged: a file changed in both shows its working-tree state.
        for c in snap.staged.iter().chain(snap.unstaged.iter()) {
            if let Some(rel) = c.path.strip_prefix(prefix) {
                files.insert(rel.to_string(), c.code);
            }
        }
        let mut dirs: HashMap<String, char> = HashMap::new();
        for (path, &code) in &files {
            for (i, _) in path.match_indices('/') {
                let dir = &path[..i];
                match dirs.get_mut(dir) {
                    Some(c) => *c = more_important(*c, code),
                    None => {
                        dirs.insert(dir.to_string(), code);
                    }
                }
            }
        }
        let (mut ignored_dirs, mut ignored_files) = (HashSet::new(), HashSet::new());
        for i in &snap.ignored {
            let Some(rel) = i.strip_prefix(prefix) else { continue };
            match rel.strip_suffix('/') {
                Some(dir) => ignored_dirs.insert(dir.to_string()),
                None => ignored_files.insert(rel.to_string()),
            };
        }
        Self { files, dirs, ignored_dirs, ignored_files }
    }

    fn status(&self, rel: &str, is_dir: bool) -> Option<char> {
        if is_dir { self.dirs.get(rel) } else { self.files.get(rel) }.copied()
    }

    /// Whether `rel` (or one of its parents) is ignored.
    fn ignored(&self, rel: &str) -> bool {
        if self.ignored_files.contains(rel) || self.ignored_dirs.contains(rel) {
            return true;
        }
        !self.ignored_dirs.is_empty() && rel.match_indices('/').any(|(i, _)| self.ignored_dirs.contains(&rel[..i]))
    }
}

/// The status a directory shows: conflict, modified, added/renamed, then the
/// rest; equal ranks pick the smaller letter, so it never depends on map order.
fn more_important(a: char, b: char) -> char {
    let rank = |c: char| match c {
        '!' => 4,
        'M' => 3,
        'A' | 'R' => 2,
        _ => 1,
    };
    if (rank(b), std::cmp::Reverse(b)) > (rank(a), std::cmp::Reverse(a)) { b } else { a }
}

/// One directory as last read: `(path, name, is_dir)` sorted for display.
struct Listing {
    mtime: SystemTime,
    items: Vec<(PathBuf, String, bool)>,
}

/// A listing whose directory changed this recently is read again anyway: a
/// change within the filesystem's timestamp granularity keeps the same mtime.
const SETTLE: Duration = Duration::from_secs(2);

pub struct Explorer {
    pub root: PathBuf,
    pub expanded: HashSet<PathBuf>,
    pub entries: Vec<Entry>,
    marks: Marks,
    listings: HashMap<PathBuf, Listing>,
}

impl Explorer {
    pub fn new(root: PathBuf) -> Self {
        let mut e = Self {
            root,
            expanded: HashSet::new(),
            entries: Vec::new(),
            marks: Marks::default(),
            listings: HashMap::new(),
        };
        e.refresh();
        e
    }

    pub fn toggle(&mut self, dir: &Path) {
        if !self.expanded.remove(dir) {
            self.expanded.insert(dir.to_path_buf());
        }
        self.refresh();
    }

    /// New git decorations (after a snapshot changed).
    pub fn set_marks(&mut self, marks: Marks) {
        self.marks = marks;
        self.annotate();
    }

    pub fn refresh(&mut self) {
        self.expanded.retain(|p| p.is_dir());
        let mut old = std::mem::take(&mut self.listings);
        let mut entries = Vec::new();
        let root = self.root.clone();
        self.walk(&mut old, &root, 0, &mut entries);
        self.entries = entries;
        self.annotate();
    }

    fn annotate(&mut self) {
        for e in &mut self.entries {
            e.status = self.marks.status(&e.rel, e.is_dir);
            e.ignored = self.marks.ignored(&e.rel);
        }
    }

    /// Append `dir`'s entries (and those of its expanded subdirectories),
    /// reusing a listing from `old` when the directory didn't change.
    fn walk(&mut self, old: &mut HashMap<PathBuf, Listing>, dir: &Path, depth: usize, out: &mut Vec<Entry>) {
        let Some(listing) = old.remove(dir).filter(|l| unchanged(dir, l.mtime)).or_else(|| read_listing(dir)) else {
            return;
        };
        for (path, name, is_dir) in &listing.items {
            let rel = path.strip_prefix(&self.root).map(|p| p.to_string_lossy().replace('\\', "/")).unwrap_or_default();
            let expanded = *is_dir && self.expanded.contains(path);
            out.push(Entry {
                path: path.clone(),
                rel,
                name: name.clone(),
                is_dir: *is_dir,
                depth,
                expanded,
                status: None,
                ignored: false,
            });
            if expanded {
                self.walk(old, path, depth + 1, out);
            }
        }
        self.listings.insert(dir.to_path_buf(), listing);
    }
}

fn mtime(dir: &Path) -> Option<SystemTime> {
    std::fs::metadata(dir).and_then(|m| m.modified()).ok()
}

/// `dir` still has mtime `was`, and that is old enough to trust.
fn unchanged(dir: &Path, was: SystemTime) -> bool {
    mtime(dir) == Some(was) && SystemTime::now().duration_since(was).is_ok_and(|age| age >= SETTLE)
}

fn read_listing(dir: &Path) -> Option<Listing> {
    // The mtime before reading: a change during the read shows up next time.
    let mtime = mtime(dir)?;
    let read = std::fs::read_dir(dir).ok()?;
    let mut items: Vec<(PathBuf, String, bool)> = read
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if name == ".git" {
                return None;
            }
            let kind = e.file_type().ok()?;
            let is_dir = kind.is_dir() || (kind.is_symlink() && e.path().is_dir());
            Some((e.path(), name, is_dir))
        })
        .collect();
    // Directories first, then by name, ignoring case.
    items.sort_by_cached_key(|(_, name, is_dir)| (!*is_dir, name.to_lowercase()));
    Some(Listing { mtime, items })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::Change;

    fn snap(changes: &[(&str, char)], ignored: &[&str]) -> Snapshot {
        Snapshot {
            unstaged: changes.iter().map(|(p, c)| Change::new(*p, *c)).collect(),
            ignored: ignored.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn ignored_matches_the_path_or_a_parent_dir() {
        let m = Marks::new(&snap(&[], &["target/", "notes.txt"]), "");
        assert!(m.ignored("target"));
        assert!(m.ignored("target/debug/mira"));
        assert!(m.ignored("notes.txt"));
        assert!(!m.ignored("targets"));
        assert!(!m.ignored("src/notes.txt"));
        // Inside a subfolder of the repository, paths are relative to it.
        let m = Marks::new(&snap(&[], &["sub/out/", "other/x"]), "sub/");
        assert!(m.ignored("out/a") && !m.ignored("other/x") && !m.ignored("sub"));
    }

    #[test]
    fn dir_status_picks_the_most_important_change() {
        let changes =
            [("src/a.rs", 'U'), ("src/b.rs", 'M'), ("src/c.rs", 'A'), ("src/x/d.rs", 'D'), ("srcx/e.rs", '!')];
        let m = Marks::new(&snap(&changes, &[]), "");
        assert_eq!(m.status("src", true), Some('M'));
        assert_eq!(m.status("src/x", true), Some('D'));
        assert_eq!(m.status("srcx", true), Some('!'));
        assert_eq!(m.status("docs", true), None);
        assert_eq!(m.status("src/a.rs", false), Some('U'));
        let m = Marks::new(&snap(&[("d/a", 'D'), ("d/b", 'U')], &[]), "");
        assert_eq!(m.status("d", true), Some('D'));
        // A file changed in the index and the tree shows the tree's letter.
        let mut both = snap(&[("f", 'M')], &[]);
        both.staged = vec![Change::new("f", 'A')];
        assert_eq!(Marks::new(&both, "").status("f", false), Some('M'));
    }

    #[test]
    fn entries_are_sorted_annotated_and_follow_the_disk() {
        let dir = crate::files::test_dir("explorer");
        std::fs::create_dir_all(dir.join("Sub")).unwrap();
        std::fs::write(dir.join("b.txt"), "").unwrap();
        std::fs::write(dir.join("A.txt"), "").unwrap();
        std::fs::write(dir.join("Sub/c.txt"), "").unwrap();
        let mut e = Explorer::new(dir.clone());
        let names = |e: &Explorer| e.entries.iter().map(|x| x.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(&e), ["Sub", "A.txt", "b.txt"]);
        e.toggle(&dir.join("Sub"));
        assert_eq!(names(&e), ["Sub", "c.txt", "A.txt", "b.txt"]);
        assert!(e.entries[0].expanded && e.entries[1].depth == 1);
        e.set_marks(Marks::new(&snap(&[("Sub/c.txt", 'M')], &["b.txt"]), ""));
        assert_eq!((e.entries[0].status, e.entries[1].status), (Some('M'), Some('M')));
        assert!(e.entries[3].ignored && !e.entries[2].ignored);
        // A new file shows on the next refresh (the directory just changed).
        std::fs::write(dir.join("Sub/d.txt"), "").unwrap();
        e.refresh();
        assert_eq!(names(&e), ["Sub", "c.txt", "d.txt", "A.txt", "b.txt"]);
        assert_eq!(e.entries[1].status, Some('M'), "marks kept across refreshes");
        // An old, unchanged mtime reuses the listing instead of reading the directory.
        let old = SystemTime::now() - Duration::from_secs(3600);
        let set_mtime = |t| std::fs::File::open(dir.join("Sub")).unwrap().set_modified(t).unwrap();
        set_mtime(old);
        e.refresh();
        std::fs::write(dir.join("Sub/e.txt"), "").unwrap();
        set_mtime(old);
        e.refresh();
        assert_eq!(e.entries.len(), 5, "cached listing reused");
        set_mtime(SystemTime::now() - Duration::from_secs(60));
        e.refresh();
        assert_eq!(e.entries.len(), 6, "re-read once the mtime moved");
        let _ = std::fs::remove_dir_all(dir);
    }
}
