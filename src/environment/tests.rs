use super::*;

#[test]
fn test_detect_environment() {
    let env = Environment::detect();
    // Should at least detect OS
    assert!(!env.os.is_empty());
    assert!(!env.shell.is_empty() || env.shell == "unknown");
}

#[test]
fn test_has_binary() {
    // 'ls' should exist on unix systems
    #[cfg(target_family = "unix")]
    assert!(has_binary("ls"));
}
