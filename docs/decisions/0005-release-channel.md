# ADR-0005: Signed artifacts through GitHub Releases

- Status: Accepted
- Date: 2026-07-29
- Owners: Release maintainers

## Context

The project needs auditable, reversible distribution for Windows, macOS, and a
Linux relay without operating a package repository or update service.

## Decision

GitHub Releases is the canonical first-release channel. Each version publishes:

- a signed Windows x86-64 archive containing the executable and per-user
  installer resources;
- a signed and notarized macOS universal archive containing the app/CLI bundle
  and LaunchAgent resources;
- Linux relay archives for x86-64 and AArch64;
- SHA-256 checksums, an SPDX SBOM, license notices, and release notes.

Installation and upgrade are explicit commands; version 1 has no background
auto-updater. The installer records owned files and supports rollback and
uninstall. Package-manager manifests may be added later but must point to the
same release assets and checksums.

Signing identities are secrets outside the repository. The release maintainer
owns selection and rotation of the Windows code-signing certificate and Apple
Developer ID Application/Installer identities. Their identifiers and CI secret
bindings must be fixed before the first release candidate; unsigned or
unnotarized artifacts cannot be labelled release candidates.

## Alternatives

Winget, Homebrew, and a custom update service were rejected as initial
canonical channels because they add external review or hosted infrastructure.
Unsigned archives were rejected for desktop releases.

## Consequences

Users get one traceable source and manual upgrades. Release CI needs protected
secrets and platform runners. Linux relay archives remain unsigned binaries but
are covered by release checksums and provenance.

## Security Impact

CI secrets are never written to logs or artifacts. Checksums, SBOMs, signing,
notarization, and provenance are blocking release gates. Installer ownership
metadata prevents broad deletion during uninstall.

## Compatibility Impact

Artifacts are per-version and per-target. Configuration and queue migrations
must support upgrade from the previous stable minor release.

## Verification

- Independently verify signatures, notarization, checksums, SBOM, and version.
- Install, self-test, upgrade, rollback, and uninstall on clean target systems.
- Confirm release jobs cannot run from untrusted pull-request contexts.
