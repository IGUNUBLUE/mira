//! Local branch navigation and a soft undo of the last commit; no network IO.
use super::{git, repo_root};
use std::path::{Path, PathBuf};

const MAX_BRANCHES: usize = 1_000;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Head {
    pub branch: Option<String>,
    pub oid: String,
}
fn head(root: &Path) -> Result<Head, String> {
    let oid = git(root, &["rev-parse", "--verify", "HEAD"])
        .map_err(|_| "This repository has no commits yet. Create a commit first.".to_string())?
        .trim()
        .to_string();
    let branch = git(root, &["symbolic-ref", "--quiet", "--short", "HEAD"]).ok().map(|s| s.trim().to_string());
    Ok(Head { branch, oid })
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Branch {
    pub reference: String,
    pub name: String,
    pub oid: String,
    pub remote: bool,
    pub local_name: String,
    pub unavailable: Option<String>,
}
#[derive(Debug, Clone)]
pub struct Listing {
    pub root: PathBuf,
    pub head: Head,
    pub branches: Vec<Branch>,
    pub truncated: bool,
}

pub fn list(root: &Path) -> Result<Listing, String> {
    let root = repo_root(root).ok_or("This folder is not a Git repository.")?;
    let head = head(&root)?;
    let count = format!("--count={}", MAX_BRANCHES + 2);
    let output = git(
        &root,
        &[
            "for-each-ref",
            "--sort=refname",
            &count,
            "--format=%(refname)%00%(objectname)%00%(symref)%00%(worktreepath)%00",
            "refs/heads",
            "refs/remotes",
        ],
    )?;
    let mut remotes: Vec<String> = git(&root, &["remote"])?.lines().map(String::from).collect();
    remotes.sort_by_key(|r| std::cmp::Reverse(r.len()));
    let fields: Vec<&str> = output.split('\0').collect();
    let local_refs: std::collections::HashSet<&str> =
        fields.chunks(4).filter_map(|row| row.first()?.trim_start_matches('\n').strip_prefix("refs/heads/")).collect();
    let mut branches = Vec::new();
    let mut truncated = false;
    for fields in fields.as_chunks::<4>().0 {
        let reference = fields[0].trim_start_matches('\n');
        if !fields[2].is_empty() {
            continue;
        } // Skip remote HEAD aliases.
        let (name, remote) = if let Some(name) = reference.strip_prefix("refs/heads/") {
            (name, false)
        } else if let Some(name) = reference.strip_prefix("refs/remotes/") {
            (name, true)
        } else {
            continue;
        };
        let local_name = if remote {
            remotes.iter().find_map(|r| name.strip_prefix(&format!("{r}/"))).unwrap_or(name)
        } else {
            name
        };
        let unavailable = if !remote && head.branch.as_deref() == Some(name) {
            Some("Current branch".into())
        } else if !remote && !fields[3].is_empty() {
            Some(format!("Open in another worktree: {}", fields[3]))
        } else if remote && local_refs.contains(local_name) {
            Some(format!("Local branch '{local_name}' already exists — select it above"))
        } else {
            None
        };
        if branches.len() == MAX_BRANCHES {
            truncated = true;
            break;
        }
        branches.push(Branch {
            reference: reference.into(),
            name: name.into(),
            oid: fields[1].into(),
            remote,
            local_name: local_name.into(),
            unavailable,
        });
    }
    Ok(Listing { root, head, branches, truncated })
}

fn idle(root: &Path) -> Result<(), String> {
    let dir = PathBuf::from(git(root, &["rev-parse", "--absolute-git-dir"])?.trim());
    if ["MERGE_HEAD", "CHERRY_PICK_HEAD", "REVERT_HEAD", "rebase-merge", "rebase-apply", "sequencer", "BISECT_LOG"]
        .iter()
        .any(|p| dir.join(p).exists())
    {
        return Err(
            "A merge, rebase, cherry-pick, revert or bisect is in progress. Finish or abort it in the terminal first."
                .into(),
        );
    }
    Ok(())
}

pub fn switch(listing: &Listing, target: &Branch) -> Result<String, String> {
    idle(&listing.root)?;
    if head(&listing.root)? != listing.head {
        return Err("The current branch or commit changed. Open the branch selector again.".into());
    }
    let current = list(&listing.root)?;
    if !current.branches.iter().any(|b| b == target) {
        return Err("The selected branch changed or is no longer available. Open the selector again.".into());
    }
    if let Some(reason) = &target.unavailable {
        return Err(reason.clone());
    }
    if target.remote {
        git(
            &listing.root,
            &["switch", "--no-overwrite-ignore", "--track", "--create", &target.local_name, "--", &target.reference],
        )?;
    } else {
        git(&listing.root, &["switch", "--no-guess", "--no-overwrite-ignore", "--", &target.local_name])?;
    }
    Ok(format!("Switched to {}", target.local_name))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UndoCommit {
    pub root: PathBuf,
    pub branch: String,
    pub head: String,
    pub parent: String,
    pub message: String,
    pub summary: String,
    pub merge: bool,
}
/// Cached remote refs only: no fetch and no claim that a commit is unshared.
pub fn prepare_undo(root: &Path) -> Result<UndoCommit, String> {
    let root = repo_root(root).ok_or("This folder is not a Git repository.")?;
    idle(&root)?;
    let head = head(&root)?;
    let branch = head.branch.ok_or("HEAD is detached. Switch to a local branch before undoing a commit.")?;
    let parents = git(&root, &["rev-list", "--parents", "-n", "1", "HEAD"])?;
    let parents: Vec<&str> = parents.split_whitespace().skip(1).collect();
    let parent = parents.first().ok_or("The first commit has no parent. Undo Last Commit needs an earlier commit; your files have not been changed.")?.to_string();
    let shared = git(&root, &["for-each-ref", "--contains", "HEAD", "--format=%(refname:short)", "refs/remotes"])?;
    if !shared.trim().is_empty() {
        return Err(format!(
            "The last commit is already in known remote history: {}. Use git revert {} in a shell to undo a shared commit without rewriting history. Remote refs reflect the last fetch; mira does not contact the remote for this action.",
            shared.lines().take(5).collect::<Vec<_>>().join(", "),
            head.oid
        ));
    }
    let message = git(&root, &["log", "-1", "--format=%B", "HEAD"])?;
    let summary = git(&root, &["log", "-1", "--format=%h %s", "HEAD"])?.trim().to_string();
    Ok(UndoCommit {
        root,
        branch,
        head: head.oid,
        parent,
        message: message.trim_end_matches('\n').into(),
        summary,
        merge: parents.len() > 1,
    })
}

pub fn undo(plan: &UndoCommit) -> Result<(), String> {
    if prepare_undo(&plan.root)? != *plan {
        return Err("The last commit or branch changed. Review Undo Last Commit again.".into());
    }
    // --soft preserves both the index and working files. Git records the old HEAD
    // in ORIG_HEAD and the reflog for recovery; never use --hard or --mixed here.
    git(&plan.root, &["reset", "--soft", &plan.parent])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    struct Repo {
        base: PathBuf,
        root: PathBuf,
    }
    impl Repo {
        fn new() -> Self {
            let base = crate::files::test_dir("branch-actions");
            let root = base.join("work");
            fs::create_dir_all(&root).unwrap();
            git(&root, &["init", "-q", "-b", "main"]).unwrap();
            fs::write(root.join("a.txt"), "base\n").unwrap();
            let repo = Self { base, root };
            repo.commit("base");
            repo
        }
        fn commit(&self, message: &str) {
            git(&self.root, &["add", "."]).unwrap();
            git(
                &self.root,
                &[
                    "-c",
                    "user.name=Test",
                    "-c",
                    "user.email=test@example.invalid",
                    "commit",
                    "--allow-empty",
                    "-qm",
                    message,
                ],
            )
            .unwrap();
        }
    }
    impl Drop for Repo {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.base).unwrap();
        }
    }
    #[test]
    fn switching_keeps_local_edits_and_rejects_conflicts_and_stale_choices() {
        let r = Repo::new();
        git(&r.root, &["branch", "other"]).unwrap();
        fs::write(r.root.join("a.txt"), "main\n").unwrap();
        r.commit("main change");
        let listing = list(&r.root).unwrap();
        let target = listing.branches.iter().find(|b| b.name == "other").unwrap();
        fs::write(r.root.join("a.txt"), "unsaved on disk\n").unwrap();
        assert!(switch(&listing, target).is_err());
        assert_eq!(head(&r.root).unwrap().branch.as_deref(), Some("main"));
        assert_eq!(fs::read_to_string(r.root.join("a.txt")).unwrap(), "unsaved on disk\n");
        git(&r.root, &["restore", "--", "a.txt"]).unwrap();
        fs::write(r.root.join("keep.txt"), "local\n").unwrap();
        assert!(switch(&listing, target).unwrap().contains("Switched to other"));
        assert_eq!(fs::read_to_string(r.root.join("keep.txt")).unwrap(), "local\n");
        assert!(switch(&listing, target).unwrap_err().contains("current branch or commit changed"));
        assert_eq!(head(&r.root).unwrap().branch.as_deref(), Some("other"));
    }
    #[test]
    fn remote_selection_creates_tracking_without_overwriting_existing_local_branch() {
        let r = Repo::new();
        let server = r.base.join("server.git");
        fs::create_dir_all(&server).unwrap();
        git(&server, &["init", "--bare", "-q"]).unwrap();
        git(&r.root, &["remote", "add", "team", server.to_str().unwrap()]).unwrap();
        git(&r.root, &["push", "team", "HEAD:refs/heads/feature/demo"]).unwrap();
        git(&r.root, &["fetch", "team"]).unwrap();
        let listing = list(&r.root).unwrap();
        let target = listing.branches.iter().find(|b| b.name == "team/feature/demo").unwrap();
        switch(&listing, target).unwrap();
        assert_eq!(head(&r.root).unwrap().branch.as_deref(), Some("feature/demo"));
        assert_eq!(git(&r.root, &["rev-parse", "--abbrev-ref", "@{upstream}"]).unwrap().trim(), "team/feature/demo");
        assert!(
            list(&r.root)
                .unwrap()
                .branches
                .iter()
                .find(|b| b.remote)
                .unwrap()
                .unavailable
                .as_ref()
                .unwrap()
                .contains("already exists")
        );
    }
    #[test]
    fn another_worktree_and_ignored_files_are_protected() {
        let r = Repo::new();
        git(&r.root, &["branch", "other"]).unwrap();
        let other = r.base.join("other-worktree");
        git(&r.root, &["worktree", "add", other.to_str().unwrap(), "other"]).unwrap();
        let listing = list(&r.root).unwrap();
        let target = listing.branches.iter().find(|b| b.name == "other").unwrap();
        assert!(target.unavailable.as_ref().unwrap().contains("another worktree"));
        assert!(switch(&listing, target).is_err());
        git(&r.root, &["worktree", "remove", other.to_str().unwrap()]).unwrap();
        git(&r.root, &["switch", "other"]).unwrap();
        fs::write(r.root.join(".env"), "tracked\n").unwrap();
        r.commit("tracked env");
        git(&r.root, &["switch", "main"]).unwrap();
        fs::write(r.root.join(".gitignore"), ".env\n").unwrap();
        fs::write(r.root.join(".env"), "local secret\n").unwrap();
        let listing = list(&r.root).unwrap();
        let target = listing.branches.iter().find(|b| b.name == "other").unwrap();
        assert!(switch(&listing, target).is_err());
        assert_eq!(fs::read_to_string(r.root.join(".env")).unwrap(), "local secret\n");
    }
    #[test]
    fn undo_keeps_the_index_working_files_message_and_recovery_ref() {
        let r = Repo::new();
        fs::write(r.root.join("a.txt"), "commit contents\n").unwrap();
        r.commit("last commit\n\nbody");
        fs::write(r.root.join("b.txt"), "already staged\n").unwrap();
        git(&r.root, &["add", "b.txt"]).unwrap();
        fs::write(r.root.join("a.txt"), "working edit\n").unwrap();
        let index = git(&r.root, &["write-tree"]).unwrap();
        let plan = prepare_undo(&r.root).unwrap();
        assert_eq!(plan.message, "last commit\n\nbody");
        undo(&plan).unwrap();
        assert_eq!(head(&r.root).unwrap().oid, plan.parent);
        assert_eq!(git(&r.root, &["write-tree"]).unwrap(), index);
        assert_eq!(fs::read_to_string(r.root.join("a.txt")).unwrap(), "working edit\n");
        assert_eq!(git(&r.root, &["rev-parse", "ORIG_HEAD"]).unwrap().trim(), plan.head);
        let staged = git(&r.root, &["diff", "--cached", "--name-only"]).unwrap();
        assert!(staged.contains("a.txt") && staged.contains("b.txt"));
    }
    #[test]
    fn undo_rejects_shared_commits_changed_heads_detached_and_active_operations() {
        let r = Repo::new();
        assert!(prepare_undo(&r.root).unwrap_err().contains("first commit"));
        r.commit("second");
        let plan = prepare_undo(&r.root).unwrap();
        r.commit("third");
        let current = head(&r.root).unwrap();
        assert!(undo(&plan).is_err());
        assert_eq!(head(&r.root).unwrap(), current);
        git(&r.root, &["update-ref", "refs/remotes/team/main", "HEAD"]).unwrap();
        assert!(prepare_undo(&r.root).unwrap_err().contains("known remote history"));
        git(&r.root, &["update-ref", "-d", "refs/remotes/team/main"]).unwrap();
        git(&r.root, &["switch", "--detach"]).unwrap();
        assert!(prepare_undo(&r.root).unwrap_err().contains("detached"));
        git(&r.root, &["switch", "main"]).unwrap();
        fs::write(r.root.join(".git/MERGE_HEAD"), &current.oid).unwrap();
        assert!(prepare_undo(&r.root).unwrap_err().contains("in progress"));
    }
}
