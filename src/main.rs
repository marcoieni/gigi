mod args;
mod cmd;

use args::CliArgs;
use camino::{Utf8Path, Utf8PathBuf};
use clap::Parser as _;
use cmd::Cmd;
use git_cmd::Repo;

fn main() -> anyhow::Result<()> {
    assert_default_repo_is_set();
    let repo_root = repo_root();
    let repo = Repo::new(repo_root.clone()).unwrap();
    let args = CliArgs::parse();
    match args.command {
        args::Command::OpenPr => open_pr(repo_root, repo),
        args::Command::Squash => squash(repo_root, repo),
    }?;
    Ok(())
}

fn assert_default_repo_is_set() {
    let output = Cmd::new("gh", ["repo", "set-default", "--view"]).run();
    if output.stdout().trim().is_empty() {
        panic!("❌ Please run `gh repo set-default` first");
    }
}

fn pr_title() -> anyhow::Result<String> {
    let output = Cmd::new("gh", ["pr", "view", "--json", "title", "-q", ".title"]).run();
    anyhow::ensure!(output.status().success(), "❌ Failed to get PR title");
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

fn squash(repo_root: Utf8PathBuf, repo: Repo) -> anyhow::Result<()> {
    anyhow::ensure!(repo.is_clean().is_ok(), "❌ Repository is not clean");
    let feature_branch = repo.original_branch();
    let default_branch = default_branch(&repo_root);
    let pr_title = pr_title()?;
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

    Cmd::new("git", ["squash"])
        .with_current_dir(&repo_root)
        .run();
    Cmd::new("git", ["add", "."])
        .with_current_dir(&repo_root)
        .run();
    Cmd::new("git", ["commit", "-m", &pr_title])
        .with_current_dir(&repo_root)
        .run();

    ensure_not_on_default_branch(&repo_root)?;
    Cmd::new("git", &["push", "--force-with-lease"])
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

fn open_pr(repo_root: Utf8PathBuf, repo: Repo) -> anyhow::Result<()> {
    Cmd::new("git", ["pull"]).with_current_dir(&repo_root).run();
    let commit_message = inquire::Text::new("Commit message").prompt().unwrap();
    anyhow::ensure!(
        !commit_message.is_empty() && commit_message.len() < 71,
        format!(
            "Commit message size should be between 1 and 70 characters. Current size: {}",
            commit_message.len()
        )
    );
    let default_branch_name = branch_name_from_commit_message(&commit_message);
    let branch_name = default_branch_name;
    // let branch_name = inquire::Text::new("Branch name")
    //     .with_default(&default_branch_name)
    //     .prompt()
    //     .unwrap();

    let staged_files = get_staged_files(&repo_root);
    println!("ℹ️ Staged files: {:?}", staged_files);
    Cmd::new("git-town", ["hack", &branch_name])
        .with_current_dir(&repo_root)
        .run();

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

    // Ensure we're not on the default branch before proposing/pushing
    ensure_not_on_default_branch(&repo_root)?;

    Cmd::new("git-town", ["propose"])
        .with_current_dir(&repo_root)
        .run();
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
