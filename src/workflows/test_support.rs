use std::{
    fs,
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use camino::{Utf8Path, Utf8PathBuf};

static NEXT_TEMP_DIR_ID: AtomicU64 = AtomicU64::new(1);

pub(super) struct TestDir {
    path: Utf8PathBuf,
}

impl TestDir {
    pub(super) fn new(name: &str) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let id = NEXT_TEMP_DIR_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "gigi-{name}-{}-{timestamp}-{id}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self {
            path: Utf8PathBuf::from_path_buf(path).unwrap(),
        }
    }

    pub(super) fn path(&self) -> &Utf8Path {
        &self.path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        drop(fs::remove_dir_all(&self.path));
    }
}

pub(super) fn git_output(repo: &Utf8Path, args: &[&str]) -> Output {
    Command::new("git")
        .args(args)
        .current_dir(repo)
        .env("LC_ALL", "C")
        .output()
        .unwrap()
}

pub(super) fn command_output(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

pub(super) fn git_success(repo: &Utf8Path, args: &[&str]) -> String {
    let output = git_output(repo, args);
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        command_output(&output)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

pub(super) fn configure_test_user(repo: &Utf8Path) {
    git_success(repo, &["config", "user.name", "Test User"]);
    git_success(repo, &["config", "user.email", "test@example.com"]);
}

pub(super) fn init_bare_repo(root: &Utf8Path, repo: &Utf8Path) {
    git_success(root, &["init", "--bare", "--quiet", repo.as_str()]);
}
