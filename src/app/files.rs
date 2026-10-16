//! The Files panel's context menu (right click) and its file operations:
//! new file or folder and rename (typed in place, as in VSCodium), delete to
//! the trash with Undo, copy the path, send it to the agent.

use super::{App, DiffSource, Msg, Tab, blocking, file_name};
use crate::fileops::{self, Trashed};

use iced::{Point, Task};
use std::path::{Path, PathBuf};

/// Widget id of the name input (new file or folder, rename).
pub const NAME_INPUT: &str = "explorer-name";

/// The open context menu: for `path` (a row, or the folder itself), at `at`.
#[derive(Debug, Clone)]
pub struct ExplorerMenu {
    pub path: PathBuf,
    pub is_dir: bool,
    /// The opened folder: it can't be renamed or deleted from here.
    pub is_root: bool,
    pub at: Point,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    NewFile,
    NewFolder,
    Rename,
    Delete,
    CopyPath,
    CopyRelativePath,
    SendPath,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditKind {
    NewFile,
    NewFolder,
    Rename,
}

/// A name being typed in the tree.
#[derive(Debug, Clone)]
pub struct NameEdit {
    pub kind: EditKind,
    /// The folder to create in, or the entry to rename.
    pub target: PathBuf,
    pub name: String,
    /// Why the last Enter didn't work (shown under the input).
    pub error: Option<String>,
}

/// A finished trash operation.
#[derive(Debug, Clone)]
pub enum TrashOutcome {
    Trashed(Vec<Trashed>),
    Restored(usize),
}

/// `path` with its prefix `from` replaced by `to` (a renamed entry or folder).
fn moved(path: &Path, from: &Path, to: &Path) -> Option<PathBuf> {
    path.strip_prefix(from).ok().map(|rest| if rest.as_os_str().is_empty() { to.to_path_buf() } else { to.join(rest) })
}

/// Characters before the extension, selected when renaming a file (`a.tar.gz` → `a`).
fn stem_chars(name: &str) -> usize {
    match name.find('.') {
        Some(0) | None => name.chars().count(),
        Some(i) => name[..i].chars().count(),
    }
}

impl App {
    pub(super) fn update_files(&mut self, msg: Msg) -> Task<Msg> {
        match msg {
            // A row says what was right-clicked; the panel then says where (see `rightclick`).
            Msg::ExplorerTarget(path, is_dir) => self.explorer_target = Some((path, is_dir)),
            Msg::ExplorerMenuAt(at) => {
                let (path, is_dir) = self.explorer_target.take().unwrap_or_else(|| (self.root.clone(), true));
                let is_root = path == self.root;
                self.explorer_menu = Some(ExplorerMenu { path, is_dir, is_root, at });
            }
            Msg::ExplorerMenuClose => self.explorer_menu = None,
            Msg::ExplorerMenu(action) => {
                let Some(menu) = self.explorer_menu.take() else { return Task::none() };
                return self.menu_action(action, menu);
            }
            Msg::ExplorerName(name) => {
                if let Some(e) = &mut self.name_edit {
                    e.name = name;
                    e.error = None;
                }
            }
            Msg::ExplorerNameSubmit => return self.submit_name(),
            Msg::ExplorerNameCancel => self.name_edit = None,
            Msg::TrashDone(result) => {
                match result {
                    Ok(TrashOutcome::Trashed(items)) => {
                        let what = match items.as_slice() {
                            [one] => file_name(&one.original),
                            many => format!("{} items", many.len()),
                        };
                        self.close_clean_tabs_under(&items);
                        self.undo_trash = Some(items);
                        self.set_status_action(format!("Moved {what} to the trash"), "Undo", Msg::UndoTrash);
                    }
                    Ok(TrashOutcome::Restored(n)) => {
                        self.set_status(format!("Restored {n} item{}", if n == 1 { "" } else { "s" }), false);
                    }
                    Err(e) => self.set_status(e, true),
                }
                self.explorer.refresh();
                self.refreshing = false;
                return self.refresh_git();
            }
            Msg::UndoTrash => {
                let Some(items) = self.undo_trash.take() else { return Task::none() };
                self.clear_status();
                return Task::perform(
                    blocking(move || {
                        // Back in reverse order, so nested deletes restore their parents first.
                        for t in items.iter().rev() {
                            fileops::restore(t)?;
                        }
                        Ok(TrashOutcome::Restored(items.len()))
                    }),
                    Msg::TrashDone,
                );
            }
            _ => {}
        }
        Task::none()
    }

    fn rel(&self, path: &Path) -> String {
        path.strip_prefix(&self.root).unwrap_or(path).to_string_lossy().into_owned()
    }

    fn menu_action(&mut self, action: MenuAction, menu: ExplorerMenu) -> Task<Msg> {
        let dir = if menu.is_dir {
            menu.path.clone()
        } else {
            menu.path.parent().map_or_else(|| self.root.clone(), Path::to_path_buf)
        };
        match action {
            MenuAction::NewFile | MenuAction::NewFolder => {
                // The new entry is typed inside the folder: open it.
                if dir != self.root && !self.explorer.expanded.contains(&dir) {
                    self.explorer.toggle(&dir);
                }
                let kind = if action == MenuAction::NewFile { EditKind::NewFile } else { EditKind::NewFolder };
                self.name_edit = Some(NameEdit { kind, target: dir, name: String::new(), error: None });
                return iced::widget::operation::focus(NAME_INPUT);
            }
            MenuAction::Rename if !menu.is_root => {
                let name = file_name(&menu.path);
                let end = if menu.is_dir { name.chars().count() } else { stem_chars(&name) };
                self.name_edit = Some(NameEdit { kind: EditKind::Rename, target: menu.path, name, error: None });
                return iced::widget::operation::focus(NAME_INPUT)
                    .chain(iced::widget::operation::select_range(NAME_INPUT, 0, end));
            }
            MenuAction::Delete if !menu.is_root => {
                let path = menu.path;
                return Task::perform(
                    blocking(move || {
                        let t = fileops::trash(&path, &fileops::home_trash()?)?;
                        Ok(TrashOutcome::Trashed(vec![t]))
                    }),
                    Msg::TrashDone,
                );
            }
            MenuAction::Rename | MenuAction::Delete => {}
            MenuAction::CopyPath => return iced::clipboard::write(menu.path.to_string_lossy().into_owned()),
            MenuAction::CopyRelativePath => return iced::clipboard::write(self.rel(&menu.path)),
            MenuAction::SendPath => {
                let rel = self.rel(&menu.path);
                let text = if menu.is_dir && !menu.is_root { format!("{rel}/ ") } else { format!("{rel} ") };
                return self.send_to_agent(text, None, false);
            }
        }
        Task::none()
    }

    /// Enter in the name input: create or rename (quick local syscalls, like saving).
    fn submit_name(&mut self) -> Task<Msg> {
        let Some(edit) = self.name_edit.clone() else { return Task::none() };
        let result = match edit.kind {
            EditKind::NewFile => fileops::create(&edit.target, &edit.name, false),
            EditKind::NewFolder => fileops::create(&edit.target, &edit.name, true),
            EditKind::Rename => fileops::rename(&edit.target, &edit.name),
        };
        let path = match result {
            Ok(path) => path,
            Err(e) => {
                if let Some(ed) = &mut self.name_edit {
                    ed.error = Some(e);
                }
                return Task::none();
            }
        };
        self.name_edit = None;
        match edit.kind {
            EditKind::NewFile => {
                self.explorer.refresh();
                self.open_file(path, None);
                self.set_status(format!("Created {}", self.rel_name(&edit.target, &edit.name)), false);
            }
            EditKind::NewFolder => {
                self.explorer.toggle(&path);
                self.set_status(format!("Created {}", self.rel_name(&edit.target, &edit.name)), false);
            }
            EditKind::Rename => self.after_rename(&edit.target, &path),
        }
        self.refreshing = false;
        self.refresh_git()
    }

    fn rel_name(&self, dir: &Path, name: &str) -> String {
        self.rel(&dir.join(name.trim()))
    }

    /// Tabs, pending confirmations and expanded folders follow a renamed entry.
    fn after_rename(&mut self, from: &Path, to: &Path) {
        let repo = self.repo_root();
        for tab in &mut self.tabs {
            let (path, crumbs) = match tab {
                Tab::File(t) => (&mut t.path, &mut t.crumbs),
                Tab::Image(t) => (&mut t.path, &mut t.crumbs),
                // An unstaged diff follows the file. A staged one stays: the index
                // still has the change under the old name (and a run's diff shows its snapshots).
                Tab::Diff(d) => {
                    if d.source == (DiffSource::Index { staged: false })
                        && let Some(rel) = moved(&repo.join(&d.rel), from, to)
                            .and_then(|new| new.strip_prefix(&repo).ok().map(|r| r.to_string_lossy().into_owned()))
                    {
                        d.rel = rel;
                        // Recomputed at the next tick if it's shown.
                        d.loaded_mtime = None;
                    }
                    continue;
                }
            };
            if let Some(new) = moved(path, from, to) {
                *crumbs = super::documents::crumbs(&self.root, &new);
                *path = new;
            }
        }
        for pending in [&mut self.pending_close, &mut self.pending_overwrite].into_iter().flatten() {
            if let Some(new) = moved(pending, from, to) {
                *pending = new;
            }
        }
        let expanded: Vec<PathBuf> =
            self.explorer.expanded.iter().map(|p| moved(p, from, to).unwrap_or_else(|| p.clone())).collect();
        self.explorer.expanded = expanded.into_iter().collect();
        self.explorer.refresh();
        self.set_status(format!("Renamed to {}", file_name(to)), false);
    }

    /// Tabs of trashed files without unsaved edits close; edited ones stay
    /// (marked deleted, as when an agent deletes a file).
    fn close_clean_tabs_under(&mut self, items: &[Trashed]) {
        for i in (0..self.tabs.len()).rev() {
            let under = self.tabs[i].file_path().is_some_and(|p| items.iter().any(|t| p.starts_with(&t.original)));
            let dirty = matches!(&self.tabs[i], Tab::File(t) if t.dirty);
            if under && !dirty {
                self.close_tab(i);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renamed_paths_follow() {
        let (from, to) = (Path::new("/w/src"), Path::new("/w/lib"));
        assert_eq!(moved(Path::new("/w/src/a.rs"), from, to), Some(PathBuf::from("/w/lib/a.rs")));
        assert_eq!(moved(Path::new("/w/src"), from, to), Some(PathBuf::from("/w/lib")));
        assert_eq!(moved(Path::new("/w/srcx/a.rs"), from, to), None);
    }

    #[test]
    fn rename_selects_the_name_without_extension() {
        assert_eq!(stem_chars("main.rs"), 4);
        assert_eq!(stem_chars("a.tar.gz"), 1);
        assert_eq!(stem_chars(".env"), 4);
        assert_eq!(stem_chars("Makefile"), 8);
        assert_eq!(stem_chars("ñandú.md"), 5);
    }
}
