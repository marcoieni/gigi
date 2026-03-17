mod types;

#[cfg(feature = "ssr")]
mod api;
#[cfg(feature = "ssr")]
mod local_repo;
#[cfg(feature = "ssr")]
mod parsing;

#[cfg(feature = "ssr")]
pub use api::{
    fetch_authored_prs, fetch_batch, fetch_notifications, fetch_pr_details, mark_notification_done,
};
#[cfg(feature = "ssr")]
pub use local_repo::{
    checkout_branch, checkout_pr, checkout_pr_for_open_with_details, current_branch,
    default_branch, ensure_local_repo, ensure_local_repo_for_pr, is_clean_repo, local_repo_dir,
    prepare_repo_for_pr_checkout, pull_ff_only,
};
#[cfg(feature = "ssr")]
pub use parsing::parse_github_name_with_owner;
pub use types::{AuthoredPrSummary, NotificationThread, Participant, PrDetails};
