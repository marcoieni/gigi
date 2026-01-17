#[derive(clap::Parser, Debug)]
#[command(about, version, author)]
pub struct CliArgs {
    /// Print command name, arguments, and directory when running commands
    #[arg(long, global = true)]
    pub verbose: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(clap::Subcommand, Debug)]
pub enum Command {
    /// Creates a new branch and opens a pull request preview in the browser
    OpenPr {
        /// Commit message (skips interactive prompt)
        #[arg(short, long)]
        message: Option<String>,
    },
    Squash {
        /// Show what would be squashed without actually performing the operation
        #[arg(long)]
        dry_run: bool,
    },
}
