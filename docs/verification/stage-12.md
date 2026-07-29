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
  four real-state tests ignored unless explicitly requested in their required
  identity, notification-setting, or session state.
- The final Rust 1.88 GNU workspace gates pass on this host: all-targets,
  all-features Clippy with warnings denied and 87 automated tests. The four
  Windows real-state tests remain explicitly ignored during normal runs.
- The local GNU Rust link used temporary official MSYS2 binutils/GCC startup
  objects under ignored `target/`; no toolchain or system PATH was modified.
- GitHub Actions run
  [`30438113181`](https://github.com/LeopardRich/codex-notifier/actions/runs/30438113181)
  for commit `69143203a1bd7754eb4fbd2bde55cdd83aa132cd` passed formatting,
  warnings-as-errors Clippy, and workspace tests on Windows, macOS, and Linux.
  Normal CI did not execute any ignored Windows real-state test.

## Real-state verification

- The earlier AUMID `LeopardRich.codex-notifier` was rejected even with a valid
  Start Menu shortcut. It did not follow Microsoft's documented PascalCase
  `CompanyName.ProductName` form and was corrected before release.
- With a temporary per-user unpackaged-app registration for
  `LeopardRich.CodexNotifier`, the explicit ignored smoke test passed on Windows
  10 22H2. WinRT reported `Ready` and accepted one task-completion Toast and one
  approval-request Toast. The Windows notification database persisted both
  exact fixed private payloads under the product AUMID, with no actions or
  attacker-controlled smoke-test text. This proves native OS acceptance and
  persistence; it does not claim that the user opened either notification.
- A fixed PowerShell-identity control Toast succeeded, proving the interactive
  session could display notifications without adopting that identity as a
  product fallback.
- Directly writing `Enabled=0` under the product notification settings key did
  not change the live `ToastNotifier.Setting`; the test restored the original
  missing value. This unsupported registry simulation is not treated as
  disabled-state evidence.
- A real Windows Settings test turned the product's notification toggle Off.
  The ignored diagnostic then reported `DisabledForApplication`. The test
  restored the toggle and verified that the originally absent `Enabled`
  registry value remained absent.
- A real Focus Assist test selected Priority only through the Windows Settings
  UI and submitted both event kinds successfully. The lower-right screen region
  remained free of a Toast popup while the Windows notification database
  persisted the exact task-completion and approval-request payloads. The test's
  cleanup assertion restored Focus Assist to Off.
- On the interactive Windows 10 host, a valid but never-registered diagnostic
  AUMID produced `application_identity_missing`. The product AUMID was not used
  for this check because Windows retains notification-history and Shell cache
  entries after the installer-owned identity resources are removed.
- GitHub Actions run
  [`30439550334`](https://github.com/LeopardRich/codex-notifier/actions/runs/30439550334)
  ran the ignored non-interactive diagnostic from a temporary SYSTEM scheduled
  task. The real Session 0 process reported `no_interactive_session`, and the
  task was unregistered in the workflow's cleanup path.
- All temporary Start Menu shortcuts and unpackaged-app registration values
  were removed. The Windows-owned notification-history key was retained, and
  the Settings window and local test server were closed. No user startup item
  or Codex hook was changed.

## Required before completion

- Repeat both Toasts on the latest supported Windows 11 release.
- Until this real-platform item is complete, Stage 12 must not be marked
  complete. Windows 10 evidence is not a Windows 11 support claim.
