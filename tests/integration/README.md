# Integration tests

Crate-owned integration tests cover real IPC, persistence, and executable
boundaries. Stage 15's process contract is in
`apps/codex-notifier/tests/receive_contract.rs`; the permanent Linux CI job
also exercises its forced-command restrictions through a temporary real
OpenSSH server. No test modifies a developer's SSH configuration or keys.
