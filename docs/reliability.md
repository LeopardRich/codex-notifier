# Reliability verification contract

Stage 18 verifies durability and bounded operation without expanding the
supported product scope. Linux remains relay-only; a recording adapter on
Linux is evidence for routing and acknowledgements, not a Linux desktop
notification implementation.

## Evidence levels

- **Automated full path** means real source parsing, process or adapter entry,
  IPC, SQLite, routing, and a recording final notification boundary.
- **Real native** means the supported operating-system API accepted the
  notification in an interactive user session.
- **Real remote** means a system OpenSSH client and server enforced the forced
  receiver and reached the destination agent.
- **Unverified** means the required operating system, permission, identity, or
  SSH server was unavailable. Component tests do not upgrade that state.

## Four-path matrix

| Path | Automated and protocol evidence | Real platform evidence | Stage 18 state |
| --- | --- | --- | --- |
| Windows local | Both Codex 0.144.5 source fixtures run through child `emit`, real Windows named-pipe IPC, SQLite, and the recording notification port. | After the expected pre-install missing-identity result, a reversible Stage 18 install registered the AUMID and ran both source payloads through the installed agent to WinRT and durable receipts. Stage 12 and Stage 14 retain the earlier native records. | Local source-to-native path verified on Windows 10. |
| macOS local | The same source/IPC/SQLite/recording contracts run in macOS 14 and current macOS CI. | Stage 13 and Stage 14 interactive Aqua runs accepted both UserNotifications event kinds and retained receipts. | Reliability path verified from permanent automation plus recorded interactive native evidence. |
| Remote to Windows | The Linux harness retains the platform-neutral contracts. | A reversible Windows 10 22H2 Session 8 run used system `ssh`, a temporary restricted Win32-OpenSSH 10.0p2 server, forced receive, the installed desktop agent, and WinRT. Both synthetic kinds and both sanitized Codex fixtures delivered; offline recovery succeeded and 100 stable-ID retries remained duplicates. | Continuous source-build engineering path verified; production-signed candidate rerun pending. |
| Remote to macOS | The Linux harness retains the platform-neutral contracts. | macOS 14 run [`30505508865`](https://github.com/LeopardRich/codex-notifier/actions/runs/30505508865) installed a temporarily trusted app and Aqua LaunchAgent, then used system `ssh` plus a restricted temporary `sshd`. Both synthetic kinds and both sanitized fixtures reached UserNotifications; relay and desktop each committed four receipts with no pending or dead-letter rows. | Continuous source-build engineering path verified; Developer ID/notarized candidate rerun pending. |

Both remote rows now have their own continuous destination-platform run; they
were not inferred by combining Linux loopback with separate local-native
evidence. They remain engineering evidence rather than release-candidate
evidence because neither desktop artifact carried its required production
identity, and the macOS artifact was not notarized.

## Recovery matrix

| Failure point | Durable state after restart | Expected behavior |
| --- | --- | --- |
| After enqueue commit | Queued event | Lease and deliver normally. |
| During delivery | Unexpired lease, then queued at expiry | Do not steal the live lease; retry after its exact expiry. |
| After native acceptance, before receipt commit | Unexpired lease, then queued at expiry | Retry without loss; a repeated native notification is possible. |
| After receipt commit | Receipt only | Return `duplicate`; do not call the native adapter again. |

The file-backed persistence contract closes and reopens the database across all
three crash windows. Cooperative shutdown additionally releases an in-flight
lease without consuming a retry attempt.

## Fault and load matrix

- One accepted event plus 100 stable-ID retries produces one recording-adapter
  invocation and one receipt.
- A one-entry queue rejects the next IPC submission with retryable safe code
  `agent_queue_full` while preserving the leased event.
- Offline OpenSSH delivery remains queued, wakes from durable backoff without a
  new submission, and deduplicates a resend after destination delivery.
- Unknown event schema versions and newer SQLite schema versions are rejected
  before delivery or migration.
- ADR-0007 fixes the 5-second hook, 30-second 100-event batch, 512 MiB RSS,
  8 MiB database, and configured-worker-count ceilings.

The automated baseline prints one payload-free line containing event count,
elapsed milliseconds, RSS bytes, and database bytes when run with
`--nocapture`. Timing covers notification-adapter acceptance and committed
receipts; it does not claim how quickly an operating system presents a banner
to a user.

## Verification commands

The reliability contracts are part of the normal workspace suite. The real
OpenSSH test is explicitly enabled only in its isolated Linux CI harness:

```text
cargo test -p codex-notifier --test agent_host_contract -- --nocapture
cargo test -p codex-notifier-persistence --test sqlite_contract
CODEX_NOTIFIER_OPENSSH_TEST=1 cargo test -p codex-notifier --test openssh_receive -- --ignored --exact real_forced_openssh_session_enforces_the_receive_boundary --nocapture
```

Real native tests require their documented interactive identity, authorization,
and cleanup procedures. A headless CI success is not a substitute.
