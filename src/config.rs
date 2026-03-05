use std::{fs, path::PathBuf};

use anyhow::Context as _;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub config_path: PathBuf,
    pub db_path: PathBuf,
    pub dashboard_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub watch_period_seconds: u64,
    pub rereview_mode: RereviewMode,
    pub ai: AiConfig,
    pub dashboard: DashboardConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AiConfig {
    pub provider: AiProvider,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DashboardConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum RereviewMode {
    #[default]
    OnUpdate,
    Manual,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AiProvider {
    #[default]
    Copilot,
    Gemini,
    Kiro,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            watch_period_seconds: 60,
            rereview_mode: RereviewMode::OnUpdate,
            ai: AiConfig::default(),
            dashboard: DashboardConfig::default(),
        }
    }
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            provider: AiProvider::Copilot,
            model: None,
        }
    }
}

impl Default for DashboardConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 8787,
        }
    }
}

pub fn default_config_toml() -> &'static str {
    r#"watch_period_seconds = 60
rereview_mode = "on_update" # or "manual"

[ai]
provider = "copilot" # or "gemini" or "kiro"
# model = "gpt-5.3-codex"

[dashboard]
host = "127.0.0.1"
port = 8787
"#
}

pub fn resolve_paths() -> anyhow::Result<AppPaths> {
    let home = std::env::var("HOME").context("HOME env var is not set")?;
    let home = PathBuf::from(home);

    let config_path = home.join(".config").join("gigi").join("config.toml");
    let data_dir = home.join(".local").join("share").join("gigi");
    let db_path = data_dir.join("gigi.db");
    let dashboard_dir = data_dir.join("dashboard");

    Ok(AppPaths {
        config_path,
        db_path,
        dashboard_dir,
    })
}

pub fn load_config(config_path: &PathBuf) -> anyhow::Result<AppConfig> {
    if !config_path.exists() {
        return Ok(AppConfig::default());
    }

    let raw = fs::read_to_string(config_path)
        .with_context(|| format!("Failed to read config file at {}", config_path.display()))?;
    let config: AppConfig = toml::from_str(&raw)
        .with_context(|| format!("Failed to parse TOML config at {}", config_path.display()))?;

    Ok(config)
}

pub fn ensure_parent_dirs(paths: &AppPaths) -> anyhow::Result<()> {
    if let Some(parent) = paths.config_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory {}", parent.display()))?;
    }
    if let Some(parent) = paths.db_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory {}", parent.display()))?;
    }
    fs::create_dir_all(&paths.dashboard_dir).with_context(|| {
        format!(
            "Failed to create dashboard directory {}",
            paths.dashboard_dir.display()
        )
    })?;
    Ok(())
}

impl AiProvider {
    pub fn as_agent(self) -> crate::args::Agent {
        match self {
            Self::Copilot => crate::args::Agent::Copilot,
            Self::Gemini => crate::args::Agent::Gemini,
            Self::Kiro => crate::args::Agent::Kiro,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_expected() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.watch_period_seconds, 60);
        assert_eq!(cfg.rereview_mode, RereviewMode::OnUpdate);
        assert_eq!(cfg.dashboard.host, "127.0.0.1");
        assert_eq!(cfg.dashboard.port, 8787);
        assert_eq!(cfg.ai.provider, AiProvider::Copilot);
    }

    #[test]
    fn toml_overrides_fields() {
        let raw = r#"
watch_period_seconds = 15
rereview_mode = "manual"

[ai]
provider = "kiro"
model = "x"

[dashboard]
host = "0.0.0.0"
port = 9000
"#;

        let cfg: AppConfig = toml::from_str(raw).unwrap();
        assert_eq!(cfg.watch_period_seconds, 15);
        assert_eq!(cfg.rereview_mode, RereviewMode::Manual);
        assert_eq!(cfg.ai.provider, AiProvider::Kiro);
        assert_eq!(cfg.ai.model.as_deref(), Some("x"));
        assert_eq!(cfg.dashboard.host, "0.0.0.0");
        assert_eq!(cfg.dashboard.port, 9000);
    }

    #[test]
    fn path_resolution_uses_home() {
        let paths = resolve_paths().unwrap();
        assert!(paths.config_path.ends_with(".config/gigi/config.toml"));
        assert!(paths.db_path.ends_with(".local/share/gigi/gigi.db"));
    }

    #[test]
    fn default_config_template_is_valid() {
        let cfg: AppConfig = toml::from_str(default_config_toml()).unwrap();
        assert_eq!(cfg.watch_period_seconds, 60);
        assert_eq!(cfg.rereview_mode, RereviewMode::OnUpdate);
        assert_eq!(cfg.ai.provider, AiProvider::Copilot);
        assert_eq!(cfg.ai.model, None);
        assert_eq!(cfg.dashboard.host, "127.0.0.1");
        assert_eq!(cfg.dashboard.port, 8787);
    }
}
