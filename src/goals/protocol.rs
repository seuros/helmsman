use serde::{Deserialize, Serialize};

pub const VERSION: u32 = 2;
pub const MAX_CHECKS: u8 = 4;
pub const CAPABILITY: &str = "chaos/goals";
pub const URI: &str = "goal://current";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Active,
    Checking,
    Complete,
    Paused,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub id: String,
    pub turn_id: String,
    pub revision: u64,
    pub objective: String,
    pub criteria: Vec<String>,
    pub status: Status,
    pub checks: u8,
    pub reason: String,
    pub attempt_id: Option<String>,
    pub evidence_digest: Option<String>,
    pub verdict_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum Evaluation {
    Complete,
    Incomplete { missing: Vec<String> },
    Unknown { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerdictRecord {
    pub revision: u64,
    pub attempt_id: String,
    pub evidence_digest: String,
    pub reference: String,
    pub evaluation: Evaluation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Context {
    pub turn_id: String,
    pub eligible: bool,
    pub pause_reason: Option<String>,
    pub evidence_digest: String,
    pub has_evidence: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Restored {
    pub version: u32,
    pub goal: Option<Snapshot>,
    pub context: Context,
    pub interrupted: bool,
}

impl Restored {
    pub fn reconcile(&self) -> Option<Snapshot> {
        let mut goal = self.goal.clone()?;
        if matches!(goal.status, Status::Active | Status::Checking)
            || (goal.status == Status::Complete && self.interrupted)
        {
            let reason = if self.interrupted {
                Some("interrupted".to_string())
            } else if goal.turn_id != self.context.turn_id {
                Some("turn_ended".to_string())
            } else {
                self.context.pause_reason.clone()
            };
            if let Some(reason) = reason {
                goal.status = Status::Paused;
                goal.reason = reason;
                goal.revision += 1;
                return Some(goal);
            }
        } else if goal.status == Status::Complete
            && goal.turn_id == self.context.turn_id
            && self.context.eligible
            && goal.evidence_digest.as_deref() != Some(&self.context.evidence_digest)
        {
            goal.status = Status::Active;
            goal.reason = "evidence_changed_after_completion".into();
            goal.revision += 1;
            return Some(goal);
        }
        None
    }
}
