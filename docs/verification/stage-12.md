# Stage 12 Verification

Status: In progress

Date: 2026-07-29

Scope: Windows native Toast adapter and diagnostics.

## Implemented evidence

- Windows implementation and dependencies are selected only by `cfg(windows)`.
- The backend uses the safe `windows-rs` WinRT projection, validates the fixed
  PascalCase product AUMID `LeopardRich.CodexNotifier`, rejects Session 0, and
  checks `ToastNotifier.Setting` before delivery.
- Missing identity, application/user/policy disablement, unavailable API,
  non-interactive session, and delivery failure have distinct stable status and
  error codes.
- Private/public policy, both event kinds, urgency, application quiet hours,
  control removal, Unicode scalar truncation, and fake-backend delivery are
  independently tested.
- Toast XML is assembled with DOM nodes. Tests prove XML metacharacters remain
  text and that no action, activation type, launch URI, or reply input is added.
- Windows packaging documentation freezes the product AUMID and the reversible
  Start Menu shortcut, unpackaged-app registry identity, icon, executable,
  startup, and ownership resources.

## Executed checks

- Host: Windows 10 22H2, build 19045, interactive session 8.
- Focused `cargo check` and all-targets/all-features Clippy with warnings denied
  passed for `codex-notifier-native-notification`.
- Ten unit tests and documentation tests passed locally. The normal suite keeps
  the real two-Toast and application-disabled state tests ignored unless
  explicitly requested in their required system states.
- The final Rust 1.88 GNU workspace gates pass on this host: all-targets,
  all-features Clippy with warnings denied and 87 automated tests. The two
  Windows real-state tests remain explicitly ignored during normal runs.
- The local GNU Rust link used temporary official MSYS2 binutils/GCC startup
  objects under ignored `target/`; no toolchain or system PATH was modified.

## Real smoke attempt

- The earlier AUMID `LeopardRich.codex-notifier` was rejected even with a valid
  Start Menu shortcut. It did not follow Microsoft's documented PascalCase
  `CompanyName.ProductName` form and was corrected before release.
- With a temporary per-user unpackaged-app registration for
  `LeopardRich.CodexNotifier`, the explicit ignored smoke test passed on Windows
  10 22H2. WinRT reported `Ready` and accepted one task-completion Toast and one
  approval-request Toast. User visual confirmation is still pending.
- A fixed PowerShell-identity control Toast succeeded, proving the interactive
  session could display notifications without adopting that identity as a
  product fallback.
- Directly writing `Enabled=0` under the product notification settings key did
  not change the live `ToastNotifier.Setting`; the test restored the original
  missing value. Disabled-state evidence must therefore use the Windows
  Settings UI rather than an unsupported registry simulation.
- All temporary Start Menu shortcuts and unpackaged-app registration values
  were removed. The Windows-owned notification-history key was retained, and
  no user startup item or Codex hook was changed.

## Required before completion

- Record user visual confirmation for both accepted Windows 10 22H2 Toasts.
- Repeat both Toasts on the latest supported Windows 11 release.
- Exercise notifications disabled, Focus Assist/Do Not Disturb, missing
  identity, and non-interactive session diagnostics in real system states.
- Until these real-platform items are complete, Stage 12 must not be marked
  complete.
