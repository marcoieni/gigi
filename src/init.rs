use anyhow::Context as _;
use tokio::fs;

use crate::config;

pub async fn run_init() -> anyhow::Result<()> {
    let paths = config::resolve_paths()?;
    config::ensure_parent_dirs(&paths).await?;

    if fs::try_exists(&paths.config_path).await? {
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

    fs::write(&paths.config_path, config::default_config_toml())
        .await
        .with_context(|| {
            format!(
                "Failed to write config file at {}",
                paths.config_path.display()
            )
        })?;

    println!("✅ Created config at {}", paths.config_path.display());
    println!("▶️  Run `gigi serve` to start the dashboard.");
    Ok(())
}
