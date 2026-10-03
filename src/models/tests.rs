use super::*;

#[test]
fn test_resolve_agi() {
    let config = Config::default();
    let resolver = ModelResolver::new(&config);

    assert_eq!(resolver.resolve("claude-opus-4-5-20251101"), "agi");
}

#[test]
fn test_resolve_engineer() {
    let config = Config::default();
    let resolver = ModelResolver::new(&config);

    assert_eq!(resolver.resolve("claude-4-5-sonnet-20251022"), "engineer");
}

#[test]
fn test_resolve_monkey() {
    let config = Config::default();
    let resolver = ModelResolver::new(&config);

    assert_eq!(resolver.resolve("claude-4-5-haiku-20251022"), "monkey");
}

#[test]
fn test_resolve_unknown_uses_default() {
    let config = Config::default();
    let resolver = ModelResolver::new(&config);

    assert_eq!(resolver.resolve("unknown-model"), "engineer");
}

#[test]
fn test_tier_aliases() {
    let config = Config::default();
    let resolver = ModelResolver::new(&config);

    // Short aliases
    assert_eq!(resolver.resolve("a"), "agi");
    assert_eq!(resolver.resolve("e"), "engineer");
    assert_eq!(resolver.resolve("eng"), "engineer");
    assert_eq!(resolver.resolve("m"), "monkey");

    // Explicit tier names
    assert_eq!(resolver.resolve("agi"), "agi");
    assert_eq!(resolver.resolve("engineer"), "engineer");
    assert_eq!(resolver.resolve("monkey"), "monkey");
}

#[test]
fn test_neutral_aliases() {
    let config = Config::default();
    let resolver = ModelResolver::new(&config);

    // Corporate-friendly neutral aliases
    assert_eq!(resolver.resolve("architect"), "agi");
    assert_eq!(resolver.resolve("standard"), "engineer");
    assert_eq!(resolver.resolve("basic"), "monkey");
    assert_eq!(resolver.resolve("simple"), "monkey");
}

#[test]
fn test_specificity_precedence() {
    let config = Config::default();
    let resolver = ModelResolver::new(&config);

    // Specific patterns should beat broad fallbacks.
    assert_eq!(resolver.resolve("gpt-5.4-xhigh"), "agi");
    assert_eq!(resolver.resolve("gpt-5.4-high"), "agi");
    assert_eq!(resolver.resolve("gpt-5.4-mini"), "monkey");
    assert_eq!(resolver.resolve("gpt-5.4"), "engineer");
}
