# Diagnostics, Status, and Self-Test

Stage 17 provides local-only, payload-free operational commands for desktop and
relay roles:

```text
codex-notifier doctor [--format human|json]
codex-notifier status [--format human|json]
codex-notifier test [task-completed|approval-requested] [--format human|json] [--wait-ms 100..180000]
```

The existing focused commands remain available:

```text
codex-notifier doctor codex --codex-version VERSION --interface <cli-hook|app-server> [--format human|json]
codex-notifier doctor ssh [--ssh-config ABSOLUTE_PATH] [--known-hosts ABSOLUTE_PATH] [--authorized-keys ABSOLUTE_PATH] [--format human|json]
```

## Read-only health checks

`doctor` never installs, repairs, migrates, creates, or removes configuration,
Codex hooks, startup resources, notification settings, SSH files, or SQLite
state. It performs these checks in stable order:

1. Configuration syntax, version, values, and role requirements.
2. Fixture-verified Codex CLI task-completion capability.
3. The bounded agent process record.
4. A same-user IPC connection and peer-identity check with no event frame.
5. State-directory metadata and a read-only current-schema SQLite snapshot.
6. Native permission/session readiness for a desktop role.
7. System OpenSSH client and strict host-key enrollment for a relay role.
8. Relay target authentication and reachability through an empty-stdin forced
   receiver exchange.

The SSH reachability exchange can only receive the fixed non-retryable
`malformed_json` acknowledgement. It cannot enqueue an event. Desktop roles
mark relay-only checks `skipped`; relay roles mark native-notification checks
`skipped`. The focused `doctor ssh` command remains the explicit receiver-key
permission check for a desktop that accepts remote events.

`status` is also read-only. It does not create or migrate an absent or legacy
database. It reports role, installation/startup state where applicable, agent
state, queue and metadata counts, oldest queued age, latest successful delivery
time, storage state, and native notification/focus state. A relay status works
without initializing a desktop installer or notification adapter.

## Output contract

Human output is line-oriented. JSON output is one compact object with
`schema_version = 1`. Both formats are rendered from the same typed result and
carry the same status, code, exit code, and fixed remediation. Output never
contains event title/body, source labels, usernames, host aliases, key
material, SSH diagnostics, configuration/database paths, or an unvalidated
profile/version string. `test` may report its generated UUIDv7 event ID and a
validated safe dead-letter code.

`doctor` reports every applicable check. Its process exit code is the first
failed check in the order above. `status` reports all fields and selects one
primary health code using installation, startup/agent, storage/dead-letter,
then notification precedence.

## Self-test behavior

`test` constructs generic private display text and submits it over the same
local IPC endpoint as Codex. It waits read-only for that event ID to reach a
delivery receipt or metadata-only dead letter:

- A desktop-role receipt means the native notification adapter accepted the
  notification.
- A relay-role receipt means the restricted desktop receiver observed the
  desktop native-delivery receipt and returned `delivered` through real SSH.
- A retryable desktop delay returns a bounded pending rejection, so the relay
  keeps its outbox event and retries normally.
- A permanent desktop failure propagates only its validated safe code and
  becomes a relay dead letter.

The default wait is 15 seconds for desktop and the configured SSH connection
timeout plus 10 seconds for relay. `--wait-ms` accepts 100 through 180,000
milliseconds. A command timeout does not delete or cancel the durable event;
normal retry policy continues.

## Exit codes

| Exit | Fault class | Typical repair layer |
| ---: | --- | --- |
| 0 | Ready or successful test delivery | None |
| 2 | Invalid command arguments | Correct the invocation |
| 3 | Incompatible Codex source payload | Use a verified adapter/version |
| 4 | Non-diagnostic operational failure | Inspect the emitted safe code |
| 10 | Configuration or desktop installation | Repair configuration/install |
| 11 | Codex missing or unsupported | Install a fixture-verified version |
| 12 | Startup registration or agent | Repair/start the per-user agent |
| 13 | Same-user IPC | Restart the correct user/profile agent |
| 14 | SQLite/storage or retained dead letters | Repair state; resolve delivery fault |
| 15 | Native notification permission/session | Repair OS notification access |
| 16 | System OpenSSH client | Install/enable the client |
| 17 | Strict SSH host-key verification | Verify and repin the host key |
| 18 | SSH public-key authentication | Repair the dedicated-key entry |
| 19 | SSH destination network | Repair routing/service/firewall |
| 20 | SSH connection timeout | Repair routing/service/firewall |
| 21 | Restricted receiver rejection | Run desktop doctor; repair receiver |
| 22 | SSH process/response contract | Align SSH and receiver versions/config |
| 23 | Self-test submission | Resolve the first doctor fault |
| 24 | Self-test wait expired | Inspect health; durable retry continues |
| 25 | Self-test dead letter | Resolve its safe delivery prerequisite |

Diagnostic messages and remediation text are fixed strings. Inspected values
select a typed code but are never interpolated into those strings.
