use std::{
    collections::BTreeMap,
    io::{BufRead as _, BufReader},
    process::{Command, ExitStatus, Stdio},
    sync::mpsc,
    thread,
};

use camino::Utf8PathBuf;
use secrecy::{ExposeSecret, SecretString};

#[derive(Debug)]
pub struct CmdOutput {
    status: ExitStatus,
    stdout: String,
    // stderr: String,
}

impl CmdOutput {
    pub fn status(&self) -> &ExitStatus {
        &self.status
    }

    pub fn stdout(&self) -> &str {
        self.stdout.trim()
    }

    // pub fn stderr(&self) -> &str {
    //     self.stderr.trim()
    // }
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
            description.push_str(&format!(" 👉 {}", dir));
        }
        description
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
    ) {
        thread::spawn(move || {
            let reader = BufReader::new(reader);
            for line in reader.lines() {
                let line = line.unwrap();
                tx.send((line, is_stdout)).unwrap();
            }
        });
    }

    fn collect_output(&self, rx: mpsc::Receiver<(String, bool)>) -> (String, String) {
        let mut output_stdout = String::new();
        let mut output_stderr = String::new();

        for (line, is_stdout) in rx {
            if is_stdout {
                if !self.hide_stdout {
                    println!("{}", line);
                }
                output_stdout.push_str(&line);
                output_stdout.push('\n');
            } else {
                if !self.hide_stderr {
                    eprintln!("{}", line);
                }
                output_stderr.push_str(&line);
                output_stderr.push('\n');
            }
        }
        (output_stdout, output_stderr)
    }

    pub fn run(&self) -> CmdOutput {
        println!("{}", self.build_command_description());

        let mut child = self
            .configure_command()
            .args(&self.args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();

        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let (tx, rx) = mpsc::channel();

        Self::spawn_output_reader(stdout, tx.clone(), true);
        Self::spawn_output_reader(stderr, tx, false);

        let (output_stdout, _output_stderr) = self.collect_output(rx);
        let status = child.wait().unwrap();

        CmdOutput {
            status,
            stdout: output_stdout,
        }
    }
}
