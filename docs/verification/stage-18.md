# Stage 18 Verification

Status: In verification

Date: 2026-07-30

Scope: local and remote reliability matrix, three crash windows, duplicate
suppression, offline and capacity recovery, incompatible versions, and bounded
performance/resource baselines.

## Implemented evidence

- ADR-0007 freezes an agreed 100-event load and conservative hook, delivery,
  RSS, database, and worker-concurrency ceilings.
- A new desktop composition contract sends both sanitized real Codex 0.144.5
  payload shapes through the actual child executable, source adapters, local
  IPC, file-backed SQLite, four-worker runtime, and recording final adapter.
  It requires two durable receipts and no queued payload.
- The load contract sends 100 distinct events over real IPC, waits for 100
  adapter acceptances, shuts down, and requires 100 SQLite receipts. It checks
  the ADR latency, RSS, database, and worker ceilings and emits only aggregate
  measurements.
- A separate real-IPC contract commits one delivery, retries its stable ID 100
  times, receives 100 `duplicate` acknowledgements, and observes exactly one
  final adapter invocation.
- The one-entry queue contract blocks one leased delivery, receives a retryable
  `agent_queue_full` rejection for the next event, shuts down, and proves the
  original row remains durable.
- The persistence crash contract now names and exercises restart after enqueue,
  during an active delivery lease, and after external acceptance but before
  acknowledgement. The event remains unavailable until each exact lease expiry
  and is finally acknowledged once.
- Existing contracts continue to reject unknown event schema and newer SQLite
  schema versions, preserve incompatible databases without migration, recover
  offline relay retries, and bound attempt/age exhaustion.
- The permanent real OpenSSH harness now routes an approval request as well as
  task-completion and self-test events through relay IPC/SQLite, system `ssh`,
  forced receive, desktop IPC/SQLite, and the recording notification boundary.

## Local Windows results

- Host: Windows 10 Pro 22H2 build 19045, interactive Session 8, Codex CLI
  0.144.5, OpenSSH client 9.5p1.
- The focused persistence suite passed 19 tests. The focused executable suite
  passed 14 tests.
- Final local formatting and strict all-target/all-feature Clippy passed. The
  full workspace suite passed 143 automated tests; four interactive Windows
  native-state tests remain intentionally ignored by the normal suite.
- The 100-event run completed durable delivery in 853 ms, reported 22,753,280
  RSS bytes, and retained a 65,536-byte SQLite footprint.
- A fresh ignored WinRT smoke attempt first reported
  `application_identity_missing` and submitted no Toast, correctly proving the
  installation precondition. The audited profile had no installed executable,
  shortcut, AUMID, startup entry, manifest, or running agent.
- A reversible install then registered the product identity and started the
  production desktop agent with `notification=ready`. Both sanitized real
  Codex 0.144.5 source payload shapes ran through the installed `emit`, Windows
  named-pipe IPC, SQLite, the production runtime, and WinRT. Receipts increased
  from one preserved historical record to three, with zero pending events and
  zero dead letters.
- External uninstall removed the executable, shortcut, AUMID, configuration,
  manifest, agent process, and exact managed hook. The pre-existing hooks
  document contains no notifier entry and the SQLite database remains. No
  verification-only identity or startup resource was left installed.
- The Windows OpenSSH server capability is absent and installation requires
  elevation. Remote-to-Windows therefore remains explicitly unverified.

## Platform matrix and limits

The normative evidence levels and four-path matrix are in
[`../reliability.md`](../reliability.md). In particular:

- Windows local now has a continuous Stage 18 source-fixture-to-WinRT run.
  macOS local reliability is covered by permanent native platform automation
  plus its earlier interactive native records.
- Real remote-to-Windows and remote-to-macOS continuous native paths remain
  unverified. The Linux OpenSSH harness proves shared transport behavior but
  does not manufacture destination-platform evidence.
- The pre-acknowledgement crash window is at-least-once and may repeat a native
  notification. Exactly-once visible delivery is claimed only for retries made
  after the receipt committed.
- OS banner presentation latency cannot be observed through the notification
  API; the budget ends when the native or recording adapter accepts the event
  and SQLite commits its receipt.

## Pending merge gates

- Green implementation-branch CI on Windows, macOS 14, current macOS, and Linux
  relay with the expanded real OpenSSH harness.
- Fast-forward merge followed by green permanent `main` CI.

Stage 19 must not begin until all pending merge gates are complete.
