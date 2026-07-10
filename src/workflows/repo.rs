use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};
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
    push_destination: String,
    branch_ref: String,
    expected_remote_head: String,
    feature_branch: String,
}

const SQUASH_RETRY_CONFIG_KEY: &str = "gigi.squashRetry";

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct SquashRetryState {
    feature_branch: String,
    push_destination: String,
    expected_remote_head: String,
    result_head: String,
}

impl PushLease {
    pub(crate) async fn prepare(
        repo_root: &Utf8Path,
        feature_branch: &str,
    ) -> anyhow::Result<Self> {
        let remote = resolve_push_remote(repo_root, feature_branch).await?;
        let push_destination = resolve_push_destination(repo_root, &remote).await?;
        let branch_ref = format!("refs/heads/{feature_branch}");
        // Read the expected SHA from the push destination rather than relying on
        // Git to associate it with a local tracking ref.
        let expected_remote_head =
            fetch_branch_head(repo_root, &push_destination, feature_branch).await?;
        let lease = Self {
            remote,
            push_destination,
            branch_ref,
            expected_remote_head,
            feature_branch: feature_branch.to_string(),
        };
        let retry_state = lease.retry_state(repo_root).await?;

        let ancestor_output = Cmd::new(
            "git",
            [
                "merge-base",
                "--is-ancestor",
                &lease.expected_remote_head,
                "HEAD",
            ],
        )
        .with_current_dir(repo_root)
        .run()
        .await?;
        if ancestor_output.status().code() == Some(1) {
            anyhow::ensure!(
                squash_retry_state(repo_root).await?.as_ref() == Some(&retry_state),
                "❌ Local branch '{feature_branch}' does not contain the current remote branch. Update or re-check out the branch before rewriting it."
            );
        } else {
            ancestor_output.ensure_success(format!(
                "❌ Failed to compare local and remote feature branch '{feature_branch}'"
            ))?;
            clear_squash_retry_state(repo_root).await?;
        }

        Ok(lease)
    }

    pub(crate) async fn force_push_head(&self, repo_root: &Utf8Path) -> anyhow::Result<()> {
        let retry_state = self.retry_state(repo_root).await?;
        set_squash_retry_state(repo_root, &retry_state).await?;

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
        clear_squash_retry_state(repo_root).await?;
        Ok(())
    }

    async fn retry_state(&self, repo_root: &Utf8Path) -> anyhow::Result<SquashRetryState> {
        Ok(SquashRetryState {
            feature_branch: self.feature_branch.clone(),
            push_destination: self.push_destination.clone(),
            expected_remote_head: self.expected_remote_head.clone(),
            result_head: resolve_revision(repo_root, "HEAD").await?,
        })
    }
}

async fn resolve_revision(repo_root: &Utf8Path, revision: &str) -> anyhow::Result<String> {
    let output = Cmd::new("git", ["rev-parse", "--verify", revision])
        .with_current_dir(repo_root)
        .run()
        .await?;
    output.ensure_success(format!("❌ Failed to resolve git revision '{revision}'"))?;
    Ok(output.stdout().to_string())
}

pub(crate) async fn fetch_branch_head(
    repo_root: &Utf8Path,
    source: &str,
    branch: &str,
) -> anyhow::Result<String> {
    let branch_ref = format!("refs/heads/{branch}");
    let output = Cmd::new("git", ["fetch", "--no-tags", source, &branch_ref])
        .with_current_dir(repo_root)
        .run()
        .await?;
    output.ensure_success(format!(
        "❌ Failed to fetch branch '{branch}' from '{source}'"
    ))?;
    resolve_revision(repo_root, "FETCH_HEAD").await
}

#[derive(Clone, Copy)]
enum GitConfigScope {
    Local,
    Effective,
}

async fn squash_retry_state(repo_root: &Utf8Path) -> anyhow::Result<Option<SquashRetryState>> {
    git_config_value(repo_root, SQUASH_RETRY_CONFIG_KEY, GitConfigScope::Local)
        .await?
        .map(|value| serde_json::from_str(&value))
        .transpose()
        .map_err(Into::into)
}

async fn set_squash_retry_state(
    repo_root: &Utf8Path,
    retry_state: &SquashRetryState,
) -> anyhow::Result<()> {
    let value = serde_json::to_string(retry_state)?;
    Cmd::new(
        "git",
        [
            "config",
            "--local",
            "--replace-all",
            SQUASH_RETRY_CONFIG_KEY,
            &value,
        ],
    )
    .with_current_dir(repo_root)
    .run()
    .await?
    .ensure_success("❌ Failed to record squash retry state")?;
    Ok(())
}

async fn clear_squash_retry_state(repo_root: &Utf8Path) -> anyhow::Result<()> {
    if git_config_value(repo_root, SQUASH_RETRY_CONFIG_KEY, GitConfigScope::Local)
        .await?
        .is_none()
    {
        return Ok(());
    }

    Cmd::new(
        "git",
        ["config", "--local", "--unset-all", SQUASH_RETRY_CONFIG_KEY],
    )
    .with_current_dir(repo_root)
    .run()
    .await?
    .ensure_success("❌ Failed to clear squash retry state")?;
    Ok(())
}

async fn git_config_value(
    repo_root: &Utf8Path,
    key: &str,
    scope: GitConfigScope,
) -> anyhow::Result<Option<String>> {
    let mut args = vec!["config"];
    if matches!(scope, GitConfigScope::Local) {
        args.push("--local");
    }
    args.extend(["--get", key]);

    let output = Cmd::new("git", args)
        .with_current_dir(repo_root)
        .run()
        .await?;
    if output.status().success() {
        return Ok((!output.stdout().is_empty()).then(|| output.stdout().to_string()));
    }
    if output.status().code() == Some(1) {
        return Ok(None);
    }
    let scope = match scope {
        GitConfigScope::Local => "local ",
        GitConfigScope::Effective => "",
    };
    output.ensure_success(format!("❌ Failed to read {scope}git config '{key}'"))?;
    unreachable!("a successful git config lookup returned above")
}

pub(crate) async fn remote_names(repo_root: &Utf8Path) -> anyhow::Result<Vec<String>> {
    let output = Cmd::new("git", ["remote"])
        .with_current_dir(repo_root)
        .run()
        .await?;
    output.ensure_success("❌ Failed to list git remotes")?;
    Ok(output.stdout().lines().map(str::to_string).collect())
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
        if let Some(value) = git_config_value(repo_root, &key, GitConfigScope::Effective).await? {
            return Ok(value);
        }
    }

    // With no configured push remote, Git uses the only remote when exactly
    // one exists before falling back to `origin`.
    let remotes = remote_names(repo_root).await?;
    if let [remote] = remotes.as_slice() {
        return Ok(remote.clone());
    }

    Ok("origin".to_string())
}

async fn resolve_push_destination(
    repo_root: &Utf8Path,
    push_remote: &str,
) -> anyhow::Result<String> {
    if !remote_names(repo_root)
        .await?
        .iter()
        .any(|remote| remote == push_remote)
    {
        return Ok(push_remote.to_string());
    }

    let output = Cmd::new("git", ["remote", "get-url", "--push", "--all", push_remote])
        .with_current_dir(repo_root)
        .run()
        .await?;
    output.ensure_success(format!(
        "❌ Failed to resolve push URL for remote '{push_remote}'"
    ))?;

    let destinations: Vec<_> = output.stdout().lines().collect();
    anyhow::ensure!(
        destinations.len() == 1,
        "❌ Remote '{push_remote}' must have exactly one push URL to create a safe force-push lease"
    );
    Ok(destinations[0].to_string())
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
    use std::fs;

    use camino::Utf8Path;
    use serde_json::json;

    use crate::workflows::test_support::{
        TestDir, command_output, configure_test_user, git_output, git_success, init_bare_repo,
    };

    use super::{
        PushLease, SQUASH_RETRY_CONFIG_KEY, parent_name_with_owner_from_json,
        resolve_push_destination, resolve_push_remote,
    };

    fn init_feature_repo(
        fixture_root: &Utf8Path,
        repo: &Utf8Path,
        remote: &Utf8Path,
        branch: &str,
    ) {
        init_bare_repo(fixture_root, remote);
        git_success(fixture_root, &["init", "--quiet", repo.as_str()]);
        configure_test_user(repo);
        git_success(repo, &["commit", "--allow-empty", "-m", "base"]);
        git_success(repo, &["switch", "-c", branch]);
        git_success(repo, &["commit", "--allow-empty", "-m", "original"]);
        git_success(repo, &["remote", "add", "origin", remote.as_str()]);
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
    async fn explicit_lease_reads_a_named_remotes_push_url() {
        let fixture = TestDir::new("named-remote-push-url");
        let fetch_remote = fixture.path().join("fetch.git");
        let push_remote = fixture.path().join("push.git");
        let repo = fixture.path().join("repo");
        let branch = "feature";
        let branch_ref = format!("refs/heads/{branch}");

        init_feature_repo(fixture.path(), &repo, &fetch_remote, branch);
        init_bare_repo(fixture.path(), &push_remote);
        git_success(
            &repo,
            &[
                "remote",
                "set-url",
                "--push",
                "origin",
                push_remote.as_str(),
            ],
        );
        git_success(&repo, &["config", "branch.feature.remote", "origin"]);
        git_success(&repo, &["push", "origin", branch]);

        assert_eq!(
            resolve_push_destination(&repo, "origin").await.unwrap(),
            push_remote
        );

        let lease = PushLease::prepare(&repo, branch).await.unwrap();
        git_success(
            &repo,
            &["commit", "--amend", "--allow-empty", "-m", "squashed"],
        );
        lease.force_push_head(&repo).await.unwrap();

        assert_eq!(
            git_success(&push_remote, &["rev-parse", &branch_ref]),
            git_success(&repo, &["rev-parse", "HEAD"])
        );
        assert!(
            !git_output(&fetch_remote, &["rev-parse", &branch_ref])
                .status
                .success()
        );
    }

    #[tokio::test]
    async fn failed_push_can_retry_the_recorded_squash() {
        let fixture = TestDir::new("retry-failed-push");
        let remote = fixture.path().join("remote.git");
        let unavailable_remote = fixture.path().join("remote-unavailable.git");
        let remote_url = format!("file://{remote}");
        let repo = fixture.path().join("repo");
        let branch = "feature";
        let branch_ref = format!("refs/heads/{branch}");

        init_feature_repo(fixture.path(), &repo, &remote, branch);
        git_success(&repo, &["push", "origin", branch]);
        git_success(
            &repo,
            &["config", "branch.feature.pushRemote", remote_url.as_str()],
        );

        let lease = PushLease::prepare(&repo, branch).await.unwrap();
        git_success(
            &repo,
            &["commit", "--amend", "--allow-empty", "-m", "squashed"],
        );

        fs::rename(&remote, &unavailable_remote).unwrap();
        assert!(lease.force_push_head(&repo).await.is_err());
        fs::rename(&unavailable_remote, &remote).unwrap();

        let retry_lease = PushLease::prepare(&repo, branch).await.unwrap();
        retry_lease.force_push_head(&repo).await.unwrap();

        assert_eq!(
            git_success(&remote, &["rev-parse", &branch_ref]),
            git_success(&repo, &["rev-parse", "HEAD"])
        );
        let retry_config = git_output(
            &repo,
            &["config", "--local", "--get", SQUASH_RETRY_CONFIG_KEY],
        );
        assert_eq!(retry_config.status.code(), Some(1));
    }

    #[tokio::test]
    async fn explicit_lease_supports_url_valued_push_remote() {
        let fixture = TestDir::new("url-push-remote");
        let remote = fixture.path().join("remote.git");
        let remote_url = format!("file://{remote}");
        let repo = fixture.path().join("repo");
        let branch = "u/infra-2026-q2-recap";
        let branch_ref = format!("refs/heads/{branch}");

        init_feature_repo(fixture.path(), &repo, &remote, branch);
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
