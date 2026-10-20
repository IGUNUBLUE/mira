//! User-initiated remote operations. Git owns transport, routing and credentials.
use super::{TimedError, git, git_timed, repo_root};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Push,
    Pull,
}
impl Operation {
    pub fn name(self) -> &'static str {
        match self {
            Self::Push => "Push",
            Self::Pull => "Pull",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub root: PathBuf,
    pub operation: Operation,
    pub branch: String,
    head: String,
    routing: String,
    pub upstream: Option<String>,
    pub remotes: Vec<String>,
    pub preferred: Option<String>,
    /// Only set by the user's publication choice; ordinary push respects Git config.
    pub publish_to: Option<String>,
}
impl Plan {
    pub fn publishing(&self) -> bool {
        self.operation == Operation::Push && self.upstream.is_none()
    }
    pub fn context(&self) -> String {
        format!(
            "Repository: {}\nBranch: {}\n{}",
            self.root.display(),
            self.branch,
            if let Some(remote) = &self.publish_to {
                format!("Publish to: {remote}/{}", self.branch)
            } else {
                format!(
                    "Tracking: {}\nRemote: {}",
                    self.upstream.as_deref().unwrap_or("not configured"),
                    self.preferred.as_deref().unwrap_or("Git configuration")
                )
            }
        )
    }
    pub fn command(&self, ipv4: bool) -> String {
        let args = self.args(ipv4);
        format!(
            "git -C {} {}",
            quote(&self.root.to_string_lossy()),
            args.iter().map(|s| quote(s)).collect::<Vec<_>>().join(" ")
        )
    }
    fn args(&self, ipv4: bool) -> Vec<String> {
        let mut args = vec![self.operation.name().to_ascii_lowercase()];
        if self.operation == Operation::Push {
            args.push("--porcelain".into());
        } else {
            args.push("--ff-only".into());
        }
        if ipv4 {
            args.push("--ipv4".into());
        }
        if let Some(remote) = &self.publish_to {
            args.extend([
                "--set-upstream".into(),
                "--".into(),
                remote.clone(),
                format!("refs/heads/{0}:refs/heads/{0}", self.branch),
            ]);
        }
        args
    }
}
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Local inspection only. No connection is made until the user starts the operation.
pub fn prepare(root: &Path, operation: Operation) -> Result<Plan, String> {
    let root = repo_root(root).ok_or("This folder is not a Git repository.")?;
    let branch = git(&root, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .map_err(|_| {
            "HEAD is detached. Check out a local branch in the terminal before pushing or pulling.".to_string()
        })?
        .trim()
        .to_string();
    let head = git(&root, &["rev-parse", "--verify", "HEAD"])
        .map_err(|_| "This branch has no commits. Create a commit before publishing it.".to_string())?
        .trim()
        .to_string();
    let tracking = git(&root, &["for-each-ref", "--format=%(upstream:short)", &format!("refs/heads/{branch}")])?;
    let upstream = (!tracking.trim().is_empty()).then(|| tracking.trim().to_string());
    let remotes: Vec<String> = git(&root, &["remote"])?.lines().map(String::from).collect();
    if remotes.is_empty() {
        return Err(
            "No Git remote is configured. Add one in the terminal with git remote add <name> <url>, then retry.".into(),
        );
    }
    if operation == Operation::Pull && upstream.is_none() {
        return Err(format!(
            "Branch '{branch}' has no tracking branch. Publish it with Push, or choose an existing remote branch in the terminal with git branch --set-upstream-to=<remote>/<branch>."
        ));
    }
    let setting =
        |key: &str| git(&root, &["config", "--get", key]).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let preferred = if operation == Operation::Push {
        setting(&format!("branch.{branch}.pushRemote")).or_else(|| setting("remote.pushDefault"))
    } else {
        None
    }
    .or_else(|| setting(&format!("branch.{branch}.remote")))
    .filter(|r| remotes.contains(r))
    .or_else(|| (remotes.len() == 1).then(|| remotes[0].clone()));
    let routing = git(
        &root,
        &["config", "--null", "--get-regexp", r"^(branch\.|remote\.|push\.|url\.|core\.sshcommand$|ssh\.variant$)"],
    )?;
    Ok(Plan { root, operation, branch, head, routing, upstream, remotes, preferred, publish_to: None })
}

/// Never silently retarget a delayed publication or retry after an agent changed branches.
pub fn execute(plan: &Plan, ipv4: bool) -> Result<String, String> {
    let current = prepare(&plan.root, plan.operation)?;
    if current.branch != plan.branch
        || current.head != plan.head
        || current.upstream != plan.upstream
        || current.remotes != plan.remotes
        || current.preferred != plan.preferred
        || current.routing != plan.routing
    {
        return Err("The branch, commit or remote configuration changed. Close this dialog and start the operation again to review the current destination.".into());
    }
    if plan.publishing() && plan.publish_to.is_none() {
        return Err("Choose a remote before publishing this branch.".into());
    }
    if plan.publish_to.as_ref().is_some_and(|r| !plan.remotes.contains(r)) {
        return Err("The selected remote is no longer available.".into());
    }
    let args = plan.args(ipv4);
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    // Explicit retry only: a timeout can mean the server accepted the push but its reply was lost.
    // Never retry a write automatically or change the user's SSH configuration.
    let mut env = vec![("LC_ALL", "C")];
    // Let OpenSSH try its next address (including IPv4) instead of waiting for
    // the OS TCP timeout on a broken IPv6 route. Keep all custom Git transports.
    if default_ssh(&plan.root) {
        env.push(("GIT_SSH_COMMAND", "ssh -o BatchMode=yes -o ConnectTimeout=8 -o ConnectionAttempts=1"));
    }
    match git_timed(&plan.root, &refs, &env, 120) {
        Ok(out) => Ok(success(plan, &out, ipv4)),
        Err(TimedError::TimedOut) => Err("The Git operation timed out after 120 seconds. The remote result could not be confirmed; the server may have received the push. Check the remote before retrying. A network, VPN, IPv6 route or an interactive credential prompt may be blocking the connection.".into()),
        Err(TimedError::Failed(e)) => Err(e),
    }
}
fn default_ssh(root: &Path) -> bool {
    ["GIT_SSH_COMMAND", "GIT_SSH", "GIT_SSH_VARIANT"].iter().all(|key| std::env::var_os(key).is_none())
        && git(root, &["config", "--get", "core.sshCommand"]).is_err()
        && git(root, &["config", "--get", "ssh.variant"]).is_err()
}

fn success(plan: &Plan, out: &str, ipv4: bool) -> String {
    let result = if plan.operation == Operation::Push {
        let refs: Vec<&str> = out
            .lines()
            .filter_map(|l| l.split('\t').nth(1))
            .filter_map(|r| r.split_once(':').map(|(_, to)| to.trim_start_matches("refs/heads/")))
            .take(3)
            .collect();
        if let Some(remote) = &plan.publish_to {
            format!("Published {} to {remote}/{}", plan.branch, plan.branch)
        } else if !refs.is_empty() {
            format!("Push completed: {}", refs.join(", "))
        } else {
            format!("Push completed for {}", plan.branch)
        }
    } else {
        format!("Pull completed for {}", plan.branch)
    };
    if ipv4 { format!("{result} (via IPv4)") } else { result }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Advice {
    pub summary: &'static str,
    pub next: &'static str,
    pub ipv4: bool,
}
/// Specific failures take priority over Git's generic trailing "could not read" message.
pub fn advice(detail: &str) -> Advice {
    let e = detail.to_ascii_lowercase();
    let (summary, next, ipv4) = if e.contains("host key verification failed")
        || e.contains("remote host identification has changed")
    {
        (
            "SSH host verification failed",
            "Verify the host fingerprint in a shell before trusting it. mira will not bypass host verification.",
            false,
        )
    } else if e.contains("permission denied")
        || e.contains("authentication failed")
        || e.contains("could not read username")
        || e.contains("terminal prompts disabled")
        || e.contains("could not read password")
        || e.contains("invalid credentials")
    {
        (
            "Git authentication or access failed",
            "Check your account, repository permissions, SSH key/agent or Git credential helper in a shell, then retry. mira cannot answer interactive credential prompts.",
            false,
        )
    } else if e.contains("protected branch")
        || e.contains("pre-receive hook declined")
        || e.contains("remote rejected")
        || e.contains("hook declined")
    {
        (
            "The remote rejected this operation",
            "Read the server response below. Check branch protection, hooks and repository permissions; use your team's pull request workflow when required.",
            false,
        )
    } else if e.contains("non-fast-forward") || e.contains("fetch first") || e.contains("[rejected]") {
        (
            "The remote branch has different commits",
            "Fetch and review the incoming commits, then merge or rebase in the terminal before retrying. Do not force push to bypass this rejection.",
            false,
        )
    } else if e.contains("could not resolve")
        || e.contains("name or service not known")
        || e.contains("temporary failure in name resolution")
    {
        (
            "The Git host could not be resolved",
            "Check the remote address, DNS, network and VPN connection, then retry.",
            false,
        )
    } else if e.contains("timed out")
        || e.contains("connection refused")
        || e.contains("network is unreachable")
        || e.contains("no route to host")
        || e.contains("failed to connect")
        || e.contains("connection reset")
        || e.contains("connection closed")
    {
        (
            "The Git connection failed",
            "Check your network, VPN and SSH/HTTPS access. Retry via IPv4 if your IPv6 route is unavailable. A timeout does not confirm whether the remote changed.",
            true,
        )
    } else if e.contains("certificate") || e.contains("ssl") {
        (
            "TLS verification failed",
            "Check your certificate trust store and network/proxy settings in the terminal. Do not disable certificate verification.",
            false,
        )
    } else if e.contains("no tracking branch") || e.contains("no upstream") || e.contains("set-upstream") {
        (
            "This branch has no tracking branch",
            "Publish the branch, or configure its intended tracking branch in the terminal, then retry.",
            false,
        )
    } else if e.contains("no git remote") || e.contains("no configured push destination") {
        ("No Git remote is configured", "Add the intended remote in a shell, then retry.", false)
    } else if e.contains("not possible to fast-forward") || e.contains("divergent branches") {
        (
            "The branches have diverged",
            "Review the commits and merge or rebase in the terminal. mira only pulls with --ff-only.",
            false,
        )
    } else if e.contains("would be overwritten")
        || e.contains("unmerged files")
        || e.contains("resolve your current index")
    {
        (
            "Local changes prevent this operation",
            "Review and commit or stash your work, or resolve the conflicts, before retrying.",
            false,
        )
    } else {
        (
            "Git could not complete the operation",
            "Read the Git output below and resolve the reported problem in a shell, then retry.",
            false,
        )
    };
    Advice { summary, next, ipv4 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    struct Fixture {
        dir: PathBuf,
        work: PathBuf,
        remote: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let dir = crate::files::test_dir("remote-flow");
            let work = dir.join("work");
            let remote = dir.join("server.git");
            fs::create_dir_all(&work).unwrap();
            fs::create_dir_all(&remote).unwrap();
            git(&remote, &["init", "--bare", "-q"]).unwrap();
            git(&work, &["init", "-q", "-b", "feature/demo"]).unwrap();
            fs::write(work.join("demo.txt"), "demo\n").unwrap();
            git(&work, &["add", "."]).unwrap();
            git(&work, &["-c", "user.name=Test", "-c", "user.email=test@example.invalid", "commit", "-qm", "initial"])
                .unwrap();
            git(&work, &["remote", "add", "team", remote.to_str().unwrap()]).unwrap();
            Self { dir, work, remote }
        }
        fn plan(&self) -> Plan {
            prepare(&self.work, Operation::Push).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.dir).unwrap();
        }
    }
    #[test]
    fn publishing_requires_a_choice_and_sets_tracking_only_after_success() {
        let f = Fixture::new();
        let mut plan = f.plan();
        assert!(plan.publishing());
        assert_eq!(plan.preferred.as_deref(), Some("team"));
        assert!(execute(&plan, false).unwrap_err().contains("Choose a remote"));
        assert!(git(&f.remote, &["show-ref", "--verify", "refs/heads/feature/demo"]).is_err());
        plan.publish_to = Some("team".into());
        assert!(execute(&plan, false).unwrap().contains("Published feature/demo to team/feature/demo"));
        assert_eq!(git(&f.remote, &["rev-parse", "refs/heads/feature/demo"]).unwrap().trim(), plan.head);
        let tracked = f.plan();
        assert!(!tracked.publishing());
        assert_eq!(tracked.upstream.as_deref(), Some("team/feature/demo"));
        assert!(execute(&tracked, true).unwrap().ends_with("(via IPv4)"));
        assert!(prepare(&f.work, Operation::Pull).is_ok());
    }
    #[test]
    fn publication_rejects_changed_branch_commit_and_remote_url() {
        let f = Fixture::new();
        let mut plan = f.plan();
        plan.publish_to = Some("team".into());
        git(&f.work, &["switch", "-qc", "another"]).unwrap();
        assert!(execute(&plan, false).unwrap_err().contains("changed"));
        git(&f.work, &["switch", "-q", "feature/demo"]).unwrap();
        git(&f.work, &["remote", "set-url", "team", "/missing-server"]).unwrap();
        assert!(execute(&plan, false).unwrap_err().contains("changed"));
        git(&f.work, &["remote", "set-url", "team", f.remote.to_str().unwrap()]).unwrap();
        git(
            &f.work,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "--allow-empty",
                "-qm",
                "next",
            ],
        )
        .unwrap();
        assert!(execute(&plan, false).unwrap_err().contains("changed"));
        assert!(git(&f.remote, &["show-ref", "--verify", "refs/heads/feature/demo"]).is_err());
    }
    #[test]
    fn failed_publication_does_not_create_tracking_or_report_success() {
        let f = Fixture::new();
        git(&f.remote, &["config", "receive.denyCurrentBranch", "refuse"]).unwrap();
        // A local rejecting receive hook stands in for protected branches and server policy.
        let hook = f.remote.join("hooks/pre-receive");
        fs::write(&hook, "#!/bin/sh\necho 'protected branch: denied by policy' >&2\nexit 1\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(hook, fs::Permissions::from_mode(0o755)).unwrap();
        let mut plan = f.plan();
        plan.publish_to = Some("team".into());
        let error = execute(&plan, false).unwrap_err();
        assert!(error.contains("protected branch"));
        assert_eq!(advice(&error).summary, "The remote rejected this operation");
        assert!(f.plan().upstream.is_none());
    }
    #[test]
    fn pull_and_publication_explain_missing_prerequisites_and_choose_no_arbitrary_remote() {
        let f = Fixture::new();
        assert!(prepare(&f.work, Operation::Pull).unwrap_err().contains("no tracking branch"));
        git(&f.work, &["remote", "add", "other", "/other-server"]).unwrap();
        assert!(f.plan().preferred.is_none());
        git(&f.work, &["config", "remote.pushDefault", "other"]).unwrap();
        assert_eq!(f.plan().preferred.as_deref(), Some("other"));
        git(&f.work, &["switch", "--detach"]).unwrap();
        assert!(prepare(&f.work, Operation::Push).unwrap_err().contains("detached"));
        git(&f.work, &["switch", "feature/demo"]).unwrap();
        git(&f.work, &["remote", "remove", "team"]).unwrap();
        git(&f.work, &["remote", "remove", "other"]).unwrap();
        assert!(prepare(&f.work, Operation::Push).unwrap_err().contains("No Git remote"));
    }
    #[test]
    fn custom_ssh_is_preserved_and_ipv4_is_passed_through_git() {
        let f = Fixture::new();
        let script = f.dir.join("ssh-test");
        let log = f.dir.join("ssh-args");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > {}\necho 'Permission denied (publickey).' >&2\nexit 255\n",
                quote(log.to_str().unwrap())
            ),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        git(&f.work, &["config", "core.sshCommand", script.to_str().unwrap()]).unwrap();
        git(&f.work, &["config", "ssh.variant", "ssh"]).unwrap();
        git(&f.work, &["remote", "set-url", "team", "git@example.invalid:repo.git"]).unwrap();
        assert!(!default_ssh(&f.work));
        let mut plan = f.plan();
        plan.publish_to = Some("team".into());
        let error = execute(&plan, true).unwrap_err();
        assert_eq!(advice(&error).summary, "Git authentication or access failed");
        assert!(std::fs::read_to_string(log).unwrap().lines().any(|arg| arg == "-4"));
    }

    #[test]
    fn errors_distinguish_authentication_network_policy_and_history() {
        for (error, summary, ipv4) in [
            (
                "Permission denied (publickey). fatal: Could not read from remote repository.",
                "Git authentication or access failed",
                false,
            ),
            ("Authentication failed for https://example.invalid", "Git authentication or access failed", false),
            ("Host key verification failed.", "SSH host verification failed", false),
            ("The Git operation timed out after 120 seconds.", "The Git connection failed", true),
            ("ssh: connect to host example.invalid port 22: Network is unreachable", "The Git connection failed", true),
            ("Could not resolve hostname example.invalid", "The Git host could not be resolved", false),
            (
                "! [remote rejected] main -> main (pre-receive hook declined)",
                "The remote rejected this operation",
                false,
            ),
            ("! [rejected] main -> main (non-fast-forward)", "The remote branch has different commits", false),
            ("fatal: Not possible to fast-forward, aborting.", "The branches have diverged", false),
            ("SSL certificate problem", "TLS verification failed", false),
        ] {
            let a = advice(error);
            assert_eq!((a.summary, a.ipv4), (summary, ipv4), "{error}");
        }
        assert_eq!(quote("a' $(whoami)"), "'a'\\'' $(whoami)'");
    }
}
