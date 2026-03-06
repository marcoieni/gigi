use camino::Utf8Path;
use serde_json::{Map, Value};

use crate::{
    args::Agent,
    cmd::{Cmd, ensure_command_available},
    terminal::strip_control_sequences,
};

#[derive(Debug, Clone)]
pub struct ReviewResult {
    pub markdown: String,
    pub requires_code_changes: bool,
    pub provider: String,
    pub model: Option<String>,
}

pub async fn review_pr(
    repo_root: &Utf8Path,
    pr_url: &str,
    agent: Option<&Agent>,
    model: Option<&str>,
) -> anyhow::Result<()> {
    eprintln!("🔍 Review started: {pr_url}");
    let result = match generate_review(repo_root, pr_url, agent, model).await {
        Ok(result) => result,
        Err(err) => {
            eprintln!("❌ Review failed: {pr_url}: {err}");
            return Err(err);
        }
    };
    println!("{}", result.markdown);
    eprintln!("✅ Review finished: {pr_url}");
    Ok(())
}

pub async fn generate_review(
    repo_root: &Utf8Path,
    pr_url: &str,
    agent: Option<&Agent>,
    model: Option<&str>,
) -> anyhow::Result<ReviewResult> {
    let metadata = fetch_pr_metadata(repo_root, pr_url).await?;
    let diff = fetch_pr_diff(repo_root, pr_url).await?;
    let prompt = build_review_prompt(&metadata, &diff);

    let (provider, resolved_model, markdown) =
        run_ai_prompt(repo_root, &prompt, agent, model).await?;
    let requires_code_changes = parse_requires_code_changes(&markdown).unwrap_or(true);

    Ok(ReviewResult {
        markdown,
        requires_code_changes,
        provider,
        model: resolved_model,
    })
}

pub async fn run_fix(
    repo_root: &Utf8Path,
    pr_url: &str,
    review_markdown: &str,
    agent: Option<&Agent>,
    model: Option<&str>,
) -> anyhow::Result<String> {
    let metadata = fetch_pr_metadata(repo_root, pr_url).await?;
    let diff = fetch_pr_diff(repo_root, pr_url).await?;
    let prompt = build_fix_prompt(&metadata, &diff, review_markdown);

    let (_, _, output) = run_ai_prompt(repo_root, &prompt, agent, model).await?;
    Ok(output)
}

pub fn parse_requires_code_changes(review_markdown: &str) -> Option<bool> {
    for line in review_markdown.lines() {
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix("REQUIRES_CODE_CHANGES:") {
            let value = value.trim().to_uppercase();
            return match value.as_str() {
                "YES" => Some(true),
                "NO" => Some(false),
                _ => None,
            };
        }
    }
    None
}

pub fn sanitize_review_markdown(review_markdown: &str) -> String {
    let stripped = strip_control_sequences(review_markdown);
    let mut lines = Vec::new();

    for line in stripped.lines() {
        lines.push(normalize_requires_code_changes_line(line));
    }

    if stripped.ends_with('\n') {
        format!("{}\n", lines.join("\n"))
    } else {
        lines.join("\n")
    }
}

fn normalize_requires_code_changes_line(line: &str) -> String {
    let trimmed = line.trim_start();
    if let Some(rest) = trimmed.strip_prefix('>') {
        let rest = rest.trim_start();
        if rest.starts_with("REQUIRES_CODE_CHANGES:") {
            return rest.to_string();
        }
    }

    line.to_string()
}

async fn fetch_pr_metadata(repo_root: &Utf8Path, pr_url: &str) -> anyhow::Result<String> {
    let output = Cmd::new(
        "gh",
        [
            "pr",
            "view",
            pr_url,
            "--json",
            "title,body,author,baseRefName,headRefName,createdAt,updatedAt,assignees,reviews,comments,commits,url",
        ],
    )
    .with_current_dir(repo_root)
    .run()
    .await?;
    output.ensure_success("❌ Failed to fetch PR metadata")?;
    anyhow::ensure!(
        !output.stdout().trim().is_empty(),
        "❌ Failed to fetch PR metadata: command returned empty output"
    );
    minimize_pr_metadata(output.stdout())
}

fn minimize_pr_metadata(metadata: &str) -> anyhow::Result<String> {
    let mut value: Value = serde_json::from_str(metadata)?;

    if let Some(author) = value.get_mut("author")
        && let Some(login) = author.get("login").and_then(Value::as_str)
    {
        *author = Value::String(login.to_string());
    }

    if let Some(comments) = value.get_mut("comments")
        && let Some(array) = comments.as_array()
    {
        let slim: Vec<Value> = array
            .iter()
            .filter_map(|comment| {
                let login = comment
                    .get("author")
                    .and_then(|author| author.get("login"))
                    .and_then(Value::as_str);
                let body = comment.get("body").and_then(Value::as_str);

                if login.is_none() && body.is_none() {
                    return None;
                }

                let mut map = Map::new();
                if let Some(login) = login {
                    map.insert("login".to_string(), Value::String(login.to_string()));
                }
                if let Some(body) = body {
                    map.insert("body".to_string(), Value::String(body.to_string()));
                }

                Some(Value::Object(map))
            })
            .collect();

        *comments = Value::Array(slim);
    }

    Ok(serde_json::to_string(&value)?)
}

async fn fetch_pr_diff(repo_root: &Utf8Path, pr_url: &str) -> anyhow::Result<String> {
    let output = Cmd::new("gh", ["pr", "diff", pr_url, "--color=never"])
        .with_current_dir(repo_root)
        .run()
        .await?;
    output.ensure_success("❌ Failed to fetch PR diff")?;
    anyhow::ensure!(
        !output.stdout().trim().is_empty(),
        "❌ Failed to fetch PR diff: command returned empty output"
    );
    Ok(output.stdout().to_string())
}

fn build_review_prompt(metadata: &str, diff: &str) -> String {
    format!(
        "You are an expert code reviewer. Review this GitHub pull request and write your review in Markdown.\n\nOutput format rules (mandatory):\n1) First line must be exactly: REQUIRES_CODE_CHANGES: YES or REQUIRES_CODE_CHANGES: NO\n2) Then include sections: Summary, Issues, Suggestions.\n3) If there are no issues, explicitly say so under Issues.\n4) Keep the response concise and specific.\n5) Refer to files and code hunks when possible.\n\nPR METADATA (JSON):\n{metadata}\n\nPR DIFF:\n{diff}\n"
    )
}

fn build_fix_prompt(metadata: &str, diff: &str, review_markdown: &str) -> String {
    format!(
        "You are an expert software engineer. Apply fixes for this GitHub pull request directly in the current repository working tree.\n\nRules:\n- Do not ask questions.\n- Implement the fixes requested by the review below.\n- Keep changes minimal and targeted.\n- Preserve existing style and conventions.\n- Do not create commits.\n\nREVIEW TO IMPLEMENT:\n{review_markdown}\n\nPR METADATA (JSON):\n{metadata}\n\nPR DIFF:\n{diff}\n"
    )
}

async fn run_ai_prompt(
    repo_root: &Utf8Path,
    prompt: &str,
    agent: Option<&Agent>,
    model: Option<&str>,
) -> anyhow::Result<(String, Option<String>, String)> {
    match agent {
        Some(Agent::Gemini) => run_gemini(repo_root, prompt, model).await,
        Some(Agent::Kiro) => run_kiro(repo_root, prompt, model).await,
        Some(Agent::Copilot) | None => run_copilot(repo_root, prompt, model).await,
    }
}

async fn run_copilot(
    repo_root: &Utf8Path,
    prompt: &str,
    model: Option<&str>,
) -> anyhow::Result<(String, Option<String>, String)> {
    let resolved_model = model.unwrap_or("gpt-5.3-codex").to_string();
    let output = Cmd::new(
        "copilot",
        ["--silent", "--model", &resolved_model, "--prompt", prompt],
    )
    .hide_stdout()
    .with_title(format!(
        "🚀 copilot --silent --model {resolved_model} --prompt ..."
    ))
    .with_current_dir(repo_root)
    .run()
    .await?;

    output.ensure_success("❌ Failed to generate output with Copilot")?;
    anyhow::ensure!(
        !output.stdout().trim().is_empty(),
        "❌ Copilot returned empty output"
    );

    Ok((
        "copilot".to_string(),
        Some(resolved_model),
        sanitize_review_markdown(output.stdout()),
    ))
}

async fn run_gemini(
    repo_root: &Utf8Path,
    prompt: &str,
    model: Option<&str>,
) -> anyhow::Result<(String, Option<String>, String)> {
    let resolved_model = model.unwrap_or("gemini-3-pro-preview").to_string();
    let output = Cmd::new(
        "gemini",
        [
            "--model",
            &resolved_model,
            "--sandbox",
            "--output-format",
            "text",
            "--prompt",
            prompt,
        ],
    )
    .hide_stdout()
    .with_title(format!(
        "🚀 gemini --model {resolved_model} --sandbox --output-format text --prompt ..."
    ))
    .with_current_dir(repo_root)
    .run()
    .await?;

    output.ensure_success("❌ Failed to generate output with Gemini")?;
    anyhow::ensure!(
        !output.stdout().trim().is_empty(),
        "❌ Gemini returned empty output"
    );

    Ok((
        "gemini".to_string(),
        Some(resolved_model),
        sanitize_review_markdown(output.stdout()),
    ))
}

async fn run_kiro(
    repo_root: &Utf8Path,
    prompt: &str,
    model: Option<&str>,
) -> anyhow::Result<(String, Option<String>, String)> {
    ensure_command_available("kiro-cli").await?;

    let resolved_model = model
        .unwrap_or(crate::config::DEFAULT_KIRO_MODEL)
        .to_string();
    let mut args = vec![
        "chat".to_string(),
        "--no-interactive".to_string(),
        "--model".to_string(),
        resolved_model.clone(),
    ];
    args.push(prompt.to_string());

    let output = Cmd::new("kiro-cli", args)
        .hide_stdout()
        .with_title(format!(
            "🚀 kiro-cli chat --no-interactive --model {resolved_model} ..."
        ))
        .with_current_dir(repo_root)
        .run()
        .await?;

    output.ensure_success("❌ Failed to generate output with Kiro")?;
    anyhow::ensure!(
        !output.stdout().trim().is_empty(),
        "❌ Kiro returned empty output"
    );

    Ok((
        "kiro".to_string(),
        Some(resolved_model),
        sanitize_review_markdown(output.stdout()),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_requires_code_changes_yes() {
        let value = parse_requires_code_changes("REQUIRES_CODE_CHANGES: YES\nrest").unwrap();
        assert!(value);
    }

    #[test]
    fn parses_requires_code_changes_no() {
        let value = parse_requires_code_changes("REQUIRES_CODE_CHANGES: NO\nrest").unwrap();
        assert!(!value);
    }

    #[test]
    fn returns_none_when_missing_field() {
        assert!(parse_requires_code_changes("No header").is_none());
    }

    #[test]
    fn parses_requires_code_changes_with_ansi_sequences() {
        let value = parse_requires_code_changes(&sanitize_review_markdown(
            "\u{1b}[38;5;141m> \u{1b}[0mREQUIRES_CODE_CHANGES: NO\u{1b}[0m\nrest",
        ));
        assert_eq!(value, Some(false));
    }
}
