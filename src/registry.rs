//! Skill registry and lock file management.

use chrono::{DateTime, Utc};
use crate::skills::SKILLS_DIR_NAME;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum RegistryError {
    #[error("Failed to read registry: {0}")]
    ReadError(#[from] std::io::Error),
    #[error("Failed to parse registry: {0}")]
    ParseError(#[from] toml::de::Error),
    #[error("Failed to serialize registry: {0}")]
    SerializeError(#[from] toml::ser::Error),
}

/// A single installed skill entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillEntry {
    /// Source repository (e.g., "owner/repo")
    pub source: String,
    /// Installation timestamp
    pub installed_at: DateTime<Utc>,
    /// Installed path (relative to skills dir or absolute)
    pub path: PathBuf,
    /// Whether this is a global or project-level install
    pub global: bool,
}

/// The skill lock file structure.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct SkillLock {
    /// Map of skill name -> entry
    #[serde(default)]
    pub skills: HashMap<String, SkillEntry>,
}

impl SkillLock {
    /// Load the lock file from disk.
    pub fn load(path: &Path) -> Result<Self, RegistryError> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let content = std::fs::read_to_string(path)?;
        let lock: SkillLock = toml::from_str(&content)?;
        Ok(lock)
    }

    /// Save the lock file to disk.
    pub fn save(&self, path: &Path) -> Result<(), RegistryError> {
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Add or update a skill entry.
    pub fn add(&mut self, name: &str, source: &str, path: PathBuf, global: bool) {
        self.skills.insert(
            name.to_string(),
            SkillEntry {
                source: source.to_string(),
                installed_at: Utc::now(),
                path,
                global,
            },
        );
    }

    /// Remove a skill entry.
    pub fn remove(&mut self, name: &str) -> Option<SkillEntry> {
        self.skills.remove(name)
    }

    /// Get a skill entry.
    pub fn get(&self, name: &str) -> Option<&SkillEntry> {
        self.skills.get(name)
    }

    /// Check if a skill is installed.
    pub fn has(&self, name: &str) -> bool {
        self.skills.contains_key(name)
    }

    /// List all installed skills.
    pub fn list(&self) -> impl Iterator<Item = (&String, &SkillEntry)> {
        self.skills.iter()
    }

    /// List skills from a specific source.
    pub fn list_from_source(&self, source: &str) -> Vec<(&String, &SkillEntry)> {
        self.skills
            .iter()
            .filter(|(_, entry)| entry.source == source)
            .collect()
    }
}

/// Registry manager for handling skill installations.
pub struct Registry {
    /// Path to the lock file
    lock_path: PathBuf,
    /// The loaded lock file
    lock: SkillLock,
    /// Global skills directory
    global_skills_dir: PathBuf,
}

impl Registry {
    /// Create a new registry.
    pub fn new() -> Result<Self, RegistryError> {
        let config_dir = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("helmsman");

        let lock_path = config_dir.join("skills.lock");
        let global_skills_dir = config_dir.join("skills");
        let lock = SkillLock::load(&lock_path)?;

        Ok(Self {
            lock_path,
            lock,
            global_skills_dir,
        })
    }

    /// Get the global skills directory.
    pub fn global_skills_dir(&self) -> &Path {
        &self.global_skills_dir
    }

    /// Get the project skills directory.
    pub fn project_skills_dir() -> PathBuf {
        PathBuf::from(SKILLS_DIR_NAME)
    }

    /// Record a skill installation.
    pub fn record_install(
        &mut self,
        name: &str,
        source: &str,
        path: PathBuf,
        global: bool,
    ) -> Result<(), RegistryError> {
        self.lock.add(name, source, path, global);
        self.lock.save(&self.lock_path)
    }

    /// Remove a skill record.
    pub fn record_remove(&mut self, name: &str) -> Result<Option<SkillEntry>, RegistryError> {
        let entry = self.lock.remove(name);
        self.lock.save(&self.lock_path)?;
        Ok(entry)
    }

    /// Get a skill entry.
    pub fn get(&self, name: &str) -> Option<&SkillEntry> {
        self.lock.get(name)
    }

    /// Check if a skill is installed.
    pub fn is_installed(&self, name: &str) -> bool {
        self.lock.has(name)
    }

    /// List all installed skills.
    pub fn list(&self) -> impl Iterator<Item = (&String, &SkillEntry)> {
        self.lock.list()
    }

    /// List skills from a specific source.
    pub fn list_from_source(&self, source: &str) -> Vec<(&String, &SkillEntry)> {
        self.lock.list_from_source(source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_skill_lock_roundtrip() {
        let temp = TempDir::new().unwrap();
        let lock_path = temp.path().join("skills.lock");

        let mut lock = SkillLock::default();
        lock.add(
            "test-skill",
            "owner/repo",
            PathBuf::from("/path/to/skill.j2"),
            true,
        );

        lock.save(&lock_path).unwrap();

        let loaded = SkillLock::load(&lock_path).unwrap();
        assert!(loaded.has("test-skill"));

        let entry = loaded.get("test-skill").unwrap();
        assert_eq!(entry.source, "owner/repo");
        assert!(entry.global);
    }

    #[test]
    fn test_skill_lock_remove() {
        let mut lock = SkillLock::default();
        lock.add(
            "test-skill",
            "owner/repo",
            PathBuf::from("/path/to/skill.j2"),
            false,
        );

        assert!(lock.has("test-skill"));
        lock.remove("test-skill");
        assert!(!lock.has("test-skill"));
    }
}
