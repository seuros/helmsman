//! Remote skill fetching from GitHub repositories.

use crate::skills::{
    is_partial_skill, parse_frontmatter, skill_name_from_path, SkillMeta, SKILL_EXTENSION,
    SKILLS_DIR_NAME,
};
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum RemoteError {
    #[error("Failed to clone repository: {0}")]
    CloneFailed(String),
    #[error("Invalid source format: {0}")]
    InvalidSource(String),
    #[error("No skills found in repository")]
    NoSkillsFound,
    #[error("Skill not found: {0}")]
    SkillNotFound(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Parsed remote source.
#[derive(Debug, Clone)]
pub struct ParsedSource {
    pub owner: String,
    pub repo: String,
    pub subpath: Option<String>,
    pub git_ref: Option<String>,
}

impl ParsedSource {
    /// Parse a source string into owner/repo/subpath.
    ///
    /// Supported formats:
    /// - `owner/repo`
    /// - `owner/repo/path/to/skill`
    /// - `https://github.com/owner/repo`
    /// - `https://github.com/owner/repo/tree/branch/path`
    pub fn parse(input: &str) -> Result<Self, RemoteError> {
        let input = input.trim();

        // Handle GitHub URLs
        if input.starts_with("https://github.com/") || input.starts_with("http://github.com/") {
            return Self::parse_github_url(input);
        }

        // Handle shorthand: owner/repo or owner/repo/path
        Self::parse_shorthand(input)
    }

    fn parse_github_url(url: &str) -> Result<Self, RemoteError> {
        // Remove protocol and domain
        let path = url
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_start_matches("github.com/")
            .trim_end_matches(".git");

        // Split by /tree/ or /blob/ for branch/path
        if let Some((repo_part, rest)) = path.split_once("/tree/") {
            let parts: Vec<&str> = repo_part.split('/').collect();
            if parts.len() < 2 {
                return Err(RemoteError::InvalidSource(url.to_string()));
            }

            // rest is "branch/path/to/file"
            let rest_parts: Vec<&str> = rest.split('/').collect();
            let git_ref = rest_parts.first().map(|s| s.to_string());
            let subpath = if rest_parts.len() > 1 {
                Some(rest_parts[1..].join("/"))
            } else {
                None
            };

            return Ok(Self {
                owner: parts[0].to_string(),
                repo: parts[1].to_string(),
                subpath,
                git_ref,
            });
        }

        // Simple URL: github.com/owner/repo
        Self::parse_shorthand(path)
    }

    fn parse_shorthand(input: &str) -> Result<Self, RemoteError> {
        let parts: Vec<&str> = input.split('/').collect();

        if parts.len() < 2 {
            return Err(RemoteError::InvalidSource(format!(
                "Expected owner/repo, got: {}",
                input
            )));
        }

        let owner = parts[0].to_string();
        let repo = parts[1].to_string();
        let subpath = if parts.len() > 2 {
            Some(parts[2..].join("/"))
        } else {
            None
        };

        Ok(Self {
            owner,
            repo,
            subpath,
            git_ref: None,
        })
    }

    /// Get the clone URL.
    pub fn clone_url(&self) -> String {
        format!("https://github.com/{}/{}.git", self.owner, self.repo)
    }

    /// Get a display name for the source.
    pub fn display_name(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }
}

/// A discovered skill in a remote repository.
#[derive(Debug, Clone)]
pub struct RemoteSkill {
    pub name: String,
    pub path: PathBuf,
    pub description: Option<String>,
}

/// Clone a repository and discover skills.
pub struct RemoteFetcher {
    temp_dir: TempDir,
    repo_path: PathBuf,
}

impl RemoteFetcher {
    /// Clone a repository to a temporary directory.
    pub fn clone(source: &ParsedSource) -> Result<Self, RemoteError> {
        let temp_dir = TempDir::new()?;
        let repo_path = temp_dir.path().join("repo");

        // Build git clone command
        let mut cmd = Command::new("git");
        cmd.arg("clone").arg("--depth=1").arg("--single-branch");

        if let Some(ref git_ref) = source.git_ref {
            cmd.arg("--branch").arg(git_ref);
        }

        cmd.arg(&source.clone_url()).arg(&repo_path);

        let output = cmd.output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(RemoteError::CloneFailed(stderr.to_string()));
        }

        Ok(Self {
            temp_dir,
            repo_path,
        })
    }

    /// Discover skills in the cloned repository.
    pub fn discover_skills(&self, subpath: Option<&str>) -> Result<Vec<RemoteSkill>, RemoteError> {
        let search_root = if let Some(sub) = subpath {
            self.repo_path.join(sub)
        } else {
            self.repo_path.clone()
        };

        let mut skills = Vec::new();

        // Search order:
        // 1. skills/*.j2
        // 2. .skills/*.j2
        // 3. Root *.j2 files

        let search_dirs = [
            search_root.join("skills"),
            search_root.join(SKILLS_DIR_NAME),
            search_root.clone(),
        ];

        for dir in &search_dirs {
            if dir.is_dir() {
                self.scan_directory(dir, &mut skills)?;
            }
        }

        // If subpath was a specific skill file, try that
        if let Some(sub) = subpath {
            let specific = self
                .repo_path
                .join(format!("{}.{}", sub, SKILL_EXTENSION));
            if specific.is_file() {
                if let Some(name) = skill_name_from_path(&specific) {
                    let skill = self.parse_skill_file(&name, &specific)?;
                    if !skills.iter().any(|s| s.name == skill.name) {
                        skills.push(skill);
                    }
                }
            }
        }

        // Fallback: deep scan if nothing found in expected locations.
        if skills.is_empty() {
            self.scan_tree(&search_root, &mut skills, 0)?;
        }

        if skills.is_empty() {
            return Err(RemoteError::NoSkillsFound);
        }

        // Sort by name
        skills.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(skills)
    }

    fn scan_directory(&self, dir: &Path, skills: &mut Vec<RemoteSkill>) -> Result<(), RemoteError> {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return Ok(()),
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = skill_name_from_path(&path) else {
                continue;
            };
            if is_partial_skill(&name) {
                continue;
            }

            let skill = self.parse_skill_file(&name, &path)?;
            if !skills.iter().any(|s| s.name == skill.name) {
                skills.push(skill);
            }
        }

        Ok(())
    }

    fn scan_tree(
        &self,
        dir: &Path,
        skills: &mut Vec<RemoteSkill>,
        depth: usize,
    ) -> Result<(), RemoteError> {
        if depth > 12 {
            return Ok(());
        }

        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return Ok(()),
        };

        for entry in entries.flatten() {
            let path = entry.path();

            if path.file_name().and_then(|s| s.to_str()) == Some(".git") {
                continue;
            }

            if path.is_dir() {
                self.scan_tree(&path, skills, depth + 1)?;
                continue;
            }

            let Some(name) = skill_name_from_path(&path) else {
                continue;
            };
            if is_partial_skill(&name) {
                continue;
            }

            let skill = self.parse_skill_file(&name, &path)?;
            if !skills.iter().any(|s| s.name == skill.name) {
                skills.push(skill);
            }
        }

        Ok(())
    }

    fn parse_skill_file(&self, name: &str, path: &Path) -> Result<RemoteSkill, RemoteError> {
        let content = std::fs::read_to_string(path)?;
        let (meta, _body): (SkillMeta, String) = parse_frontmatter(&content);

        Ok(RemoteSkill {
            name: name.to_string(),
            path: path.to_path_buf(),
            description: meta.description,
        })
    }

    /// Get a specific skill by name.
    pub fn get_skill(&self, name: &str, subpath: Option<&str>) -> Result<RemoteSkill, RemoteError> {
        let skills = self.discover_skills(subpath)?;
        skills
            .into_iter()
            .find(|s| s.name == name)
            .ok_or_else(|| RemoteError::SkillNotFound(name.to_string()))
    }

    /// Copy a skill to a destination directory.
    pub fn install_skill(
        &self,
        skill: &RemoteSkill,
        dest_dir: &Path,
    ) -> Result<PathBuf, RemoteError> {
        std::fs::create_dir_all(dest_dir)?;

        let dest_file = dest_dir.join(format!("{}.{}", skill.name, SKILL_EXTENSION));
        std::fs::copy(&skill.path, &dest_file)?;

        Ok(dest_file)
    }

    /// Get the temp directory path (for debugging).
    pub fn temp_path(&self) -> &Path {
        self.temp_dir.path()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_shorthand() {
        let source = ParsedSource::parse("owner/repo").unwrap();
        assert_eq!(source.owner, "owner");
        assert_eq!(source.repo, "repo");
        assert!(source.subpath.is_none());
    }

    #[test]
    fn test_parse_shorthand_with_path() {
        let source = ParsedSource::parse("owner/repo/skills/commit").unwrap();
        assert_eq!(source.owner, "owner");
        assert_eq!(source.repo, "repo");
        assert_eq!(source.subpath, Some("skills/commit".to_string()));
    }

    #[test]
    fn test_parse_github_url() {
        let source = ParsedSource::parse("https://github.com/owner/repo").unwrap();
        assert_eq!(source.owner, "owner");
        assert_eq!(source.repo, "repo");
    }

    #[test]
    fn test_parse_github_url_with_tree() {
        let source = ParsedSource::parse("https://github.com/owner/repo/tree/main/skills").unwrap();
        assert_eq!(source.owner, "owner");
        assert_eq!(source.repo, "repo");
        assert_eq!(source.git_ref, Some("main".to_string()));
        assert_eq!(source.subpath, Some("skills".to_string()));
    }
}
