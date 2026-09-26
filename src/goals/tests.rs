use super::*;
use mcp_host::registry::tools::ToolRegistry;
use mcp_host::server::multiplexer::{ClientCapabilities, RequestMultiplexer};
use std::sync::atomic::{AtomicU8, Ordering};

fn session() -> Session {
    let mut session = Session::new();
    session.capabilities = Some(
        serde_json::from_value(json!({
            "experimental":{CAPABILITY:{"version":VERSION,"client":"free_chaos","durable":true}}
        }))
        .unwrap(),
    );
    session
}

fn empty() -> Restored {
    Restored {
        version: VERSION,
        goal: None,
        interrupted: false,
        context: Context {
            turn_id: "turn".into(),
            eligible: true,
            pause_reason: None,
            evidence_digest: "digest".into(),
            has_evidence: true,
        },
    }
}

fn goal(status: Status) -> Snapshot {
    Snapshot {
        id: "goal".into(),
        turn_id: "turn".into(),
        revision: 1,
        objective: "fix".into(),
        criteria: vec!["test passes".into()],
        status,
        checks: 2,
        reason: "test".into(),
        attempt_id: Some("attempt".into()),
        evidence_digest: Some("digest".into()),
        verdict_ref: None,
    }
}

struct Host {
    state: Arc<AsyncMutex<Restored>>,
    failure: Arc<AtomicU8>,
    evaluation: Arc<AsyncMutex<Evaluation>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Host {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn host(goals: &Goals, session: &Session) -> Host {
    let state = Arc::new(AsyncMutex::new(empty()));
    let failure = Arc::new(AtomicU8::new(0));
    let evaluation = Arc::new(AsyncMutex::new(Evaluation::Complete));
    let saved = Arc::clone(&state);
    let fault = Arc::clone(&failure);
    let verdict = Arc::clone(&evaluation);
    let mux = Arc::new(RequestMultiplexer::new());
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    goals.initialized(
        &session.id,
        ClientRequester::new(tx, Arc::clone(&mux), ClientCapabilities::default()),
    );
    let task = tokio::spawn(async move {
        while let Some(request) = rx.recv().await {
            let params = request.params.unwrap();
            assert_eq!(params["version"], VERSION);
            let mut state = saved.lock().await;
            let result = match params["operation"].as_str().unwrap() {
                "restore" => json!(state.clone()),
                "checkpoint" => {
                    let next: Snapshot = serde_json::from_value(params["goal"].clone()).unwrap();
                    assert_eq!(
                        params["expected_revision"],
                        state.goal.as_ref().map_or(0, |g| g.revision)
                    );
                    let failure = fault.swap(0, Ordering::SeqCst);
                    if failure != 1 {
                        state.goal = Some(next.clone());
                        state.interrupted = false;
                    }
                    if failure != 0 {
                        mux.route_response(&serde_json::from_value(json!({
                            "jsonrpc":"2.0","id":request.id,"error":{"code":-32603,"message":"unconfirmed"},
                        })).unwrap());
                        continue;
                    }
                    json!(next)
                }
                "evaluate" => {
                    let goal = state.goal.as_ref().unwrap();
                    assert_eq!(goal.status, Status::Checking);
                    assert!(goal.checks > 0);
                    assert_eq!(params["revision"], goal.revision);
                    json!(VerdictRecord {
                        revision: goal.revision,
                        attempt_id: goal.attempt_id.clone().unwrap(),
                        evidence_digest: goal.evidence_digest.clone().unwrap(),
                        reference: "verdict".into(),
                        evaluation: verdict.lock().await.clone(),
                    })
                }
                other => panic!("unexpected operation {other}"),
            };
            mux.route_response(
                &serde_json::from_value(json!({
                    "jsonrpc":"2.0","id":request.id,"result":result,
                }))
                .unwrap(),
            );
        }
    });
    Host {
        state,
        failure,
        evaluation,
        task,
    }
}

#[tokio::test]
async fn state_controls_listing_direct_calls_and_coalesced_notifications() {
    let mut session = session();
    let goals = Arc::new(Goals::default());
    let registry = ToolRegistry::new();
    for kind in Kind::ALL {
        registry.register(GoalTool(kind, Arc::clone(&goals)));
    }
    let (sender, mut receiver) = mcp_host::server::NotificationSender::bounded(16);
    session.set_notification_channel(sender.clone());
    let logger = mcp_host::logging::McpLogger::new(sender, "goals");
    let mut state = empty();
    for (status, expected) in [
        (None, [true, false, false]),
        (Some(Status::Active), [false, true, true]),
        (Some(Status::Checking), [false, false, true]),
        (Some(Status::Complete), [false, false, false]),
        (Some(Status::Paused), [false, false, false]),
    ] {
        state.goal = status.map(goal);
        publish(&session, &state);
        assert!(receiver.try_recv().is_ok() || status == Some(Status::Paused));
        assert!(receiver.try_recv().is_err());
        for ((kind, name), expected) in [
            (Kind::Start, "start_goal"),
            (Kind::Check, "check_goal"),
            (Kind::Pause, "pause_goal"),
        ]
        .into_iter()
        .zip(expected)
        {
            assert_eq!(
                GoalTool(kind, Arc::clone(&goals)).is_visible(&VisibilityContext::new(&session)),
                expected
            );
            if !expected {
                let result = registry
                    .call_for_session(
                        name,
                        json!({}),
                        None,
                        &session,
                        &logger,
                        &VisibilityContext::new(&session),
                        None,
                        None,
                    )
                    .await;
                assert!(matches!(result, Err(ToolError::NotFound(_))));
            }
        }
    }
}

#[tokio::test]
async fn verdict_transitions_preserve_the_consumed_budget() {
    for checks in [0, MAX_CHECKS - 1] {
        for (evaluation, expected, reason) in [
            (
                Evaluation::Complete,
                Status::Complete,
                "jev_evidence_check_passed",
            ),
            (
                Evaluation::Incomplete {
                    missing: vec!["tests".into()],
                },
                if checks + 1 < MAX_CHECKS {
                    Status::Active
                } else {
                    Status::Paused
                },
                if checks + 1 < MAX_CHECKS {
                    "unmet_checks: tests"
                } else {
                    "check_limit"
                },
            ),
            (
                Evaluation::Unknown {
                    reason: "backend_unavailable".into(),
                },
                Status::Paused,
                "backend_unavailable",
            ),
        ] {
            let session = session();
            let goals = Goals::default();
            let host = host(&goals, &session);
            let mut initial = goal(Status::Active);
            initial.checks = checks;
            initial.evidence_digest = None;
            host.state.lock().await.goal = Some(initial.clone());
            *host.evaluation.lock().await = evaluation;
            let (sender, _) = mcp_host::server::NotificationSender::bounded(16);
            let logger = mcp_host::logging::McpLogger::new(sender, "goals");
            goals
                .execute(
                    Kind::Check,
                    ExecutionContext::new(json!({"claim":"done"}), &session, &logger),
                )
                .await
                .unwrap();
            let stored = host.state.lock().await;
            let goal = stored.goal.as_ref().unwrap();
            assert_eq!(goal.status, expected);
            assert_eq!(goal.reason, reason);
            assert_eq!(goal.checks, checks + 1);
            assert_eq!(goal.revision, initial.revision + 2);
            assert_eq!(goal.verdict_ref.as_deref(), Some("verdict"));
            assert_eq!(super::state(&session).unwrap().goal.as_ref(), Some(goal));
        }
    }
}

#[tokio::test]
async fn failed_commit_does_not_publish_and_lost_ack_is_restorable() {
    for failure in [1, 2] {
        let session = session();
        let goals = Goals::default();
        let host = host(&goals, &session);
        let mut state = goals.refresh(&session).await.unwrap();
        host.failure.store(failure, Ordering::SeqCst);
        let requester = Goals::requester(&goals.client(&session.id)).await.unwrap();
        assert!(
            Goals::commit(&requester, &session, &mut state, goal(Status::Active))
                .await
                .is_err()
        );
        assert!(state.goal.is_none());
        assert!(super::state(&session).unwrap().goal.is_none());
        let restored = goals.refresh(&session).await.unwrap();
        assert_eq!(restored.goal.is_some(), failure == 2);
        assert_eq!(restored.goal, host.state.lock().await.goal);
    }
}

#[tokio::test]
async fn check_consumes_before_evaluation_and_completion_keeps_reply_open() {
    let session = session();
    let goals = Goals::default();
    let host = host(&goals, &session);
    let (sender, _) = mcp_host::server::NotificationSender::bounded(16);
    let logger = mcp_host::logging::McpLogger::new(sender, "goals");
    goals
        .execute(
            Kind::Start,
            ExecutionContext::new(
                json!({"objective":"fix","criteria":["tests pass"]}),
                &session,
                &logger,
            ),
        )
        .await
        .unwrap();
    let result = goals
        .execute(
            Kind::Check,
            ExecutionContext::new(json!({"claim":"tests passed"}), &session, &logger),
        )
        .await
        .unwrap();
    assert_eq!(result[CAPABILITY]["goal"]["status"], "complete");
    assert_eq!(result[CAPABILITY]["goal"]["checks"], 1);
    assert!(
        result[CAPABILITY]["guidance"]
            .as_str()
            .unwrap()
            .contains("turn remains open")
    );
    assert_eq!(host.state.lock().await.goal.as_ref().unwrap().checks, 1);
    assert!(
        goals
            .execute(
                Kind::Check,
                ExecutionContext::new(json!({"claim":"different words"}), &session, &logger,)
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn restart_pauses_interrupted_work_without_refunding_checks() {
    let session = session();
    let goals = Goals::default();
    let host = host(&goals, &session);
    {
        let mut state = host.state.lock().await;
        state.goal = Some(goal(Status::Checking));
        state.interrupted = true;
    }
    let state = goals.refresh(&session).await.unwrap();
    let goal = state.goal.as_ref().unwrap();
    assert_eq!(goal.status, Status::Paused);
    assert_eq!(goal.checks, 2);
    assert_eq!(goal.revision, 2);
    assert_eq!(goal.reason, "interrupted");
    assert!(!visible(Kind::Start, &state));
    assert!(!visible(Kind::Pause, &state));
    assert_eq!(host.state.lock().await.goal, state.goal);
}

#[test]
fn unsupported_clients_and_unrestored_sessions_have_no_goal_tools() {
    let goals = Arc::new(Goals::default());
    let mut session = session();
    assert!(
        !GoalTool(Kind::Start, Arc::clone(&goals)).is_visible(&VisibilityContext::new(&session))
    );
    publish(&session, &empty());
    for capability in [
        json!(null),
        json!({"version":1,"client":"free_chaos","check":true}),
        json!({"version":2,"client":"codex","durable":true}),
        json!({"version":2,"client":"free_chaos"}),
    ] {
        session.capabilities =
            Some(serde_json::from_value(json!({"experimental":{CAPABILITY:capability}})).unwrap());
        assert!(
            !GoalTool(Kind::Start, Arc::clone(&goals))
                .is_visible(&VisibilityContext::new(&session))
        );
        assert!(!GoalResource(Arc::clone(&goals)).is_visible(&VisibilityContext::new(&session)));
    }
}
