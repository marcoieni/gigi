# gigi

![logo](./assets/logo.png)

**gigi** stands for **gi**t **gi**zmo.

> [!NOTE]
> `gigi` is a CLI implementing some opinionated commands to simplify my
> day-to-day work with git and GitHub.

> [!WARNING]
> This software runs `git` and `gh` commands. Use it at your own risk.

## Install locally

1. Clone this repo
2. Run `cargo install --path .`

OR run `cargo install --git https://github.com/marcoieni/gigi`

## Commands

Run `cargo run -- --help` to see the help menu with all available commands.

### Open PR

Open a PR with the current changes. The PR title and branch name are automatically
set from the commit message.

If there are any staged changes, only those are included in the PR.

If you specify an agent with `--agent`, gigi will use it to generate a commit message,
that you can edit before creating the PR.

If you don't specify an agent, gigi will prompt you to enter a commit message.

Examples:

- `gigi open-pr --agent copilot`
- `gigi open-pr --message "feat: add thing"`

### Squash

The squash subcommand squashes all the commits of the PR into one, rebasing
the default branch and setting the PR title as the commit message.

The authors of the original commits are set as co-authors in the new commit
message.

Examples:

- `gigi squash`
- `gigi squash --dry-run`
- `gigi squash --add-co-author`

#### Diagram

Before running `gigi squash`:

```
main ──●─┐
         │
         ●  feat: first implementation    (alice)
         ●  fix: handle edge case         (bob)
         ●  docs: update usage            (alice)
         │
         ▼  PR: "feat: add caching" (#123)
```

After alice runs `gigi squash`:

```

main ──●─┐
         │
         ●  feat: add caching             (alice)
         │
         │  Co-authored-by: Bob <bob@example.com>
         │
         ▼  PR: "feat: add caching" (#123)
```

### Checkout PR

Clone a GitHub PR repository into `~/proj/<owner>/<repo>` (if missing), pull the default branch, checkout the PR locally, and open VS Code.

Examples:

- `gigi checkout-pr https://github.com/OWNER/REPO/pull/123`

### Review

Review a GitHub PR with an AI agent. The first positional argument is the PR URL.

Examples:

- `gigi review https://github.com/OWNER/REPO/pull/123`
- `gigi review --agent gemini --model gemini-3-flash-preview https://github.com/OWNER/REPO/pull/123`

### Init

Initialize `~/.config/gigi/config.toml` with the default settings used by `serve`.
If the file already exists, it is left unchanged.

Examples:

- `gigi init`

Example `~/.config/gigi/config.toml`:

```toml
watch_period_seconds = 60
rereview_mode = "on_update" # or "manual"
initial_review_lookback_days = 3
initial_review_max_prs = 10

[ai]
provider = "copilot" # or "gemini" or "kiro"
# model = "gpt-5.3-codex"
# when provider = "kiro", the default model is "claude-opus-4.6"

[dashboard]
host = "127.0.0.1"
port = 8787
```

### Serve

Run a local server that periodically watches GitHub notifications, your open PRs,
and PRs/issues assigned to you, stores data and reviews in SQLite, and exposes a dashboard.

Defaults:

- Config file: `~/.config/gigi/config.toml`
- DB file: `~/.local/share/gigi/gigi.db`
- Dashboard: `http://127.0.0.1:8787`

Example:

- `gigi serve`

On startup, `serve` only auto-reviews PRs opened or updated within
`initial_review_lookback_days`, and runs at most `initial_review_max_prs`
reviews. The dashboard includes a "Review now" button to manually review
any skipped PR.

### Sync

Sync a fork with its upstream repository and update the local default branch.

Examples:

- `gigi sync`

## Alias

You can set an alias to recompile and run `gigi` from your local project.

With the following command you can `gigi <command>` in your projects, and
it will run the latest version of `gigi` in that directory.

```
alias gigi='RUST_BACKTRACE=1 cargo run --manifest-path ~/path/to/gigi/Cargo.toml --'
```

## Contributing

This project is mainly for my personal use, so I'll only accept contributions
that align with my workflow.

If you want to contribute, please open an issue to discuss your idea before
opening a PR.
