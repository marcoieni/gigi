use camino::Utf8Path;
use git_cmd::Repo;

use crate::{authors, cmd::Cmd};

use super::repo::{
    commit, current_branch, default_branch, ensure_not_on_default_branch, view_pr_in_browser,
};

async fn pr_title(repo_root: &Utf8Path) -> anyhow::Result<String> {
    let current_branch = current_branch(repo_root).await?;
    let output = Cmd::new(
        "gh",
        [
            "pr",
            "list",
            "--head",
            &current_branch,
            "--json",
            "title",
            "-q",
            ".[0].title",
        ],
    )
    .with_current_dir(repo_root)
    .run()
    .await?;
    output.ensure_success("❌ Failed to get PR title")?;
    anyhow::ensure!(
        !output.stdout().is_empty(),
        "❌ Failed to get PR title: command returned empty output"
    );
    Ok(output.stdout().to_string())
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

async fn print_dry_run_summary(
    repo_root: &Utf8Path,
    merge_base: &str,
    pr_title: &str,
    co_authors: &[String],
    co_authors_text: &str,
) -> anyhow::Result<()> {
    println!("\n🔍 DRY RUN: The following commits would be squashed:");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    let commits_to_squash = authors::get_commits_to_squash(repo_root, merge_base).await?;
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

    if !co_authors.is_empty() {
        println!("\n👥 Co-authors detected: {}", co_authors.len());
    }

    println!("\n💡 To perform the actual squash, run without --dry-run");
    Ok(())
}

async fn perform_squash_and_push(
    repo_root: &Utf8Path,
    merge_base: &str,
    commit_message: &str,
    default_branch: &str,
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
    let push_output = Cmd::new("git", ["push", "--force-with-lease"])
        .with_current_dir(repo_root)
        .run()
        .await?;
    push_output.ensure_success("❌ git push --force-with-lease failed")?;
    Ok(())
}

pub async fn squash(repo_root: &Utf8Path, repo: &Repo, dry_run: bool) -> anyhow::Result<()> {
    anyhow::ensure!(repo.is_clean().is_ok(), "❌ Repository is not clean");
    let feature_branch = repo.original_branch();
    let default_branch = default_branch(repo_root).await?;
    let pr_title = pr_title(repo_root).await?;
    anyhow::ensure!(
        feature_branch != default_branch,
        "❌ You are on the main branch. Switch to a feature branch to squash"
    );

    sync_feature_branch_with_default(repo_root, &default_branch).await?;
    let merge_base = compute_merge_base(repo_root, &default_branch).await?;

    let co_authors = authors::get_co_authors(repo_root, &merge_base).await?;
    let co_authors_text = authors::format_co_authors(&co_authors);

    if dry_run {
        return print_dry_run_summary(
            repo_root,
            &merge_base,
            &pr_title,
            &co_authors,
            &co_authors_text,
        )
        .await;
    }

    let commit_message = format!("{pr_title}{co_authors_text}");
    perform_squash_and_push(repo_root, &merge_base, &commit_message, &default_branch).await?;
    view_pr_in_browser(repo_root).await?;

    Ok(())
}
