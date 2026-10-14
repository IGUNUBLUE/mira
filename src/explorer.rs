//! Lazy file tree. Only expanded directories are read from disk.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Entry {
    pub path: PathBuf,
    /// Path relative to the workspace root, `/`-separated.
    pub rel: String,
    pub name: String,
    pub is_dir: bool,
    pub depth: usize,
}

pub struct Explorer {
    pub root: PathBuf,
    pub expanded: HashSet<PathBuf>,
    pub entries: Vec<Entry>,
}

impl Explorer {
    pub fn new(root: PathBuf) -> Self {
        let mut e = Self { root, expanded: HashSet::new(), entries: Vec::new() };
        e.refresh();
        e
    }

    pub fn toggle(&mut self, dir: &Path) {
        if !self.expanded.remove(dir) {
            self.expanded.insert(dir.to_path_buf());
        }
        self.refresh();
    }

    pub fn refresh(&mut self) {
        self.expanded.retain(|p| p.is_dir());
        let mut entries = Vec::new();
        self.walk(&self.root.clone(), 0, &mut entries);
        self.entries = entries;
    }

    fn walk(&self, dir: &Path, depth: usize, out: &mut Vec<Entry>) {
        let Ok(read) = std::fs::read_dir(dir) else { return };
        let mut items: Vec<(PathBuf, String, bool)> = read
            .filter_map(Result::ok)
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                if name == ".git" {
                    return None;
                }
                let is_dir = e.file_type().ok()?.is_dir()
                    || (e.file_type().ok()?.is_symlink() && e.path().is_dir());
                Some((e.path(), name, is_dir))
            })
            .collect();
        items.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase())));

        for (path, name, is_dir) in items {
            let rel = path
                .strip_prefix(&self.root)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            let expanded = is_dir && self.expanded.contains(&path);
            out.push(Entry { path: path.clone(), rel, name, is_dir, depth });
            if expanded {
                self.walk(&path, depth + 1, out);
            }
        }
    }
}

/// Whether `rel` (or one of its parents) is in the git-ignored list.
pub fn is_ignored(rel: &str, ignored: &[String]) -> bool {
    ignored.iter().any(|i| match i.strip_suffix('/') {
        Some(dir) => rel == dir || rel.starts_with(i.as_str()),
        None => rel == i,
    })
}

/// Aggregate status of a directory from its changed descendants.
pub fn dir_status(rel: &str, decorations: &std::collections::HashMap<String, char>) -> Option<char> {
    let prefix = format!("{rel}/");
    decorations
        .iter()
        .find(|(p, _)| p.starts_with(&prefix))
        .map(|(_, c)| *c)
}
