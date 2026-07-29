# Linux relay packaging

Stage 16 implements the source-built relay role, but distributable relay-only
archives and managed systemd user-service assets remain Stage 19 deliverables.
Linux desktop notifications are not supported.

Until release packaging exists, create the versioned user configuration shown
in [`docs/relay-ssh.md`](../../docs/relay-ssh.md), configure the dedicated
OpenSSH relationship, and run the built executable as:

```text
codex-notifier agent
```

Any temporary service-manager entry is user-owned and must point to the same
configuration/state environment as local `emit` commands. Remove that entry
before deleting the source-built executable. Stopping or removing it does not
implicitly delete the durable SQLite queue.
