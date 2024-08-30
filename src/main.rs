mod cmd;

use camino::Utf8PathBuf;
use cmd::Cmd;
use git_cmd::Repo;

fn main() -> anyhow::Result<()> {
    let repo = get_repo();
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

    let staged_files = get_staged_files();
    println!("ℹ️ Staged files: {:?}", staged_files);
    Cmd::new("git-town", ["hack", &branch_name]).run();

    if staged_files.is_empty() {
        run_git_add(changed_files(&repo));
    } else {
        run_git_add(staged_files);
    }

    let output = Cmd::new("git", ["commit", "-m", &commit_message]).run();
    if output.stdout().contains("nothing to commit") {
        panic!("❌ Nothing to commit");
    }

    Cmd::new("git-town", ["propose"]).run();
    Ok(())
}

fn get_staged_files() -> Vec<Utf8PathBuf> {
    let output = Cmd::new("git", ["diff", "--name-only", "--cached"]).run();
    output.stdout().lines().map(Utf8PathBuf::from).collect()
}

fn changed_files(repo: &Repo) -> Vec<Utf8PathBuf> {
    let git_root = repo.git(&["rev-parse", "--show-toplevel"]).unwrap();
    let git_root = camino::Utf8Path::new(&git_root);
    let changed_files = repo.changes_except_typechanges().unwrap();
    assert!(!changed_files.is_empty(), "Run git add first");
    changed_files.iter().map(|f| git_root.join(f)).collect()
}

fn get_repo() -> git_cmd::Repo {
    let current_dir = std::env::current_dir().unwrap();
    let current_dir = camino::Utf8PathBuf::from_path_buf(current_dir).unwrap();
    git_cmd::Repo::new(current_dir).unwrap()
}

fn run_git_add(changed_files: Vec<Utf8PathBuf>) {
    assert!(!changed_files.is_empty(), "No files to add");
    let mut git_add_args = vec!["add".to_string()];
    let changed_files: Vec<String> = changed_files.iter().map(|f| f.to_string()).collect();
    git_add_args.extend(changed_files);

    Cmd::new("git", &git_add_args).run();
}

fn branch_name_from_commit_message(commit_message: &str) -> String {
    let commit_message = commit_message
        .replace(['`', ':', ')', '"'], "")
        .replace(['(', '/'], "-");
    let trimmed = commit_message.trim().to_lowercase();
    trimmed.replace(" ", "-")
}
