# codex-notifier application

Composition root for the single executable. It binds the secure per-profile IPC
endpoint before opening SQLite or initializing a role adapter, maps validated
desktop/relay configuration into the application runtime, converts submissions
to structured acknowledgements, and coordinates IPC shutdown with bounded
worker drain.

Stages 10-11 add bounded `emit task-completed` and `emit approval-requested`
stdin entries. They accept only fixture-verified Codex CLI 0.144.5 shapes,
create a fresh `UUIDv7`, normalize private display content, and submit through
the same per-profile local IPC endpoint. Source compatibility, IPC, and
structured agent rejections retain distinct safe error codes.

The minimal `doctor codex` command reports the same version/interface
capability and installation selection used by the adapters. Approval events are
display-only: no response decision, command, arguments, action URL, or remote
approval behavior is exposed. Broader command parsing remains assigned to
later diagnostics and installation stages.
