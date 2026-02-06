use anyhow::Context;
use camino::{Utf8Path, Utf8PathBuf};

use inquire::validator::Validation;

use crate::cmd::{Cmd, CmdOutput};

/// Check if copilot CLI is installed.
fn is_copilot_installed() -> bool {
    std::process::Command::new("which")
        .arg("copilot")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn get_untracked_files(repo_root: &Utf8Path) -> Vec<String> {
    let status_output = Cmd::new("git", ["status", "--porcelain", "-z"])
        .with_current_dir(repo_root)
        .hide_stdout()
        .run();

    if !status_output.status().success() {
        return Vec::new();
    }

    status_output
        .stdout()
        // `git status --porcelain -z` returns NUL-separated entries
        .split('\0')
        .filter(|&entry| entry.starts_with("?? "))
        .map(|entry| entry.trim_start_matches("?? ").to_string())
        .collect()
}

fn read_untracked_file(repo_root: &Utf8Path, relative_path: &str) -> anyhow::Result<String> {
    let full_path: Utf8PathBuf = repo_root.join(relative_path);
    match std::fs::read_to_string(&full_path) {
        Ok(content) => Ok(content),
        Err(_) => match std::fs::read(&full_path) {
            Ok(_bytes) => Ok("<binary file>".to_string()),
            Err(e) => anyhow::bail!("❌ Unable to read file {full_path}: {e}"),
        },
    }
}

fn build_untracked_context(repo_root: &Utf8Path) -> anyhow::Result<String> {
    let untracked_files = get_untracked_files(repo_root);
    if untracked_files.is_empty() {
        return Ok(String::new());
    }

    let mut context = String::from("\n\n# Untracked files\n");
    for relative_path in untracked_files {
        let content = read_untracked_file(repo_root, &relative_path)?;
        context.push_str(&format!("\n## {relative_path}\n{content}\n"));
    }
    Ok(context)
}

fn get_diff(repo_root: &Utf8Path) -> anyhow::Result<Option<String>> {
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

    let untracked_context = build_untracked_context(repo_root)?;
    let mut diff_with_untracked = diff.clone();
    if !untracked_context.is_empty() {
        diff_with_untracked.push_str(&untracked_context);
    }

    if diff_with_untracked.trim().is_empty() {
        Ok(None)
    } else {
        Ok(Some(diff_with_untracked))
    }
}

fn build_commit_prompt(diff: &str) -> String {
    format!(
        "Don't ask me questions or confirmation. Write a git commit message (max 70 characters) for these changes in one line: {}",
        diff.lines().collect::<Vec<_>>().join("\n")
    )
}

/// Generate a commit message using GitHub Copilot CLI.
pub fn generate_copilot_commit_message(
    repo_root: &Utf8Path,
    model: Option<&str>,
) -> anyhow::Result<String> {
    if !is_copilot_installed() {
        anyhow::bail!("❌ GitHub Copilot CLI is not installed");
    }

    let diff = get_diff(repo_root)
        .context("can't get repository diff")?
        .context("no changes to generate commit message for")?;

    println!("🤖 Generating commit message with GitHub Copilot...");

    let prompt = build_commit_prompt(&diff);
    let model = model.unwrap_or("gpt-5-mini");
    let output = Cmd::new(
        "copilot",
        ["--silent", "--model", model, "--prompt", &prompt],
    )
    .hide_stdout()
    .with_title(format!("🚀 copilot --silent --model {model} --prompt ..."))
    .with_current_dir(repo_root)
    .run();

    process_model_output(&output)
}

fn process_model_output(output: &CmdOutput) -> anyhow::Result<String> {
    if output.status().success() {
        let msg = output.stdout().trim().to_string();
        if msg.is_empty() {
            anyhow::bail!("❌ Generated commit message is empty")
        } else {
            if !is_commit_message_valid(&msg) {
                eprintln!(
                    "⚠️ {} Please adjust it before submitting.",
                    commit_message_size_rule(&msg)
                );
            }
            Ok(msg)
        }
    } else {
        anyhow::bail!(
            "❌ Failed to generate commit message with Gemini: {}",
            output.stderr()
        );
    }
}

/// Generate a commit message using Gemini CLI.
pub fn generate_gemini_commit_message(
    repo_root: &Utf8Path,
    model: Option<&str>,
) -> anyhow::Result<String> {
    let diff = get_diff(repo_root)
        .context("can't get repository diff")?
        .context("no changes to generate commit message for")?;

    println!("🤖 Generating commit message with Gemini...");

    let prompt = build_commit_prompt(&diff);
    let model = model.unwrap_or("gemini-3-flash-preview");
    let output = Cmd::new(
        "gemini",
        [
            "--model",
            model,
            "--sandbox",
            "--output-format",
            "text",
            "--prompt",
            &prompt,
        ],
    )
    .hide_stdout()
    .with_title(format!(
        "🚀 gemini --model {model} --sandbox --output-format text --prompt ..."
    ))
    .with_current_dir(repo_root)
    .run();

    process_model_output(&output)
}

pub fn generate_commit_message(
    repo_root: &Utf8Path,
    agent: Option<&crate::args::Agent>,
    model: Option<&str>,
) -> anyhow::Result<String> {
    match agent {
        Some(crate::args::Agent::Gemini) => generate_gemini_commit_message(repo_root, model),
        Some(crate::args::Agent::Copilot) => generate_copilot_commit_message(repo_root, model),
        None => Ok("".to_string()),
    }
}

/// Ask the user for a commit message and enforce size rules.
pub fn prompt_commit_message(initial_value: &str) -> anyhow::Result<String> {
    let msg = inquire::Text::new("Commit message:")
        .with_initial_value(initial_value)
        .with_validator(|input: &str| {
            if is_commit_message_valid(input) {
                Ok(Validation::Valid)
            } else {
                Ok(Validation::Invalid(commit_message_size_rule(input).into()))
            }
        })
        .prompt()
        .unwrap();
    Ok(msg)
}

pub fn check_commit_message(message: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        is_commit_message_valid(message),
        "{}",
        commit_message_size_rule(message)
    );
    Ok(())
}

fn commit_message_size_rule(message: &str) -> String {
    format!(
        "Commit message size should be between 1 and 70 characters. Current size: {}",
        message.len()
    )
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

    #[test]
    fn test_commit_message_size_rule() {
        assert_eq!(
            commit_message_size_rule("abc"),
            "Commit message size should be between 1 and 70 characters. Current size: 3"
        );
    }
}
