# codex-notifier 0.1.0

Initial release candidate for the local Windows/macOS notification bridge and
Linux remote relay described in the root README files.

## Included

- Windows x86-64 desktop archive.
- macOS universal application archive for Intel and Apple silicon.
- Linux relay archives for x86-64 and AArch64.
- SHA-256 checksums, SPDX 2.3 SBOM, and third-party license notices.

## Known limits

- The ordinary Codex CLI approval hook remains unavailable; approval requests
  use the verified app-server interface.
- Real remote-to-Windows and remote-to-macOS continuous native paths remain
  unverified as recorded in `docs/reliability.md`.
- Linux is relay-only and has no desktop notification adapter.
- Version 1 has explicit manual install/upgrade commands and no auto-updater.
