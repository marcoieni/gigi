use camino::Utf8Path;

use crate::cmd::Cmd;

pub fn open_vscode(repo_dir: &Utf8Path) -> anyhow::Result<()> {
    let code = Cmd::new("code", ["."])
        .with_title("🧑‍💻 code .")
        .with_current_dir(repo_dir)
        .run();

    if let Ok(code) = code
        && code.status().success()
    {
        return Ok(());
    }

    let open = Cmd::new("open", ["-a", "Visual Studio Code", "."])
        .with_title("🧑‍💻 open -a \"Visual Studio Code\" .")
        .with_current_dir(repo_dir)
        .run()?;
    open.ensure_success(
        "❌ Failed to open VS Code (tried `code .` and `open -a 'Visual Studio Code' .`)",
    )?;

    Ok(())
}

pub fn open_terminal(repo_dir: &Utf8Path) -> anyhow::Result<()> {
    let open = Cmd::new("open", ["-a", "Terminal", repo_dir.as_str()])
        .with_title("🖥️ open -a Terminal")
        .run()?;
    open.ensure_success("❌ Failed to open Terminal")?;
    Ok(())
}
