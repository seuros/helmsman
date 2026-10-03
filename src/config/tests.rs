use super::*;

#[test]
fn test_default_config() {
    let config = Config::default();
    assert_eq!(config.defaults.tier, "engineer");
    assert_eq!(MCP_NAME, "helmsman");
}

#[test]
fn test_expand_tilde() {
    let expanded = expand_tilde("~/test");
    assert!(!expanded.to_string_lossy().starts_with("~"));
}
