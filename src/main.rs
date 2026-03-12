mod args;
mod authors;
mod checkout;
mod cmd;
mod commit;
mod config;
mod dashboard;
mod db;
mod github;
mod icons;
mod init;
mod launcher;
mod review;
mod serve;
mod terminal;
mod web;
mod workflows;

use anyhow::Context;
use args::CliArgs;
use clap::Parser as _;
use git_cmd::Repo;
use review::review_pr;

use crate::{
    checkout::checkout_pr,
    workflows::{ensure_default_repo_and_root, open_pr, squash, sync_fork},
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = CliArgs::parse();
    cmd::set_verbose(args.verbose);

    match args.command {
        args::Command::CheckoutPr { pr } => checkout_pr(&pr).await,

        args::Command::OpenPr {
            message,
            agent,
            model,
        } => {
            let repo_root = ensure_default_repo_and_root().await?;
            open_pr(&repo_root, message, agent.as_ref(), model.as_deref()).await
        }

        args::Command::Review { pr, agent, model } => {
            let repo_root = ensure_default_repo_and_root().await?;
            review_pr(&repo_root, &pr, agent.as_ref(), model.as_deref()).await
        }

        args::Command::Init => init::run_init().await,

        args::Command::Serve => serve::run_serve().await,

        args::Command::Squash { dry_run } => {
            let repo_root = ensure_default_repo_and_root().await?;
            let repo = Repo::new(repo_root.clone())
                .context("❌ Failed to open git repository for squash")?;
            squash(&repo_root, &repo, dry_run).await
        }

        args::Command::Sync => {
            let repo_root = ensure_default_repo_and_root().await?;
            sync_fork(&repo_root).await
        }
    }?;

    Ok(())
}
