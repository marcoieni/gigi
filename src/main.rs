use std::process::{Command, ExitStatus};

use camino::Utf8PathBuf;
use tracing::debug;

fn main() {
    let commit_message = inquire::Text::new("Commit message").prompt().unwrap();
    let default_branch_name = branch_name_from_commit_message(&commit_message);
    let branch_name = inquire::Text::new("Branch name")
        .with_default(&default_branch_name)
        .prompt()
        .unwrap();

    let staged_files = get_staged_files();
    println!("ℹ️ Staged files: {:?}", staged_files);
    run_cmd("git-town", ["hack", &branch_name]);

    if staged_files.is_empty() {
        run_git_add(changed_files());
    } else {
        run_git_add(staged_files);
    }

    let output = run_cmd("git", ["commit", "-m", &commit_message]);
    if output.stdout.contains("nothing to commit") {
        panic!("❌ Nothing to commit");
    }

    run_cmd("git-town", ["propose"]);
}

fn get_staged_files() -> Vec<Utf8PathBuf> {
    let output = run_cmd("git", ["diff", "--name-only", "--cached"]);
    output.stdout.lines().map(Utf8PathBuf::from).collect()
}

fn changed_files() -> Vec<Utf8PathBuf> {
    let current_dir = std::env::current_dir().unwrap();
    let current_dir = camino::Utf8PathBuf::from_path_buf(current_dir).unwrap();
    let repo = git_cmd::Repo::new(current_dir).unwrap();
    let git_root = repo.git(&["rev-parse", "--show-toplevel"]).unwrap();
    let git_root = camino::Utf8Path::new(&git_root);
    let changed_files = repo.changes_except_typechanges().unwrap();
    assert!(!changed_files.is_empty(), "Run git add first");
    changed_files.iter().map(|f| git_root.join(f)).collect()
}

fn run_git_add(changed_files: Vec<Utf8PathBuf>) {
    assert!(!changed_files.is_empty(), "No files to add");
    let mut git_add_args = vec!["add".to_string()];
    let changed_files: Vec<String> = changed_files.iter().map(|f| f.to_string()).collect();
    git_add_args.extend(changed_files);

    run_cmd("git", &git_add_args);
}

fn branch_name_from_commit_message(commit_message: &str) -> String {
    let commit_message = commit_message
        .replace(['`', ':', ')', '"'], "")
        .replace('(', "-");
    let trimmed = commit_message.trim().to_lowercase();
    trimmed.replace(" ", "-")
}

pub struct CmdOutput {
    pub status: ExitStatus,
    pub stdout: String,
    pub stderr: String,
}

fn run_cmd<I, S>(cmd_name: &str, args: I) -> CmdOutput
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let args: Vec<String> = args
        .into_iter()
        .map(|arg| arg.as_ref().to_string())
        .collect();
    println!("🚀 {} {}", cmd_name, args.join(" "));
    let child = Command::new(cmd_name).args(args).spawn().unwrap();
    let output = child.wait_with_output().unwrap();

    let output_stdout = String::from_utf8(output.stdout).unwrap();
    let output_stderr = String::from_utf8(output.stderr).unwrap();

    debug!("{cmd_name} stderr: {}", output_stderr);
    debug!("{cmd_name} stdout: {}", output_stdout);
    assert!(output.status.success());

    CmdOutput {
        status: output.status,
        stdout: output_stdout,
        stderr: output_stderr,
    }
}
