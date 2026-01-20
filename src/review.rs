use camino::Utf8Path;

use crate::{args::Agent, cmd::Cmd};

pub fn review_pr(
    repo_root: &Utf8Path,
    pr_url: &str,
    agent: Option<&Agent>,
    model: Option<&str>,
) -> anyhow::Result<()> {
    let metadata = fetch_pr_metadata(repo_root, pr_url)?;
    let diff = fetch_pr_diff(repo_root, pr_url)?;
    let prompt = build_review_prompt(pr_url, &metadata, &diff);

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
            "title,body,author,baseRefName,headRefName,createdAt,updatedAt,labels,assignees,reviewRequests,reviews,comments,commits,files,additions,deletions,state,mergeable,url",
        ],
    )
    .with_current_dir(repo_root)
    .run();
    anyhow::ensure!(
        output.status().success() && !output.stdout().trim().is_empty(),
        "❌ Failed to fetch PR metadata"
    );
    Ok(output.stdout().to_string())
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
