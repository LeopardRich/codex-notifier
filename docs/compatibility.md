# Compatibility Matrix

Status: Stage 01 completed for the initial Codex CLI 0.144.5 target on Windows.
Support is interface-specific; no macOS result or broader version range may be
inferred from this file.

## Target matrix

| Codex version | Interface | Host OS | Task completed | Approval requested | Evidence |
| --- | --- | --- | --- | --- | --- |
| 0.144.5 | `codex exec` | Windows 10 Pro 22H2 (19045.6466) | Supported through the `Stop` hook | Unverified: the non-interactive run used `approval: never` | `codex-0.144.5-windows-cli-task-completed.json`, captured 2026-07-29 |
| 0.144.5 | `codex app-server` | Windows 10 Pro 22H2 (19045.6466) | Not targeted | Supported through `item/commandExecution/requestApproval` | `codex-0.144.5-windows-cli-approval-requested.json`, captured and declined 2026-07-29 |

Codex CLI 0.144.5 is the only target version selected for the initial probe.
Minimum supported Windows and macOS versions remain architecture decisions for
Stage 02 and are not support claims here. The app-server result proves an
external approval event exists; it does not prove the lifecycle
`PermissionRequest` hook fires in every Codex interface.

## Documented capability

The current official Codex hooks manual documents both relevant lifecycle
events:

- `Stop` runs when a turn stops and includes the latest assistant message.
- `PermissionRequest` runs before Codex asks for approval and includes the tool
  name and tool input.

Source: [Codex Hooks](https://developers.openai.com/codex/config-advanced#hooks),
retrieved 2026-07-29. Documentation establishes the intended interface, but it
does not satisfy this project's real-invocation gate.

## Verification evidence

The task-completion probe used a project-local command hook alongside the
pre-existing user hooks. A real `codex exec` turn invoked the external recorder
and returned the expected fixed response. The committed fixture preserves only
field names, safe enums, and redacted values.

The approval probe used the versioned app-server JSON-RPC contract with
`approvalPolicy: "untrusted"` and a read-only sandbox. A real command execution
request produced `item/commandExecution/requestApproval`; the external client
sanitized the request and replied `decline`. The proposed command was not run,
and the marker file was not created.

The initial sandboxed authentication failure was caused by the execution
environment redirecting Codex to an isolated home directory. A read-only check
outside that sandbox confirmed the user's real Codex login remained intact.
No login, logout, credential write, or user-hook modification was performed.

## Fallback behavior

Adapter selection must follow the interface-specific rows above. The planned
CLI lifecycle-hook path must report approval notifications as unavailable until
its `PermissionRequest` hook is verified in that interface; app-server support
must not be silently treated as CLI-hook support. Unsupported paths must not
read Codex session transcripts or scrape terminal output as a fallback.
