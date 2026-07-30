# codex-notifier 0.1.0

Planned initial release for the local Windows/macOS notification bridge and
Linux remote relay described in the root README files.

The current branch and `main` bundles are engineering verification artifacts,
not release candidates. Publication is prohibited until the production desktop
trust gates and every item in `docs/release-checklist.md` have a recorded go
decision.

## Included

- Windows x86-64 desktop archive.
- macOS universal application archive for Intel and Apple silicon.
- Linux relay archives for x86-64 and AArch64.
- SHA-256 checksums, SPDX 2.3 SBOM, and third-party license notices.

## Known limits

- The ordinary Codex CLI approval hook remains unavailable; approval requests
  use the verified app-server interface.
- Real remote-to-Windows and remote-to-macOS continuous native paths remain
  unverified as recorded in `docs/reliability.md`; both are current candidate
  blockers.
- Linux is relay-only and has no desktop notification adapter.
- Version 1 has explicit manual install/upgrade commands and no auto-updater.
- The protected `release` GitHub Environment, production Windows signing, and
  Apple Developer ID/notarization bindings are not configured.
