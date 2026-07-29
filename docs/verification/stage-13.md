# Stage 13 Verification

Status: In progress

Date: 2026-07-29

Scope: macOS UserNotifications adapter, application identity, authorization,
and packaging resources.

## Implemented evidence

- The implementation and all native dependencies are selected only under
  `cfg(target_os = "macos")`; other targets do not link or initialize them.
- The backend uses Apple's modern UserNotifications framework through the safe
  `usernotifications-rs` facade. Deprecated `NSUserNotification`, `osascript`,
  bundle-identity spoofing, and hand-written unsafe shims are not used.
- The fixed bundle identifier is `io.github.leopardrich.codex-notifier`. The
  backend requires that exact identifier, a `.app` main bundle, and the current
  user's Aqua launch domain.
- Read-only diagnostics distinguish missing identity, authorization not yet
  requested, denial/application disablement, no GUI session, API
  unavailability, and delivery failure. Focus/Do Not Disturb is explicitly
  reported as `system_managed` because the public framework does not expose a
  reliable current Focus state.
- Shared tests cover private/public mapping, both event kinds, stable event IDs,
  Unicode scalar truncation, control removal, urgency, and silent delivery.
  macOS-only tests cover bundle-ID validation, permission classification,
  framework denial mapping, display-only content, and active/passive
  interruption levels that never bypass Focus.
- The ignored `macos_smoke` test creates and ad-hoc signs a temporary product-ID
  app bundle, explicitly requests permission, then submits both real event
  kinds. Normal test runs cannot display notifications or prompt for access.
- Packaging documentation freezes the Info.plist, icon, Developer ID signing,
  notarization, Aqua LaunchAgent, upgrade, uninstall, and ownership resources.

## Automated checks

- `cargo fmt --all -- --check`, locked metadata, and `git diff --check` pass.
- On the final combined Stage 12/13 tree, Rust 1.88 GNU full-workspace Clippy
  passes with all targets, all features, and warnings denied. The workspace
  suite passes 87 automated tests; the two Windows real-state tests stay
  explicitly ignored, and the macOS-only smoke target is empty on Windows.
- On Windows 10 22H2, focused Rust 1.88 GNU `cargo check` and all-targets,
  all-features Clippy with warnings denied pass for the native notification
  crate. Its ten Windows/shared unit tests and documentation tests pass; both
  platform smoke targets remain ignored or empty as intended on this host.
- Target-specific dependency trees prove that Linux includes neither native
  implementation, Windows includes WinRT/sysinfo but no Apple crates, and
  macOS includes Foundation/rustix/`usernotifications-rs` but no Windows or
  deprecated `mac-notification-sys` dependency.
- Cross-target `cargo check` and all-targets/all-features Clippy with warnings
  denied pass for `aarch64-apple-darwin` against the official Rust 1.88 Apple
  standard-library archive, whose SHA-256 matched the published checksum. This
  type-checks the exact Foundation and `usernotifications-rs` APIs used by the
  backend and its tests. Dependency build scripts ran with `DOCS_RS=1`, so the
  Swift bridge and Apple framework linker were intentionally not invoked on
  Windows.
- GitHub Actions run
  [`30438113181`](https://github.com/LeopardRich/codex-notifier/actions/runs/30438113181)
  for commit `69143203a1bd7754eb4fbd2bde55cdd83aa132cd` passed formatting,
  warnings-as-errors Clippy, and all normal workspace tests on a native
  `macos-latest` runner. This exercised the Swift bridge build and Apple
  framework linking. The ignored interactive smoke test did not run, so this is
  not authorization or notification-display evidence.

## Required before completion

- Run the ignored smoke test from an interactive signed bundle and visually
  confirm both event kinds on macOS 14.
- Repeat the same confirmation on the latest supported macOS release.
- Record first authorization, denial, Focus/Do Not Disturb, and no-GUI session
  diagnostics in real system states.
- Until these interactive items are complete, Stage 13 is not a support claim.
