#![recursion_limit = "256"]

pub mod dashboard;
pub mod db;
pub mod github;
pub mod icons;

#[cfg(feature = "ssr")]
mod args;
#[cfg(feature = "ssr")]
mod authors;
#[cfg(feature = "ssr")]
mod checkout;
#[cfg(feature = "ssr")]
mod cmd;
#[cfg(feature = "ssr")]
mod commit;
#[cfg(feature = "ssr")]
mod config;
#[cfg(feature = "ssr")]
mod init;
#[cfg(feature = "ssr")]
mod launcher;
#[cfg(feature = "ssr")]
mod review;
#[cfg(feature = "ssr")]
mod serve;
#[cfg(feature = "ssr")]
mod terminal;
#[cfg(feature = "ssr")]
mod web;
#[cfg(feature = "ssr")]
mod workflows;

pub mod app;

#[cfg(feature = "ssr")]
pub async fn run_cli() -> anyhow::Result<()> {
    use anyhow::Context as _;
    use clap::Parser as _;
    use git_cmd::Repo;

    let args = args::CliArgs::parse();
    cmd::set_verbose(args.verbose);

    match args.command {
        args::Command::CheckoutPr { pr } => checkout::checkout_pr(&pr).await,

        args::Command::OpenPr {
            message,
            agent,
            model,
        } => {
            let repo_root = workflows::ensure_default_repo_and_root().await?;
            workflows::open_pr(&repo_root, message, agent.as_ref(), model.as_deref()).await
        }

        args::Command::Review { pr, agent, model } => {
            let repo_root = workflows::ensure_default_repo_and_root().await?;
            review::review_pr(&repo_root, &pr, agent.as_ref(), model.as_deref()).await
        }

        args::Command::Init => init::run_init().await,

        args::Command::Serve => serve::run_serve().await,

        args::Command::Squash { dry_run } => {
            let repo_root = workflows::ensure_default_repo_and_root().await?;
            let repo = Repo::new(repo_root.clone())
                .context("❌ Failed to open git repository for squash")?;
            workflows::squash(&repo_root, &repo, dry_run).await
        }

        args::Command::Sync => {
            let repo_root = workflows::ensure_default_repo_and_root().await?;
            workflows::sync_fork(&repo_root).await
        }
    }?;

    Ok(())
}

#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
    console_error_panic_hook::set_once();
    leptos::mount::hydrate_body(app::App);
}
