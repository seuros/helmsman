pub use mcp_goals::{
    CAPABILITY, Evaluation, GoalLifecycleEvent, MAX_CHECKS, Restored, Snapshot, Status, VERSION,
    VerdictRecord,
};

pub const URI: &str = "goal://current";

pub fn advance(
    goal: &mut Snapshot,
    event: GoalLifecycleEvent,
    reason: impl Into<String>,
) -> Result<(), String> {
    goal.status = goal.status.next(event).ok_or("illegal goal transition")?;
    goal.reason = reason.into();
    Ok(())
}

pub fn reconcile(state: &Restored) -> Option<Snapshot> {
    let mut goal = state.goal.clone()?;
    let context = &state.context;
    let resumable = matches!(goal.status, Status::Active | Status::Checking)
        || (goal.status == Status::Complete && state.interrupted);
    let (event, reason) = if resumable {
        let reason = if state.interrupted {
            "interrupted".to_string()
        } else if goal.turn_id != context.turn_id {
            "turn_ended".to_string()
        } else {
            context.pause_reason.clone()?
        };
        (GoalLifecycleEvent::Pause(true), reason)
    } else {
        (
            GoalLifecycleEvent::Reopen(
                goal.turn_id == context.turn_id
                    && context.eligible
                    && goal.evidence_digest.as_deref() != Some(&context.evidence_digest),
            ),
            "evidence_changed_after_completion".to_string(),
        )
    };
    advance(&mut goal, event, reason).ok()?;
    goal.revision += 1;
    Some(goal)
}
