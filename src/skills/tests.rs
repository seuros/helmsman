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
