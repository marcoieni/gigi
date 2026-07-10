use camino::{Utf8Path, Utf8PathBuf};
use serde_json::Value;

use crate::{cmd::Cmd, github};

pub async fn ensure_default_repo_and_root() -> anyhow::Result<Utf8PathBuf> {
    if !is_default_repo_set().await? {
        set_default_repo().await?;
    }
    repo_root().await
}

pub(crate) async fn ensure_clean_repo(repo_root: &Utf8Path) -> anyhow::Result<()> {
    let output = Cmd::new("git", ["status", "--porcelain"])
        .with_current_dir(repo_root)
        .run()
        .await?;
    output.ensure_success("❌ Failed to check repository status")?;
    anyhow::ensure!(
        output.stdout().trim().is_empty(),
        "❌ Repository is not clean. Commit or stash changes first."
    );
    Ok(())
}

#[derive(Debug)]
pub(crate) struct PushLease {
    remote: String,
    branch_ref: String,
    expected_remote_head: String,
}

impl PushLease {
    pub(crate) async fn prepare(
        repo_root: &Utf8Path,
        feature_branch: &str,
    ) -> anyhow::Result<Self> {
        let remote = resolve_push_remote(repo_root, feature_branch).await?;
        let branch_ref = format!("refs/heads/{feature_branch}");

        // Capture the expected SHA directly from the push destination so the lease
        // does not depend on Git being able to associate it with a tracking ref.
        let fetch_output = Cmd::new("git", ["fetch", "--no-tags", &remote, &branch_ref])
            .with_current_dir(repo_root)
            .run()
            .await?;
        fetch_output.ensure_success(format!(
            "❌ Failed to fetch remote feature branch '{feature_branch}'"
        ))?;

        let remote_head_output = Cmd::new("git", ["rev-parse", "--verify", "FETCH_HEAD"])
            .with_current_dir(repo_root)
            .run()
            .await?;
        remote_head_output.ensure_success(format!(
            "❌ Failed to resolve remote feature branch '{feature_branch}'"
        ))?;
        let expected_remote_head = remote_head_output.stdout().to_string();

        let ancestor_output = Cmd::new(
            "git",
            ["merge-base", "--is-ancestor", &expected_remote_head, "HEAD"],
        )
        .with_current_dir(repo_root)
        .run()
        .await?;
        if ancestor_output.status().code() == Some(1) {
            anyhow::bail!(
                "❌ Local branch '{feature_branch}' does not contain the current remote branch. Update or re-check out the branch before rewriting it."
            );
        }
        ancestor_output.ensure_success(format!(
            "❌ Failed to compare local and remote feature branch '{feature_branch}'"
        ))?;

        Ok(Self {
            remote,
            branch_ref,
            expected_remote_head,
        })
    }

    pub(crate) async fn force_push_head(self, repo_root: &Utf8Path) -> anyhow::Result<()> {
        let lease = format!(
            "--force-with-lease={}:{}",
            self.branch_ref, self.expected_remote_head
        );
        let refspec = format!("HEAD:{}", self.branch_ref);
        Cmd::new("git", ["push", &self.remote, &lease, &refspec])
            .with_current_dir(repo_root)
            .run()
            .await?
            .ensure_success("❌ git push --force-with-lease failed")?;
        Ok(())
    }
}

async fn git_config_value(repo_root: &Utf8Path, key: &str) -> anyhow::Result<Option<String>> {
    let output = Cmd::new("git", ["config", "--get", key])
        .with_current_dir(repo_root)
        .run()
        .await?;
    if output.status().success() {
        return Ok((!output.stdout().is_empty()).then(|| output.stdout().to_string()));
    }
    if output.status().code() == Some(1) {
        return Ok(None);
    }
    output.ensure_success(format!("❌ Failed to read git config '{key}'"))?;
    unreachable!("a successful git config lookup returned above")
}

async fn resolve_push_remote(repo_root: &Utf8Path, feature_branch: &str) -> anyhow::Result<String> {
    // Match Git's push-remote precedence. `gh pr checkout` may set `pushRemote`
    // to a URL instead of the name of the remote that owns the tracking ref.
    let keys = [
        format!("branch.{feature_branch}.pushRemote"),
        "remote.pushDefault".to_string(),
        format!("branch.{feature_branch}.remote"),
    ];
    for key in keys {
        if let Some(value) = git_config_value(repo_root, &key).await? {
            return Ok(value);
        }
    }

    // With no configured push remote, Git uses the only remote when exactly
    // one exists before falling back to `origin`.
    let remotes_output = Cmd::new("git", ["remote"])
        .with_current_dir(repo_root)
        .run()
        .await?;
    remotes_output.ensure_success("❌ Failed to list git remotes")?;
    let mut remotes = remotes_output.stdout().lines();
    if let (Some(remote), None) = (remotes.next(), remotes.next()) {
        return Ok(remote.to_string());
    }

    Ok("origin".to_string())
}

struct RepoInfo {
    is_fork: bool,
    default_branch: String,
    parent_name_with_owner: Option<String>,
    parent_default_branch: Option<String>,
}

async fn fetch_repo_info(repo_root: &Utf8Path) -> anyhow::Result<RepoInfo> {
    let origin_repo = origin_name_with_owner(repo_root).await?;
    let output = Cmd::new(
        "gh",
        [
            "repo",
            "view",
            &origin_repo,
            "--json",
            "isFork,parent,defaultBranchRef",
        ],
    )
    .with_current_dir(repo_root)
    .run()
    .await?;
    output.ensure_success("❌ Failed to fetch repository info")?;
    anyhow::ensure!(
        !output.stdout().trim().is_empty(),
        "❌ Failed to fetch repository info: command returned empty output"
    );

    let value: Value = serde_json::from_str(output.stdout())?;
    let is_fork = value
        .get("isFork")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let default_branch = value
        .get("defaultBranchRef")
        .and_then(|value| value.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("main")
        .to_string();

    let (parent_name_with_owner, parent_default_branch) = if is_fork {
        let parent = value.get("parent");
        let name = parent_name_with_owner_from_json(parent);
        let mut branch = parent
            .and_then(|value| value.get("defaultBranchRef"))
            .and_then(|value| value.get("name"))
            .and_then(Value::as_str)
            .map(|branch| branch.to_string());

        if branch.is_none()
            && let Some(parent_repo) = name.as_deref()
        {
            branch = Some(fetch_default_branch(repo_root, parent_repo).await?);
        }
        (name, branch)
    } else {
        (None, None)
    };

    Ok(RepoInfo {
        is_fork,
        default_branch,
        parent_name_with_owner,
        parent_default_branch,
    })
}

fn parent_name_with_owner_from_json(parent: Option<&Value>) -> Option<String> {
    parent.and_then(|value| {
        value
            .get("nameWithOwner")
            .and_then(Value::as_str)
            .map(|name| name.to_string())
            .or_else(|| {
                let owner = value
                    .get("owner")
                    .and_then(|owner| owner.get("login"))
                    .and_then(Value::as_str)?;
                let name = value.get("name").and_then(Value::as_str)?;
                Some(format!("{owner}/{name}"))
            })
    })
}

async fn fetch_default_branch(
    repo_root: &Utf8Path,
    repo_name_with_owner: &str,
) -> anyhow::Result<String> {
    let output = Cmd::new(
        "gh",
        [
            "repo",
            "view",
            repo_name_with_owner,
            "--json",
            "defaultBranchRef",
        ],
    )
    .with_current_dir(repo_root)
    .run()
    .await?;
    output.ensure_success("❌ Failed to fetch repository default branch")?;
    anyhow::ensure!(
        !output.stdout().trim().is_empty(),
        "❌ Failed to fetch repository default branch: command returned empty output"
    );

    let value: Value = serde_json::from_str(output.stdout())?;
    Ok(value
        .get("defaultBranchRef")
        .and_then(|branch| branch.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("main")
        .to_string())
}

async fn origin_name_with_owner(repo_root: &Utf8Path) -> anyhow::Result<String> {
    let output = Cmd::new("git", ["remote", "get-url", "origin"])
        .with_current_dir(repo_root)
        .run()
        .await?;
    output.ensure_success("❌ Failed to detect origin remote URL")?;
    anyhow::ensure!(
        !output.stdout().trim().is_empty(),
        "❌ Failed to detect origin remote URL: command returned empty output"
    );

    github::parse_github_name_with_owner(output.stdout()).ok_or_else(|| {
        anyhow::anyhow!(
            "❌ Failed to parse origin remote URL as GitHub repository: {}",
            output.stdout()
        )
    })
}

async fn ensure_upstream_remote(
    repo_root: &Utf8Path,
    parent_name_with_owner: &str,
) -> anyhow::Result<()> {
    let output = Cmd::new("git", ["remote", "get-url", "upstream"])
        .with_current_dir(repo_root)
        .run()
        .await?;
    if output.status().success() && !output.stdout().trim().is_empty() {
        return Ok(());
    }

    let upstream_url = format!("https://github.com/{parent_name_with_owner}.git");
    let add_output = Cmd::new("git", ["remote", "add", "upstream", &upstream_url])
        .with_current_dir(repo_root)
        .run()
        .await?;
    add_output.ensure_success("❌ Failed to add upstream remote")?;
    Ok(())
}

pub async fn sync_fork(repo_root: &Utf8Path) -> anyhow::Result<()> {
    ensure_clean_repo(repo_root).await?;

    let repo_info = fetch_repo_info(repo_root).await?;
    if !repo_info.is_fork {
        println!("ℹ️ Repository is not a fork. Nothing to sync.");
        return Ok(());
    }

    let parent_name_with_owner = repo_info
        .parent_name_with_owner
        .ok_or_else(|| anyhow::anyhow!("❌ Failed to detect parent repository"))?;
    let parent_default_branch = repo_info
        .parent_default_branch
        .ok_or_else(|| anyhow::anyhow!("❌ Failed to detect parent default branch"))?;

    ensure_upstream_remote(repo_root, &parent_name_with_owner).await?;

    let current = current_branch(repo_root).await?;
    Cmd::new("git", ["fetch", "upstream"])
        .with_current_dir(repo_root)
        .run()
        .await?
        .ensure_success("❌ Failed to fetch upstream remote")?;
    Cmd::new("git", ["checkout", &repo_info.default_branch])
        .with_current_dir(repo_root)
        .run()
        .await?
        .ensure_success(format!(
            "❌ Failed to checkout default branch '{}'",
            repo_info.default_branch
        ))?;
    let pull_output = Cmd::new(
        "git",
        ["pull", "--ff-only", "upstream", &parent_default_branch],
    )
    .with_current_dir(repo_root)
    .run()
    .await?;
    pull_output.ensure_success("❌ Failed to sync default branch from upstream")?;
    let push_output = Cmd::new("git", ["push", "origin", &repo_info.default_branch])
        .with_current_dir(repo_root)
        .run()
        .await?;
    push_output.ensure_success("❌ Failed to push synced default branch to origin")?;

    if current != repo_info.default_branch {
        Cmd::new("git", ["checkout", &current])
            .with_current_dir(repo_root)
            .run()
            .await?
            .ensure_success(format!("❌ Failed to switch back to branch '{current}'"))?;
    }

    Ok(())
}

async fn is_default_repo_set() -> anyhow::Result<bool> {
    let output = Cmd::new("gh", ["repo", "set-default", "--view"])
        .hide_stdout()
        .hide_stderr()
        .run()
        .await?;
    Ok(!output.stdout().trim().is_empty())
}

async fn set_default_repo() -> anyhow::Result<()> {
    let remotes_output = Cmd::new("git", ["remote"]).hide_stdout().run().await?;
    let has_upstream = remotes_output
        .stdout()
        .lines()
        .any(|line| line.trim() == "upstream");

    let remote = if has_upstream { "upstream" } else { "origin" };
    Cmd::new("gh", ["repo", "set-default", remote])
        .run()
        .await?
        .ensure_success(format!(
            "❌ Failed to set default GitHub repo to remote '{remote}'"
        ))?;
    Ok(())
}

pub(crate) async fn default_branch(repo_root: &Utf8Path) -> anyhow::Result<String> {
    let output = Cmd::new(
        "gh",
        [
            "repo",
            "view",
            "--json",
            "defaultBranchRef",
            "-q",
            ".defaultBranchRef.name",
        ],
    )
    .with_current_dir(repo_root)
    .run()
    .await?;
    output.ensure_success("❌ Failed to detect default branch")?;
    anyhow::ensure!(
        !output.stdout().trim().is_empty(),
        "❌ Failed to detect default branch: command returned empty output"
    );
    Ok(output.stdout().to_string())
}

pub(crate) async fn current_branch(repo_root: &Utf8Path) -> anyhow::Result<String> {
    let output = Cmd::new("git", ["branch", "--show-current"])
        .with_current_dir(repo_root)
        .run()
        .await?;
    output.ensure_success("❌ Failed to detect current branch")?;
    anyhow::ensure!(
        !output.stdout().trim().is_empty(),
        "❌ Failed to detect current branch: command returned empty output"
    );
    Ok(output.stdout().to_string())
}

pub(crate) async fn ensure_not_on_default_branch(
    repo_root: &Utf8Path,
    default_branch: &str,
) -> anyhow::Result<()> {
    let current_branch = current_branch(repo_root).await?;
    anyhow::ensure!(
        current_branch != default_branch,
        "❌ Cannot push to default branch '{default_branch}'. Switch to a feature branch first."
    );
    Ok(())
}

pub(crate) async fn commit(repo_root: &Utf8Path, commit_message: &str) -> anyhow::Result<()> {
    let output = Cmd::new("git", ["commit", "-m", commit_message])
        .with_current_dir(repo_root)
        .run()
        .await?;
    output.ensure_success("❌ git commit failed")?;

    if output.stdout().contains("nothing to commit") {
        anyhow::bail!("❌ Nothing to commit");
    }
    Ok(())
}

pub(crate) async fn view_pr_in_browser(repo_root: &Utf8Path) -> anyhow::Result<()> {
    Cmd::new("gh", ["pr", "view", "--web"])
        .with_current_dir(repo_root)
        .run()
        .await?
        .ensure_success("❌ Failed to open PR in browser")?;
    Ok(())
}

pub(crate) async fn repo_root() -> anyhow::Result<Utf8PathBuf> {
    let git_root = Cmd::new("git", ["rev-parse", "--show-toplevel"])
        .hide_stdout()
        .run()
        .await?;
    git_root.ensure_success("❌ Failed to resolve git repository root")?;
    anyhow::ensure!(
        !git_root.stdout().trim().is_empty(),
        "❌ Failed to resolve git repository root: command returned empty output"
    );
    Ok(Utf8PathBuf::from(git_root.stdout()))
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        process::{Command, Output},
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    use camino::{Utf8Path, Utf8PathBuf};
    use serde_json::json;

    use super::{PushLease, parent_name_with_owner_from_json, resolve_push_remote};

    static NEXT_TEMP_DIR_ID: AtomicU64 = AtomicU64::new(1);

    struct TestDir {
        path: Utf8PathBuf,
    }

    impl TestDir {
        fn new(name: &str) -> Self {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let id = NEXT_TEMP_DIR_ID.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "gigi-{name}-{}-{timestamp}-{id}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self {
                path: Utf8PathBuf::from_path_buf(path).unwrap(),
            }
        }

        fn path(&self) -> &Utf8Path {
            &self.path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            drop(fs::remove_dir_all(&self.path));
        }
    }

    fn git_output(repo: &Utf8Path, args: &[&str]) -> Output {
        Command::new("git")
            .args(args)
            .current_dir(repo)
            .env("LC_ALL", "C")
            .output()
            .unwrap()
    }

    fn command_output(output: &Output) -> String {
        format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    }

    fn git_success(repo: &Utf8Path, args: &[&str]) -> String {
        let output = git_output(repo, args);
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            command_output(&output)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    }

    #[test]
    fn test_parent_name_with_owner_from_json_name_with_owner_field() {
        let parent = json!({"nameWithOwner": "rust-lang/rust-forge"});
        assert_eq!(
            parent_name_with_owner_from_json(Some(&parent)),
            Some("rust-lang/rust-forge".to_string())
        );
    }

    #[test]
    fn test_parent_name_with_owner_from_json_owner_name_fallback() {
        let parent = json!({
            "name": "rust-forge",
            "owner": { "login": "rust-lang" }
        });
        assert_eq!(
            parent_name_with_owner_from_json(Some(&parent)),
            Some("rust-lang/rust-forge".to_string())
        );
    }

    #[tokio::test]
    async fn resolve_push_remote_uses_git_precedence() {
        let fixture = TestDir::new("push-remote-precedence");
        let repo = fixture.path().join("repo");
        git_success(fixture.path(), &["init", repo.as_str()]);
        git_success(&repo, &["config", "branch.feature.remote", "branch-remote"]);
        git_success(&repo, &["config", "remote.pushDefault", "default-remote"]);
        git_success(
            &repo,
            &["config", "branch.feature.pushRemote", "branch-push-remote"],
        );

        assert_eq!(
            resolve_push_remote(&repo, "feature").await.unwrap(),
            "branch-push-remote"
        );

        git_success(&repo, &["config", "--unset", "branch.feature.pushRemote"]);
        assert_eq!(
            resolve_push_remote(&repo, "feature").await.unwrap(),
            "default-remote"
        );

        git_success(&repo, &["config", "--unset", "remote.pushDefault"]);
        assert_eq!(
            resolve_push_remote(&repo, "feature").await.unwrap(),
            "branch-remote"
        );

        git_success(&repo, &["config", "--unset", "branch.feature.remote"]);

        git_success(&repo, &["remote", "add", "only", "."]);
        assert_eq!(resolve_push_remote(&repo, "feature").await.unwrap(), "only");

        git_success(&repo, &["remote", "add", "second", "."]);
        assert_eq!(
            resolve_push_remote(&repo, "feature").await.unwrap(),
            "origin"
        );
    }

    #[tokio::test]
    async fn explicit_lease_supports_url_valued_push_remote() {
        let fixture = TestDir::new("url-push-remote");
        let remote = fixture.path().join("remote.git");
        let remote_url = format!("file://{remote}");
        let repo = fixture.path().join("repo");
        let branch = "u/infra-2026-q2-recap";
        let branch_ref = format!("refs/heads/{branch}");

        git_success(
            fixture.path(),
            &["init", "--bare", "--quiet", remote.as_str()],
        );
        git_success(fixture.path(), &["init", "--quiet", repo.as_str()]);
        git_success(&repo, &["config", "user.name", "Test User"]);
        git_success(&repo, &["config", "user.email", "test@example.com"]);
        git_success(&repo, &["commit", "--allow-empty", "-m", "base"]);
        git_success(&repo, &["switch", "-c", branch]);
        git_success(&repo, &["commit", "--allow-empty", "-m", "original"]);
        git_success(&repo, &["remote", "add", "origin", remote.as_str()]);
        git_success(&repo, &["push", "--set-upstream", "origin", branch]);
        git_success(
            &repo,
            &[
                "config",
                &format!("branch.{branch}.pushRemote"),
                &remote_url,
            ],
        );

        let lease = PushLease::prepare(&repo, branch).await.unwrap();
        git_success(
            &repo,
            &["commit", "--amend", "--allow-empty", "-m", "squashed"],
        );

        let implicit_push = git_output(&repo, &["push", "--force-with-lease"]);
        assert!(!implicit_push.status.success());
        assert!(command_output(&implicit_push).contains("stale info"));

        lease.force_push_head(&repo).await.unwrap();

        let local_head = git_success(&repo, &["rev-parse", "HEAD"]);
        let remote_head = git_success(&remote, &["rev-parse", &branch_ref]);
        assert_eq!(local_head, remote_head);

        let stale_lease = PushLease::prepare(&repo, branch).await.unwrap();
        git_success(&repo, &["commit", "--allow-empty", "-m", "remote update"]);
        git_success(
            &repo,
            &["push", remote.as_str(), &format!("HEAD:{branch_ref}")],
        );
        let updated_remote_head = git_success(&remote, &["rev-parse", &branch_ref]);
        git_success(
            &repo,
            &["commit", "--amend", "--allow-empty", "-m", "local rewrite"],
        );

        let stale_error = stale_lease.force_push_head(&repo).await.unwrap_err();
        assert!(format!("{stale_error:#}").contains("stale info"));
        assert_eq!(
            git_success(&remote, &["rev-parse", &branch_ref]),
            updated_remote_head
        );
    }
}
