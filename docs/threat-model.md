# Threat Model

- Status: Accepted for protocol version 1
- Review date: 2026-07-30
- Owners: Project maintainers
- Related decisions: ADR-0001 through ADR-0007

## Scope and invariants

This model covers Codex ingestion, local IPC, SQLite state, direct OpenSSH
transport, structured logs, Windows/macOS notification APIs, and per-user
installation. It covers local and remote delivery by the same OS user. It does
not claim to protect a host after administrator/root compromise or a user's
unlocked Codex session from that user.

Security invariants:

1. Untrusted event bytes are validated before persistence or a native API.
2. Event content never becomes a command, argument, address, URL, or path.
3. Only the current OS user can access local IPC and state by default.
4. Remote senders are authenticated by a dedicated SSH key and receivers by a
   pinned host key.
5. Normal logs and diagnostics never contain event bodies or credentials.
6. Notifications are display-only: no approval button, HTTP port, embedded SSH
   server, or remote-control action exists in version 1.
7. Every queue, frame, string, retry, worker pool, log file, and retention
   period has a hard upper bound.

## Assets

| Asset | Security property |
| --- | --- |
| User attention | Notifications are authentic, deduplicated, and not flooded. |
| Event envelope | Integrity, bounded confidentiality, correct source/interface mapping. |
| Delivery state | Queue and receipts survive crashes without silent loss or replay display. |
| SSH credentials | Private keys remain outside project config, logs, and payloads. |
| Host identity | Relay connects only to the enrolled desktop host. |
| Local endpoints | Other users cannot submit events or impersonate the agent. |
| Configuration ownership | Install/uninstall changes only artifacts owned by this tool. |
| Diagnostic output | Actionable status without usernames, paths, keys, or event text. |

## Actors and assumptions

- The legitimate user owns the Codex session, desktop agent, and configured
  relay relationship.
- A malicious local process may run as another OS user and probe IPC or state.
- A same-user malicious process can access many user resources; this project
  limits accidental exposure but cannot establish a security boundary against
  full same-user compromise.
- A compromised relay may send arbitrary, repeated, malformed, or stale data.
- A network attacker may observe, delay, replay, or redirect SSH connections.
- A malicious event source may include oversized JSON, control characters,
  commands, paths, forged IDs, timestamps, or extension fields.
- A supply-chain attacker may tamper with dependencies, CI, installers, or
  release artifacts.

## Trust boundaries

| Boundary | Untrusted side | Trusted side | Principal controls |
| --- | --- | --- | --- |
| Codex ingestion | Version-specific raw hook/app-server payload | Source adapter and canonical event | Exact version/interface adapter, allowlist mapping, fixture contract |
| Local IPC | Local client bytes and endpoint namespace | Per-user agent | OS ACL/peer identity, bounded framing, timeout, rate limit |
| SQLite | Filesystem and persisted bytes | Queue/deduplication logic | User-only directory, schema version, transactions, integrity checks |
| SSH | Relay stdin and network | Restricted desktop `receive` | Key auth, forced command, host-key pinning, one bounded envelope |
| Logs | Event/error values | Structured sinks and diagnostics | Typed fields, redaction, escaping, rotation and retention |
| Native notification | Canonical display fields and OS session state | Windows/macOS adapter | Privacy mapping, escaping, truncation, permission diagnostics |
| Installer | Existing Codex/SSH/startup configuration and release archive | Owned per-user artifacts | Signature/checksum, ownership manifest, atomic update and rollback |

## Boundary analysis

### Codex ingestion

Threats include version drift, forged event kinds, prompt/model leakage,
unbounded fields, transcript-path reliance, and a hook that blocks Codex.

Mitigations:

- Select adapters by exact Codex version and interface from the compatibility
  matrix; unsupported combinations fail closed.
- Map only documented fields required for the canonical event. Discard prompts,
  assistant output, commands, environment values, raw transcript paths, and
  absolute working directories.
- Bound stdin before parsing and return quickly after local submission.
- Never fall back to terminal scraping or private session-log parsing.
- Keep sanitized real fixtures and contract tests for every Supported row.

Residual risk: a future Codex release can change behavior without changing a
parseable field shape. Compatibility must be revalidated before support expands.

### Local IPC

Threats include cross-user submission, endpoint squatting, stale socket/pipe
hijacking, frame truncation, slow senders, proxy-variable interference, and
connection floods.

Mitigations:

- Windows named pipes use a current-user security descriptor; Unix sockets use
  a user-only directory and mode with peer-credential checks where available.
- Endpoint names derive from a bounded profile identifier, never event text.
- The agent atomically owns a single-instance lock and validates/removes stale
  endpoints only after ownership checks.
- Length-prefixed frames are bounded before allocation; reads and writes have
  deadlines and bounded concurrent connections.
- Local IPC ignores HTTP, HTTPS, and SOCKS proxy variables.

Residual risk: a fully compromised same-user process can submit plausible
events. Deduplication and rate limits contain impact but cannot authenticate a
process beyond the OS user without another secret.

### SQLite state

Threats include queue tampering, replay, symlink/path substitution, schema
downgrade, partial writes, lock starvation, disk exhaustion, and payload
disclosure from backups.

Mitigations:

- Resolve state paths from platform rules, create user-only directories, and
  reject unexpected ownership, file type, or non-writable state roots.
- Use transactions for enqueue, lease, acknowledgement, retry, deduplication,
  dead-letter transitions, and migrations.
- Validate every row read from storage and bind SQL parameters.
- Enforce queue count, payload bytes, retry age, receipt retention, and database
  maintenance limits. Never store SSH private keys or raw Codex payloads.
- Check schema versions and database integrity; migration failure stops startup
  without deleting or rewriting the source database.

Residual risk: SQLite is not encrypted at rest. An administrator, the same OS
user, or a backup tool can read canonical display text. Private notification
defaults minimize its sensitivity but do not provide storage encryption.

### SSH transport

Threats include relay impersonation, desktop impersonation, stolen keys,
command injection, extra arguments, multiple concatenated events, replay,
forwarding abuse, PTY/shell access, and denial of service.

Mitigations:

- Use system OpenSSH with a parameter array and a configured alias; the event
  is sent only through stdin.
- Pin or explicitly enroll host keys. Never use a disabled host-key policy.
- Use a dedicated key restricted to the exact forced command with PTY, shell,
  agent forwarding, and port forwarding disabled.
- `receive` accepts no event-derived arguments and exactly one 16 KiB envelope,
  then closes. It returns a bounded acknowledgement without echoing input.
- UUIDv7 receipts deduplicate at-least-once delivery. Rate, retry age, attempt
  count, and dead-letter retention are bounded.

Residual risk: a compromised authorized relay can create valid notification
spam up to configured limits. Key revocation and rate limiting are required
operational responses.

### Logs and diagnostics

Threats include prompt/body leakage, forged log records through newlines or
terminal escapes, credential leakage, path disclosure, and unbounded disk use.

Mitigations:

- Emit typed structured records containing event ID, kind, state, duration,
  correlation ID, and safe error code only. Display title/body and raw payload
  are never log fields at any level.
- Encode control characters through the structured serializer; diagnostic
  messages are fixed templates rather than interpolated payloads.
- Rotate by size, keep a bounded count/age, and set user-only permissions.
- Machine-readable and human-readable diagnostics share the same redacted
  status model. Debug logging cannot disable redaction.

Residual risk: event IDs and timing can reveal activity patterns to someone
who can read the user's logs.

### Native notifications

Threats include lock-screen disclosure, XML/string injection, control-character
spoofing, overlong text, notification floods, disabled permissions, and a fake
approval action.

Mitigations:

- Apply ADR-0003 private text by default. Public mode remains allowlisted and
  excludes commands, prompts, model output, credentials, paths, and URLs.
- Pass data through native structured APIs, escape platform markup, strip
  controls, normalize Unicode, and truncate at canonical limits.
- Provide no action, reply, deep link, or approval URL. Deduplicate before the
  native call and bound delivery rate.
- Diagnose permission denied, focus mode, missing app identity, and non-GUI
  sessions without claiming a notification was seen.

Residual risk: the OS and third-party screen capture or notification-sync
features control final presentation. The application cannot guarantee secrecy
after handing private generic text to the OS.

### Installer and release artifacts

Threats include archive replacement, malicious dependencies, hook clobbering,
duplicate startup persistence, privilege escalation, overbroad uninstall, and
signing-secret exposure.

Mitigations:

- Publish signed/notarized artifacts, checksums, SBOM, license notices, and CI
  provenance from protected release jobs.
- Install per-user by default; never require administrator rights for the local
  path. Record every owned hook fragment, config field, startup item, and file.
- Merge Codex/SSH configuration structurally, preserve unrelated user content,
  write atomically with backups, and make repeated install idempotent.
- Uninstall removes only recorded owned artifacts and preserves event data
  unless the user explicitly authorizes deletion. Upgrade supports rollback.
- Signing keys and SSH private keys stay in external secret stores and never in
  repository defaults, command output, or logs.

Residual risk: compromise of a signing identity or protected CI environment can
produce trusted malicious artifacts. Revocation and release withdrawal are part
of the Stage 20 rollback plan.

## Abuse cases and required tests

| Abuse case | Expected result |
| --- | --- |
| 16,385-byte envelope | Reject before JSON allocation with `payload_too_large`. |
| Unknown version, kind, or top-level field | Reject with a stable error and no persistence. |
| Newline, ANSI escape, XML, or Unicode control input | No log or notification structure injection. |
| Shell metacharacters in every string field | Remain data; executable and argv are unchanged. |
| Replayed `event_id` 100 times | One user-visible notification and bounded receipt refresh. |
| Cross-user IPC client | Authentication/ACL rejection before event parsing. |
| Changed SSH host key | Permanent transport error; never auto-accept replacement. |
| PTY, forwarding, shell, extra argument, or concatenated event | Forced-command entry rejects the session/input. |
| Locked/full/unwritable database | Classified error without deleting queued events. |
| Installer rerun and uninstall | No duplicate hooks/startup items; unrelated user config remains. |

## Review triggers

Review and supersede this model before adding a protocol version, event kind,
notification action, reverse tunnel, network listener, hosted service, mobile
client, storage encryption, privileged installer, or new release channel.
Also review after a security incident, signing-key rotation, or change to the
minimum supported Codex/OS versions.

## Stage 20 rereview

The 2026-07-30 release audit found no new protocol, event, transport, native
API, privilege, or data-retention boundary. The seven invariants and existing
residual risks therefore remain valid. Stage 19 adds a fixed-input packaging
boundary, checksum/SBOM/license verification, protected signing/notarization
branches, and state-preserving package lifecycle tests without adding a
network service or privileged installer.

Gitleaks 8.30.1 scanned all 129 commits at the audit baseline with its release
archive verified against the publisher's SHA-256 list; no secret was found.
The same checksum-pinned full-history scan is a permanent supply-chain CI gate.
No private-key, certificate, credential, database, or built release archive is
tracked. Machine-specific proxy instructions remain development-only and do
not enter application defaults or fixed package inputs.

Production Windows and Apple identities, notarization credentials, and the
protected `release` GitHub Environment are absent. Consequently the production
trust branch itself has not run, and the current unsigned/ad-hoc artifacts are
not release candidates. Continuous remote-to-Windows and remote-to-macOS native
runs now exist for optimized source builds with temporary engineering trust,
but neither has been rerun with a production-signed candidate. Production trust
and candidate reruns remain blocking evidence gaps, not accepted residual
risks; the Stage 20 audit must return no-go until they are closed.
