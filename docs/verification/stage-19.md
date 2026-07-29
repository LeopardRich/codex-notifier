# Stage 19 Verification

Status: Complete

Date: 2026-07-30

Scope: versioned Windows, macOS universal, and Linux relay archives; protected
desktop signing/notarization; checksums, SPDX, license notices, provenance,
dependency audit, package lifecycle, and first-release upgrade handling.

## Implemented evidence

- `codex-notifier --version` provides an independent package-version check and
  rejects trailing arguments.
- Platform builders validate bounded version and full commit values, copy only
  fixed release inputs, emit schema-v1 release metadata, normalize timestamps
  where the archive format supports it, and create per-archive SHA-256
  sidecars.
- The Windows x86-64 archive contains the executable, MIT license, notices,
  SPDX, and metadata. Push builds are explicitly unsigned; protected tag builds
  require a valid Authenticode result and exact configured signer thumbprint.
- The macOS builder combines Intel and Apple-silicon binaries with `lipo`,
  creates the fixed product app/Info.plist and bounded icon resource, and signs
  the complete bundle. Push builds are ad-hoc verification artifacts. Protected
  tag builds require Developer ID, hardened runtime, `notarytool --wait`, a
  stapled/validated ticket, and successful Gatekeeper assessment.
- Linux x86-64 and AArch64 relay archives include the executable, systemd user
  unit template, SSH example, and reversible install/uninstall scripts. They
  never add a Linux native notification adapter.
- The cross-platform verifier checks each sidecar before extraction, rejects
  absolute/traversal paths and Linux link entries, enforces top-level layout,
  required files, release metadata, SPDX, version, architecture, and target
  signature/notarization state.
- The aggregate job independently verifies every sidecar, writes and verifies
  `SHA256SUMS`, requires exactly two desktop ZIPs and two Linux tarballs, and
  uploads one release bundle.
- Tag-only publication requires a protected `release` GitHub Environment,
  exact `vVERSION`, successful prior package jobs, GitHub build provenance, and
  an existing source tag. Missing signing/notarization bindings fail closed.

## Supply-chain and local Windows checks

- Pinned `cargo-deny 0.20.2` reports advisories, licenses, bans, and sources
  green. Three duplicate dependency-version families remain visible warnings;
  unknown registries/git sources and disallowed licenses are blocking.
- Pinned `cargo-sbom 0.10.0` generated a valid SPDX 2.3 document containing 120
  packages. Pinned `cargo-about 0.9.1` generated 35,537 bytes of dependency
  license notices. Neither command changed `Cargo.lock`.
- Rust 1.88 GNU formatting, strict all-target/all-feature Clippy, and all 143
  normal automated tests pass. Four real Windows notification-state tests
  remain intentionally ignored in the normal suite.
- `actionlint 1.7.7`, downloaded with its published SHA-256 checksum, reports no
  workflow findings.
- A local optimized Windows verification archive passed sidecar, extraction,
  metadata, SPDX, explicit unsigned-state, and packaged `--version` checks.
- That exact extracted archive completed install, idempotent reinstall,
  `status` at version 0.1.0, and external uninstall on Windows 10 build 19045.
  The install root, AUMID, and manifest were removed; existing SQLite state and
  its three receipts remained.

## Branch CI

- Initial implementation run
  [`30499362310`](https://github.com/LeopardRich/codex-notifier/actions/runs/30499362310)
  compiled both macOS architectures and passed the four normal platform jobs,
  supply-chain audit, Windows package lifecycle, Linux x86-64 package
  lifecycle, and Linux AArch64 package build. Its macOS package job failed only
  because Xcode 15.4 requires the input path before `-verify_arch`; the
  aggregate bundle correctly remained blocked.
- Commit `1bbbb27` corrected that `lipo` invocation consistently in universal
  assembly, package construction, and independent archive verification.
- Corrected branch-head run
  [`30499908270`](https://github.com/LeopardRich/codex-notifier/actions/runs/30499908270)
  passed on commit `1bbbb27`: all normal Windows, macOS 14, current macOS, and
  Linux relay gates; supply-chain audit/SBOM/notices; all four package jobs;
  Windows, macOS, and Linux x86-64 install/reinstall/status/uninstall
  lifecycles; Linux AArch64 architecture verification; and the aggregate
  checksum release bundle were green.
- The tag-only attestation/publication job was skipped as designed on the
  branch push. No signing or notarization identity was available or inferred.

## Explicit release limits

- No production Windows signing certificate, Apple Developer ID, or Apple
  notarization credential is available in this workspace. CI implementation
  can be verified without them, but no desktop verification artifact may be
  labelled a release candidate.
- `0.1.0` has no previous stable package. Existing tests cover packaged
  reinstall, changed-binary rollback, schema-v0 pending-event migration, and
  state-preserving uninstall. A literal previous-stable package upgrade becomes
  mandatory for the next minor release and this exception cannot be repeated.
- Signing-identity selection, protected environment bindings, and a real
  signed/notarized tag run remain Stage 20 release-candidate gates.

## Completion decision

- The implemented verification artifacts, blocking CI, lifecycle coverage,
  documented first-release upgrade exception, and fail-closed protected
  release path satisfy the Stage 19 implementation scope.
- Stage 19 is permanently closed only after this evidence commit is green on
  the implementation branch, fast-forwarded to `main`, and permanent `main` CI
  is green.

Stage 20 must not begin before that gate. A formal tag remains prohibited until
the production signing, notarization, protected-environment, and candidate
audit gates recorded above are actually satisfied.
