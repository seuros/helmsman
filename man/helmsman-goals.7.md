# helmsman-goals(7)

## NAME

helmsman-goals - Jev-evaluated goals for FreeChaOS

## SETUP

Connect Helmsman as an MCP server and configure Jev with `/reflex` in FreeChaOS.
Submit `/goal <request>`. The model rephrases the request without weakening its
scope, derives observable criteria, and calls `start_goal`. Bare `/goal` shows
usage. Pasted text, mentions, and attachments use normal message submission.
You can also explicitly request the gate:

> Use the goal gate to fix this regression. Require the focused regression test
> to pass and the affected package to compile. Call check_goal before replying.
> Pause if you need my input.

Helmsman owns the lifecycle, visibility, and status resource. FreeChaOS stores
durable checkpoints and owns evidence, credentials, and evaluation.
There is no driver database, scheduler, or automatic restart of work.

## TOOLS

| Tool | Arguments | Purpose |
|------|-----------|---------|
| `start_goal` | `objective`, `criteria` | Start one goal per user turn |
| `check_goal` | `claim` | Evaluate a proposed conclusion against host evidence |
| `pause_goal` | `{}` | Stop checking without claiming completion |

The objective must be nonblank and at most 4096 UTF-8 bytes. Supply 1–8 nonblank
criteria, each at most 1024 bytes. The claim may be empty; its limit is 8192 bytes.

Read `goal://current` for the latest durable status. It retains terminal results
and does not support subscriptions. `get_goal` is no longer a tool.

| State | Visible tools |
|-------|---------------|
| Eligible turn, not started | `start_goal` |
| Active | `check_goal`, `pause_goal` |
| Checking | `pause_goal` |
| Paused or complete in this turn | None |

Restoration precedes discovery and direct calls. Hidden tools cannot be called
directly. Visibility changes emit a coalesced `tools/list_changed` notification.

## LIFECYCLE

**work → check_goal → Jev verdict → model response**

Completion or pause does not end the turn. The model reports the result afterward;
success is a status event, not a warning. This does not authorize unrelated work.

If the agent skips `check_goal`, the stop boundary checks after Stop hooks and
returns the verdict for a wrap-up reply. The next stop does not repeat a terminal
check. New non-goal tool evidence reopens a passed goal without resetting its
budget. Goal-tool calls/results and `goal://current` reads are not evidence.

A goal belongs to one regular user turn. Starting again in that turn is rejected,
even after pausing. New user input, abort, an explicit Stop-hook stop, or turn
termination fences unfinished work. The next driver refresh checkpoints a pause.
Restarts preserve the goal definition, revisions, consumed checks, and terminal
results. Interrupted active/checking goals become paused before tools are exposed.
New or forked conversations do not inherit an actionable goal.

## EVALUATION

Jev assesses the original request/objective and every criterion. Each must receive
`pass` with probability >= 0.85 and confidence >= 0.8. Confident failures may
continue work. Unknown, missing evidence, evaluator failure, timeout, or stale
results pause the goal; paused does not mean complete.

- Four checks total, shared by explicit checks and stop fallback.
- At most three work continuations; terminal wrap-up spends no extra check.
- Unchanged normalized evidence cannot earn another check.
- Terminal checks are hidden; inspect the resource instead.
- Each check has a 30-second overall deadline and no HTTP retries.

The worker's answer is a claim, not evidence. Tool outputs remain untrusted,
worker-selected observations. Jev does not replace tests, review, permissions,
or sandbox enforcement and does not guarantee correctness.

## PROTOCOL

Helmsman exposes the tools only to clients advertising:

```json
{
  "experimental": {
    "chaos/goals": {"version": 2, "client": "free_chaos", "durable": true}
  }
}
```

`experimental` is MCP's custom-extension field, not a runtime mode.
This is capability negotiation, not authentication. Direct calls are gated too.
Older clients do not see goal tools or the status resource.

Tools advertise `_meta["chaos/goals"] = {"version":2}`. The driver makes
acknowledged server-to-client requests on `chaos/goals`:

```json
{
  "version": 2,
  "operation": "restore"
}
```

- `restore` returns the last committed snapshot, current turn context, and an
  interruption flag.
- `checkpoint` takes `expected_revision` and `goal`. The snapshot contains `id`,
  `turn_id`, `revision`, `objective`, `criteria`, `status`, `checks`, `reason`,
  `attempt_id`, `evidence_digest`, and `verdict_ref`.
- `evaluate` takes `revision` and `claim`. It requires a committed checking
  snapshot with a consumed attempt. The host records a verdict bound to that
  revision, attempt, and evidence before returning its reference.

The driver proposes → the host commits → the host acknowledges → the driver
publishes state and visibility. Queuing a write is not a commit. Missing storage
or an unconfirmed write fails closed; it never reports durable success.
An identical checkpoint retry is idempotent; conflicting or stale revisions fail.
After an uncertain acknowledgement, restore reconciles the committed state.

Conversation and driver identities come from the configured transport, not model
arguments. Checkpoints are structural journal entries, not model messages.
Retired connections and late verdicts are rejected. Starting requires the exact
arguments of an approved, discovered goal-tool call. Completion requires a
matching host-recorded Jev verdict; neither tool arguments nor a driver claim can
manufacture one. A conversation's goal remains bound to its original driver route.
Any compatible driver may implement this contract; its configured name is not
special.

Tool results and the resource include `next_action`: `work_then_check`, `wait_for_check`,
`respond`, `report_blocker`, or `none`. An assessment also returns `guidance`.
Checking cannot create, resume, or reset a goal.

## PRIVACY

Checks use the first configured Jev backend in name order, without remote
fallback. The payload contains the current user request, objective, criteria,
worker claim, and bounded tool text/arguments:

- User request: at most 16 KiB; larger requests cannot start a goal.
- Claim: at most 8 KiB.
- Tool evidence: at most 48 KiB / 24 observations, with per-field truncation.

No background file scan, reasoning, system/developer messages, images, or
credentials are added. Credentials never pass through Helmsman.
**Tool text and arguments may contain secrets.** Opt in only when the configured
endpoint may receive that material.

## SEE ALSO

[Helmsman setup](../README.md#quick-start)
