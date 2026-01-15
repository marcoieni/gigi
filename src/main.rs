mod args;
mod authors;
mod cmd;
mod commit;

use args::CliArgs;
use camino::{Utf8Path, Utf8PathBuf};
use clap::Parser as _;
use cmd::Cmd;
use git_cmd::Repo;

use crate::commit::{check_commit_message, prompt_commit_message};

fn main() -> anyhow::Result<()> {
    let args = CliArgs::parse();
    if !is_default_repo_set() {
        set_default_repo();
    }
    let repo_root = repo_root();
    let repo = Repo::new(repo_root.clone()).unwrap();
    match args.command {
        args::Command::OpenPr { message } => open_pr(repo_root, repo, message),
        args::Command::Squash { dry_run } => squash(repo_root, repo, dry_run),
    }?;
    Ok(())
}

fn is_default_repo_set() -> bool {
    let output = Cmd::new("gh", ["repo", "set-default", "--view"])
        .hide_stdout()
        .hide_stderr()
        .run();
    !output.stdout().trim().is_empty()
}

fn set_default_repo() {
    // Check if "upstream" remote exists
    let remotes_output = Cmd::new("git", ["remote"]).hide_stdout().run();
    let has_upstream = remotes_output
        .stdout()
        .lines()
        .any(|line| line.trim() == "upstream");

    let remote = if has_upstream { "upstream" } else { "origin" };
    Cmd::new("gh", ["repo", "set-default", remote]).run();
}

fn pr_title(repo_root: &Utf8Path) -> anyhow::Result<String> {
    let current_branch = current_branch(repo_root);
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
    .run();
    anyhow::ensure!(
        output.status().success() && !output.stdout().is_empty(),
        "❌ Failed to get PR title"
    );
    Ok(output.stdout().to_string())
}

fn default_branch(repo_root: &Utf8Path) -> String {
    Cmd::new(
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
    .stdout()
    .to_string()
}

fn current_branch(repo_root: &Utf8Path) -> String {
    Cmd::new("git", ["branch", "--show-current"])
        .with_current_dir(repo_root)
        .run()
        .stdout()
        .to_string()
}

fn branch_exists_locally(repo_root: &Utf8Path, branch_name: &str) -> bool {
    let output = Cmd::new("git", ["branch", "--list", branch_name])
        .with_current_dir(repo_root)
        .run();
    !output.stdout().trim().is_empty()
}

fn branch_exists_remotely(repo_root: &Utf8Path, branch_name: &str) -> bool {
    let output = Cmd::new("git", ["ls-remote", "--heads", "origin", branch_name])
        .with_current_dir(repo_root)
        .run();
    !output.stdout().trim().is_empty()
}

fn ensure_not_on_default_branch(repo_root: &Utf8Path) -> anyhow::Result<()> {
    let current_branch = current_branch(repo_root);
    let default_branch = default_branch(repo_root);
    anyhow::ensure!(
        current_branch != default_branch,
        "❌ Cannot push to default branch '{}'. Switch to a feature branch first.",
        default_branch
    );
    Ok(())
}

fn squash(repo_root: Utf8PathBuf, repo: Repo, dry_run: bool) -> anyhow::Result<()> {
    anyhow::ensure!(repo.is_clean().is_ok(), "❌ Repository is not clean");
    let feature_branch = repo.original_branch();
    let default_branch = default_branch(&repo_root);
    let pr_title = pr_title(&repo_root)?;
    anyhow::ensure!(
        feature_branch != default_branch,
        "❌ You are on the main branch. Switch to a feature branch to squash"
    );

    // sync branch
    Cmd::new("git", ["checkout", &default_branch])
        .with_current_dir(&repo_root)
        .run();
    Cmd::new("git", ["pull"]).with_current_dir(&repo_root).run();
    Cmd::new("git", ["checkout", feature_branch])
        .with_current_dir(&repo_root)
        .run();
    Cmd::new("git", ["merge", "origin", &default_branch])
        .with_current_dir(&repo_root)
        .run();

    let co_authors = authors::get_co_authors(&repo_root, &default_branch)?;
    let co_authors_text = authors::format_co_authors(&co_authors);

    if dry_run {
        println!("\n🔍 DRY RUN: The following commits would be squashed:");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

        let commits_to_squash = authors::get_commits_to_squash(&repo_root, &default_branch)?;
        if commits_to_squash.is_empty() {
            println!("⚠️  No commits to squash (already at merge base)");
        } else {
            for (i, commit) in commits_to_squash.iter().enumerate() {
                println!(
                    "{:2}. {} {} (by {})",
                    i + 1,
                    commit.hash,
                    commit.message,
                    commit.author
                );
            }
        }

        println!("\n📝 The resulting commit message would be:");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        let commit_message = format!("{}{}", pr_title, co_authors_text);
        println!("{}", commit_message);
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

        if !co_authors.is_empty() {
            println!("\n👥 Co-authors detected: {}", co_authors.len());
        }

        println!("\n💡 To perform the actual squash, run without --dry-run");
        return Ok(());
    }

    let merge_base = Cmd::new("git", ["merge-base", "HEAD", &default_branch])
        .with_current_dir(&repo_root)
        .run();
    anyhow::ensure!(
        merge_base.status().success(),
        "❌ Failed to find merge base"
    );
    Cmd::new("git", ["reset", "--soft", merge_base.stdout()])
        .with_current_dir(&repo_root)
        .run();
    Cmd::new("git", ["add", "."])
        .with_current_dir(&repo_root)
        .run();

    // Create commit message with co-authors
    let commit_message = format!("{}{}", pr_title, co_authors_text);
    Cmd::new("git", ["commit", "-m", &commit_message])
        .with_current_dir(&repo_root)
        .run();

    ensure_not_on_default_branch(&repo_root)?;
    Cmd::new("git", ["push", "--force-with-lease"])
        .with_current_dir(&repo_root)
        .run();

    view_pr_in_browser(&repo_root);

    Ok(())
}

fn view_pr_in_browser(repo_root: &Utf8Path) {
    Cmd::new("gh", ["pr", "view", "-w", "pr", "show"])
        .with_current_dir(repo_root)
        .run()
        .stdout()
        .to_string();
}

fn open_pr(repo_root: Utf8PathBuf, repo: Repo, message: Option<String>) -> anyhow::Result<()> {
    let commit_message = match message {
        Some(msg) => {
            check_commit_message(&msg)?;
            msg
        }
        None => prompt_commit_message(&repo_root)?,
    };

    // Always start from an up-to-date default branch, then create a feature branch
    let default_branch_name = default_branch(&repo_root);
    // Derive branch name from commit message (simple slug)
    let branch_name = branch_name_from_commit_message(&commit_message);

    // Check if branch exists locally or remotely
    if branch_exists_locally(&repo_root, &branch_name) {
        anyhow::bail!(
            "❌ Branch '{}' already exists locally. Please use a different commit message or delete the existing branch.",
            branch_name
        );
    }

    if branch_exists_remotely(&repo_root, &branch_name) {
        anyhow::bail!(
            "❌ Branch '{}' already exists on remote. Please use a different commit message or delete the remote branch.",
            branch_name
        );
    }

    // Update default branch locally
    Cmd::new("git", ["checkout", &default_branch_name])
        .with_current_dir(&repo_root)
        .run();
    Cmd::new("git", ["pull", "--ff-only"])
        .with_current_dir(&repo_root)
        .run();

    // Create the feature branch
    Cmd::new("git", ["checkout", "-b", &branch_name])
        .with_current_dir(&repo_root)
        .run();

    let staged_files = get_staged_files(&repo_root);
    println!("ℹ️ Staged files: {:?}", staged_files);
    if staged_files.is_empty() {
        run_git_add(changed_files(&repo), repo.directory());
    } else {
        run_git_add(staged_files, &repo_root);
    }

    let output = Cmd::new("git", ["commit", "-m", &commit_message])
        .with_current_dir(&repo_root)
        .run();
    if output.stdout().contains("nothing to commit") {
        panic!("❌ Nothing to commit");
    }

    // Ensure we're not on the default branch before pushing
    ensure_not_on_default_branch(&repo_root)?;

    // Push branch (set upstream)
    Cmd::new("git", ["push", "-u", "origin", &branch_name])
        .with_current_dir(&repo_root)
        .run();

    // If a PR already exists, open it; otherwise create a new one.
    let pr_view = Cmd::new("gh", ["pr", "view", "--json", "number", "-q", ".number"]) // relies on current branch
        .with_current_dir(&repo_root)
        .run();
    if pr_view.status().success() {
        // Open existing PR in browser
        Cmd::new("gh", ["pr", "view", "--web"]) // show existing PR
            .with_current_dir(&repo_root)
            .run();
    } else {
        // Create new PR using commit message as title
        Cmd::new(
            "gh",
            [
                "pr",
                "create",
                "--title",
                &commit_message,
                "--body",
                "",
                "--web",
            ],
        )
        .with_current_dir(&repo_root)
        .run();
    }
    Ok(())
}

fn repo_root() -> Utf8PathBuf {
    let git_root = Cmd::new("git", ["rev-parse", "--show-toplevel"]).run();
    camino::Utf8PathBuf::from(git_root.stdout())
}

fn get_staged_files(curr_dir: &Utf8Path) -> Vec<Utf8PathBuf> {
    let output = Cmd::new("git", ["diff", "--name-only", "--cached"])
        .with_current_dir(curr_dir)
        .run();
    output.stdout().lines().map(Utf8PathBuf::from).collect()
}

fn changed_files(repo: &Repo) -> Vec<Utf8PathBuf> {
    let git_root = repo.git(&["rev-parse", "--show-toplevel"]).unwrap();
    let git_root = camino::Utf8Path::new(&git_root);
    let changed_files = repo.changes_except_typechanges().unwrap();
    assert!(!changed_files.is_empty(), "Run git add first");
    changed_files.iter().map(|f| git_root.join(f)).collect()
}

fn run_git_add(changed_files: Vec<Utf8PathBuf>, repo_root: &Utf8Path) {
    assert!(!changed_files.is_empty(), "No files to add");
    let mut git_add_args = vec!["add".to_string()];
    let changed_files: Vec<String> = changed_files.iter().map(|f| f.to_string()).collect();
    git_add_args.extend(changed_files);

    Cmd::new("git", &git_add_args)
        .with_current_dir(repo_root)
        .run();
}

fn branch_name_from_commit_message(commit_message: &str) -> String {
    let commit_message = commit_message
        .replace(['`', ':', ')', '"'], "")
        .replace(['(', '/', '.'], "-");
    let trimmed = commit_message.trim().to_lowercase();
    trimmed.replace(" ", "-")
}
