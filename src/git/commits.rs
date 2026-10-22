//! Read-only commit inspection. Comparisons use the first parent, or an empty
//! tree for a root commit; neither the index nor the working tree is consulted.

use super::{Change, MAX_CHANGES, git, literal};
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct Details {
    pub oid: String,
    pub parent: Option<String>,
    pub label: String,
    pub comparison: String,
    pub body: String,
    pub files: Vec<Change>,
    pub truncated: bool,
}

pub fn inspect(root: &Path, hash: &str, prefix: &str) -> Result<Details, String> {
    if hash.len() < 4 || hash.len() > 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Invalid commit ID".into());
    }
    let oid = git(root, &["rev-parse", "--verify", &format!("{hash}^{{commit}}")])?.trim().to_string();
    let bytes = git(root, &["cat-file", "-s", &oid])?.trim().parse::<u64>().map_err(|e| e.to_string())?;
    if bytes > 256 * 1024 {
        return Err("Commit metadata is too large to display (over 256 KiB)".into());
    }
    // Read actual parent headers, including at a shallow boundary. Treating a
    // missing shallow parent as a root would incorrectly show every file added.
    let raw = git(root, &["cat-file", "commit", &oid])?;
    let (headers, message) = raw.split_once("\n\n").ok_or("Invalid commit object")?;
    let parent = headers.lines().find_map(|l| l.strip_prefix("parent ")).map(String::from);
    let mut args = match parent.as_deref() {
        Some(parent) => {
            if git(root, &["cat-file", "-e", &format!("{parent}^{{commit}}")]).is_err() {
                return Err("Parent commit is unavailable locally (possibly shallow history). Fetch more history in the terminal to inspect this commit.".into());
            }
            vec!["diff", parent, oid.as_str()]
        }
        None => vec!["diff-tree", "--root", "--no-commit-id", "-r", oid.as_str()],
    };
    args.extend(["--name-status", "-z", "--find-renames", "--no-ext-diff", "--no-textconv", "--"]);
    let scope = literal(prefix);
    if !prefix.is_empty() {
        args.push(&scope);
    }
    let output = git(root, &args)?;
    let (files, truncated) = parse_files(&output)?;
    let short = &oid[..oid.len().min(8)];
    let comparison = match &parent {
        Some(parent) => format!("{} ⇄ {short} · first parent", &parent[..parent.len().min(8)]),
        None => format!("Empty tree ⇄ {short} · initial commit"),
    };
    Ok(Details {
        label: format!("Commit {short}"),
        comparison,
        body: message.split_once('\n').map(|(_, body)| body.trim().to_string()).unwrap_or_default(),
        oid,
        parent,
        files,
        truncated,
    })
}

fn parse_files(output: &str) -> Result<(Vec<Change>, bool), String> {
    let mut parts = output.split('\0').filter(|s| !s.is_empty());
    let mut files = Vec::new();
    while let Some(status) = parts.next() {
        if files.len() == MAX_CHANGES {
            return Ok((files, true));
        }
        let code = status.chars().next().ok_or("Missing change status")?;
        let first = parts.next().ok_or("Missing changed path")?;
        let (path, orig) = if matches!(code, 'R' | 'C') {
            (parts.next().ok_or("Missing renamed path")?, Some(first.to_string()))
        } else {
            (first, None)
        };
        files.push(Change { path: path.to_string(), code, orig });
    }
    Ok((files, false))
}

/// `None` means one side exceeds the byte budget. Missing objects and Git
/// failures are errors, never empty files or a misleading "No changes".
pub fn contents(root: &Path, details: &Details, rel: &str, max: u64) -> Result<Option<(String, String)>, String> {
    let change = details.files.iter().find(|c| c.path == rel).ok_or("File is not part of this commit")?;
    let old = match details.parent.as_deref().filter(|_| change.code != 'A') {
        Some(parent) => blob(root, parent, change.orig.as_deref().unwrap_or(rel), max)?,
        None => Some(String::new()),
    };
    // Refuse before reading the other side if the first one is too large.
    let Some(old) = old else { return Ok(None) };
    let new = if change.code == 'D' { Some(String::new()) } else { blob(root, &details.oid, rel, max)? };
    Ok(new.map(|new| (old, new)))
}

fn blob(root: &Path, rev: &str, path: &str, max: u64) -> Result<Option<String>, String> {
    let entry = git(root, &["ls-tree", "-z", rev, "--", &literal(path)])?;
    let header = entry.split_once('\t').map(|(header, _)| header).ok_or("File is missing from the commit tree")?;
    let mut fields = header.split_whitespace();
    let (_mode, kind, oid) = (fields.next(), fields.next(), fields.next());
    let oid = oid.ok_or("Invalid tree entry")?;
    if kind == Some("commit") {
        return Ok(Some(format!("Submodule commit {oid}\n")));
    }
    if kind != Some("blob") {
        return Err("Cannot diff this Git object type".into());
    }
    let size = git(root, &["cat-file", "-s", oid])?.trim().parse::<u64>().map_err(|e| e.to_string())?;
    if size > max {
        return Ok(None);
    }
    git(root, &["cat-file", "blob", oid]).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::PathBuf};

    struct Repo(PathBuf);
    impl Repo {
        fn new() -> Self {
            let root = crate::files::test_dir("commit-inspection");
            git(&root, &["init", "-q", "-b", "main"]).unwrap();
            Self(root)
        }
        fn write(&self, path: &str, text: &str) {
            let path = self.0.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        fn commit(&self, message: &str) -> String {
            git(&self.0, &["add", "-A"]).unwrap();
            git(
                &self.0,
                &[
                    "-c",
                    "user.name=Test",
                    "-c",
                    "user.email=test@example.invalid",
                    "-c",
                    "commit.gpgsign=false",
                    "commit",
                    "--allow-empty",
                    "-qm",
                    message,
                ],
            )
            .unwrap();
            git(&self.0, &["rev-parse", "HEAD"]).unwrap().trim().to_string()
        }
    }
    impl Drop for Repo {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn initial_commit_compares_against_empty_without_reading_local_edits() {
        let repo = Repo::new();
        repo.write(".env", "VALUE=committed\n");
        let oid = repo.commit("Initial\n\nCommit body");
        repo.write(".env", "VALUE=staged\n");
        git(&repo.0, &["add", ".env"]).unwrap();
        repo.write(".env", "VALUE=working\n");
        let index = fs::read(repo.0.join(".git/index")).unwrap();
        let details = inspect(&repo.0, &oid, "").unwrap();
        assert!(details.parent.is_none());
        assert_eq!(details.body, "Commit body");
        assert_eq!(details.files, [Change::new(".env", 'A')]);
        assert_eq!(
            contents(&repo.0, &details, ".env", 1024).unwrap(),
            Some((String::new(), "VALUE=committed\n".into()))
        );
        assert_eq!(fs::read(repo.0.join(".git/index")).unwrap(), index);
        assert_eq!(fs::read_to_string(repo.0.join(".env")).unwrap(), "VALUE=working\n");
    }

    #[test]
    fn modified_deleted_added_and_renamed_paths_use_their_own_commit_sides() {
        let repo = Repo::new();
        repo.write("modified.txt", "before\n");
        repo.write("deleted.txt", "removed\n");
        repo.write("old\tname.txt", "rename contents\n");
        repo.commit("Base");
        repo.write("modified.txt", "after\n");
        repo.write("added.txt", "added\n");
        fs::remove_file(repo.0.join("deleted.txt")).unwrap();
        fs::rename(repo.0.join("old\tname.txt"), repo.0.join("new\nname.txt")).unwrap();
        let oid = repo.commit("Changes");
        let details = inspect(&repo.0, &oid, "").unwrap();
        let rename = details.files.iter().find(|c| c.code == 'R').unwrap();
        assert_eq!((rename.orig.as_deref(), rename.path.as_str()), (Some("old\tname.txt"), "new\nname.txt"));
        assert_eq!(
            contents(&repo.0, &details, "new\nname.txt", 1024).unwrap(),
            Some(("rename contents\n".into(), "rename contents\n".into()))
        );
        assert_eq!(
            contents(&repo.0, &details, "modified.txt", 1024).unwrap(),
            Some(("before\n".into(), "after\n".into()))
        );
        assert_eq!(
            contents(&repo.0, &details, "deleted.txt", 1024).unwrap(),
            Some(("removed\n".into(), String::new()))
        );
        assert_eq!(contents(&repo.0, &details, "added.txt", 1024).unwrap(), Some((String::new(), "added\n".into())));
    }

    #[test]
    fn folder_scope_and_paths_with_globs_are_literal() {
        let repo = Repo::new();
        repo.write("app/[id]/page.txt", "literal\n");
        repo.write("app/i/page.txt", "other\n");
        let oid = repo.commit("Pages");
        let details = inspect(&repo.0, &oid, "app/[id]/").unwrap();
        assert_eq!(details.files, [Change::new("app/[id]/page.txt", 'A')]);
        assert_eq!(
            contents(&repo.0, &details, "app/[id]/page.txt", 1024).unwrap(),
            Some((String::new(), "literal\n".into()))
        );
    }

    #[test]
    fn merge_is_compared_to_the_first_parent() {
        let repo = Repo::new();
        repo.write("base", "base\n");
        repo.commit("Base");
        git(&repo.0, &["checkout", "-qb", "feature"]).unwrap();
        repo.write("feature.txt", "feature\n");
        repo.commit("Feature");
        git(&repo.0, &["checkout", "-q", "main"]).unwrap();
        repo.write("main.txt", "main\n");
        let parent = repo.commit("Main");
        git(
            &repo.0,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "merge",
                "--no-ff",
                "-qm",
                "Merge",
                "feature",
            ],
        )
        .unwrap();
        let oid = git(&repo.0, &["rev-parse", "HEAD"]).unwrap();
        let details = inspect(&repo.0, oid.trim(), "").unwrap();
        assert_eq!(details.parent.as_deref(), Some(parent.as_str()));
        assert_eq!(details.files, [Change::new("feature.txt", 'A')]);
    }

    #[test]
    fn empty_commit_has_no_files() {
        let repo = Repo::new();
        repo.write("file", "base");
        repo.commit("Base");
        let oid = repo.commit("Empty");
        assert!(inspect(&repo.0, &oid, "").unwrap().files.is_empty());
    }

    #[test]
    fn binary_and_large_files_do_not_become_text_or_unbounded_reads() {
        let repo = Repo::new();
        repo.write("binary", "a\0b");
        repo.write("large", "123456789");
        let oid = repo.commit("Files");
        let details = inspect(&repo.0, &oid, "").unwrap();
        let (old, new) = contents(&repo.0, &details, "binary", 4).unwrap().unwrap();
        assert!(crate::diff::compute(&old, &new, Some(3)).binary);
        assert!(contents(&repo.0, &details, "large", 4).unwrap().is_none());
    }

    #[test]
    fn unavailable_parent_is_an_error_instead_of_an_initial_commit() {
        let repo = Repo::new();
        repo.write("file", "base");
        let parent = repo.commit("Base");
        repo.write("file", "changed");
        let oid = repo.commit("Change");
        fs::remove_file(repo.0.join(".git/objects").join(&parent[..2]).join(&parent[2..])).unwrap();
        assert!(inspect(&repo.0, &oid, "").unwrap_err().contains("Parent commit is unavailable"));
    }

    #[test]
    fn affected_file_lists_are_bounded() {
        let output = (0..=MAX_CHANGES).map(|i| format!("A\0file{i}\0")).collect::<String>();
        let (files, truncated) = parse_files(&output).unwrap();
        assert_eq!(files.len(), MAX_CHANGES);
        assert!(truncated);
    }

    #[test]
    fn missing_blob_is_reported_instead_of_becoming_empty_content() {
        let repo = Repo::new();
        repo.write("file", "content");
        let oid = repo.commit("File");
        let details = inspect(&repo.0, &oid, "").unwrap();
        let blob = git(&repo.0, &["rev-parse", &format!("{oid}:file")]).unwrap();
        let blob = blob.trim();
        fs::remove_file(repo.0.join(".git/objects").join(&blob[..2]).join(&blob[2..])).unwrap();
        assert!(contents(&repo.0, &details, "file", 1024).is_err());
    }

    #[test]
    fn submodule_entries_show_the_recorded_commit_without_reading_a_checkout() {
        let repo = Repo::new();
        repo.write("base", "base");
        let target = repo.commit("Base");
        git(&repo.0, &["update-index", "--add", "--cacheinfo", &format!("160000,{target},module")]).unwrap();
        git(
            &repo.0,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-qm",
                "Submodule",
            ],
        )
        .unwrap();
        let oid = git(&repo.0, &["rev-parse", "HEAD"]).unwrap();
        let details = inspect(&repo.0, oid.trim(), "").unwrap();
        assert_eq!(
            contents(&repo.0, &details, "module", 1024).unwrap(),
            Some((String::new(), format!("Submodule commit {target}\n")))
        );
    }
}
