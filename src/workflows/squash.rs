use camino::Utf8Path;
use git_cmd::Repo;
use serde::Deserialize;

use crate::{authors, checkout::parse_github_pr_url, cmd::Cmd, github};

use super::repo::{
    PushLease, commit, current_branch, ensure_not_on_default_branch, view_pr_in_browser,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PullRequest {
    number: u64,
    title: String,
    base_ref_name: String,
    url: String,
}

async fn current_pull_request(
    repo_root: &Utf8Path,
    current_branch: &str,
) -> anyhow::Result<PullRequest> {
    let output = Cmd::new(
        "gh",
        [
            "pr",
            "list",
            "--head",
            current_branch,
            "--json",
            "number,title,baseRefName,url",
            "--limit",
            "1",
        ],
    )
    .with_current_dir(repo_root)
    .run()
    .await?;
    output.ensure_success("❌ Failed to get current PR")?;
    let pull_requests: Vec<PullRequest> = serde_json::from_str(output.stdout())?;
    pull_requests
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("❌ No open PR found for branch '{current_branch}'"))
}

async fn fetch_and_merge_base_from(
    repo_root: &Utf8Path,
    base_repo_url: &str,
    base_branch: &str,
) -> anyhow::Result<String> {
    let base_branch_ref = format!("refs/heads/{base_branch}");
    let fetch_output = Cmd::new(
        "git",
        ["fetch", "--no-tags", base_repo_url, &base_branch_ref],
    )
    .with_current_dir(repo_root)
    .run()
    .await?;
    fetch_output.ensure_success(format!(
        "❌ Failed to fetch PR base branch '{base_branch}' from {base_repo_url}"
    ))?;

    let base_commit_output = Cmd::new("git", ["rev-parse", "--verify", "FETCH_HEAD"])
        .with_current_dir(repo_root)
        .run()
        .await?;
    base_commit_output.ensure_success("❌ Failed to resolve the fetched PR base branch")?;
    let base_commit = base_commit_output.stdout().to_string();

    let merge_output = Cmd::new("git", ["merge", "--no-edit", &base_commit])
        .with_current_dir(repo_root)
        .run()
        .await?;
    merge_output.ensure_success("❌ Failed to merge the PR base branch")?;
    Ok(base_commit)
}

async fn fetch_and_merge_pull_request_base(
    repo_root: &Utf8Path,
    pull_request: &PullRequest,
) -> anyhow::Result<String> {
    let base_repo = parse_github_pr_url(&pull_request.url)?;
    let base_name_with_owner = format!("{}/{}", base_repo.owner, base_repo.repo);
    let base_source =
        if let Some(remote) = configured_remote_for_repo(repo_root, &base_name_with_owner).await? {
            remote
        } else {
            github_clone_url(repo_root, &base_repo.owner, &base_repo.repo).await?
        };
    fetch_and_merge_base_from(repo_root, &base_source, &pull_request.base_ref_name).await
}

async fn configured_remote_for_repo(
    repo_root: &Utf8Path,
    name_with_owner: &str,
) -> anyhow::Result<Option<String>> {
    let remotes_output = Cmd::new("git", ["remote"])
        .with_current_dir(repo_root)
        .run()
        .await?;
    remotes_output.ensure_success("❌ Failed to list git remotes")?;

    for remote in remotes_output.stdout().lines() {
        let url_output = Cmd::new("git", ["remote", "get-url", remote])
            .with_current_dir(repo_root)
            .run()
            .await?;
        url_output.ensure_success(format!("❌ Failed to resolve remote '{remote}'"))?;
        if github::parse_github_name_with_owner(url_output.stdout()).as_deref()
            == Some(name_with_owner)
        {
            return Ok(Some(remote.to_string()));
        }
    }

    Ok(None)
}

async fn github_clone_url(repo_root: &Utf8Path, owner: &str, repo: &str) -> anyhow::Result<String> {
    let protocol_output = Cmd::new(
        "gh",
        ["config", "get", "git_protocol", "--host", "github.com"],
    )
    .with_current_dir(repo_root)
    .run()
    .await?;
    protocol_output.ensure_success("❌ Failed to detect the configured GitHub git protocol")?;

    github_clone_url_for_protocol(protocol_output.stdout(), owner, repo)
}

fn github_clone_url_for_protocol(
    protocol: &str,
    owner: &str,
    repo: &str,
) -> anyhow::Result<String> {
    match protocol {
        "ssh" => Ok(format!("git@github.com:{owner}/{repo}.git")),
        "https" => Ok(format!("https://github.com/{owner}/{repo}.git")),
        protocol => anyhow::bail!("❌ Unsupported GitHub git protocol '{protocol}'"),
    }
}

fn print_dry_run_summary(
    commits_to_squash: &[authors::PullRequestCommit],
    pr_title: &str,
    detected_co_authors: &[String],
    additional_co_authors: &[String],
    co_authors_text: &str,
) {
    println!("\n🔍 DRY RUN: The following commits would be squashed:");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    let commits_to_squash = authors::get_commits_to_squash(commits_to_squash);
    if commits_to_squash.is_empty() {
        println!("⚠️  No commits to squash (already at merge base)");
    } else {
        for (index, commit) in commits_to_squash.iter().enumerate() {
            println!(
                "{:2}. {} {} (by {})",
                index + 1,
                commit.hash,
                commit.message,
                commit.author
            );
        }
    }

    println!("\n📝 The resulting commit message would be:");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    let commit_message = format!("{pr_title}{co_authors_text}");
    println!("{commit_message}");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    if !detected_co_authors.is_empty() {
        println!("\n👥 Co-authors detected: {}", detected_co_authors.len());
    }

    if !additional_co_authors.is_empty() {
        println!(
            "➕ Additional co-authors selected: {}",
            additional_co_authors.len()
        );
    }

    println!("\n💡 To perform the actual squash, run without --dry-run");
}

async fn perform_squash_and_push(
    repo_root: &Utf8Path,
    merge_base: &str,
    commit_message: &str,
    default_branch: &str,
    push_lease: PushLease,
) -> anyhow::Result<()> {
    let reset_output = Cmd::new("git", ["reset", "--soft", merge_base])
        .with_current_dir(repo_root)
        .run()
        .await?;
    reset_output.ensure_success("❌ git reset --soft failed")?;

    let add_output = Cmd::new("git", ["add", "."])
        .with_current_dir(repo_root)
        .run()
        .await?;
    add_output.ensure_success("❌ git add failed")?;

    commit(repo_root, commit_message).await?;

    ensure_not_on_default_branch(repo_root, default_branch).await?;
    push_lease.force_push_head(repo_root).await?;
    Ok(())
}

pub async fn squash(
    repo_root: &Utf8Path,
    repo: &Repo,
    dry_run: bool,
    add_co_author: bool,
) -> anyhow::Result<()> {
    anyhow::ensure!(repo.is_clean().is_ok(), "❌ Repository is not clean");
    let feature_branch = current_branch(repo_root).await?;
    let pull_request = current_pull_request(repo_root, &feature_branch).await?;
    anyhow::ensure!(
        feature_branch != pull_request.base_ref_name,
        "❌ You are on the PR base branch. Switch to the PR feature branch to squash"
    );

    let push_lease = PushLease::prepare(repo_root, &feature_branch).await?;
    let base_commit = fetch_and_merge_pull_request_base(repo_root, &pull_request).await?;

    let pull_request_commits =
        authors::get_pull_request_commits(repo_root, pull_request.number).await?;
    let detected_co_authors = authors::get_co_authors(repo_root, &pull_request_commits).await?;
    let additional_co_authors = if add_co_author {
        let selectable_co_authors =
            authors::get_selectable_co_authors(repo_root, &detected_co_authors).await?;
        if selectable_co_authors.is_empty() {
            println!("ℹ️ No additional co-authors available to select.");
            Vec::new()
        } else {
            authors::prompt_for_additional_co_authors(&selectable_co_authors)?
        }
    } else {
        Vec::new()
    };
    let mut co_authors = detected_co_authors.clone();
    co_authors.extend(additional_co_authors.iter().cloned());
    let co_authors_text = authors::format_co_authors(&co_authors);

    if dry_run {
        print_dry_run_summary(
            &pull_request_commits,
            &pull_request.title,
            &detected_co_authors,
            &additional_co_authors,
            &co_authors_text,
        );
        return Ok(());
    }

    let commit_message = format!("{}{co_authors_text}", pull_request.title);
    perform_squash_and_push(
        repo_root,
        &base_commit,
        &commit_message,
        &pull_request.base_ref_name,
        push_lease,
    )
    .await?;
    view_pr_in_browser(repo_root).await?;

    Ok(())
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

    use super::{
        configured_remote_for_repo, fetch_and_merge_base_from, github_clone_url_for_protocol,
    };

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
                "gigi-squash-{name}-{}-{timestamp}-{id}",
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

    #[tokio::test]
    async fn uses_matching_configured_remote_for_pr_base() {
        let fixture = TestDir::new("configured-base-remote");
        let repo = fixture.path().join("repo");
        git_success(fixture.path(), &["init", "--quiet", repo.as_str()]);
        git_success(
            &repo,
            &[
                "remote",
                "add",
                "origin",
                "git@github.com:contributor/project.git",
            ],
        );
        git_success(
            &repo,
            &[
                "remote",
                "add",
                "upstream",
                "git@github.com:organization/project.git",
            ],
        );

        assert_eq!(
            configured_remote_for_repo(&repo, "organization/project")
                .await
                .unwrap(),
            Some("upstream".to_string())
        );
    }

    #[test]
    fn base_repo_fallback_uses_configured_git_protocol() {
        assert_eq!(
            github_clone_url_for_protocol("ssh", "organization", "project").unwrap(),
            "git@github.com:organization/project.git"
        );
        assert_eq!(
            github_clone_url_for_protocol("https", "organization", "project").unwrap(),
            "https://github.com/organization/project.git"
        );
    }

    #[tokio::test]
    async fn uses_pr_base_repo_when_origin_default_branch_is_stale() {
        let fixture = TestDir::new("stale-origin");
        let upstream = fixture.path().join("upstream.git");
        let fork = fixture.path().join("fork.git");
        let upstream_work = fixture.path().join("upstream-work");
        let local = fixture.path().join("local");

        git_success(
            fixture.path(),
            &["init", "--bare", "--quiet", upstream.as_str()],
        );
        git_success(
            fixture.path(),
            &["init", "--bare", "--quiet", fork.as_str()],
        );
        git_success(
            fixture.path(),
            &[
                "init",
                "--quiet",
                "--initial-branch",
                "main",
                upstream_work.as_str(),
            ],
        );
        git_success(&upstream_work, &["config", "user.name", "Test User"]);
        git_success(
            &upstream_work,
            &["config", "user.email", "test@example.com"],
        );
        fs::write(upstream_work.join("base.txt"), "base\n").unwrap();
        git_success(&upstream_work, &["add", "base.txt"]);
        git_success(&upstream_work, &["commit", "--quiet", "-m", "base"]);
        let stale_base = git_success(&upstream_work, &["rev-parse", "HEAD"]);
        git_success(
            &upstream_work,
            &["push", "--quiet", upstream.as_str(), "main"],
        );
        git_success(&upstream_work, &["push", "--quiet", fork.as_str(), "main"]);

        git_success(fixture.path(), &["init", "--quiet", local.as_str()]);
        git_success(&local, &["config", "user.name", "Test User"]);
        git_success(&local, &["config", "user.email", "test@example.com"]);
        git_success(&local, &["remote", "add", "origin", fork.as_str()]);
        git_success(&local, &["fetch", "--quiet", "origin", "main"]);
        git_success(&local, &["switch", "--quiet", "-C", "main", "FETCH_HEAD"]);
        git_success(
            &local,
            &["fetch", "--quiet", upstream.as_str(), "refs/heads/main"],
        );
        git_success(
            &local,
            &["switch", "--quiet", "-c", "feature", "FETCH_HEAD"],
        );
        fs::write(local.join("feature.txt"), "feature work\n").unwrap();
        git_success(&local, &["add", "feature.txt"]);
        git_success(&local, &["commit", "--quiet", "-m", "feature"]);

        fs::write(upstream_work.join("upstream.txt"), "new upstream work\n").unwrap();
        git_success(&upstream_work, &["add", "upstream.txt"]);
        git_success(
            &upstream_work,
            &["commit", "--quiet", "-m", "upstream update"],
        );
        let current_base = git_success(&upstream_work, &["rev-parse", "HEAD"]);
        git_success(
            &upstream_work,
            &["push", "--quiet", upstream.as_str(), "main"],
        );

        assert_eq!(
            git_success(&local, &["merge-base", "HEAD", "origin/main"]),
            stale_base
        );

        let fetched_base = fetch_and_merge_base_from(&local, upstream.as_str(), "main")
            .await
            .unwrap();
        assert_eq!(fetched_base, current_base);

        git_success(&local, &["reset", "--soft", &fetched_base]);
        assert_eq!(
            git_success(&local, &["diff", "--cached", "--name-only"]),
            "feature.txt"
        );
    }
}
