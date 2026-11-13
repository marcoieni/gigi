#![cfg(unix)]
//! End-to-end tests for `gigi squash`.
//!
//! These drive the compiled binary because bugs in the CLI wiring in `main.rs`
//! are invisible to unit tests of the workflow helpers.

// Reuse the unit tests' temp dir instead of adding a dev-dependency. Its git
// helpers don't isolate the user's git config, so this file has its own.
#[allow(dead_code, reason = "only part of the shared helpers is used here")]
#[path = "../src/workflows/test_support.rs"]
mod test_support;

use std::{env, ffi::OsString, fs, iter, os::unix::fs::PermissionsExt as _, process::Command};

use camino::{Utf8Path, Utf8PathBuf};
use test_support::{TestDir, command_output};

/// Stands in for `gh`, answering exactly the calls `gigi squash` makes, so
/// neither the network nor the user's real `gh` setup (auth, `git_protocol`)
/// is involved. Any other call fails, so a changed call can't go unnoticed.
const FAKE_GH: &str = r#"#!/bin/sh
case "$*" in
"repo set-default --view") echo org/project ;;
"pr list --head feature --json number,title,baseRefName,url --limit 1")
  echo '[{"number":1,"title":"Add a and b","baseRefName":"main","url":"https://github.com/org/project/pull/1"}]' ;;
"config get git_protocol --host github.com") echo https ;;
"api --paginate --slurp repos/{owner}/{repo}/pulls/1/commits?per_page=100")
  echo '[[{"sha":"0000000","commit":{"author":{"name":"Test User","email":"test@example.com"},"message":"wip"}}]]' ;;
"pr view --web") ;;
*) echo "fake gh: unexpected invocation: gh $*" >&2; exit 1 ;;
esac
"#;

/// Runs git and gigi confined to the fixture.
struct Sandbox {
    home: Utf8PathBuf,
    path: OsString,
}

impl Sandbox {
    /// Builds a command that inherits nothing from the developer's environment:
    /// git exports `GIT_DIR` and `GIT_INDEX_FILE` to hooks (so a test run from
    /// a hook would modify the developer's repo), `GIT_CONFIG_COUNT` can
    /// rewrite URLs, and `GIT_TEMPLATE_DIR` can install hooks.
    fn command(&self, program: &str, cwd: &Utf8Path) -> Command {
        let mut command = Command::new(program);
        command
            .current_dir(cwd)
            .env_clear()
            .env("PATH", &self.path)
            // Git's global config is read from here, not from the user's.
            .env("HOME", &self.home)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            // If the GitHub URL rewrite broke, fail instead of using the network.
            .env("GIT_ALLOW_PROTOCOL", "file")
            .env("LC_ALL", "C");
        command
    }

    fn git(&self, cwd: &Utf8Path, args: &[&str]) -> String {
        let output = self.command("git", cwd).args(args).output().unwrap();
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            command_output(&output)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    }
}

#[test]
fn squash_works_when_upstream_is_not_a_remote_tracking_branch() {
    let fixture = TestDir::new("squash-url-upstream");
    let root = fixture.path();
    let bin = root.join("bin");
    let base = root.join("base.git");
    let head = root.join("head.git");
    let work = root.join("work");
    // The fake `gh` shadows any real one; git still resolves as usual.
    let inherited_path = env::var_os("PATH").unwrap_or_default();
    let path = iter::once(bin.clone().into()).chain(env::split_paths(&inherited_path));
    let sandbox = Sandbox {
        home: root.to_owned(),
        path: env::join_paths(path).unwrap(),
    };
    let git = |cwd: &Utf8Path, args: &[&str]| sandbox.git(cwd, args);

    fs::create_dir(&bin).unwrap();
    fs::write(bin.join("gh"), FAKE_GH).unwrap();
    fs::set_permissions(bin.join("gh"), fs::Permissions::from_mode(0o755)).unwrap();

    // With no remote matching the PR's base repo, gigi fetches the base from
    // `https://github.com/org/project.git`; `insteadOf` serves it locally.
    let base_rewrite = format!("url.{base}.insteadOf");
    for (key, value) in [
        ("user.name", "Test User"),
        ("user.email", "test@example.com"),
        (&base_rewrite, "https://github.com/org/project.git"),
    ] {
        git(root, &["config", "--global", key, value]);
    }

    git(root, &["init", "--bare", base.as_str()]);
    git(root, &["init", "--bare", head.as_str()]);
    git(root, &["init", work.as_str()]);
    git(&work, &["commit", "--allow-empty", "-m", "base"]);
    // The PR's base branch, whatever `git init` named the initial branch.
    git(&work, &["push", base.as_str(), "HEAD:refs/heads/main"]);
    git(&work, &["switch", "-c", "feature"]);
    for file in ["a.txt", "b.txt"] {
        fs::write(work.join(file), file).unwrap();
        git(&work, &["add", file]);
        git(&work, &["commit", "-m", "wip"]);
    }
    git(&work, &["push", head.as_str(), "feature"]);
    // `gh pr checkout` tracks a fork PR's branch by URL rather than through a
    // named remote, so Git has no remote-tracking ref for its upstream. A path
    // is the same kind of remote.
    git(&work, &["config", "branch.feature.remote", head.as_str()]);
    git(
        &work,
        &["config", "branch.feature.merge", "refs/heads/feature"],
    );

    // Make sure the fixture reproduces the reported error: resolving
    // `@{upstream}`, as `git_cmd::Repo::new` did, fails in this state.
    let upstream = sandbox
        .command("git", &work)
        .args([
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ])
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&upstream.stderr)
            .contains("not stored as a remote-tracking branch"),
        "unexpected upstream resolution: {}",
        command_output(&upstream)
    );

    let output = sandbox
        .command(env!("CARGO_BIN_EXE_gigi"), &work)
        .arg("squash")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "gigi squash failed: {}",
        command_output(&output)
    );

    // The fork branch is now a single commit, titled after the PR, on top of
    // `base`, and it still carries all of the PR's changes.
    assert_eq!(
        git(&head, &["log", "--format=%s", "feature"]),
        "Add a and b\nbase"
    );
    assert_eq!(
        git(&head, &["diff", "--name-only", "feature^", "feature"]),
        "a.txt\nb.txt"
    );
}
