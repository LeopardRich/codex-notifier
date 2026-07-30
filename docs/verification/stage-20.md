# Stage 20 Release Candidate Audit

Status: Audit complete - release blocked

Date: 2026-07-30

Audited version: `0.1.0`

Audit baseline: commit `ab7be5624d23c9c7870efdb6bf2b8043d7f00c15`

Decision: **NO-GO**. No formal tag or GitHub Release may be created from the
available unsigned/ad-hoc verification artifacts.

## Stage 01-19 evidence chain

| Stage | Evidence | Audit result |
| ---: | --- | --- |
| 01 | [`../compatibility.md`](../compatibility.md) and sanitized fixtures under `tests/fixtures/` | Exact Codex CLI 0.144.5 task-completion and app-server approval capabilities are evidenced; ordinary CLI approval remains unverified. |
| 02 | Accepted [ADRs](../decisions/README.md), [`../event-protocol-v1.md`](../event-protocol-v1.md), and [`../threat-model.md`](../threat-model.md) | Architecture, protocol, release channel, privacy, SSH, and security boundaries are frozen. |
| 03 | [`stage-03.md`](stage-03.md) | Rust workspace and three-platform quality gates complete. |
| 04 | [`stage-04.md`](stage-04.md) | Canonical bounded event model complete. |
| 05 | [`stage-05.md`](stage-05.md) | Layered configuration and platform paths complete. |
| 06 | [`stage-06.md`](stage-06.md) | Structured redacted logging complete. |
| 07 | [`stage-07.md`](stage-07.md) | Transactional SQLite outbox/deduplication complete. |
| 08 | [`stage-08.md`](stage-08.md) | Same-user bounded local IPC complete. |
| 09 | [`stage-09.md`](stage-09.md) | Role-aware agent lifecycle/routing complete. |
| 10 | [`stage-10.md`](stage-10.md) | Task-completion ingestion complete for the verified fixture/interface. |
| 11 | [`stage-11.md`](stage-11.md) | Approval-request ingestion complete for app-server; audit restored the missing explicit completion status without changing its limits. |
| 12 | [`stage-12.md`](stage-12.md) | Windows native adapter and real platform states complete. |
| 13 | [`stage-13.md`](stage-13.md) | macOS native adapter and real platform states complete; production Apple trust remains separate. |
| 14 | [`stage-14.md`](stage-14.md) | Reversible Windows/macOS local lifecycle complete. |
| 15 | [`stage-15.md`](stage-15.md) | Restricted real OpenSSH receiver boundary complete. |
| 16 | [`stage-16.md`](stage-16.md) | Durable system-OpenSSH relay and recovery complete. |
| 17 | [`stage-17.md`](stage-17.md) | Read-only diagnostics/status and delivery-aware tests complete. |
| 18 | [`stage-18.md`](stage-18.md) and [`../reliability.md`](../reliability.md) | Crash, duplicate, fault, load, and honest four-path matrix complete. |
| 19 | [`stage-19.md`](stage-19.md) and [`../release.md`](../release.md) | Versioned verification packages, supply-chain documents, lifecycle gates, and fail-closed production workflow complete. |

All dedicated Stage 03-19 records now carry `Status: Complete`. Stage 01 and 02
predate the per-stage verification-file convention and retain their canonical
evidence in the documents identified above; neither is inferred from a later
implementation test.

## Versioned verification record

Permanent `main` run
[`30500661470`](https://github.com/LeopardRich/codex-notifier/actions/runs/30500661470)
passed on the exact audit baseline. It rebuilt the versioned Windows x86-64,
macOS universal, Linux x86-64, and Linux AArch64 archives; verified sidecar and
aggregate checksums, package layout/version/architecture/SBOM, all normal
platform tests, real OpenSSH gates, and headless native diagnostics; exercised
Windows, macOS, and Linux x86-64 install/reinstall/status/uninstall lifecycles;
and uploaded the complete bundle. The tag-only publication job was skipped.

These are reproducible engineering test steps, not reproducible-byte or
release-candidate claims. Windows metadata says `unsigned-verification`; macOS
metadata says `ad-hoc-verification`. No production trust or provenance was
fabricated. Version `0.1.0` has the documented one-time no-previous-stable
upgrade exception; the next minor release must use a literal previous package.

## Security and configuration audit

- The publisher checksum for Gitleaks 8.30.1 Windows x64 archive is
  `d29144deff3a68aa93ced33dddf84b7fdc26070add4aa0f4513094c8332afc4e`.
  The verified binary scanned all 129 commits, approximately 1.41 MB, with
  `gitleaks git --redact --no-banner --log-opts="--all" .`; no leak was found.
- CI now downloads the Linux x64 Gitleaks 8.30.1 archive, verifies SHA-256
  `551f6fc83ea457d62a0d98237cbad105af8d557003051f41f3e7ca7b3f2470eb`,
  and scans the complete fetched history before supply-chain documents exist.
- `git ls-files` found no tracked `.pem`, `.key`, `.pfx`, `.p12`, `.db`,
  `.sqlite`, `.zip`, `.tar.gz`, `.env`, credential, or secret file. Focused
  searches found no development-host workspace or user path. Sanitized test
  paths and uppercase SSH placeholders remain deliberate non-secret fixtures.
- The only committed proxy endpoint is the development instruction in
  `AGENTS.md`. No proxy, private key, token, password, user path, or host
  credential is an application default or fixed package input.

## Threat and compatibility rereview

The 2026-07-30 rereview in [`../threat-model.md`](../threat-model.md) found no
new product boundary. Stage 19 packaging adds supply-chain exposure already
covered by the installer/release boundary; checksum-pinned history scanning and
fixed archive inputs strengthen its mitigations. A compromised protected CI
environment/signing identity remains a residual risk requiring revocation and
withdrawal.

[`../compatibility.md`](../compatibility.md) now separates Codex interface,
desktop OS, relay architecture, artifact, and remote-path evidence. Linux
remains relay-only. Windows Server, ordinary CLI approval, macOS Codex fixture
capture, and native AArch64 relay execution remain outside verified claims.
Both continuous remote desktop paths now have optimized source-build evidence;
neither has production-signed candidate evidence.

## Local audit implementation checks

- Rust 1.88 GNU formatting and strict all-target/all-feature Clippy passed.
- All 144 normal workspace tests passed. Four interactive Windows native-state
  tests remained intentionally ignored and retain their documented manual
  requirements.
- `cargo deny check advisories licenses bans sources` passed. The same three
  target-driven duplicate-version families remain visible warnings.
- Actionlint 1.7.7, `git diff --check`, the exact staged-patch Gitleaks scan,
  and a repository-wide relative Markdown link audit passed.
- `Cargo.lock` did not change during the audit.

## Branch CI

- Initial audit run
  [`30501960312`](https://github.com/LeopardRich/codex-notifier/actions/runs/30501960312)
  passed the normal four-platform matrix, checksum-pinned full-history Gitleaks
  scan, dependency/SBOM/license gates, both Linux packages, and the macOS
  universal package on commit `e4b7338`. Windows package construction and
  verification also passed, but its lifecycle exposed a real shutdown race:
  install and idempotent reinstall succeeded, then uninstall observed the
  removed agent status record before the Windows process released its installed
  executable and returned `install_platform_filesystem_failed`. The aggregate
  bundle correctly remained blocked.
- Commit `a45b388` adds a bounded Windows-only removal retry after ownership and
  symlink validation. A deterministic exclusive-delete handle test releases the
  lock after 150 ms and proves removal waits and completes; permanent locks
  still fail after the five-second bound.
- Corrected run
  [`30502436974`](https://github.com/LeopardRich/codex-notifier/actions/runs/30502436974)
  passed every job on commit `a45b388`, including the rebuilt Windows archive's
  install/reinstall/status/uninstall/root-removal lifecycle, the macOS and Linux
  lifecycles, all four packages, full-history secret scan, and aggregate
  checksum bundle. Tag-only attestation/publication remained skipped as
  designed.
- A supplemental Windows 10 Session 8 run then exercised the optimized branch
  executable through a temporary restricted Win32-OpenSSH 10.0p2 server to the
  installed WinRT adapter. Both synthetic kinds and sanitized fixtures passed,
  offline recovery succeeded, 100 stable-ID retries remained duplicates, and
  all temporary install/SSH resources were removed.
- Supplemental macOS run
  [`30505508865`](https://github.com/LeopardRich/codex-notifier/actions/runs/30505508865)
  passed local native evidence plus the continuous restricted
  OpenSSH-to-UserNotifications path on commit `421c1ab`; normal CI run
  [`30505508875`](https://github.com/LeopardRich/codex-notifier/actions/runs/30505508875)
  passed all permanent platform and package jobs. The app used a temporary
  non-production trusted identity and was not notarized.

## Candidate rerun matrix

| Required candidate path | Available result | Decision |
| --- | --- | --- |
| Windows local and complete uninstall | Unsigned archive lifecycle passed; earlier real source-to-WinRT path passed | Insufficient: no Authenticode candidate run |
| macOS local and complete uninstall | Ad-hoc universal archive lifecycle passed; earlier real native paths passed | Insufficient: no Developer ID/notarized candidate run |
| Remote to Windows native | Continuous optimized source build passed both event kinds, both fixtures, offline recovery, deduplication, and cleanup through restricted OpenSSH and WinRT | Insufficient: no Authenticode candidate run |
| Remote to macOS native | Continuous optimized source build passed both event kinds, both fixtures, install/uninstall, restricted OpenSSH, and UserNotifications in run `30505508865` | Insufficient: no Developer ID/notarized candidate run |

The matrix deliberately does not combine component evidence into a platform
claim. The complete operational rerun procedure is fixed in
[`../release-checklist.md`](../release-checklist.md).

## Blocking issues and disposition

1. Repository settings expose only `release-verification`; the protected
   `release` GitHub Environment, reviewer/tag policy, production secret names,
   and identity variables do not exist.
2. No Windows production signing certificate/thumbprint is available, so the
   Authenticode branch has never produced or verified a candidate.
3. No Apple Developer ID Application certificate or notarization API binding
   is available, so notarization, stapling, and candidate Gatekeeper checks have
   never run.
4. The four core paths have not been rerun with exact candidate artifacts. The
   remote destination procedures are now exercised, but no production-signed
   Windows or signed/notarized macOS candidate exists to use in those reruns.

The audit itself is complete and its result is no-go. The implementation may
merge after exact branch-head and permanent `main` CI are green, but Stage 20
does not authorize a tag. A future release attempt must close every blocker,
execute the checklist on newly built candidate artifacts, append evidence, and
obtain a go decision before publication.
