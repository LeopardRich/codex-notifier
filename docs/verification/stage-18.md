# Stage 18 Verification

Status: Complete

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
- The original Stage 18 pass found no installed Windows OpenSSH server and did
  not elevate to add one. The later supplemental run below used a verified
  portable OpenSSH distribution without installing a service or changing that
  historical result.

## Supplemental remote platform closure

- Remote-to-Windows was exercised on the same Windows 10 Pro 22H2 interactive
  Session 8 with the current optimized executable and a reversible desktop
  install. The official portable Win32-OpenSSH 10.0p2 Preview archive was
  verified at SHA-256
  `23f50f3458c4c5d0b12217c6a5ddfde0137210a30fa870e98b29827f7b43aba5`,
  and a non-service `sshd` listened only on `127.0.0.1:42222`.
- A dedicated `restrict` key, strict host-key checking, forced `receive`, and
  PTY/command rejection were exercised. Both synthetic self-test kinds and
  both sanitized Codex 0.144.5 fixtures continuously traversed relay
  IPC/SQLite, system `ssh`, forced receive, desktop IPC/SQLite, and WinRT.
  Relay receipts increased from two to four and desktop receipts from seven to
  nine for the fixture pair, with zero pending rows or dead letters.
- An event queued while `sshd` was offline delivered automatically after the
  daemon restarted. One hundred retries of a stable delivered ID all returned
  `duplicate`, and desktop receipts remained 11 to 11.
- Cleanup stopped both agents and `sshd`, removed all installed product,
  identity, startup, manifest, configuration, and temporary SSH resources, and
  restored the original Codex hooks and SSH configuration byte-for-byte.
  SQLite retained 11 receipts, zero pending rows, and zero dead letters.
- Remote-to-macOS was exercised by the macOS 14 Aqua job in run
  [`30505508865`](https://github.com/LeopardRich/codex-notifier/actions/runs/30505508865)
  on commit `421c1ab`. The job built the current optimized executable, signed
  its app with the existing temporary non-production trusted identity,
  installed it with its real LaunchAgent, and started a restricted temporary
  system `sshd` on `127.0.0.1:42223`.
- Both remote synthetic self-tests returned `route=remote` and
  `delivery=delivered`; both sanitized fixtures followed the same relay-to-
  UserNotifications path. Relay and desktop databases each committed four
  receipts with zero pending rows and zero dead letters, notification status
  remained `ready`, and the captured desktop showed the native notification.
  The relay stopped cooperatively, uninstall removed the app and LaunchAgent,
  the original SSH configuration was restored, and SQLite state was retained.
- These two runs close the missing continuous engineering paths. They do not
  satisfy the Stage 20 candidate matrix: Windows was unsigned, macOS used a
  temporary non-production identity rather than Apple Developer ID and was not
  notarized, and neither run used the exact protected candidate archives.

## Platform matrix and limits

The normative evidence levels and four-path matrix are in
[`../reliability.md`](../reliability.md). In particular:

- Windows local now has a continuous Stage 18 source-fixture-to-WinRT run.
  macOS local reliability is covered by permanent native platform automation
  plus its earlier interactive native records.
- Remote-to-Windows and remote-to-macOS now each have a continuous optimized
  source-build run to the real destination native API. The Linux OpenSSH
  harness remains shared transport evidence only and was not used to infer
  either platform result. Production-signed candidate reruns remain pending.
- The pre-acknowledgement crash window is at-least-once and may repeat a native
  notification. Exactly-once visible delivery is claimed only for retries made
  after the receipt committed.
- OS banner presentation latency cannot be observed through the notification
  API; the budget ends when the native or recording adapter accepts the event
  and SQLite commits its receipt.

## Branch CI

- Implementation-branch run
  [`30496487746`](https://github.com/LeopardRich/codex-notifier/actions/runs/30496487746)
  passed on commit `7ba28ff`: formatting, strict Clippy, and the complete normal
  suite were green on Windows, macOS 14, current macOS, and Ubuntu 22.04.
- The Windows job also passed the real Session 0 diagnostic. Both macOS jobs
  built the native smoke target and passed the signed no-Aqua diagnostic.
- The Ubuntu job installed the real OpenSSH server and passed the forced-command
  boundary, empty receiver probe, offline retry/recovery, remote self-test,
  duplicate acknowledgement, and both task-completion and approval-request
  desktop deliveries.
- Evidence-head run
  [`30496736306`](https://github.com/LeopardRich/codex-notifier/actions/runs/30496736306)
  exposed that the provisional 10-second batch ceiling was too narrow for
  hosted Windows variance: all 100 durable deliveries completed in 11,414 ms
  with 23,617,536 RSS bytes and a 65,536-byte database. macOS 14, current macOS,
  and the real OpenSSH job remained green. ADR-0007 now uses a conservative
  30-second batch ceiling, which still bounds average sequential acceptance to
  300 ms while preserving the independent 5-second per-hook limit.
- Corrected branch-head run
  [`30496978161`](https://github.com/LeopardRich/codex-notifier/actions/runs/30496978161)
  passed all four jobs on commit `39e93e5`, including the revised Windows load
  gate, Windows Session 0 diagnostic, both macOS no-Aqua diagnostics, and the
  real OpenSSH delivery/recovery matrix.
- Supplemental native-session run
  [`30505508865`](https://github.com/LeopardRich/codex-notifier/actions/runs/30505508865)
  passed the macOS 14 local authorization/banner, Focus suppression, continuous
  remote OpenSSH-to-UserNotifications, and reversible install/uninstall gates
  on commit `421c1ab`. Matching normal CI run
  [`30505508875`](https://github.com/LeopardRich/codex-notifier/actions/runs/30505508875)
  passed every permanent platform and package job on the same commit.
- The `actions/checkout@v4` Node.js 20 deprecation notice is a non-blocking
  upstream action warning; all project checks completed successfully.

## Completion decision

- The Windows/macOS local and continuous remote-native evidence,
  cross-platform reliability contracts, and real Linux OpenSSH harness satisfy
  the Stage 18 implementation scope without adding Linux desktop support or
  promoting engineering builds to release candidates.
- Crash recovery prefers no loss in the pre-acknowledgement window, stable-ID
  retries after a committed receipt invoke the final adapter only once, and
  capacity, compatibility, latency, memory, storage, retry, and worker bounds
  all have blocking tests.
- Stage 18 is permanently closed only after the evidence commit is green on the
  implementation branch, fast-forwarded to `main`, and permanent `main` CI is
  green. Stage 19 must not begin before that gate.
