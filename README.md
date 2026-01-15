# propo

> [!NOTE]
> This software is still highly experimental and the resulting git actions
> might not follow best practices.

TODO:

- [ ] find a better name
- [ ] run `gh repo set-default` automatically

## Install locally

1. Clone this repo
2. Run `cargo install --path .`

## Alias

With the following command you can `p <command>` in your projects, and
it will run the latest version of `propo` in that directory.

```
alias p='RUST_BACKTRACE=1 cargo run --manifest-path ~/path/to/propo/Cargo.toml --'
```

## Commands

Run `cargo run -- --help` to see the help menu with all available commands.

### Open PR

Open a PR with the current changes. The PR title and branch name are automatically
set from the commit message.

If there are any staged changes, only those are included in the PR.

### Squash

The squash subcommand squashes all the commits of the PR into one, rebasing
the default branch and setting the PR title as the commit message.
