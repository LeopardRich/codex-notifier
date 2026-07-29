# Stage 10 Verification

- Status: Complete
- Date: 2026-07-29
- Host: Windows 10 Pro 22H2 (19045.6466)

## Implemented

- Fixture-gated task-completion normalization for the exact Codex CLI 0.144.5
  CLI lifecycle-hook interface; app-server and unknown versions fail closed.
- Strict `Stop` schema requiring all nine observed fields, including fields
  whose values may be null, with unknown-field rejection and a 32 KiB input
  limit.
- Privacy mapping that replaces the source session identifier with a SHA-256
  digest, uses trusted fixed source/routing context, discards raw cwd,
  transcript, model, turn and assistant-message values, and emits the fixed
  private task-completion presentation from ADR-0003.
- Application emitter that assigns a fresh `UUIDv7`, normalizes at receive
  time, submits through per-user local IPC, and preserves distinct source, IPC,
  and structured agent-rejection error codes.
- Bounded `emit task-completed` executable entry that reads one hook payload
  from stdin. Hook installation remains assigned to Stage 14.

## Verified

- Six source-adapter contract tests cover the Stage 01 canonical snapshot and
  session hash; privacy exclusion; every missing and type-changed field;
  unknown and sensitive extra fields; invalid values; oversized input; and
  explicit version, interface, and trusted-context selection.
- Application contracts prove source failures occur before IPC failures and
  run the real executable as a child process with fixture payload on stdin.
  The child reaches a real `AgentHost` over platform local IPC within a five
  second timeout, receives an acknowledgement, and leaves no fixture-sensitive
  value in the canonical event.
- `cargo fmt --all -- --check`, workspace Clippy with `-D warnings`, and
  `cargo test --workspace` pass in CI. The workspace totals are 71 tests on
  Windows and 73 tests on macOS/Linux.
- GitHub Actions run
  [`30417585199`](https://github.com/LeopardRich/codex-notifier/actions/runs/30417585199)
  completed successfully for `windows-desktop`, `macos-desktop`, and
  `linux-relay` from commit `9066fe7`.

## Local Environment Note

- `cargo fmt --all -- --check`, workspace metadata, `git diff --check`, and
  focused all-targets Clippy for the Codex source and application crates pass
  locally.
- Local test linking cannot find `dlltool.exe`, and the executable composition
  crate cannot rebuild bundled SQLite because no usable C compiler is
  available. No machine toolchain was installed or changed as a workaround.
- The existing global Codex hook was not modified; its SHA-256 remains
  `86C1B85D6CDDA62D413EBBAC01F0AE0B826C99AF0231F1A88DB1FD5B9FCA2230`.

Stage 10 is complete. Approval-request ingestion remains assigned to Stage 11.
