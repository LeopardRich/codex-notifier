# Release packaging and verification

Stage 19 builds the same four versioned artifacts on every push:

| Artifact | Target | Desktop trust requirement |
| --- | --- | --- |
| `codex-notifier-vVERSION-windows-x86_64.zip` | Windows x86-64 desktop | Authenticode signature from the protected Windows identity. |
| `codex-notifier-vVERSION-macos-universal.zip` | macOS 14+, Intel and Apple silicon | Developer ID Application signature, Apple notarization, stapled ticket, and Gatekeeper acceptance. |
| `codex-notifier-vVERSION-linux-x86_64.tar.gz` | Linux x86-64 relay | Release checksum and GitHub provenance. |
| `codex-notifier-vVERSION-linux-aarch64.tar.gz` | Linux AArch64 relay | Release checksum and GitHub provenance. |

Each archive includes `LICENSE`, `THIRD-PARTY-LICENSES.html`, the SPDX 2.3
SBOM, and schema-v1 release metadata containing only product, version, target,
commit, and trust mode. The release bundle adds `SHA256SUMS` and release notes.
`codex-notifier --version`, archive name, release metadata, and macOS bundle
version must agree.

## Verification and production modes

Push and pull-request jobs create **verification artifacts**. Windows binaries
are explicitly marked `unsigned-verification`; macOS bundles are ad-hoc signed
and marked `ad-hoc-verification`. CI tests their layout, architecture, version,
checksums, install/reinstall/uninstall behavior, and state preservation. These
artifacts are not release candidates and must not be redistributed as signed
desktop builds.

Only a protected `vVERSION` tag enters the `release` GitHub Environment. The
Windows and macOS package jobs fail when any binding below is absent. The
environment must require maintainer review and restrict deployment branches or
tags before the first candidate:

| Binding | Kind | Purpose |
| --- | --- | --- |
| `WINDOWS_PFX_BASE64` | Environment secret | Base64 PKCS#12 Windows code-signing certificate. |
| `WINDOWS_PFX_PASSWORD` | Environment secret | PKCS#12 password. |
| `WINDOWS_SIGNING_CERT_SHA1` | Environment variable | Expected signer thumbprint, verified independently after signing. |
| `APPLE_CERTIFICATE_P12_BASE64` | Environment secret | Base64 Developer ID Application certificate. |
| `APPLE_CERTIFICATE_PASSWORD` | Environment secret | Apple PKCS#12 password. |
| `APPLE_DEVELOPER_ID_APPLICATION` | Environment variable | Exact `Developer ID Application` identity. |
| `APPLE_NOTARY_KEY_BASE64` | Environment secret | Base64 App Store Connect API private key. |
| `APPLE_NOTARY_KEY_ID` | Environment secret | Notary API key ID. |
| `APPLE_NOTARY_ISSUER` | Environment secret | Notary API issuer ID. |

Secrets are written only under the runner temporary directory, are removed in
the same job, and are never uploaded. The macOS script signs with hardened
runtime, submits with `notarytool --wait`, staples and validates the ticket,
and runs Gatekeeper assessment before recreating the distributed archive. The
Windows job validates Authenticode and the fixed protected thumbprint. The
publication job then creates GitHub artifact attestations and a GitHub Release.

## Supply-chain gates

The permanent workflow pins:

- `cargo-deny 0.20.2` for RustSec advisories, license allowlisting, dependency
  bans, and registry/source policy;
- `cargo-about 0.9.1` for complete third-party license texts;
- `cargo-sbom 0.10.0` for SPDX 2.3 JSON.

`Cargo.lock` must remain unchanged during those commands. Unknown registries
and git sources are denied. Duplicate dependency versions remain visible
warnings because target-specific Apple/Windows bindings currently require
them; advisories, disallowed licenses, wildcard registry requirements, and
unknown sources are blocking.

## Independent verification

Download all files from one GitHub Release into an empty directory. Verify the
aggregate file before extracting anything:

```text
sha256sum -c SHA256SUMS
```

Then run the repository verifier on the matching operating system. Examples:

```text
pwsh packaging/scripts/verify-release.ps1 -Archive codex-notifier-v0.1.0-windows-x86_64.zip -Package windows-x86_64 -Version 0.1.0 -Commit FULL_COMMIT_SHA -SignatureMode production
pwsh packaging/scripts/verify-release.ps1 -Archive codex-notifier-v0.1.0-macos-universal.zip -Package macos-universal -Version 0.1.0 -Commit FULL_COMMIT_SHA -SignatureMode production
pwsh packaging/scripts/verify-release.ps1 -Archive codex-notifier-v0.1.0-linux-x86_64.tar.gz -Package linux-x86_64 -Version 0.1.0 -Commit FULL_COMMIT_SHA -SignatureMode checksums-and-provenance
```

The verifier recomputes the archive sidecar, checks the exact required layout,
parses release metadata and SPDX, executes `--version` where the architecture
matches, and invokes Authenticode, `codesign`, `stapler`, `spctl`, `lipo`, or
`file` as appropriate.

## Install, upgrade, and removal

Windows and macOS archives use the existing explicit desktop commands:

```text
codex-notifier install --codex-version 0.144.5
codex-notifier status --format json
codex-notifier uninstall
```

Invoke Windows uninstall from the extracted archive, not the installed copy.
On macOS invoke the executable inside the extracted signed app bundle. Re-run
`install` from a newer verified archive to upgrade; ownership validation,
atomic replacement, rollback, and preserved SQLite state are the Stage 14
lifecycle contract.

Linux archives include `install.sh`, `uninstall.sh`, and a systemd user-unit
template. They default to `~/.local/bin` and the standard XDG config/state
directories. `--no-enable` and `--no-disable` support hosts without a live
systemd user manager and CI. Removal keeps configuration and SQLite state.

Version `0.1.0` is the first release, so no previous stable archive exists for
a literal old-package-to-new-package test. CI instead gates idempotent packaged
reinstall, changed-binary rollback contracts, schema-v0-to-v1 migration with a
pending event, and state-preserving uninstall. A real previous-stable package
upgrade becomes mandatory for the next minor release; this first-release
exception cannot be reused.

## Release prohibition

Do not create a formal version tag when desktop production signing,
notarization, protected-environment review, or any package/lifecycle gate is
unverified. Ad-hoc or unsigned artifacts can support engineering verification
only; renaming them does not make them release candidates.

The complete go/no-go checklist, required four-path candidate reruns, evidence
record, rollback, and withdrawal procedure are in
[`release-checklist.md`](release-checklist.md). A failed audit must remain a
documented no-go; it must not be bypassed by editing artifact names or release
notes.
