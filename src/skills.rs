//! Shared skill utilities and metadata.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const SKILLS_DIR_NAME: &str = ".skills";
pub const SKILL_EXTENSION: &str = "j2";

/// Skill metadata from frontmatter.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillMeta {
    pub name: Option<String>,
    pub description: Option<String>,
    pub topics: Option<Vec<String>>,
    pub tiers: Option<Vec<String>>,
    pub authors: Option<Vec<String>>,
}

/// Parse YAML frontmatter from skill content.
/// Returns (metadata, content without frontmatter).
pub fn parse_frontmatter<T: DeserializeOwned + Default>(content: &str) -> (T, String) {
    let trimmed = content.trim_start();

    if !trimmed.starts_with("---") {
        return (T::default(), content.to_string());
    }

    let (end_pos, close_len) = if let Some(pos) = trimmed[3..].find("\n---") {
        (pos, 4)
    } else if let Some(pos) = trimmed[3..].find("\r\n---") {
        (pos, 5)
    } else {
        return (T::default(), content.to_string());
    };

    let yaml_start = 3;
    let yaml_end = 3 + end_pos;
    let yaml_content = trimmed[yaml_start..yaml_end].trim();
    let remaining_start = 3 + end_pos + close_len;
    let remaining = trimmed.get(remaining_start..).unwrap_or("");

    let meta: T = serde_yaml::from_str(yaml_content).unwrap_or_default();
    (meta, remaining.to_string())
}

/// Extract a skill name from a .j2 path (no partial filtering).
pub fn skill_name_from_path(path: &Path) -> Option<String> {
    if path.extension().and_then(|e| e.to_str()) != Some(SKILL_EXTENSION) {
        return None;
    }

    let name = path.file_stem()?.to_str()?;
    Some(name.to_string())
}

/// Returns true if a skill name represents a partial.
pub fn is_partial_skill(name: &str) -> bool {
    name.starts_with('_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_frontmatter_basic() {
        let content = r#"---
name: Test Skill
description: A test skill description
---
# Content"#;
        let (meta, body): (SkillMeta, String) = parse_frontmatter(content);
        assert_eq!(
            meta.description,
            Some("A test skill description".to_string())
        );
        assert!(body.contains("# Content"));
    }

    #[test]
    fn test_skill_name_from_path() {
        let path = Path::new("skills/commit.j2");
        assert_eq!(skill_name_from_path(path), Some("commit".to_string()));
    }

    #[test]
    fn test_is_partial_skill() {
        assert!(is_partial_skill("_partial"));
        assert!(!is_partial_skill("commit"));
    }
}
