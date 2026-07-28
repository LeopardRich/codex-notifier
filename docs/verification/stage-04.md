# Stage 04 Verification

- Status: Complete
- Date: 2026-07-29
- Host: Windows 10 Pro 22H2 (19045.6466)

## Implemented

- Strict protocol version 1 event, source, presentation, routing, and extension
  types independent of Codex, SSH, IPC, SQLite, and native notification APIs.
- Canonical lowercase `UUIDv7` validation and deterministic compact JSON.
- Duplicate-key rejection at every JSON nesting level.
- NFC normalization, control-character rejection, canonical UTC millisecond
  timestamps, initial-ingestion time bounds, and all protocol byte/count/depth
  limits.
- Stable safe error codes that do not echo attacker-controlled fields.

## Verified locally

- Both target event kinds round-trip without field loss.
- Fourteen contract tests cover deterministic serialization, unknown
  version/kind/fields, malformed shape, duplicate keys, `UUIDv7`, timestamps,
  strings, routing, extensions, and exact 16,384-byte and 4,096-byte boundaries.
- `cargo fmt --all -- --check` exits 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` exits 0.
- `cargo test --workspace` exits 0.
- `cargo tree -p codex-notifier-core --edges normal` contains only general
  serialization, UUID, time, Unicode normalization, and error libraries.
- GitHub Actions run
  [`30388589617`](https://github.com/LeopardRich/codex-notifier/actions/runs/30388589617)
  completed successfully for `windows-desktop`, `macos-desktop`, and
  `linux-relay` from commit `4338c42`.

Stage 04 is complete. Platform adapters and source-specific payload mapping
remain assigned to their later stages.
