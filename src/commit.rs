use camino::Utf8Path;

use crate::cmd::Cmd;


/// Check if copilot CLI is installed.
fn is_copilot_installed() -> bool {
    std::process::Command::new("which")
        .arg("copilot")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Generate a commit message using GitHub Copilot CLI.
fn generate_ai_commit_message(repo_root: &Utf8Path) -> Option<String> {
    if !is_copilot_installed() {
        return None;
    }

    // Get the diff to help copilot understand what changed
    let diff_output = Cmd::new("git", ["diff", "--cached"])
        .with_current_dir(repo_root)
        .hide_stdout()
        .run();
    let diff = diff_output.stdout();

    // If no staged changes, check unstaged changes
    let diff = if diff.trim().is_empty() {
        Cmd::new("git", ["diff"])
            .with_current_dir(repo_root)
            .hide_stdout()
            .run()
            .stdout()
            .to_string()
    } else {
        diff.to_string()
    };

    if diff.trim().is_empty() {
        return None;
    }

    println!("🤖 Generating commit message with GitHub Copilot...");

    // Run copilot and capture stdout
    let prompt = format!(
        "Don't ask me questions or confirmation. Just write a short git commit message for these changes in one line: {}",
        diff.lines().take(20).collect::<Vec<_>>().join("\n")
    );
    let output = Cmd::new(
        "copilot",
        ["--silent", "--model", "gpt-5-mini", "--prompt", &prompt],
    )
    .with_current_dir(repo_root)
    .run();

    if output.status().success() {
        let msg = output.stdout().trim().to_string();
        if msg.is_empty() {
            None
        } else {
            Some(msg)
        }
    } else {
        None
    }
}

/// Ask the user for a commit message and enforce size rules.
pub fn prompt_commit_message(repo_root: &Utf8Path) -> anyhow::Result<String> {
    let initial_value = generate_ai_commit_message(repo_root).unwrap_or_default();

    let msg = inquire::Text::new("Commit message")
        .with_initial_value(&initial_value)
        .prompt()
        .unwrap();
    check_commit_message(&msg)?;
    Ok(msg)
}

pub fn check_commit_message(message: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        !message.is_empty() && message.len() < 71,
        "Commit message size should be between 1 and 70 characters. Current size: {}",
        message.len()
    );
    Ok(())
}
