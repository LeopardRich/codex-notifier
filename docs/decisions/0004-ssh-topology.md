# ADR-0004: Direct system OpenSSH transport

- Status: Accepted
- Date: 2026-07-29
- Owners: Project maintainers

## Context

Remote Codex events must reach a user's desktop without a hosted relay or a
project-owned public listener. The expected deployments already use a trusted
LAN or VPN and can configure host aliases.

## Decision

The first release uses a direct, outbound OpenSSH client process from the relay
to an existing OpenSSH server on the desktop. The relay invokes a configured
host alias with an argument array and sends exactly one event envelope over
stdin. A dedicated authorized key is restricted to
`codex-notifier receive`; PTY allocation, agent forwarding, port forwarding,
shell access, and unrelated commands are disabled.

Host-key checking is mandatory. No event field may affect the executable,
arguments, host alias, command, path, or environment. Version 1 provides no
reverse SSH tunnel, HTTP endpoint, embedded SSH server, background network
listener, or automatic firewall change. Local desktop delivery remains fully
independent of SSH.

## Alternatives

- Reverse SSH was rejected for the first release because Stage 01 has no real
  topology evidence requiring it and it adds lifecycle and trust complexity.
- A hosted relay and HTTP webhook were rejected as out of scope.
- An embedded SSH implementation was rejected in favor of the maintained
  system OpenSSH client and server.

## Consequences

Remote delivery requires desktop SSH reachability, typically through a LAN or
VPN. Users behind an unreachable NAT cannot use the first remote path. A future
reverse transport must be a separate adapter and superseding ADR.

## Security Impact

The SSH server authenticates the relay key and the relay pins the desktop host
key. Forced-command restrictions limit a stolen relay key, but compromise of
either endpoint or the user's account remains outside the protocol boundary.

## Compatibility Impact

Windows and macOS use the same `relay.ssh_host_alias` configuration. Platform
documentation may differ for enabling the existing OpenSSH server and setting
authorized-key restrictions.

## Verification

- Exercise a real OpenSSH session with the forced command.
- Reject PTY, forwarding, shell, extra arguments, and multiple envelopes.
- Verify shell metacharacters remain stdin data and never alter argv.
