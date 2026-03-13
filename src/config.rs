use std::path::PathBuf;

use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use tokio::fs;

pub const DEFAULT_KIRO_MODEL: &str = "claude-opus-4.6";

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub config_path: PathBuf,
    pub db_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub watch_period_seconds: u64,
    pub rereview_mode: RereviewMode,
    pub initial_review_lookback_days: u64,
    pub initial_review_max_prs: usize,
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
            initial_review_lookback_days: 3,
            initial_review_max_prs: 10,
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
initial_review_lookback_days = 3
initial_review_max_prs = 10

[ai]
provider = "copilot" # or "gemini" or "kiro"
# model = "gpt-5.3-codex"
# when provider = "kiro", the default model is "claude-opus-4.6"

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
    Ok(AppPaths {
        config_path,
        db_path,
    })
}

pub async fn load_config(config_path: &PathBuf) -> anyhow::Result<AppConfig> {
    if !fs::try_exists(config_path).await? {
        return Ok(AppConfig::default());
    }

    let raw = fs::read_to_string(config_path)
        .await
        .with_context(|| format!("Failed to read config file at {}", config_path.display()))?;
    let config: AppConfig = toml::from_str(&raw)
        .with_context(|| format!("Failed to parse TOML config at {}", config_path.display()))?;

    Ok(config)
}

pub async fn ensure_parent_dirs(paths: &AppPaths) -> anyhow::Result<()> {
    if let Some(parent) = paths.config_path.parent() {
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("Failed to create directory {}", parent.display()))?;
    }
    if let Some(parent) = paths.db_path.parent() {
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("Failed to create directory {}", parent.display()))?;
    }
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

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Copilot => "copilot",
            Self::Gemini => "gemini",
            Self::Kiro => "kiro",
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
        assert_eq!(cfg.initial_review_lookback_days, 3);
        assert_eq!(cfg.initial_review_max_prs, 10);
        assert_eq!(cfg.dashboard.host, "127.0.0.1");
        assert_eq!(cfg.dashboard.port, 8787);
        assert_eq!(cfg.ai.provider, AiProvider::Copilot);
    }

    #[test]
    fn toml_overrides_fields() {
        let raw = r#"
watch_period_seconds = 15
rereview_mode = "manual"
initial_review_lookback_days = 7
initial_review_max_prs = 5

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
        assert_eq!(cfg.initial_review_lookback_days, 7);
        assert_eq!(cfg.initial_review_max_prs, 5);
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
        assert_eq!(cfg.initial_review_lookback_days, 3);
        assert_eq!(cfg.initial_review_max_prs, 10);
        assert_eq!(cfg.ai.provider, AiProvider::Copilot);
        assert_eq!(cfg.ai.model, None);
        assert_eq!(cfg.dashboard.host, "127.0.0.1");
        assert_eq!(cfg.dashboard.port, 8787);
    }
}
