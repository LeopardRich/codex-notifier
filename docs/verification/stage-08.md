# Stage 08 Verification

- Status: Complete
- Date: 2026-07-29
- Host: Windows 10 Pro 22H2 (19045.6466)

## Implemented

- Per-profile Windows named-pipe and macOS/Linux Unix-domain-socket adapters
  built on `interprocess` 2.4.2 with Tokio I/O.
- Owner-only Windows pipe DACL, peer PID/current-user comparison, owned Unix
  `0700` runtime directory, `0600` socket, and effective-user peer checks.
- Four-byte big-endian framing with 16,384-byte requests, 2,048-byte
  acknowledgements, strict event/acknowledgement validation, and stable safe
  error classifications.
- Bounded connect/read/write deadlines and a 1-256 connection task semaphore.
- Active-listener protection, owned stale Unix socket recovery, conservative
  cleanup, and rejection of unsafe endpoint types, owners, and permissions.

## Verified

- IPC unit and contract tests cover valid structured submission, status/error
  validation, identity mismatch rejection, oversized/truncated/slow frames,
  absent-endpoint deadlines, active endpoint protection, Unix stale recovery,
  concurrency bounds, and a real child submission with invalid proxy
  environment variables.
- `cargo fmt --all -- --check` exits 0 locally.
- `cargo clippy -p codex-notifier-ipc --all-targets -- -D warnings` exits 0
  locally.
- `cargo test --workspace` passes all Stage 04-08 tests in CI. The IPC crate
  runs two unit and eight contract tests on Windows, and three unit and nine
  contract tests on macOS/Linux, including a real child-process submission.
- GitHub Actions run
  [`30398042844`](https://github.com/LeopardRich/codex-notifier/actions/runs/30398042844)
  completed successfully for `windows-desktop`, `macos-desktop`, and
  `linux-relay` from commit `29e3e23`.

## Local Environment Note

- Full workspace Clippy cannot rebuild bundled SQLite because no usable local C
  compiler is available. IPC test linking also cannot find `dlltool.exe` in
  the installed Rust GNU environment. No machine toolchain, Codex login, or
  global hook was modified as a workaround.

Stage 08 is complete. Agent lifecycle and role routing remain assigned to
Stage 09.
