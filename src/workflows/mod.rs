mod open;
mod repo;
mod squash;
#[cfg(test)]
mod test_support;

pub use open::open_pr;
pub use repo::{ensure_default_repo_and_root, sync_fork};
pub use squash::squash;
