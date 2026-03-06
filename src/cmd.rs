use std::{
    collections::BTreeMap,
    env,
    path::Path,
    process::{ExitStatus, Stdio},
    sync::atomic::{AtomicBool, Ordering},
};

use anyhow::Context as _;
use camino::Utf8PathBuf;
use secrecy::{ExposeSecret, SecretString};
use tokio::{fs, process::Command};

static VERBOSE: AtomicBool = AtomicBool::new(false);

pub fn set_verbose(verbose: bool) {
    VERBOSE.store(verbose, Ordering::SeqCst);
}

pub async fn ensure_command_available(cmd_name: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        is_command_available(cmd_name).await,
        "❌ Required command `{cmd_name}` is not installed or not available in PATH"
    );
    Ok(())
}

fn is_verbose() -> bool {
    VERBOSE.load(Ordering::SeqCst)
}

async fn is_command_available(cmd_name: &str) -> bool {
    if cmd_name.contains(std::path::MAIN_SEPARATOR) {
        return command_candidate_exists(Path::new(cmd_name)).await;
    }

    let Some(path_var) = env::var_os("PATH") else {
        return false;
    };

    for path in env::split_paths(&path_var) {
        if command_candidate_exists(&path.join(cmd_name)).await {
            return true;
        }
    }

    false
}

async fn command_candidate_exists(path: &Path) -> bool {
    fs::try_exists(path).await.unwrap_or(false)
}

#[derive(Debug)]
pub struct CmdOutput {
    status: ExitStatus,
    stdout: String,
    stderr: String,
    invocation: String,
    current_dir: Option<Utf8PathBuf>,
}

impl CmdOutput {
    pub fn status(&self) -> &ExitStatus {
        &self.status
    }

    pub fn stdout(&self) -> &str {
        self.stdout.trim()
    }

    pub fn stderr(&self) -> &str {
        self.stderr.trim()
    }

    pub fn stderr_or_stdout(&self) -> &str {
        if self.stderr().is_empty() {
            self.stdout()
        } else {
            self.stderr()
        }
    }

    fn command_summary(&self) -> String {
        match &self.current_dir {
            Some(dir) => format!("`{}` (cwd: {})", self.invocation, dir),
            None => format!("`{}`", self.invocation),
        }
    }

    pub fn ensure_success(&self, context: impl std::fmt::Display) -> anyhow::Result<()> {
        if self.status().success() {
            return Ok(());
        }

        let command = self.command_summary();
        let details = self.stderr_or_stdout();
        if details.is_empty() {
            anyhow::bail!(
                "{context}: command {command} exited with status {}",
                self.status
            );
        }

        anyhow::bail!("{context}: command {command} failed: {details}");
    }
}

pub struct Cmd {
    name: String,
    env_vars: BTreeMap<String, SecretString>,
    args: Vec<String>,
    current_dir: Option<Utf8PathBuf>,
    hide_stdout: bool,
    hide_stderr: bool,
    title: Option<String>,
}

impl Cmd {
    pub fn new<I, S>(cmd_name: &str, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let args: Vec<String> = args
            .into_iter()
            .map(|arg| arg.as_ref().to_string())
            .collect();
        Self {
            name: cmd_name.to_string(),
            args,
            current_dir: None,
            hide_stdout: false,
            hide_stderr: false,
            env_vars: BTreeMap::new(),
            title: None,
        }
    }

    // pub fn with_env_vars(&mut self, env_vars: BTreeMap<String, SecretString>) -> &mut Self {
    //     self.env_vars = env_vars;
    //     self
    // }

    pub fn with_current_dir(&mut self, dir: impl Into<Utf8PathBuf>) -> &mut Self {
        self.current_dir = Some(dir.into());
        self
    }

    pub fn hide_stdout(&mut self) -> &mut Self {
        self.hide_stdout = true;
        self
    }

    pub fn hide_stderr(&mut self) -> &mut Self {
        self.hide_stderr = true;
        self
    }

    pub fn with_title(&mut self, title: impl Into<String>) -> &mut Self {
        self.title = Some(title.into());
        self
    }

    fn build_command_description(&self) -> String {
        let mut description = self
            .title
            .clone()
            .unwrap_or_else(|| format!("🚀 {} {}", self.name, self.args.join(" ")));
        if let Some(dir) = &self.current_dir {
            description.push_str(&format!(" 👉 {dir}"));
        }
        description
    }

    fn format_invocation(&self) -> String {
        let mut invocation = self.name.clone();
        if !self.args.is_empty() {
            invocation.push(' ');
            invocation.push_str(&self.args.join(" "));
        }
        invocation
    }

    fn spawn_context(&self) -> String {
        match &self.current_dir {
            Some(dir) => format!(
                "failed to spawn command `{}` in {}",
                self.format_invocation(),
                dir
            ),
            None => format!("failed to spawn command `{}`", self.format_invocation()),
        }
    }

    fn configure_command(&self) -> Command {
        let mut command = Command::new(&self.name);
        if let Some(dir) = &self.current_dir {
            command.current_dir(dir);
        }
        for (key, value) in &self.env_vars {
            command.env(key, value.expose_secret());
        }
        command
    }

    fn print_verbose_output(&self, stdout: &str, stderr: &str) {
        if !is_verbose() {
            return;
        }

        if !self.hide_stdout {
            for line in stdout.lines() {
                println!("{line}");
            }
        }

        if !self.hide_stderr {
            for line in stderr.lines() {
                eprintln!("{line}");
            }
        }
    }

    pub async fn run(&self) -> anyhow::Result<CmdOutput> {
        if is_verbose() {
            println!("{}", self.build_command_description());
        }

        let output = self
            .configure_command()
            .args(&self.args)
            .output()
            .await
            .with_context(|| self.spawn_context())?;

        let output_stdout =
            String::from_utf8(output.stdout).context("command produced non-UTF-8 stdout")?;
        let output_stderr =
            String::from_utf8(output.stderr).context("command produced non-UTF-8 stderr")?;
        self.print_verbose_output(&output_stdout, &output_stderr);

        Ok(CmdOutput {
            status: output.status,
            stdout: output_stdout,
            stderr: output_stderr,
            invocation: self.format_invocation(),
            current_dir: self.current_dir.clone(),
        })
    }

    #[allow(dead_code)]
    pub async fn run_interactive(&self) -> anyhow::Result<CmdOutput> {
        if is_verbose() {
            println!("{}", self.build_command_description());
        }

        let status = self
            .configure_command()
            .args(&self.args)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .await
            .with_context(|| self.spawn_context())?;

        Ok(CmdOutput {
            status,
            stdout: String::new(),
            stderr: String::new(),
            invocation: self.format_invocation(),
            current_dir: self.current_dir.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cmd_new_stores_args() {
        let cmd = Cmd::new("git", ["status", "-s"]);
        assert_eq!(cmd.name, "git");
        assert_eq!(cmd.args, vec!["status", "-s"]);
    }

    #[test]
    fn test_cmd_new_empty_args() {
        let cmd = Cmd::new("ls", Vec::<&str>::new());
        assert_eq!(cmd.name, "ls");
        assert!(cmd.args.is_empty());
    }

    #[test]
    fn test_build_command_description_default() {
        let cmd = Cmd::new("git", ["status"]);
        let desc = cmd.build_command_description();
        assert_eq!(desc, "🚀 git status");
    }

    #[test]
    fn test_build_command_description_with_dir() {
        let mut cmd = Cmd::new("git", ["status"]);
        cmd.with_current_dir("/some/path");
        let desc = cmd.build_command_description();
        assert_eq!(desc, "🚀 git status 👉 /some/path");
    }

    #[test]
    fn test_build_command_description_with_title() {
        let mut cmd = Cmd::new("git", ["status"]);
        cmd.with_title("Checking repo status");
        let desc = cmd.build_command_description();
        assert_eq!(desc, "Checking repo status");
    }

    #[test]
    fn test_build_command_description_with_title_and_dir() {
        let mut cmd = Cmd::new("git", ["status"]);
        cmd.with_title("Checking status");
        cmd.with_current_dir("/repo");
        let desc = cmd.build_command_description();
        assert_eq!(desc, "Checking status 👉 /repo");
    }

    #[test]
    fn test_cmd_output_stdout_trims() {
        use std::process::ExitStatus;
        #[cfg(unix)]
        let status = {
            use std::os::unix::process::ExitStatusExt;
            ExitStatus::from_raw(0)
        };
        let output = CmdOutput {
            status,
            stdout: "  hello world  \n".to_string(),
            stderr: String::new(),
            invocation: "echo hello world".to_string(),
            current_dir: None,
        };
        assert_eq!(output.stdout(), "hello world");
    }
}
