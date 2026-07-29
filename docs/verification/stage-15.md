# Stage 15 Verification

Status: Complete

Date: 2026-07-30

Scope: restricted system OpenSSH receive entry, one-event stdin framing,
desktop-agent IPC forwarding, safe acknowledgements, forced-key setup, and SSH
security diagnostics.

## Implemented evidence

- The executable implements argument-free `receive`. It requires
  `SSH_ORIGINAL_COMMAND` to equal `codex-notifier receive`, requires a bounded
  `SSH_CONNECTION` marker, rejects any `SSH_TTY`, and validates this session
  shape before reading stdin.
- The receiver reads at most 16,385 bytes into memory so a 16,385-byte envelope
  is rejected as `payload_too_large`. At or below the 16,384-byte protocol
  limit, the frozen duplicate-safe canonical parser accepts exactly one JSON
  object and rejects concatenated input, unknown fields/kinds/versions,
  malformed identifiers/timestamps, and field/extension limit violations.
- A valid event is submitted to the endpoint derived only from validated local
  desktop configuration. No event field can select an executable, argument,
  host, endpoint, path, URL, environment value, or notification action. The
  local agent's matching acknowledgement is returned unchanged.
- Session, input, configuration, and IPC failures produce one compact
  protocol-v1 rejection. Codes and messages are fixed and bounded. Invalid
  input without a trustworthy request ID receives a fresh UUIDv7 response
  correlation ID; no acknowledgement includes input text, original command,
  host alias, path, key, or stack trace.
- `doctor ssh` resolves the configured host alias through bounded system
  `ssh -G` output, requires effective `StrictHostKeyChecking yes` (`true` in
  some OpenSSH output), and uses `ssh-keygen -F` to check the resolved host,
  port, or `HostKeyAlias`. Optional absolute SSH-config, known-hosts, and
  authorized-key paths support nondefault installations without being printed.
- Authorized-file diagnostics reject missing, symlinked, wrongly owned, or
  overly permissive Unix files/directories and require exact `0600`/`0700`
  modes. The Windows diagnostic requires protected inheritance, a current
  user/`SYSTEM`/`Administrators` owner, and no write-capable allow entry for
  another principal.
- The macOS and Windows forced-key templates use a dedicated Ed25519 public key
  with `restrict,command="... receive"`. The relay SSH config template disables
  PTY/agent forwarding, clears configured forwarding, selects only the
  dedicated identity, and requires strict host-key checking. Setup, fingerprint
  verification, permissions, rotation, and reversible removal are documented
  in [`docs/restricted-ssh.md`](../restricted-ssh.md).

## Automated checks

- On Windows 10 22H2, Rust 1.88 GNU `cargo fmt --all -- --check`, full-workspace
  all-targets/all-features Clippy with warnings denied, and all 110 automated
  workspace tests passed. Four interactive Windows notification tests remain
  intentionally ignored during the normal suite.
- Receive crate tests cover exact command/session matching, PTY markers,
  16,385-byte input, concatenated envelopes, shell metacharacter round trips,
  bounded redacted errors, strict host-key config parsing, Unix modes, and real
  Windows ACL allow/deny transitions on a temporary file.
- Executable process tests use isolated configuration and a real local IPC
  server. They cover matching accepted acknowledgements, unchanged
  metacharacter-heavy event bytes, hostile proxy variables, shell/extra-command
  and PTY markers, concatenated/oversized input, redacted output, missing SSH
  state, a ready pinned alias, and rejection of `accept-new` host-key policy.
- Permanent branch CI run
  [`30485637255`](https://github.com/LeopardRich/codex-notifier/actions/runs/30485637255)
  passed formatting, warnings-as-errors Clippy, normal tests, the Windows
  Session 0 diagnostic, macOS smoke-target/no-Aqua checks, and the real OpenSSH
  job across Windows, Ubuntu 22.04, macOS 14, and macOS latest.

## Real OpenSSH verification

- The permanent Ubuntu 22.04 CI job installs the system OpenSSH server and
  starts a temporary loopback-only `sshd`. It generates fresh Ed25519 client
  and host keys, pins the generated host key in an isolated `known_hosts`,
  disables password authentication, enables `StrictModes`, and authorizes only
  `restrict,command="<fixed-binary> receive"`.
- The SSH login user runs the receiver and a real desktop-role `AgentHost`
  under the same identity. The agent uses its normal per-user Unix IPC,
  transactional SQLite queue, workers, and deduplication path with only the
  final native-notification adapter replaced by a recording fake.
- A valid public event containing shell operators, quotes, substitutions, and
  path-like display text traversed the real SSH session. It received the same
  event ID with `accepted`, reached the agent byte-for-byte in canonical form,
  and produced exactly one delivery.
- A requested command containing `; touch <marker>` ran only the fixed forced
  receiver, returned `ssh_session_rejected`, and did not create the marker. A
  shell request and an extra command were likewise rejected before event
  parsing. Their outputs contained none of the supplied payload, command, key
  name, path, or stack marker.
- Two concatenated valid envelopes returned `malformed_json`. A forced PTY
  request was denied by the authorized-key restriction. An active local
  forwarding probe opened the client listener but received administrative
  prohibition and never reached its held target socket; remote forwarding
  failed at establishment with `ExitOnForwardFailure=yes`.
- After the entire rejection matrix, the desktop agent still contained exactly
  one delivered event, proving that rejected session and forwarding attempts
  did not enter persistence or the notification adapter.

## Explicit limits

- The real OpenSSH server exercise is an Ubuntu loopback protocol/security
  harness. It validates system OpenSSH forced-command semantics and the real
  desktop-agent composition, but it is not Linux desktop-notification support;
  the final notification port is a fake.
- The Windows and macOS forced-key paths are templates compiled and documented
  for their installed application layouts. They have not yet been exercised
  against real Windows or macOS OpenSSH servers, so no platform-specific remote
  delivery claim is made for those server setups.
- The diagnostic checks configuration/enrollment/permissions without opening a
  network session or changing SSH state. The actual OpenSSH connection remains
  responsible for detecting a changed live host key.
- Relay outbox sending, OpenSSH client process delivery, retry/backoff, and
  remote error classification remain Stage 16. No embedded server, reverse
  tunnel, Linux notification adapter, firewall change, or hosted relay was
  added.

## Completion decision

- One bounded event can traverse a real dedicated-key forced OpenSSH session to
  the desktop agent and receive a matching acknowledgement. Arbitrary commands,
  shell requests, PTY allocation, local/remote forwarding, and concatenated
  events are rejected, while shell metacharacters remain inert display data and
  every error response stays redacted.
- Stage 15 is complete. Stage 16 may begin only after these changes are merged
  to `main` and permanent CI is green there.
