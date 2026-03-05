use std::fs;

use anyhow::Context as _;

use crate::config;

pub fn run_init() -> anyhow::Result<()> {
    let paths = config::resolve_paths()?;
    config::ensure_parent_dirs(&paths)?;

    if paths.config_path.exists() {
        anyhow::ensure!(
            paths.config_path.is_file(),
            "❌ Config path exists but is not a file: {}",
            paths.config_path.display()
        );
        println!(
            "📄 Config already exists at {}",
            paths.config_path.display()
        );
        return Ok(());
    }

    fs::write(&paths.config_path, config::default_config_toml()).with_context(|| {
        format!(
            "Failed to write config file at {}",
            paths.config_path.display()
        )
    })?;

    println!("✅ Created config at {}", paths.config_path.display());
    println!("▶️  Run `gigi serve` to start the dashboard.");
    Ok(())
}
