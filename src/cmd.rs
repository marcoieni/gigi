use std::{
    collections::BTreeMap,
    env,
    io::{BufRead as _, BufReader},
    path::Path,
    process::{Command, ExitStatus, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};

use anyhow::Context as _;
use camino::Utf8PathBuf;
use secrecy::{ExposeSecret, SecretString};

static VERBOSE: AtomicBool = AtomicBool::new(false);

pub fn set_verbose(verbose: bool) {
    VERBOSE.store(verbose, Ordering::SeqCst);
}

pub fn ensure_command_available(cmd_name: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        is_command_available(cmd_name),
        "❌ Required command `{cmd_name}` is not installed or not available in PATH"
    );
    Ok(())
}

fn is_verbose() -> bool {
    VERBOSE.load(Ordering::SeqCst)
}

fn is_command_available(cmd_name: &str) -> bool {
    if cmd_name.contains(std::path::MAIN_SEPARATOR) {
        return command_candidate_exists(Path::new(cmd_name));
    }

    let Some(path_var) = env::var_os("PATH") else {
        return false;
    };

    env::split_paths(&path_var).any(|path| command_candidate_exists(&path.join(cmd_name)))
}

fn command_candidate_exists(path: &Path) -> bool {
    path.is_file()
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

    fn spawn_output_reader<R: std::io::Read + Send + 'static>(
        reader: R,
        tx: mpsc::Sender<(String, bool)>,
        is_stdout: bool,
    ) -> thread::JoinHandle<()> {
        thread::spawn(move || {
            let reader = BufReader::new(reader);
            for line in reader.lines() {
                let Ok(line) = line else {
                    break;
                };
                if tx.send((line, is_stdout)).is_err() {
                    break;
                }
            }
        })
    }

    fn collect_output(&self, rx: mpsc::Receiver<(String, bool)>) -> (String, String) {
        let mut output_stdout = String::new();
        let mut output_stderr = String::new();

        for (line, is_stdout) in rx {
            if is_stdout {
                if is_verbose() && !self.hide_stdout {
                    println!("{line}");
                }
                output_stdout.push_str(&line);
                output_stdout.push('\n');
            } else {
                if is_verbose() && !self.hide_stderr {
                    eprintln!("{line}");
                }
                output_stderr.push_str(&line);
                output_stderr.push('\n');
            }
        }
        (output_stdout, output_stderr)
    }

    pub fn run(&self) -> anyhow::Result<CmdOutput> {
        if is_verbose() {
            println!("{}", self.build_command_description());
        }

        let mut child = self
            .configure_command()
            .args(&self.args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| self.spawn_context())?;

        let stdout = child
            .stdout
            .take()
            .context("spawned command did not expose stdout pipe")?;
        let stderr = child
            .stderr
            .take()
            .context("spawned command did not expose stderr pipe")?;
        let (tx, rx) = mpsc::channel();

        let stdout_reader = Self::spawn_output_reader(stdout, tx.clone(), true);
        let stderr_reader = Self::spawn_output_reader(stderr, tx, false);

        let (output_stdout, output_stderr) = self.collect_output(rx);
        stdout_reader
            .join()
            .map_err(|_| anyhow::anyhow!("stdout reader thread panicked"))?;
        stderr_reader
            .join()
            .map_err(|_| anyhow::anyhow!("stderr reader thread panicked"))?;
        let status = child.wait().with_context(|| {
            format!("failed to wait for command `{}`", self.format_invocation())
        })?;

        Ok(CmdOutput {
            status,
            stdout: output_stdout,
            stderr: output_stderr,
            invocation: self.format_invocation(),
            current_dir: self.current_dir.clone(),
        })
    }

    #[allow(dead_code)]
    pub fn run_interactive(&self) -> anyhow::Result<CmdOutput> {
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
