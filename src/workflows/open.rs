use anyhow::Context;
use camino::{Utf8Path, Utf8PathBuf};

use crate::{
    args,
    cmd::Cmd,
    commit::{check_commit_message, generate_commit_message, prompt_commit_message},
};

use super::repo::{commit, default_branch, ensure_not_on_default_branch};

async fn resolve_commit_message(
    repo_root: &Utf8Path,
    message: Option<String>,
    agent: Option<&args::Agent>,
    model: Option<&str>,
) -> anyhow::Result<String> {
    match message {
        Some(msg) => {
            check_commit_message(&msg)?;
            Ok(msg)
        }
        None => {
            let initial_message = generate_commit_message(repo_root, agent, model)
                .await
                .context("❌ Failed to generate commit message")?;
            prompt_commit_message(&initial_message)
        }
    }
}

async fn ensure_branch_does_not_exist(
    repo_root: &Utf8Path,
    branch_name: &str,
) -> anyhow::Result<()> {
    if branch_exists_locally(repo_root, branch_name).await? {
        anyhow::bail!(
            "❌ Branch '{branch_name}' already exists locally. Please use a different commit message or delete the existing branch."
        );
    }
    Ok(())
}

async fn create_feature_branch_from_default(
    repo_root: &Utf8Path,
    default_branch_name: &str,
    branch_name: &str,
) -> anyhow::Result<()> {
    Cmd::new("git", ["checkout", default_branch_name])
        .with_current_dir(repo_root)
        .run()
        .await?
        .ensure_success(format!(
            "❌ Failed to checkout default branch '{default_branch_name}'"
        ))?;
    Cmd::new("git", ["pull", "--ff-only"])
        .with_current_dir(repo_root)
        .run()
        .await?
        .ensure_success(format!(
            "❌ Failed to update default branch '{default_branch_name}'"
        ))?;
    Cmd::new("git", ["checkout", "-b", branch_name])
        .with_current_dir(repo_root)
        .run()
        .await?
        .ensure_success(format!("❌ Failed to create branch '{branch_name}'"))?;
    Ok(())
}

async fn stage_and_commit_changes(
    repo_root: &Utf8Path,
    commit_message: &str,
) -> anyhow::Result<()> {
    let staged_files = get_staged_files(repo_root).await?;
    if staged_files.is_empty() {
        Cmd::new("git", ["add", "-A"])
            .with_current_dir(repo_root)
            .run()
            .await?
            .ensure_success("❌ Failed to stage changes with `git add -A`")?;
    }

    commit(repo_root, commit_message).await?;

    Ok(())
}

async fn push_branch_and_open_pr(
    repo_root: &Utf8Path,
    branch_name: &str,
    commit_message: &str,
) -> anyhow::Result<()> {
    Cmd::new("git", ["push", "-u", "origin", branch_name])
        .with_current_dir(repo_root)
        .run()
        .await?
        .ensure_success(format!(
            "❌ Failed to push branch '{branch_name}' to origin"
        ))?;

    let pr_exists = Cmd::new("gh", ["pr", "view", "--json", "number", "-q", ".number"])
        .with_current_dir(repo_root)
        .run()
        .await?
        .status()
        .success();

    if pr_exists {
        Cmd::new("gh", ["pr", "view", "--web"])
            .with_current_dir(repo_root)
            .run()
            .await?
            .ensure_success("❌ Failed to open existing PR in browser")?;
    } else {
        Cmd::new(
            "gh",
            [
                "pr",
                "create",
                "--title",
                commit_message,
                "--body",
                "",
                "--web",
            ],
        )
        .with_current_dir(repo_root)
        .run()
        .await?
        .ensure_success("❌ Failed to create PR in browser")?;
    }

    Ok(())
}

pub async fn open_pr(
    repo_root: &Utf8Path,
    message: Option<String>,
    agent: Option<&args::Agent>,
    model: Option<&str>,
) -> anyhow::Result<()> {
    let commit_message = resolve_commit_message(repo_root, message, agent, model).await?;
    let default_branch_name = default_branch(repo_root).await?;
    let branch_name =
        branch_name_for_new_pr(repo_root, &branch_name_from_commit_message(&commit_message))
            .await?;

    ensure_branch_does_not_exist(repo_root, &branch_name).await?;
    create_feature_branch_from_default(repo_root, &default_branch_name, &branch_name).await?;
    stage_and_commit_changes(repo_root, &commit_message).await?;
    ensure_not_on_default_branch(repo_root, &default_branch_name).await?;
    push_branch_and_open_pr(repo_root, &branch_name, &commit_message).await?;

    Ok(())
}

async fn branch_exists_locally(repo_root: &Utf8Path, branch_name: &str) -> anyhow::Result<bool> {
    let output = Cmd::new("git", ["branch", "--list", branch_name])
        .with_current_dir(repo_root)
        .run()
        .await?;
    output.ensure_success(format!(
        "❌ Failed to check whether branch '{branch_name}' exists"
    ))?;
    Ok(!output.stdout().trim().is_empty())
}

async fn branch_has_associated_pr(repo_root: &Utf8Path, branch_name: &str) -> anyhow::Result<bool> {
    let output = Cmd::new(
        "gh",
        [
            "pr",
            "list",
            "--state",
            "all",
            "--head",
            branch_name,
            "--json",
            "number",
            "--limit",
            "1",
            "-q",
            ".[0].number",
        ],
    )
    .with_current_dir(repo_root)
    .hide_stderr()
    .run()
    .await?;
    Ok(output.status().success() && !output.stdout().trim().is_empty())
}

async fn current_utc_branch_timestamp() -> anyhow::Result<String> {
    let output = Cmd::new("date", ["-u", "+%Y-%m-%dT%H-%M-%SZ"])
        .run()
        .await?;
    output.ensure_success("❌ Failed to generate timestamp for unique branch name")?;
    anyhow::ensure!(
        !output.stdout().trim().is_empty(),
        "❌ Failed to generate timestamp for unique branch name: command returned empty output"
    );
    Ok(output.stdout().to_string())
}

fn branch_name_with_timestamp(branch_name: &str, timestamp: &str) -> String {
    format!("{branch_name}-{timestamp}")
}

async fn branch_name_for_new_pr(repo_root: &Utf8Path, branch_name: &str) -> anyhow::Result<String> {
    if !branch_has_associated_pr(repo_root, branch_name).await? {
        return Ok(branch_name.to_string());
    }

    let timestamp = current_utc_branch_timestamp().await?;
    Ok(branch_name_with_timestamp(branch_name, &timestamp))
}

async fn get_staged_files(curr_dir: &Utf8Path) -> anyhow::Result<Vec<Utf8PathBuf>> {
    let output = Cmd::new("git", ["diff", "--name-only", "--cached"])
        .with_current_dir(curr_dir)
        .run()
        .await?;
    output.ensure_success("❌ Failed to list staged files")?;
    Ok(output.stdout().lines().map(Utf8PathBuf::from).collect())
}

fn branch_name_from_commit_message(commit_message: &str) -> String {
    let commit_message = commit_message
        .replace(['`', ':', ')', '"', '\''], "")
        .replace(['(', '/', '.'], "-");
    let trimmed = commit_message.trim().to_lowercase();
    trimmed.replace(" ", "-")
}

#[cfg(test)]
mod tests {
    use super::{branch_name_from_commit_message, branch_name_with_timestamp};

    #[test]
    fn test_branch_name_simple() {
        assert_eq!(branch_name_from_commit_message("Fix bug"), "fix-bug");
    }

    #[test]
    fn test_branch_name_with_special_chars() {
        assert_eq!(
            branch_name_from_commit_message("feat(scope): add feature"),
            "feat-scope-add-feature"
        );
    }

    #[test]
    fn test_branch_name_with_backticks() {
        assert_eq!(
            branch_name_from_commit_message("Fix `foo` function"),
            "fix-foo-function"
        );
    }

    #[test]
    fn test_branch_name_with_quotes() {
        assert_eq!(
            branch_name_from_commit_message("Update \"config\" file"),
            "update-config-file"
        );
    }

    #[test]
    fn test_branch_name_with_slashes() {
        assert_eq!(
            branch_name_from_commit_message("Fix path/to/file.rs"),
            "fix-path-to-file-rs"
        );
    }

    #[test]
    fn test_branch_name_preserves_hyphens() {
        assert_eq!(
            branch_name_from_commit_message("Add pre-commit hook"),
            "add-pre-commit-hook"
        );
    }

    #[test]
    fn test_branch_name_trims_whitespace() {
        assert_eq!(branch_name_from_commit_message("  Fix bug  "), "fix-bug");
    }

    #[test]
    fn test_branch_name_with_timestamp() {
        assert_eq!(
            branch_name_with_timestamp("feat-add-cache", "2026-02-16T00-14-36Z"),
            "feat-add-cache-2026-02-16T00-14-36Z"
        );
    }
}
