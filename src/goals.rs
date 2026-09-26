//! Driver lifecycle; the negotiated host journals and evaluates checkpoints.

mod protocol;
#[cfg(test)]
mod tests;

use mcp_host::protocol::errors::{ErrorType, McpError};
use mcp_host::registry::resources::{Resource, ResourceError, ResourceReadFuture};
use mcp_host::registry::tools::{Tool, ToolError, ToolFuture, ToolOutput};
use mcp_host::server::capability_hydration::{
    CapabilityHydrationContext, CapabilityHydrationFuture, CapabilityHydrator,
};
use mcp_host::server::multiplexer::ClientRequester;
use mcp_host::server::session::Session;
use mcp_host::server::visibility::{ExecutionContext, VisibilityContext};
use protocol::*;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{Mutex as AsyncMutex, watch};

#[derive(Clone, Copy)]
pub enum Kind {
    Start,
    Check,
    Pause,
}

impl Kind {
    pub const ALL: [Self; 3] = [Self::Start, Self::Check, Self::Pause];

    fn name(self) -> &'static str {
        match self {
            Self::Start => "start_goal",
            Self::Check => "check_goal",
            Self::Pause => "pause_goal",
        }
    }
}

pub struct GoalTool(pub Kind, pub Arc<Goals>);
pub struct GoalResource(pub Arc<Goals>);

const INFLIGHT: &str = "chaos/goals/inflight";

struct Attempt<'a>(&'a Session, String);

impl Drop for Attempt<'_> {
    fn drop(&mut self) {
        if self.0.get_state(INFLIGHT) == Some(json!(self.1)) {
            self.0.remove_state(INFLIGHT);
        }
    }
}

struct Client {
    requester: watch::Sender<Option<ClientRequester>>,
    serial: AsyncMutex<()>,
}

#[derive(Default)]
pub struct Goals {
    clients: Mutex<HashMap<String, Arc<Client>>>,
}

fn supported(session: &Session) -> bool {
    session
        .capabilities
        .as_ref()
        .and_then(|c| c.experimental.as_ref())
        .and_then(|c| c.get(CAPABILITY))
        .is_some_and(|c| {
            c["version"] == VERSION && c["client"] == "free_chaos" && c["durable"] == true
        })
}

fn state(session: &Session) -> Option<Restored> {
    session
        .get_state(CAPABILITY)
        .and_then(|v| serde_json::from_value(v).ok())
}

fn visible(kind: Kind, state: &Restored) -> bool {
    let context = &state.context;
    match kind {
        Kind::Start => {
            context.eligible
                && state
                    .goal
                    .as_ref()
                    .is_none_or(|g| g.turn_id != context.turn_id)
        }
        Kind::Check => state
            .goal
            .as_ref()
            .is_some_and(|g| g.turn_id == context.turn_id && g.status == Status::Active),
        Kind::Pause => state.goal.as_ref().is_some_and(|g| {
            g.turn_id == context.turn_id && matches!(g.status, Status::Active | Status::Checking)
        }),
    }
}

fn publish(session: &Session, state: &Restored) {
    session.set_state(CAPABILITY, json!(state));
    session.batch(|batch| {
        for kind in Kind::ALL {
            let name = kind.name();
            let hidden = session.is_tool_hidden(name);
            if visible(kind, state) {
                if hidden {
                    batch.unhide_tool(name);
                }
            } else if !hidden {
                batch.hide_tool(name);
            }
        }
    });
}

fn status(state: &Restored) -> Value {
    let next_action = match state.goal.as_ref().map(|g| g.status) {
        Some(Status::Active) => "work_then_check",
        Some(Status::Checking) => "wait_for_check",
        Some(Status::Complete) => "respond",
        Some(Status::Paused) => "report_blocker",
        None => "none",
    };
    json!({CAPABILITY:{
        "version":VERSION, "scope":"turn", "goal":state.goal, "next_action":next_action,
        "guidance": match next_action {
            "respond" => "The goal is complete, but the turn remains open. Respond to the user with the result and limitations.",
            "report_blocker" => "Completion is unverified. Report the blocker; do not restart this goal.",
            "work_then_check" => "Work within the original scope. Gather new evidence before checking. Pause if blocked.",
            _ => "Do not run a check alongside other work."
        }
    }})
}

impl Goals {
    fn client(&self, id: &str) -> Arc<Client> {
        Arc::clone(
            self.clients
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .entry(id.into())
                .or_insert_with(|| {
                    Arc::new(Client {
                        requester: watch::channel(None).0,
                        serial: AsyncMutex::new(()),
                    })
                }),
        )
    }

    pub fn initialized(&self, id: &str, requester: ClientRequester) {
        self.client(id).requester.send_replace(Some(requester));
    }

    async fn requester(client: &Client) -> Result<ClientRequester, String> {
        let mut receiver = client.requester.subscribe();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(requester) = receiver.borrow().clone() {
                    return Ok(requester);
                }
                receiver
                    .changed()
                    .await
                    .map_err(|_| "goal host disconnected".to_string())?;
            }
        })
        .await
        .map_err(|_| "goal host not initialized".to_string())?
    }

    async fn request(requester: &ClientRequester, mut value: Value) -> Result<Value, String> {
        value["version"] = json!(VERSION);
        requester
            .request_raw(CAPABILITY, Some(value), Some(Duration::from_secs(45)))
            .await
            .map_err(|_| "goal host request failed; completion is unverified".into())
    }

    async fn commit(
        requester: &ClientRequester,
        session: &Session,
        state: &mut Restored,
        goal: Snapshot,
    ) -> Result<(), String> {
        let expected = state.goal.as_ref().map_or(0, |g| g.revision);
        let ack: Snapshot = serde_json::from_value(
            Self::request(
                requester,
                json!({
                    "operation":"checkpoint", "expected_revision":expected, "goal":goal,
                }),
            )
            .await?,
        )
        .map_err(|_| "invalid goal acknowledgement")?;
        if ack != goal {
            return Err("goal acknowledgement mismatch".into());
        }
        state.goal = Some(ack);
        state.interrupted = false;
        publish(session, state);
        Ok(())
    }

    async fn restore(requester: &ClientRequester, session: &Session) -> Result<Restored, String> {
        let mut state: Restored =
            serde_json::from_value(Self::request(requester, json!({"operation":"restore"})).await?)
                .map_err(|_| "invalid goal restore")?;
        if state.version != VERSION {
            return Err("incompatible goal host".into());
        }
        if let Some(goal) = &state.goal
            && goal.status == Status::Checking
            && session.get_state(INFLIGHT) != Some(json!(goal.attempt_id))
        {
            state.interrupted = true;
        }
        if let Some(goal) = state.reconcile() {
            Self::commit(requester, session, &mut state, goal).await?;
        } else {
            publish(session, &state);
        }
        Ok(state)
    }

    async fn refresh(&self, session: &Session) -> Result<Restored, String> {
        let client = self.client(&session.id);
        let _serial = client.serial.lock().await;
        let requester = Self::requester(&client).await?;
        Self::restore(&requester, session).await
    }

    async fn execute(&self, kind: Kind, ctx: ExecutionContext<'_>) -> Result<Value, String> {
        let client = self.client(&ctx.session.id);
        let serial = client.serial.lock().await;
        let requester = Self::requester(&client).await?;
        let mut state = Self::restore(&requester, ctx.session).await?;
        if !visible(kind, &state) {
            return Err("goal operation is not available".into());
        }
        let revision = state.goal.as_ref().map_or(0, |g| g.revision) + 1;
        match kind {
            Kind::Start => {
                let args: Start = serde_json::from_value(ctx.params)
                    .map_err(|_| "expected objective and criteria")?;
                if args.objective.trim().is_empty()
                    || args.objective.len() > 4096
                    || args.criteria.is_empty()
                    || args.criteria.len() > 8
                    || args
                        .criteria
                        .iter()
                        .any(|s| s.trim().is_empty() || s.len() > 1024)
                {
                    return Err("invalid goal definition".into());
                }
                let goal = Snapshot {
                    id: uuid::Uuid::new_v4().to_string(),
                    turn_id: state.context.turn_id.clone(),
                    revision,
                    objective: args.objective,
                    criteria: args.criteria,
                    status: Status::Active,
                    checks: 0,
                    reason: "awaiting_checkpoint".into(),
                    attempt_id: None,
                    evidence_digest: None,
                    verdict_ref: None,
                };
                Self::commit(&requester, ctx.session, &mut state, goal).await?;
            }
            Kind::Pause => {
                if ctx.params != json!({}) {
                    return Err("expected no arguments".into());
                }
                let mut goal = state.goal.clone().ok_or("no goal")?;
                goal.revision = revision;
                goal.status = Status::Paused;
                goal.reason = "requested".into();
                Self::commit(&requester, ctx.session, &mut state, goal).await?;
            }
            Kind::Check => {
                let args: Check = serde_json::from_value(ctx.params)
                    .map_err(|_| "expected a claim, not evidence or a verdict")?;
                if args.claim.len() > 8192 {
                    return Err("claim too large".into());
                }
                let mut goal = state.goal.clone().ok_or("no goal")?;
                goal.revision = revision;
                let reason = if goal.checks >= MAX_CHECKS {
                    Some("check_limit")
                } else if !state.context.has_evidence {
                    Some("no_tool_evidence")
                } else if goal.evidence_digest.as_deref() == Some(&state.context.evidence_digest) {
                    Some("unchanged_evidence")
                } else {
                    None
                };
                if let Some(reason) = reason {
                    goal.status = Status::Paused;
                    goal.reason = reason.into();
                    Self::commit(&requester, ctx.session, &mut state, goal).await?;
                } else {
                    goal.status = Status::Checking;
                    goal.checks += 1;
                    goal.attempt_id = Some(uuid::Uuid::new_v4().to_string());
                    goal.evidence_digest = Some(state.context.evidence_digest.clone());
                    goal.verdict_ref = None;
                    goal.reason = "evaluating".into();
                    let attempt = goal.attempt_id.clone().ok_or("missing attempt")?;
                    ctx.session.set_state(INFLIGHT, json!(attempt));
                    let _attempt = Attempt(ctx.session, attempt);
                    Self::commit(&requester, ctx.session, &mut state, goal.clone()).await?;
                    drop(serial);
                    let result = Self::request(
                        &requester,
                        json!({
                            "operation":"evaluate", "revision":revision, "claim":args.claim,
                        }),
                    )
                    .await;
                    let _serial = client.serial.lock().await;
                    state = Self::restore(&requester, ctx.session).await?;
                    if state.goal.as_ref() != Some(&goal) {
                        return Ok(status(&state));
                    }
                    goal.revision += 1;
                    match result.and_then(|v| {
                        serde_json::from_value::<VerdictRecord>(v)
                            .map_err(|_| "invalid verdict".into())
                    }) {
                        Ok(verdict)
                            if verdict.revision == revision
                                && Some(&verdict.attempt_id) == goal.attempt_id.as_ref()
                                && Some(&verdict.evidence_digest)
                                    == goal.evidence_digest.as_ref() =>
                        {
                            goal.verdict_ref = Some(verdict.reference);
                            (goal.status, goal.reason) = match verdict.evaluation {
                                Evaluation::Complete => {
                                    (Status::Complete, "jev_evidence_check_passed".into())
                                }
                                Evaluation::Incomplete { missing } if goal.checks < MAX_CHECKS => (
                                    Status::Active,
                                    format!("unmet_checks: {}", missing.join(", ")),
                                ),
                                Evaluation::Incomplete { .. } => {
                                    (Status::Paused, "check_limit".into())
                                }
                                Evaluation::Unknown { reason } => (Status::Paused, reason),
                            };
                        }
                        _ => {
                            goal.status = Status::Paused;
                            goal.reason = "evaluation_unavailable".into();
                        }
                    }
                    Self::commit(&requester, ctx.session, &mut state, goal).await?;
                }
            }
        }
        Ok(status(&state))
    }
}

impl CapabilityHydrator for Goals {
    fn hydrate<'a>(&'a self, ctx: CapabilityHydrationContext) -> CapabilityHydrationFuture<'a> {
        Box::pin(async move {
            if !supported(&ctx.session) {
                return Ok(());
            }
            self.refresh(&ctx.session)
                .await
                .map(|_| ())
                .map_err(|error| {
                    McpError::builder(ErrorType::Internal, "goal_restore_failed")
                        .message(error)
                        .build()
                })
        })
    }
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

impl Tool for GoalTool {
    fn name(&self) -> &str {
        self.0.name()
    }
    fn description(&self) -> Option<&str> {
        Some(match self.0 {
            Kind::Start => {
                "Start a completion goal only at the user's request. Rephrase the objective without weakening scope and supply observable criteria. After working, call check_goal before responding. One start per turn, four checks, no new permissions."
            }
            Kind::Check => {
                "Ask Jev to evaluate this goal against host-recorded evidence. Supply a claim, not evidence. Do not run alongside other work. Completion closes the goal, not the turn; respond afterward. Shares four checks with the stop fallback."
            }
            Kind::Pause => {
                "Pause the active goal if blocked or stopped. Pausing is not completion and does not reset the budget."
            }
        })
    }
    fn input_schema(&self) -> Value {
        match self.0 {
            Kind::Start => json!({"type":"object","additionalProperties":false,"properties":{
                "objective":{"type":"string","minLength":1,"maxLength":4096},
                "criteria":{"type":"array","minItems":1,"maxItems":8,"items":{"type":"string","minLength":1,"maxLength":1024}}
            },"required":["objective","criteria"]}),
            Kind::Check => json!({"type":"object","additionalProperties":false,"properties":{
                "claim":{"type":"string","maxLength":8192}},"required":["claim"]}),
            Kind::Pause => json!({"type":"object","properties":{},"additionalProperties":false}),
        }
    }
    fn annotations(&self) -> Option<mcp_host::protocol::types::ToolAnnotations> {
        Some(mcp_host::protocol::types::ToolAnnotations {
            title: None,
            read_only_hint: Some(false),
            destructive_hint: Some(false),
            idempotent_hint: Some(matches!(self.0, Kind::Pause)),
            open_world_hint: Some(!matches!(self.0, Kind::Pause)),
        })
    }
    fn meta(&self) -> Option<Map<String, Value>> {
        Some(Map::from_iter([(
            CAPABILITY.into(),
            json!({"version":VERSION}),
        )]))
    }
    fn is_visible(&self, ctx: &VisibilityContext<'_>) -> bool {
        supported(ctx.session) && state(ctx.session).is_some_and(|s| visible(self.0, &s))
    }
    fn execute<'a>(&'a self, ctx: ExecutionContext<'a>) -> ToolFuture<'a> {
        Box::pin(async move {
            if !self.is_visible(&VisibilityContext::new(ctx.session)) {
                return Err(ToolError::NotFound(self.name().into()));
            }
            let result = self
                .1
                .execute(self.0, ctx)
                .await
                .map_err(ToolError::Internal)?;
            ToolOutput::structured(result).map_err(|e| ToolError::Internal(e.to_string()))
        })
    }
}

impl Resource for GoalResource {
    fn uri(&self) -> &str {
        URI
    }
    fn name(&self) -> &str {
        "current_goal"
    }
    fn description(&self) -> Option<&str> {
        Some("Last durable goal state. Paused goals never resume automatically.")
    }
    fn mime_type(&self) -> Option<&str> {
        Some("application/json")
    }
    fn is_visible(&self, ctx: &VisibilityContext<'_>) -> bool {
        supported(ctx.session)
    }
    fn read<'a>(&'a self, ctx: ExecutionContext<'a>) -> ResourceReadFuture<'a> {
        Box::pin(async move {
            if !supported(ctx.session) {
                return Err(ResourceError::NotFound(URI.into()));
            }
            let state = self
                .0
                .refresh(ctx.session)
                .await
                .map_err(ResourceError::Internal)?;
            Ok(vec![self.text_content(&status(&state).to_string())])
        })
    }
}
