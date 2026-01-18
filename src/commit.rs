use camino::Utf8Path;

use inquire::validator::Validation;

use crate::cmd::Cmd;

/// Check if copilot CLI is installed.
fn is_copilot_installed() -> bool {
    std::process::Command::new("which")
        .arg("copilot")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn get_diff(repo_root: &Utf8Path) -> Option<String> {
    // Get the diff to help understand what changed
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
        None
    } else {
        Some(diff)
    }
}

fn build_commit_prompt(diff: &str) -> String {
    format!(
        "Don't ask me questions or confirmation. Just write a short git commit message for these changes in one line: {}",
        diff.lines().collect::<Vec<_>>().join("\n")
    )
}

/// Generate a commit message using GitHub Copilot CLI.
pub fn generate_copilot_commit_message(repo_root: &Utf8Path) -> Option<String> {
    if !is_copilot_installed() {
        return None;
    }

    let diff = get_diff(repo_root)?;

    println!("🤖 Generating commit message with GitHub Copilot...");

    let prompt = build_commit_prompt(&diff);
    // gpt-5-mini should be free (no premium requests)
    let model = "gpt-5-mini";
    let output = Cmd::new(
        "copilot",
        ["--silent", "--model", model, "--prompt", &prompt],
    )
    .hide_stdout()
    .with_title(format!("🚀 copilot --silent --model {model} --prompt ..."))
    .with_current_dir(repo_root)
    .run();

    if output.status().success() {
        let msg = output.stdout().trim().to_string();
        if msg.is_empty() { None } else { Some(msg) }
    } else {
        None
    }
}

/// Generate a commit message using Gemini CLI.
pub fn generate_gemini_commit_message(repo_root: &Utf8Path) -> Option<String> {
    let diff = get_diff(repo_root)?;

    println!("🤖 Generating commit message with Gemini...");

    let prompt = build_commit_prompt(&diff);
    let output = Cmd::new(
        "gemini",
        ["--model", "gemini-3-flash-preview", "--sandbox", &prompt],
    )
    .hide_stdout()
    .with_title("🚀 gemini --model gemini-3-flash-preview --sandbox ...".to_string())
    .with_current_dir(repo_root)
    .run();

    if output.status().success() {
        let msg = output.stdout().trim().to_string();
        if msg.is_empty() { None } else { Some(msg) }
    } else {
        None
    }
}

/// Ask the user for a commit message and enforce size rules.
pub fn prompt_commit_message(
    repo_root: &Utf8Path,
    agent: Option<crate::args::Agent>,
) -> anyhow::Result<String> {
    let initial_value = match agent {
        Some(crate::args::Agent::Copilot) => generate_copilot_commit_message(repo_root),
        Some(crate::args::Agent::Gemini) => generate_gemini_commit_message(repo_root),
        None => generate_copilot_commit_message(repo_root),
    }
    .unwrap_or_default();

    let msg = inquire::Text::new("Commit message:")
        .with_initial_value(&initial_value)
        .with_validator(|input: &str| {
            if is_commit_message_valid(input) {
                Ok(Validation::Valid)
            } else {
                Ok(Validation::Invalid(
                    format!(
                        "Commit message size should be between 1 and 70 characters. Current size: {}",
                        input.len()
                    )
                    .into(),
                ))
            }
        })
        .prompt()
        .unwrap();
    Ok(msg)
}

pub fn check_commit_message(message: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        is_commit_message_valid(message),
        "Commit message size should be between 1 and 70 characters. Current size: {}",
        message.len()
    );
    Ok(())
}

fn is_commit_message_valid(message: &str) -> bool {
    !message.is_empty() && message.len() <= 70
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_message() {
        assert!(is_commit_message_valid("Fix bug"));
    }

    #[test]
    fn test_empty_message_invalid() {
        assert!(!is_commit_message_valid(""));
    }

    #[test]
    fn test_message_at_max_length() {
        let msg = "a".repeat(70);
        assert!(is_commit_message_valid(&msg));
    }

    #[test]
    fn test_message_over_max_length() {
        let msg = "a".repeat(71);
        assert!(!is_commit_message_valid(&msg));
    }

    #[test]
    fn test_single_char_valid() {
        assert!(is_commit_message_valid("a"));
    }

    #[test]
    fn test_check_commit_message_valid() {
        assert!(check_commit_message("Valid message").is_ok());
    }

    #[test]
    fn test_check_commit_message_empty() {
        assert!(check_commit_message("").is_err());
    }

    #[test]
    fn test_check_commit_message_too_long() {
        let msg = "a".repeat(71);
        assert!(check_commit_message(&msg).is_err());
    }
}
