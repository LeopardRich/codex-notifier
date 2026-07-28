# codex-notifier application

Composition root for the single executable. It binds the secure per-profile IPC
endpoint before opening SQLite or initializing a role adapter, maps validated
desktop/relay configuration into the application runtime, converts submissions
to structured acknowledgements, and coordinates IPC shutdown with bounded
worker drain.

The user-facing command parser remains assigned to later ingestion,
diagnostics, and installation stages.
