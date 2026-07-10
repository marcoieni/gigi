use camino::Utf8Path;
use git_cmd::Repo;
use serde::Deserialize;

use crate::{authors, cmd::Cmd};

use super::repo::{
    PushLease, commit, current_branch, default_branch, ensure_not_on_default_branch,
    view_pr_in_browser,
};

#[derive(Debug, Deserialize)]
struct PullRequest {
    number: u64,
    title: String,
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
            "number,title",
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

async fn sync_feature_branch_with_default(
    repo_root: &Utf8Path,
    default_branch: &str,
) -> anyhow::Result<()> {
    let fetch_output = Cmd::new("git", ["fetch", "origin", default_branch])
        .with_current_dir(repo_root)
        .run()
        .await?;
    fetch_output.ensure_success("git fetch failed")?;

    let merge_ref = format!("origin/{default_branch}");
    let merge_output = Cmd::new("git", ["merge", "--no-edit", &merge_ref])
        .with_current_dir(repo_root)
        .run()
        .await?;
    merge_output.ensure_success("git merge failed")?;
    Ok(())
}

async fn compute_merge_base(repo_root: &Utf8Path, default_branch: &str) -> anyhow::Result<String> {
    let merge_ref = format!("origin/{default_branch}");
    let merge_base = Cmd::new("git", ["merge-base", "HEAD", &merge_ref])
        .with_current_dir(repo_root)
        .run()
        .await?;
    merge_base.ensure_success("❌ Failed to find merge base")?;
    Ok(merge_base.stdout().to_string())
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
    let default_branch = default_branch(repo_root).await?;
    let pull_request = current_pull_request(repo_root, &feature_branch).await?;
    anyhow::ensure!(
        feature_branch != default_branch,
        "❌ You are on the main branch. Switch to a feature branch to squash"
    );

    let push_lease = PushLease::prepare(repo_root, &feature_branch).await?;
    sync_feature_branch_with_default(repo_root, &default_branch).await?;
    let merge_base = compute_merge_base(repo_root, &default_branch).await?;

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
        &merge_base,
        &commit_message,
        &default_branch,
        push_lease,
    )
    .await?;
    view_pr_in_browser(repo_root).await?;

    Ok(())
}
