# Stage 16 Verification

Status: Complete

Date: 2026-07-30

Scope: relay-role system OpenSSH process delivery, bounded acknowledgement
handling, durable exponential retry with random jitter, attempt exhaustion,
offline recovery, and at-least-once desktop deduplication.

## Implemented evidence

- `role = "relay"` now composes `OpenSshDelivery` and never initializes a
  native notification API. Desktop composition remains independent of SSH.
- `OpenSshConfig` accepts only a bounded validated host alias, a 100 ms to
  120-second connection timeout, and an optional absolute client-config path.
  The application uses the normal system configuration; the explicit path
  exists for isolated deployments and verification.
- Each attempt starts the system `ssh` executable with an argument array. The
  adapter forces batch mode, no password prompts, no PTY, no agent/configured
  forwarding, one connection attempt, strict host-key checking, the configured
  alias, and exact command `codex-notifier receive`. No event value enters the
  executable, argv, path, alias, command, or environment.
- The compact canonical event is written only through child stdin. Stdout is
  capped at the protocol's 2,048-byte acknowledgement limit and stderr at
  8 KiB. Overflow kills the child. Captured diagnostics are classified and
  discarded; they are never returned through delivery errors or logs.
- The total child lifetime is bounded by the configured connection timeout plus
  a fixed five-second allowance. Cooperative agent shutdown kills the child,
  releases the lease, and reverses the attempt instead of consuming retry
  budget.
- Relay SQLite leases use the connection timeout plus a ten-second allowance,
  so the maximum 125-second child lifetime remains exclusively leased for 130
  seconds. Age maintenance cannot remove a still-active unexpired lease.
- Successful exit still requires one fully validated protocol-v1
  acknowledgement with the same event ID. `accepted`, `duplicate`, and
  `delivered` acknowledge the relay outbox. `rejected` retains only a validated
  safe code and retry flag; its message is not persisted.
- Fixed failure classifications distinguish executable absence, connection
  timeout, network unavailability, authentication failure, host-key failure,
  generic process failure, oversized output, malformed/mismatched
  acknowledgement, and remote rejection. Network/timeout/process conditions
  and retryable remote rejection are transient. Authentication, host key,
  invalid acknowledgement, overflow, and permanent remote rejection are dead
  letters.
- Relay configuration adds `retry_initial_delay_ms`, `retry_max_delay_ms`, and
  `retry_max_attempts`. Defaults are 1,000 ms, 60,000 ms, and 20 attempts;
  validated hard bounds are 100-60,000 ms, 100-3,600,000 ms, and 1-1,000.
- The retry base doubles per consumed attempt and caps before random jitter is
  selected from 75-100 percent of the base. SQLite records `available_at_ms`.
  The queue exposes its next availability time so workers wake at a delayed
  retry or expired lease without requiring a new IPC submission.
- Leases retain their one-based attempt number. A retry transition tells the
  runtime whether it was scheduled or already exhausted into `retry_exhausted`,
  so reports do not miscount exhausted work as a live retry.

## Automated checks

- On Windows 10 22H2 with Rust 1.88 GNU, formatter, warnings-as-errors
  all-target/all-feature Clippy, and the workspace suite pass. The completed
  Stage 16 suite contains 121 automated tests; four interactive Windows native
  notification tests remain intentionally ignored by the normal suite.
- SSH transport unit tests freeze every process argument and prove a
  metacharacter-heavy event is absent from argv. They cover alias/timeout/path
  bounds; distinct timeout, disconnected-network, authentication, changed-host
  key, and generic process classifications; transient/permanent policy; all
  successful acknowledgement statuses; retryable remote rejection; malformed
  JSON; and mismatched IDs.
- Configuration tests cover default, layered, inclusive, inverted, zero, and
  excessive retry settings. The redacted configuration summary still excludes
  host aliases and paths.
- SQLite tests cover earliest wake time for an empty queue, immediate work, a
  leased recovery boundary, a future retry, and an earlier age limit. They also
  prove age maintenance cannot delete an active unexpired lease. Existing tests
  continue to prove transactionally durable enqueue, restart recovery, exact
  attempt exhaustion, metadata-only dead letters, and cross-table
  deduplication.
- Agent-host tests inject a retryable transport failure, verify the random
  75-100 ms retry wakes without another submission, then acknowledge the same
  durable row. A second test holds a permanent transient condition through the
  configured two-attempt limit and records exactly one dead letter, one live
  scheduled retry, and no delivery. A third stops the first agent after a
  future retry is committed, reopens the same SQLite state, and delivers it
  from a new agent without another submission. A separate unit test freezes the
  lease formula and proves the maximum 125-second SSH process lifetime is
  covered by a 130-second durable lease.
- Final code branch CI run
  [`30490753397`](https://github.com/LeopardRich/codex-notifier/actions/runs/30490753397)
  passed formatting, strict Clippy, normal tests, the Windows Session 0 check,
  both macOS no-Aqua checks, and the enhanced Ubuntu OpenSSH integration job on
  commit `bae9962`.

## Real OpenSSH recovery verification

- The permanent Ubuntu 22.04 job uses the Stage 15 loopback harness with the
  real system `ssh` and `sshd`, fresh Ed25519 host/client keys, an isolated
  pinned `known_hosts`, and
  `restrict,command="<fixed-binary> receive"`.
- A real desktop-role `AgentHost` uses Unix IPC, SQLite receipts, worker
  delivery, and a recording final notification fake. A separate real
  relay-role `AgentHost` uses its own Unix IPC and SQLite outbox plus the
  production `OpenSshDelivery` adapter.
- The relay accepts a metacharacter-heavy event over real local IPC while
  `sshd` is offline. Its first real system-SSH attempt fails, remains queued,
  and schedules a bounded retry. Starting `sshd` later causes the worker to wake
  from the stored availability time and deliver without another submission.
- The receiver returns the matching acknowledgement; the relay commits one
  delivery receipt and removes its payload. The completed relay report proves
  at least one retry, exactly one delivered event, and no dead letter.
- Sending the same stable event ID again through a separate real SSH process
  returns `duplicate`. The desktop recording adapter remains at one visible
  delivery for that event, covering the acknowledgement-loss/at-least-once
  resend case.
- The original Stage 15 direct valid request and shell, PTY, concatenation,
  local-forwarding, and remote-forwarding rejection matrix still run in the
  same job. The added relay path does not weaken the forced-command boundary.

## Explicit limits

- The permanent real server harness is Ubuntu loopback. It verifies the shared
  system OpenSSH protocol and relay client implementation, but does not claim a
  Linux desktop notification path.
- Windows and macOS compile and run all platform-independent sender tests, but
  their real OpenSSH server setup, dedicated-key restrictions, and remote native
  notification paths remain explicitly unverified until Stage 18 evidence is
  available.
- Authentication, host-key-change, timeout, and receiver-rejection decisions
  have deterministic automated classification tests. The real integration
  exercises the disconnected-network recovery path and real accepted/duplicate
  acknowledgements; it does not modify a developer's SSH files or external
  network.
- Source-built relay operation is implemented. Signed release archives,
  systemd user-service installation, upgrades, and uninstall verification are
  Stage 19 work.

## Completion decision

- A remote role can durably accept an event, invoke system OpenSSH without
  event-derived arguments, survive temporary SSH unavailability, automatically
  retry with bounded jittered backoff, validate the destination
  acknowledgement, and preserve at-least-once delivery while the desktop
  displays the stable event ID only once.
- Permanent trust/configuration failures stop retrying, attempt/age/capacity
  bounds prevent indefinite resource use, and normal errors retain no payload,
  key, alias, path, username, or raw SSH diagnostic.
- Stage 16 is complete only after these changes are merged to `main` and the
  permanent four-platform CI run is green there. Stage 17 must not begin before
  that gate.
