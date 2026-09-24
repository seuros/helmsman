//! Stateless goal protocol adapter. The capable host owns all goal state.

use mcp_host::registry::tools::{Tool, ToolError, ToolFuture, ToolOutput};
use mcp_host::server::session::Session;
use mcp_host::server::visibility::{ExecutionContext, VisibilityContext};
use serde::Deserialize;
use serde_json::{Map, Value, json};

const CAPABILITY: &str = "chaos/goals";

#[derive(Clone, Copy)]
pub enum GoalTool {
    Start,
    Get,
    Check,
    Pause,
}

fn capability(session: &Session) -> Option<&Value> {
    session
        .capabilities
        .as_ref()
        .and_then(|caps| caps.experimental.as_ref())
        .and_then(|caps| caps.get(CAPABILITY))
        .filter(|caps| caps["version"] == 1 && caps["client"] == "free_chaos")
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Start {
    objective: String,
    criteria: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Check {
    claim: String,
}

impl GoalTool {
    fn supported(&self, session: &Session) -> bool {
        capability(session)
            .is_some_and(|caps| !matches!(self, Self::Check) || caps["check"] == true)
    }

    fn command(&self, params: Value) -> Result<Value, ToolError> {
        match self {
            Self::Start => {
                let args: Start = serde_json::from_value(params).map_err(|_| {
                    ToolError::InvalidArguments("expected objective and criteria".into())
                })?;
                if args.objective.trim().is_empty()
                    || args.objective.len() > 4096
                    || args.criteria.is_empty()
                    || args.criteria.len() > 8
                    || args
                        .criteria
                        .iter()
                        .any(|s| s.trim().is_empty() || s.len() > 1024)
                {
                    return Err(ToolError::InvalidArguments(
                        "provide an objective (1–4096 bytes) and 1–8 criteria (1–1024 bytes each)"
                            .into(),
                    ));
                }
                Ok(json!({"operation":"start","objective":args.objective,"criteria":args.criteria}))
            }
            Self::Check => {
                let args: Check = serde_json::from_value(params).map_err(|_| {
                    ToolError::InvalidArguments(
                        "expected a claim, not evidence or a verdict".into(),
                    )
                })?;
                if args.claim.len() > 8192 {
                    return Err(ToolError::InvalidArguments(
                        "claim must be at most 8192 bytes".into(),
                    ));
                }
                Ok(json!({"operation":"check","claim":args.claim}))
            }
            Self::Get | Self::Pause => {
                if params != json!({}) {
                    return Err(ToolError::InvalidArguments("expected no arguments".into()));
                }
                Ok(json!({"operation": if matches!(self, Self::Get) {"get"} else {"pause"}}))
            }
        }
    }
}

impl Tool for GoalTool {
    fn name(&self) -> &str {
        match self {
            Self::Start => "start_goal",
            Self::Get => "get_goal",
            Self::Check => "check_goal",
            Self::Pause => "pause_goal",
        }
    }

    fn description(&self) -> Option<&str> {
        Some(match self {
            Self::Start => {
                "Opt into a turn-scoped Jev completion gate, only when the user requests it. Sends the user request, objective, criteria, bounded tool evidence and proposed conclusion to the host's configured Jev backend. After doing the work, call check_goal before your final response. One start per turn; at most 4 checks. No new permissions. Never use this for an unrelated task."
            }
            Self::Get => {
                "Read the host-owned goal status. No worker-provided evidence or completion claims are accepted."
            }
            Self::Check => {
                "Ask Jev to evaluate the current goal before your final response. Supply your proposed conclusion as a claim, never as evidence. The host supplies recorded tool evidence and returns goal status, next_action and guidance. Completion closes the goal, not the turn: respond to the user afterward. Do not run alongside other work. Shares the four-check budget with the stop fallback; unchanged evidence cannot earn another check."
            }
            Self::Pause => {
                "Pause the goal when the user stops it, work is blocked, or evaluation is inappropriate. Pausing is not completion."
            }
        })
    }

    fn input_schema(&self) -> Value {
        match self {
            Self::Start => json!({
                "type":"object","additionalProperties":false,
                "properties":{
                    "objective":{"type":"string","minLength":1,"maxLength":4096},
                    "criteria":{"type":"array","minItems":1,"maxItems":8,
                        "items":{"type":"string","minLength":1,"maxLength":1024}}
                }, "required":["objective","criteria"]
            }),
            Self::Check => json!({
                "type":"object","additionalProperties":false,
                "properties":{"claim":{"type":"string","maxLength":8192}},
                "required":["claim"]
            }),
            _ => json!({"type":"object","properties":{},"additionalProperties":false}),
        }
    }

    fn output_schema(&self) -> Option<Value> {
        Some(
            json!({"type":"object","properties":{CAPABILITY:{"type":"object"}},"required":[CAPABILITY]}),
        )
    }

    fn annotations(&self) -> Option<mcp_host::protocol::types::ToolAnnotations> {
        Some(mcp_host::protocol::types::ToolAnnotations {
            title: None,
            read_only_hint: Some(matches!(self, Self::Get)),
            destructive_hint: Some(false),
            idempotent_hint: Some(matches!(self, Self::Get | Self::Pause)),
            open_world_hint: Some(matches!(self, Self::Start | Self::Check)),
        })
    }

    fn meta(&self) -> Option<Map<String, Value>> {
        Some(Map::from_iter([(CAPABILITY.into(), json!({"version":1}))]))
    }

    fn is_visible(&self, ctx: &VisibilityContext<'_>) -> bool {
        self.supported(ctx.session)
    }

    fn execute<'a>(&'a self, ctx: ExecutionContext<'a>) -> ToolFuture<'a> {
        Box::pin(async move {
            // Also guard direct invocation, not just tools/list.
            if !self.supported(ctx.session) {
                return Err(ToolError::NotFound(self.name().into()));
            }
            let command = self.command(ctx.params.clone())?;
            ToolOutput::structured(json!({CAPABILITY: {
                "version":1, "policy":"jev-evidence-v1", "command":command
            }}))
            .map_err(|e| ToolError::Internal(e.to_string()))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_arguments_without_accepting_completion() {
        assert!(
            GoalTool::Start
                .command(json!({"objective":"fix", "criteria":["tests pass"]}))
                .is_ok()
        );
        assert!(
            GoalTool::Start
                .command(json!({"objective":"fix", "criteria":[]}))
                .is_err()
        );
        assert!(GoalTool::Get.command(json!({"complete":true})).is_err());
        assert!(GoalTool::Pause.command(json!({})).is_ok());
        assert_eq!(
            GoalTool::Check
                .command(json!({"claim":"Tests passed."}))
                .unwrap(),
            json!({"operation":"check","claim":"Tests passed."})
        );
        assert!(
            GoalTool::Check
                .command(json!({"claim":"done","complete":true}))
                .is_err()
        );
        assert!(
            GoalTool::Check
                .command(json!({"claim":"x".repeat(8193)}))
                .is_err()
        );
    }

    #[tokio::test]
    async fn listing_and_direct_execution_require_free_chaos_v1() {
        let mut session = Session::new();
        let (sender, _receiver) = mcp_host::server::NotificationSender::bounded(16);
        let logger = mcp_host::logging::McpLogger::new(sender, "goal-test");
        for capability in [
            json!(null),
            json!({"version":1,"client":"codex"}),
            json!({"version":2,"client":"free_chaos"}),
            json!({"version":1,"client":"free_chaos"}),
            json!({"version":1,"client":"free_chaos","check":true}),
        ] {
            session.capabilities = Some(
                serde_json::from_value(json!({
                    "experimental":{CAPABILITY:capability}
                }))
                .expect("capabilities"),
            );
            for tool in [GoalTool::Get, GoalTool::Check] {
                let supported = capability["version"] == 1
                    && capability["client"] == "free_chaos"
                    && (!matches!(tool, GoalTool::Check) || capability["check"] == true);
                assert_eq!(
                    tool.is_visible(&VisibilityContext::new(&session)),
                    supported
                );
                let params = if matches!(tool, GoalTool::Check) {
                    json!({"claim":"done"})
                } else {
                    json!({})
                };
                let result = tool
                    .execute(ExecutionContext::new(params, &session, &logger))
                    .await;
                assert_eq!(result.is_ok(), supported);
            }
        }
    }
}
