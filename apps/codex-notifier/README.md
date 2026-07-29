# codex-notifier application

Composition root for the single executable. It binds the secure per-profile IPC
endpoint before opening SQLite or initializing a role adapter, maps validated
desktop/relay configuration into the application runtime, converts submissions
to structured acknowledgements, and coordinates IPC shutdown with bounded
worker drain.

Stage 10 adds the bounded `emit task-completed` stdin entry. It accepts only the
fixture-verified Codex CLI 0.144.5 `Stop` hook shape, creates a fresh `UUIDv7`,
normalizes private display content, and submits the event through the same
per-profile local IPC endpoint. Source compatibility, IPC, and structured agent
rejections retain distinct safe error codes. Other command parsing remains
assigned to later ingestion, diagnostics, and installation stages.
