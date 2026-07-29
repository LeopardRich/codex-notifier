# Integration tests

Crate-owned integration tests cover real IPC, persistence, and executable
boundaries. The portable receive process contract is in
`apps/codex-notifier/tests/receive_contract.rs`. The permanent Linux-only
`apps/codex-notifier/tests/openssh_receive.rs` job exercises forced-command
restrictions and the real relay agent/system OpenSSH sender through a temporary
server. It starts with the server offline, then verifies durable retry,
recovery, acknowledgements, and desktop deduplication. No test modifies a
developer's SSH configuration or keys.
