//! Claude Code hook integration (CC-specific).
//!
//! Hooks are only meaningful when running inside Claude Code.
//! Use [`is_claude_code()`] to guard CC-specific behaviour.

use crate::config::Config;
use crate::engine::{self, TemplateEngine};
use crate::models::DEFAULT_MODEL_ID;
use crate::server::HelmsmanServer;

/// Returns true when the process is running inside a Claude Code session.
///
/// Claude Code sets `CLAUDECODE=1` in the environment.
pub fn is_claude_code() -> bool {
    std::env::var("CLAUDECODE").as_deref() == Ok("1")
}

/// Persist environment variables for subsequent Bash tool calls.
///
/// Claude Code provides `CLAUDE_ENV_FILE` — a path to a shell script that is
/// sourced before every Bash command in the session. Writing `export KEY=value`
/// lines to this file makes them available to all future tool invocations.
///
/// Does nothing when not running inside Claude Code (file path absent).
pub fn persist_env(vars: &[(&str, &str)]) -> std::io::Result<()> {
    let path = match std::env::var("CLAUDE_ENV_FILE") {
        Ok(p) if !p.is_empty() => p,
        _ => return Ok(()), // not in CC, or env file not set
    };

    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    for (key, value) in vars {
        writeln!(file, "export {}={}", key, shell_escape(value))?;
    }
    Ok(())
}

/// Minimal shell escaping — wraps value in single quotes, escaping internal single quotes.
fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Entry point for `helmsman ping`.
pub fn cmd_ping() {
    if is_claude_code() {
        let entrypoint =
            std::env::var("CLAUDE_CODE_ENTRYPOINT").unwrap_or_else(|_| "unknown".into());
        let session = std::env::var("HELMSMAN_SESSION_ID").unwrap_or_else(|_| "no session".into());
        let model = std::env::var("HELMSMAN_MODEL").unwrap_or_else(|_| "unknown model".into());

        println!("🧭 PONG");
        println!();
        println!("Hi Claude. I can see you.");
        println!("You are running inside Claude Code ({}).", entrypoint);
        println!("Model  : {}", model);
        println!("Session: {}", session);
        println!();
        println!("Helmsman is watching. Behave.");
    } else {
        println!("🧭 PONG");
        println!();
        println!("Hello, human. You are not Claude.");
        println!("CLAUDECODE is not set — running outside Claude Code.");
        println!();
        println!("If you expected to be Claude, something has gone terribly wrong.");
    }
}

/// Entry point for `helmsman hook`.
pub fn cmd_hook(
    event_override: Option<&str>,
    model_override: Option<&str>,
    dump: Option<&std::path::Path>,
) -> Result<(), Box<dyn std::error::Error>> {
    use std::io::Read;

    if !is_claude_code() && event_override.is_none() && dump.is_none() {
        eprintln!("helmsman hook is designed to run as a Claude Code hook.");
        eprintln!("Wire it up in .claude/settings.json:");
        eprintln!(
            r#"  {{"hooks":{{"SessionStart":[{{"hooks":[{{"type":"command","command":"helmsman hook"}}]}}]}}}}"#
        );
        eprintln!(
            "Or test manually: echo '{{\"hook_event_name\":\"SessionStart\",...}}' | helmsman hook --event SessionStart"
        );
        return Ok(());
    }

    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;

    // Dump raw payload if requested (append so all hook events accumulate)
    if let Some(path) = dump {
        use std::io::Write;
        let pretty = serde_json::from_str::<serde_json::Value>(&input)
            .map(|v| serde_json::to_string_pretty(&v).unwrap_or(input.clone()))
            .unwrap_or(input.clone());
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        writeln!(file, "{}", pretty)?;
        writeln!(file, "---")?;
    }

    let hook_data: serde_json::Value =
        serde_json::from_str(&input).unwrap_or(serde_json::Value::Null);

    let event = event_override
        .map(|s| s.to_string())
        .or_else(|| hook_data["hook_event_name"].as_str().map(String::from))
        .unwrap_or_default();

    match event.as_str() {
        "SessionStart" => handle_session_start(&hook_data, model_override)?,
        "PreCompact" => handle_pre_compact(&hook_data)?,
        _ => {} // Unknown event — exit 0, no-op
    }

    Ok(())
}

/// Handle the `SessionStart` hook event.
///
/// Renders `AGENTS.md.j2` for the session model and emits it as
/// `hookSpecificOutput.additionalContext` so Claude Code injects it into the session.
fn handle_session_start(
    data: &serde_json::Value,
    model_override: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let model_id = model_override
        .map(String::from)
        .or_else(|| data["model"].as_str().map(String::from))
        .unwrap_or_else(|| DEFAULT_MODEL_ID.to_string());

    // cd into project dir so skill/template search works from project root
    if let Some(cwd) = data["cwd"].as_str() {
        let _ = std::env::set_current_dir(cwd); // best-effort
    }

    // Persist useful session variables for Bash tool calls
    let _ = persist_env(&[
        ("HELMSMAN_MODEL", &model_id),
        (
            "HELMSMAN_SESSION_ID",
            data["session_id"].as_str().unwrap_or(""),
        ),
    ]);

    let config = Config::load()?;
    let templates_dir = config.templates_dir().unwrap_or_else(|_| ".".into());
    let engine = TemplateEngine::new(&templates_dir)?;
    let helmsman = HelmsmanServer::new(&config, engine);

    match helmsman.render_instructions(&model_id, None) {
        Ok(instructions) => {
            let response = serde_json::json!({
                "hookSpecificOutput": {
                    "hookEventName": "SessionStart",
                    "additionalContext": instructions
                }
            });
            println!("{}", response);
        }
        Err(engine::EngineError::TemplateNotFound(_)) => {} // No template — silent exit 0
        Err(_) => {}                                        // Other error — silent exit 0
    }

    Ok(())
}

/// Handle the `PreCompact` hook — re-inject instructions so they survive auto-compaction.
fn handle_pre_compact(data: &serde_json::Value) -> Result<(), Box<dyn std::error::Error>> {
    // Only act on auto-compaction — manual is user-initiated
    if data["trigger"].as_str().unwrap_or("auto") != "auto" {
        return Ok(());
    }

    if let Some(cwd) = data["cwd"].as_str() {
        let _ = std::env::set_current_dir(cwd);
    }

    let model_id = std::env::var("HELMSMAN_MODEL").unwrap_or_else(|_| DEFAULT_MODEL_ID.to_string());

    let config = Config::load()?;
    let templates_dir = config.templates_dir().unwrap_or_else(|_| ".".into());
    let engine = TemplateEngine::new(&templates_dir)?;
    let helmsman = HelmsmanServer::new(&config, engine);

    if let Ok(instructions) = helmsman.render_instructions(&model_id, None) {
        let response = serde_json::json!({
            "systemMessage": format!(
                "Context compaction occurred. Project instructions:\n\n{}",
                instructions
            )
        });
        println!("{}", response);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
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

        let model = Some("claude-opus-4-6")
            .map(String::from)
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
}
