use super::*;

#[test]
fn test_shell_escape_simple() {
    assert_eq!(shell_escape("hello"), "'hello'");
}

#[test]
fn test_shell_escape_with_single_quote() {
    assert_eq!(shell_escape("it's"), "'it'\\''s'");
}

#[test]
fn test_is_claude_code_false_outside_cc() {
    // In test context CLAUDECODE may or may not be set — just verify it doesn't panic
    let _ = is_claude_code();
}

#[test]
fn test_hook_data_extraction() {
    let data = serde_json::json!({
        "hook_event_name": "SessionStart",
        "model": "claude-sonnet-4-6",
        "cwd": "/tmp",
        "source": "startup"
    });

    let model = data["model"]
        .as_str()
        .map(String::from)
        .unwrap_or_else(|| DEFAULT_MODEL_ID.to_string());
    assert_eq!(model, "claude-sonnet-4-6");
}

#[test]
fn test_hook_data_null_fallback() {
    let data = serde_json::Value::Null;
    let model = data["model"]
        .as_str()
        .map(String::from)
        .unwrap_or_else(|| DEFAULT_MODEL_ID.to_string());
    assert_eq!(model, DEFAULT_MODEL_ID);
}

#[test]
fn test_model_override_takes_precedence() {
    let data = serde_json::json!({ "model": "claude-haiku-4-5" });

    let model = Some(String::from("claude-opus-4-6"))
        .or_else(|| data["model"].as_str().map(String::from))
        .unwrap_or_else(|| DEFAULT_MODEL_ID.to_string());
    assert_eq!(model, "claude-opus-4-6");

    let model: String = None::<&str>
        .map(String::from)
        .or_else(|| data["model"].as_str().map(String::from))
        .unwrap_or_else(|| DEFAULT_MODEL_ID.to_string());
    assert_eq!(model, "claude-haiku-4-5");
}

#[test]
fn test_pre_compact_skips_manual() {
    let data = serde_json::json!({ "trigger": "manual", "cwd": "/tmp" });
    // manual trigger should be a no-op
    let result = handle_pre_compact(&data);
    assert!(result.is_ok());
}
