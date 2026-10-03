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
