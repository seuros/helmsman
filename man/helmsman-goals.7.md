# helmsman-goals(7)

## NAME

helmsman-goals - Jev-evaluated goals for FreeChaOS

## SETUP

Connect Helmsman as an MCP server and configure Jev with `/reflex` in FreeChaOS.
Explicitly ask the agent to start a goal with observable acceptance criteria:

> Use the goal gate to fix this regression. Require the focused regression test
> to pass and the affected package to compile. Call check_goal before replying.
> Pause if you need my input.

Helmsman is stateless. FreeChaOS owns the goal, evidence, credentials, and verdict.
There is no `/goal` slash command, durable resume, scheduler, or idle wake-up.

## TOOLS

| Tool | Arguments | Purpose |
|------|-----------|---------|
| `start_goal` | `objective`, `criteria` | Start one goal per user turn |
| `get_goal` | `{}` | Read host-owned status |
| `check_goal` | `claim` | Evaluate a proposed conclusion against host evidence |
| `pause_goal` | `{}` | Stop checking without claiming completion |

The objective must be nonblank and at most 4096 UTF-8 bytes. Supply 1–8 nonblank
criteria, each at most 1024 bytes. The claim may be empty; its limit is 8192 bytes.

## LIFECYCLE

**work → check_goal → Jev verdict → model response**

Completion or pause does not end the turn. The model reports the result afterward;
success is a status event, not a warning. This does not authorize unrelated work.

If the agent skips `check_goal`, the stop boundary checks after Stop hooks and
returns the verdict for a wrap-up reply. The next stop does not repeat a terminal
check. New non-goal tool evidence reopens a passed goal without resetting its
budget. Goal-tool calls and results are not evidence.

A goal belongs to one regular user turn. Starting again in that turn is rejected,
even after pausing. New user input, abort, an explicit Stop-hook stop, or turn
termination pauses unfinished work. `get_goal` can read the last in-memory status
in later turns; restarting the harness loses it.

## EVALUATION

Jev assesses the original request/objective and every criterion. Each must receive
`pass` with probability >= 0.85 and confidence >= 0.8. Confident failures may
continue work. Unknown, missing evidence, evaluator failure, timeout, or stale
results pause the goal; paused does not mean complete.

- Four checks total, shared by explicit checks and stop fallback.
- At most three work continuations; terminal wrap-up spends no extra check.
- Unchanged normalized evidence cannot earn another check.
- Terminal checks return stored status; concurrent checks return `wait_for_check`.
- Each check has a 30-second overall deadline and no HTTP retries.

The worker's answer is a claim, not evidence. Tool outputs remain untrusted,
worker-selected observations. Jev does not replace tests, review, permissions,
or sandbox enforcement and does not guarantee correctness.

## PROTOCOL

Helmsman exposes the tools only to clients advertising:

```json
{
  "experimental": {
    "chaos/goals": {"version": 1, "client": "free_chaos", "check": true}
  }
}
```

`experimental` is MCP's custom-extension field, not a runtime mode.
This is capability negotiation, not authentication. Direct calls are gated too.
Older v1 clients without `"check":true` see only start, get, and pause.

Tools advertise `_meta["chaos/goals"] = {"version":1}`. Successful driver calls
return `structuredContent["chaos/goals"]` with a command:

```json
{
  "version": 1,
  "policy": "jev-evidence-v1",
  "command": {
    "operation": "start",
    "objective": "Fix the regression",
    "criteria": ["The focused regression test passes"]
  }
}
```

`get` and `pause` have no other fields. `check` requires a `claim` string.
There is no worker-settable `complete` operation.

The host accepts only marked, discovered routes with the exact version, policy,
tool name, and arguments. It replaces the command with host status before
journaling. Any driver can implement the contract; its configured name is not
special. The host supplies evidence, resolves credentials, calls Jev, and fences
the verdict to the matching turn, revision, and evidence digest.

Status includes `checking` and `next_action`: `work_then_check`, `wait_for_check`,
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
