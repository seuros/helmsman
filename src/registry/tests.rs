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
fn test_skill_lock_parses_chrono_era_timestamps() {
    // Lock files written before the jiff migration store chrono-formatted
    // RFC 3339 timestamps; they must keep loading.
    let toml_src = r#"
            [skills.legacy]
            source = "owner/repo"
            installed_at = "2026-07-01T12:34:56.789012Z"
            path = "/path/to/skill.j2"
            global = false
        "#;

    let lock: SkillLock = toml::from_str(toml_src).unwrap();
    assert!(lock.has("legacy"));
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
