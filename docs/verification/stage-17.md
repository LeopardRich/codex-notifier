# Stage 17 Verification

Status: Branch verification pending

Date: 2026-07-30

Scope: read-only `doctor`, role-aware `status`, delivery-aware local/remote
`test`, matching human/JSON reports, stable health exit codes, fixed layered
remediation, and payload-free diagnostics.

## Implemented evidence

- Bare `doctor` loads configuration without a write probe and checks, in stable
  order, configuration, fixture-gated Codex CLI support, agent state,
  same-user IPC, SQLite state, and role-specific native notification or
  OpenSSH/target readiness.
- The IPC probe connects and validates peer identity without sending a frame.
  It cannot enqueue an event. The relay target probe invokes the same bounded
  fixed-argument OpenSSH process with empty stdin and accepts only the forced
  receiver's non-retryable `malformed_json` acknowledgement.
- SQLite exposes separate read-only snapshot and per-event APIs. They reject
  missing, symlinked, non-file, legacy/newer, corrupt, cross-table-duplicate,
  and invalid-timestamp state without creating, migrating, pruning, or
  repairing a database.
- `status` works for desktop and relay roles. It reports role, validated
  installation version, startup and agent state, queue depth, oldest queued
  age, receipt/dead-letter counts, latest successful delivery time, storage,
  notification, and focus metadata. It does not expose a configured profile.
- Early configuration, platform, or ownership failures use the same schema-v1
  status envelope instead of the generic operational stderr path.
- Human and compact JSON output are rendered from the same typed doctor,
  status, or test result. Every actionable result contains a stable code,
  documented exit code, and fixed remediation string.
- `test` accepts either event kind, submits through normal local IPC, and waits
  read-only for that event ID's receipt or metadata-only dead letter. A timeout
  leaves the durable event and normal retry policy unchanged.
- For explicit remote self-tests, `receive` waits up to three seconds for the
  desktop native-delivery receipt. It returns `delivered`, a retryable bounded
  pending rejection, or a permanent safe-code rejection. The relay therefore
  records self-test success only after the desktop native adapter accepted the
  notification.
- Focused `doctor codex` and `doctor ssh` remain compatible and gain optional
  JSON output. Comprehensive health checks never alter Codex hooks, startup
  resources, SSH files, notification permission, or state.

## Automated checks

- On Windows 10 22H2 with Rust 1.88 GNU, `cargo fmt --all -- --check`, strict
  all-target/all-feature workspace Clippy, and `cargo test --workspace` pass.
- The completed local suite contains 139 automated tests. Four tests that show
  real Windows Toasts or require temporary operating-system state remain
  intentionally ignored by the normal suite.
- Persistence contracts prove read-only inspection does not create a missing
  database or migrate a version-zero database. Snapshot tests cover counts,
  oldest/latest timestamps, event pending/delivered state, cross-table
  corruption, invalid timestamps, and unsafe database types.
- File-backed stores and read-only inspectors use a bounded 250 ms SQLite busy
  timeout. A contention contract holds a read transaction while a delivery
  acknowledgement waits, releases it after 50 ms, and proves the writer
  succeeds instead of terminating the agent worker.
- IPC contracts prove the health probe reaches a same-user listener without an
  event frame. SSH unit contracts freeze the empty-input acknowledgement and
  all existing bounded process/error behavior.
- Application unit tests cover notification and SSH fault classes, independent
  exit codes, fixed remediation, status precedence, human/JSON parity, unsafe
  version/profile redaction, and passive config loading that creates no state.
- Executable integration tests use real IPC and SQLite with delivery fakes for
  all four role/event combinations: desktop/relay and task-completion/approval.
  They also verify retryable pending timeout, permanent dead letter, relay
  status on Linux-compatible configuration, structured early failures,
  read-only behavior, and path/username/key/event-text redaction.

## Permanent CI additions

- The existing Ubuntu 22.04 real OpenSSH job now runs the empty-stdin receiver
  probe after starting the isolated `sshd` and proves it creates no delivery.
- The same job submits a `local-test` event through the real relay-role agent,
  system `ssh`, forced receiver, desktop-role agent, SQLite queues, and final
  recording notification adapter. The relay can acknowledge it only after the
  desktop delivery count advances.
- Existing Stage 15 forced command, PTY, shell, concatenation, and forwarding
  rejection checks and Stage 16 offline recovery/deduplication checks remain in
  the same job.
- Windows Session 0 and macOS 14/current no-Aqua diagnostics remain permanent
  matrix gates. Branch run identifiers will be recorded after they complete.

## Privacy and read-only audit

- Diagnostic models have no event title/body, source label, username, alias,
  key, raw SSH stderr, endpoint, or path field. Tests inject representative
  Windows/macOS paths, usernames, private-key markers, and event text and prove
  neither output format contains them.
- Only validated safe codes and a self-test UUIDv7 may reflect runtime state.
  Messages and remediation are fixed strings selected by typed status.
- `doctor` may execute `codex --version`, connect and close local IPC, query
  native settings, run non-mutating OpenSSH/key tools, and perform the empty
  forced-receiver exchange. It never runs an installer, authorization request,
  migration, queue transition, or repair action.

## Explicit limits

- Local Windows verification uses adapter fakes for automated self-tests and
  does not display a real Toast. The already ignored interactive native smoke
  tests remain the explicit manual path.
- The permanent remote diagnostic/self-test harness is Ubuntu loopback with a
  recording final adapter. Real remote delivery into Windows and macOS native
  APIs remains Stage 18 verification; no Linux desktop support is implied.
- A normal relay event still acknowledges durable desktop enqueue. Only the
  explicit synthetic `local-test` source waits for the native delivery receipt.
- `doctor` reports faults and remediation but never changes system state.

## Completion decision

- Stage 17 can be marked complete after the branch run passes all four jobs,
  these changes are fast-forwarded to `main`, and the permanent `main` run is
  green with the enhanced real OpenSSH path.
- Until those gates pass, this document records implemented and locally
  verified behavior rather than final Stage 17 completion.
