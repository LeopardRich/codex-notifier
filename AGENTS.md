# codex-notifier

## Project Goal

Build a small notification bridge for Codex CLI events that require user
attention, initially permission requests and task completion.

`README.md` and `README-zh.md` are the current product and architecture sources
of truth. They define these delivery paths:

- A Codex CLI event on a local Windows machine becomes a Windows system
  notification.
- A Codex CLI event on a local macOS machine becomes a macOS system
  notification.
- A Codex CLI event on a remote server is forwarded over SSH to a companion
  process on the user's computer, then becomes the native notification for
  that computer's operating system.

Do not add Linux desktop notification support, mobile clients, or hosted relay
services unless the project requirements are explicitly expanded.

## Current State

The repository is currently in architecture-only status. The planned
implementation is a Rust workspace producing one `codex-notifier` executable,
but no Cargo workspace or application code exists yet. Do not document build,
test, or lint commands before the corresponding configuration exists.

## Architecture Guidelines

- Keep event detection, transport, and native notification delivery as
  separate components with explicit interfaces.
- Use one structured event model across local and remote paths. At minimum,
  distinguish the event kind, display text, source host, timestamp, and a
  stable event identifier.
- Keep operating-system-specific code behind a notification adapter.
- Make the local path independent of SSH so it continues to work without any
  remote configuration.
- Treat remote delivery as untrusted input. Authenticate the sender, validate
  and bound all fields, and prevent event text from being interpreted as shell
  commands.
- Never commit private keys, access tokens, machine-specific paths, or host
  credentials. Provide example configuration with placeholder values instead.
- Prefer established libraries for SSH and native notifications over custom
  protocol or platform implementations.
- Avoid background network listeners when an SSH command, tunnel, or other
  narrowly scoped mechanism can satisfy the architecture.

## Implementation Expectations

- Preserve Windows and macOS behavior behind the same user-facing commands and
  configuration shape where platform differences allow it.
- Make failures actionable: distinguish unsupported platforms, missing native
  notification permissions, SSH/authentication failures, malformed events,
  and delivery failures.
- Avoid logging secrets or full remote payloads by default.
- Keep setup reversible and document any shell hooks, Codex configuration, SSH
  configuration, login items, scheduled tasks, or launch agents that are
  installed.
- Use UTF-8 for source and documentation. Keep product identifiers, command
  names, configuration keys, and filenames in English.

## Network Access

- When development commands or tools need Internet access, use the local proxy
  at `127.0.0.1:7890`.
- Configure both HTTP and HTTPS proxy settings for tools that distinguish
  between them. Use the same endpoint for SOCKS-capable tools when SOCKS is the
  required proxy protocol.
- Apply proxy settings through command options or process environment variables
  appropriate to the current shell and tool. Do not commit machine-specific
  proxy settings, credentials, or proxy configuration to application defaults.
- Localhost IPC and communication between `codex-notifier` processes must not
  be routed through the proxy.

## Testing Expectations

- Unit-test event parsing, validation, deduplication, and routing independently
  from native notification APIs.
- Use adapter fakes for platform notification tests and SSH transport tests.
- Add platform-specific smoke tests where practical, and make their operating
  system requirements explicit.
- Cover both permission-request and task-completion events through local and
  remote routing paths.
- Run the repository's formatter, linter, type checker, and tests once those
  tools are established. Report any checks that cannot run on the current OS.

## Change Discipline

- Keep changes scoped to the architecture described above.
- Keep the English and Chinese README architecture descriptions aligned when a
  change alters a component, supported platform, or event-delivery path.
- Add setup and development commands to the main README as soon as the first
  implementation establishes them.
- Do not claim that a native notification or remote delivery path works unless
  it has been exercised on the relevant platform or is clearly marked as
  unverified.
