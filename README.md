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

If you have `copilot` installed, gigi will use it to generate a commit message,
that you can edit before creating the PR.

Examples:

- `gigi open-pr`
- `gigi open-pr --message "feat: add thing"`

### Squash

The squash subcommand squashes all the commits of the PR into one, rebasing
the default branch and setting the PR title as the commit message.

The authors of the original commits are set as co-authors in the new commit
message.

Examples:

- `gigi squash`
- `gigi squash --dry-run`

### Review

Review a GitHub PR with an AI agent. The first positional argument is the PR URL.

Examples:

- `gigi review https://github.com/OWNER/REPO/pull/123`
- `gigi review --agent gemini --model gemini-3-flash-preview https://github.com/OWNER/REPO/pull/123`

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
