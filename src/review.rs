use camino::Utf8Path;
use serde_json::{Map, Value};

use crate::{args::Agent, cmd::Cmd};

pub fn review_pr(
    repo_root: &Utf8Path,
    pr_url: &str,
    agent: Option<&Agent>,
    model: Option<&str>,
) -> anyhow::Result<()> {
    let metadata = fetch_pr_metadata(repo_root, pr_url)?;
    //println!("----\n\nmetadata: {}\n\n----\n", metadata);
    let diff = fetch_pr_diff(repo_root, pr_url)?;
    let prompt = build_review_prompt(&metadata, &diff);

    let review = match agent {
        Some(Agent::Gemini) => generate_gemini_review(repo_root, &prompt, model),
        Some(Agent::Copilot) | None => generate_copilot_review(repo_root, &prompt, model),
    }?;

    println!("{review}");
    Ok(())
}

fn fetch_pr_metadata(repo_root: &Utf8Path, pr_url: &str) -> anyhow::Result<String> {
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
    .run();
    anyhow::ensure!(
        output.status().success() && !output.stdout().trim().is_empty(),
        "❌ Failed to fetch PR metadata"
    );
    minimize_pr_metadata(output.stdout())
}

fn minimize_pr_metadata(metadata: &str) -> anyhow::Result<String> {
    let mut value: Value = serde_json::from_str(metadata)?;

    if let Some(author) = value.get_mut("author") {
        if let Some(login) = author.get("login").and_then(Value::as_str) {
            *author = Value::String(login.to_string());
        }
    }

    if let Some(comments) = value.get_mut("comments") {
        if let Some(array) = comments.as_array() {
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
    }

    Ok(serde_json::to_string(&value)?)
}

fn fetch_pr_diff(repo_root: &Utf8Path, pr_url: &str) -> anyhow::Result<String> {
    let output = Cmd::new("gh", ["pr", "diff", pr_url, "--color=never"])
        .with_current_dir(repo_root)
        .run();
    anyhow::ensure!(
        output.status().success() && !output.stdout().trim().is_empty(),
        "❌ Failed to fetch PR diff"
    );
    Ok(output.stdout().to_string())
}

fn build_review_prompt(metadata: &str, diff: &str) -> String {
    format!(
        "You are an expert code reviewer. Review this GitHub pull request and write your review in Markdown.\n\nRules:\n- Do not ask questions unless information is missing.\n- Be concise but specific.\n- Include a short summary, then a list of issues (if any) with severity labels (BLOCKER, MAJOR, MINOR), and then suggestions.\n- If there are no issues, say so explicitly.\n- Refer to files and code hunks where possible.\n\nPR METADATA (JSON):\n{metadata}\n\nPR DIFF:\n{diff}\n"
    )
}

fn generate_copilot_review(
    repo_root: &Utf8Path,
    prompt: &str,
    model: Option<&str>,
) -> anyhow::Result<String> {
    let model = model.unwrap_or("gpt-5.2-codex");
    let output = Cmd::new(
        "copilot",
        ["--silent", "--model", model, "--prompt", prompt],
    )
    .hide_stdout()
    .with_title(format!("🚀 copilot --silent --model {model} --prompt ..."))
    .with_current_dir(repo_root)
    .run();

    anyhow::ensure!(
        output.status().success() && !output.stdout().trim().is_empty(),
        "❌ Failed to generate PR review with Copilot"
    );

    Ok(output.stdout().to_string())
}

fn generate_gemini_review(
    repo_root: &Utf8Path,
    prompt: &str,
    model: Option<&str>,
) -> anyhow::Result<String> {
    let model = model.unwrap_or("gemini-3-pro-preview");
    let output = Cmd::new("gemini", ["--model", model, "--sandbox", prompt])
        .hide_stdout()
        .with_title(format!("🚀 gemini --model {model} --sandbox ..."))
        .with_current_dir(repo_root)
        .run();

    anyhow::ensure!(
        output.status().success() && !output.stdout().trim().is_empty(),
        "❌ Failed to generate PR review with Gemini"
    );

    Ok(output.stdout().to_string())
}
