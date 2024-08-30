mod args;
mod cmd;

use args::CliArgs;
use camino::{Utf8Path, Utf8PathBuf};
use clap::Parser as _;
use cmd::Cmd;
use git_cmd::Repo;

fn main() -> anyhow::Result<()> {
    let repo_root = repo_root();
    let repo = Repo::new(repo_root.clone()).unwrap();
    let args = CliArgs::parse();
    match args.command {
        args::Command::OpenPr => open_pr(repo_root, repo),
        args::Command::Squash => squash(repo_root, repo),
    }?;
    Ok(())
}

fn squash(repo_root: Utf8PathBuf, repo: Repo) -> anyhow::Result<()> {
    let current_branch = repo.original_branch();
    anyhow::ensure!(
        current_branch != "master" && current_branch != "main",
        "❌ You are on the main branch. Switch to a feature branch to squash"
    );
    let current_commit_message = repo.current_commit_message().unwrap();
    Cmd::new("git", ["squash"])
        .with_current_dir(&repo_root)
        .run();
    Cmd::new("git", ["add", "."])
        .with_current_dir(&repo_root)
        .run();
    Cmd::new("git", ["commit", "-m", &current_commit_message])
        .with_current_dir(&repo_root)
        .run();
    Cmd::new("git", ["push", "--force-with-lease"])
        .with_current_dir(&repo_root)
        .run();
    Ok(())
}

fn open_pr(repo_root: Utf8PathBuf, repo: Repo) -> anyhow::Result<()> {
    let commit_message = inquire::Text::new("Commit message").prompt().unwrap();
    anyhow::ensure!(
        !commit_message.is_empty() && commit_message.len() < 71,
        format!(
            "Commit message size should be between 1 and 70 characters. Current size: {}",
            commit_message.len()
        )
    );
    let default_branch_name = branch_name_from_commit_message(&commit_message);
    let branch_name = inquire::Text::new("Branch name")
        .with_default(&default_branch_name)
        .prompt()
        .unwrap();

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
        .replace(['(', '/'], "-");
    let trimmed = commit_message.trim().to_lowercase();
    trimmed.replace(" ", "-")
}
