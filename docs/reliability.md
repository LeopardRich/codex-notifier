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
| Remote to Windows | The Linux harness proves relay SQLite, offline retry, system OpenSSH, forced receive, desktop IPC/SQLite, and both event kinds. | Windows local native delivery is verified separately. No real Windows OpenSSH server-to-native run has been executed. | End-to-end platform path unverified. |
| Remote to macOS | The Linux harness proves the same platform-neutral remote chain and both event kinds. | macOS local native delivery is verified separately. No real macOS OpenSSH server-to-native run has been executed. | End-to-end platform path unverified. |

The remote rows must remain unverified until one continuous run reaches the
real destination native API on that operating system. Linux loopback and local
native evidence cannot be combined into a claim that such a run occurred.

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
- ADR-0007 fixes the 5-second hook, 10-second 100-event batch, 512 MiB RSS,
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
