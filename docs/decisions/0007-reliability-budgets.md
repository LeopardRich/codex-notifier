# ADR-0007: Reliability and resource budgets

- Status: Accepted
- Date: 2026-07-30
- Owners: Project maintainers

## Context

The durable queue and bounded worker model need release gates that distinguish
normal at-least-once recovery from accidental loss, duplicate flooding, or
unbounded resource use. Stage 18 also needs fixed load and latency ceilings;
measuring an implementation without a prior pass/fail budget is not a useful
reliability decision.

## Decision

Use the following release-gating profile for one user-level agent:

- A supported `emit` child must parse one real source fixture, submit it, and
  exit after durable acknowledgement within 5 seconds.
- A 100-event sequential IPC burst must reach notification-adapter acceptance
  and durable receipts within 30 seconds.
- The four-worker agent must remain at or below 512 MiB resident memory in the
  test process during that burst.
- The SQLite main, WAL, and shared-memory files must total at most 8 MiB after
  retaining the 100 delivery receipts.
- Active delivery concurrency must never exceed the configured worker count.
- One initial delivery followed by 100 submissions of the same stable event ID
  must invoke the notification adapter exactly once.

These are conservative release ceilings, not expected steady-state values.
Queue count, payload bytes, event age, retry attempts, and retention continue
to use their existing configuration and protocol hard limits.

Committed enqueue is the durability boundary. A crash after enqueue or during
delivery leaves the event recoverable after lease expiry. A crash after the
native API accepts a notification but before the receipt transaction commits
also leaves the event recoverable, but can repeat the native notification.
The system does not claim exactly-once behavior across that external side
effect. Once a receipt commits, stable-ID retries do not call the native API.

## Alternatives

Exactly-once native display was rejected because SQLite and Windows/macOS
notification services do not share a transaction. Recording a receipt before
calling the native API would replace possible duplication with silent loss.
Unbounded soak tests were rejected as permanent CI gates because they are not
deterministic or proportional to this per-user tool.

## Consequences

The 30-second batch ceiling limits average sequential acceptance to 300 ms per
event while tolerating hosted Windows named-pipe scheduling and security-scan
variance; the separate 5-second hook ceiling remains the user-facing bound.
The normal retry path is flood-resistant and measurable. Operators can still
see a repeated notification after the narrow pre-acknowledgement crash window,
which is preferable to losing an attention event. The generous CI ceilings
catch large regressions without treating hosted-runner scheduling noise as a
product failure.

## Security Impact

The profile exercises only bounded canonical events and retains no raw Codex
payload in receipts. Resource ceilings limit denial-of-service amplification;
the existing queue-full acknowledgement remains retryable and payload-free.

## Compatibility Impact

No protocol, command, configuration, or platform support claim changes. These
budgets apply to protocol version 1 and may be superseded only by a later ADR.

## Verification

- Cross-platform integration tests run real child `emit`, local IPC, SQLite,
  the four-worker runtime, and a recording notification boundary.
- Persistence tests simulate the three crash windows by closing and reopening
  the same file-backed database around committed and leased states.
- The permanent Linux job runs both event kinds through system OpenSSH, the
  forced receiver, desktop IPC, SQLite, and its recording notification port.
- Interactive native verification remains platform-gated and is never inferred
  from a recording adapter or a headless diagnostic.
