//! Configuration loading and management for Helmsman.

use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Failed to read config: {0}")]
    ReadError(#[from] std::io::Error),
    #[error("Failed to parse config: {0}")]
    ParseError(#[from] toml::de::Error),
    #[error("Templates directory not found: {0}")]
    TemplatesNotFound(PathBuf),
}

/// Main configuration structure.
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub models: HashMap<String, String>,
    #[serde(default)]
    pub defaults: DefaultsConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_name")]
    pub name: String,
    #[serde(default = "default_version")]
    pub version: String,
    pub templates_dir: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            name: default_name(),
            version: default_version(),
            templates_dir: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct DefaultsConfig {
    #[serde(default = "default_tier")]
    pub tier: String,
}

impl Default for DefaultsConfig {
    fn default() -> Self {
        Self {
            tier: default_tier(),
        }
    }
}

fn default_name() -> String {
    "helmsman".to_string()
}

fn default_version() -> String {
    "0.1.0".to_string()
}

fn default_tier() -> String {
    "engineer".to_string()
}

/// Models file structure (models.toml)
#[derive(Debug, Clone, Deserialize, Default)]
struct ModelsFile {
    #[serde(default)]
    models: HashMap<String, String>,
}

/// Embedded models.toml - compiled into binary
const EMBEDDED_MODELS: &str = include_str!("models.toml");

impl Config {
    /// Load config from the first available location.
    /// Search order:
    /// 1. $HELMSMAN_CONFIG env var
    /// 2. ./helmsman.toml (project-local)
    /// 3. ~/.config/helmsman/helmsman.toml (user global)
    ///
    /// Also loads models.toml from same search paths.
    pub fn load() -> Result<Self, ConfigError> {
        let search_paths = Self::config_search_paths();

        let mut config = None;
        for path in &search_paths {
            if path.exists() {
                config = Some(Self::load_from(path)?);
                break;
            }
        }

        let mut config = config.unwrap_or_default();

        // Load models from models.toml
        let models = Self::load_models();
        if !models.is_empty() {
            config.models = models;
        }

        Ok(config)
    }

    /// Load config from a specific path.
    pub fn load_from(path: &Path) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    }

    /// Load models from embedded models.toml (compiled into binary)
    fn load_models() -> HashMap<String, String> {
        toml::from_str::<ModelsFile>(EMBEDDED_MODELS)
            .map(|m| m.models)
            .unwrap_or_default()
    }

    /// Get the templates directory path.
    /// Search order:
    /// 1. $HELMSMAN_TEMPLATES env var
    /// 2. Config file templates_dir setting
    /// 3. ./templates/ (project-local)
    /// 4. ~/.config/helmsman/templates/ (user global)
    pub fn templates_dir(&self) -> Result<PathBuf, ConfigError> {
        // Check env var first
        if let Ok(env_path) = std::env::var("HELMSMAN_TEMPLATES") {
            let path = PathBuf::from(env_path);
            if path.exists() {
                return Ok(path);
            }
        }

        // Check config setting
        if let Some(ref dir) = self.server.templates_dir {
            let path = expand_tilde(dir);
            if path.exists() {
                return Ok(path);
            }
        }

        // Check project-local
        let local = PathBuf::from("./templates");
        if local.exists() {
            return Ok(local);
        }

        // Check user global
        if let Some(config_dir) = dirs::config_dir() {
            let global = config_dir.join("helmsman").join("templates");
            if global.exists() {
                return Ok(global);
            }
        }

        Err(ConfigError::TemplatesNotFound(PathBuf::from("templates")))
    }

    fn config_search_paths() -> Vec<PathBuf> {
        let mut paths = Vec::new();

        // Env var override
        if let Ok(env_path) = std::env::var("HELMSMAN_CONFIG") {
            paths.push(PathBuf::from(env_path));
        }

        // Project-local
        paths.push(PathBuf::from("./helmsman.toml"));

        // User global
        if let Some(config_dir) = dirs::config_dir() {
            paths.push(config_dir.join("helmsman").join("helmsman.toml"));
        }

        paths
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server: ServerConfig::default(),
            models: Self::load_models(),
            defaults: DefaultsConfig::default(),
        }
    }
}

/// Expand ~ to home directory.
fn expand_tilde(path: &str) -> PathBuf {
    if let Some(stripped) = path.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(stripped);
    }
    PathBuf::from(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.server.name, "helmsman");
        assert_eq!(config.defaults.tier, "engineer");
    }

    #[test]
    fn test_expand_tilde() {
        let expanded = expand_tilde("~/test");
        assert!(!expanded.to_string_lossy().starts_with("~"));
    }
}
