# ADR-0001: Initial supported versions

- Status: Accepted
- Date: 2026-07-29
- Owners: Project maintainers

## Context

Adapters and native APIs require explicit version floors. Stage 01 produced
real external-event evidence only for Codex CLI 0.144.5 on Windows 10 22H2.
No macOS native notification has been exercised yet.

## Decision

- Codex CLI 0.144.5 is the minimum version eligible for the first release.
  Eligibility remains capability-based: installation enables only adapters
  whose exact interface is marked Supported in `docs/compatibility.md`.
- Windows 10 22H2, build 19045, x86-64 is the minimum desktop target.
- macOS 14 Sonoma on Apple silicon or x86-64 is the minimum macOS target.
- Linux has no desktop adapter. The relay build targets maintained x86-64 and
  AArch64 distributions with glibc 2.35 or newer.
- A platform/version is not advertised as supported until its required real
  smoke tests have passed. Until then it is a build target only.

For Codex 0.144.5, `Stop` from `codex exec` is verified for task completion and
the app-server approval request is verified. The lifecycle
`PermissionRequest` hook remains unverified for the ordinary CLI path and must
be reported unavailable there.

## Alternatives

- Supporting older Codex releases was rejected because no real payloads exist.
- Windows 11-only support was rejected because Windows 10 22H2 provides the
  required notification APIs and is the verified Stage 01 host.
- macOS 13 was rejected to reduce signing, notification, and CI compatibility
  combinations before any macOS implementation exists.

## Consequences

The first release has a narrow, auditable matrix. A version increase requires
new fixtures and smoke-test evidence; lowering a floor requires a superseding
ADR and CI coverage.

## Security Impact

Capability detection fails closed. Unsupported Codex versions do not install
speculative hooks, read transcripts, or scrape terminal output.

## Compatibility Impact

Installers and `doctor` must report the exact Codex interface and OS status.
Build success alone never changes an Unverified row to Supported.

## Verification

- Match adapter fixtures to every supported Codex version and interface.
- Compile on every target triple in CI.
- Run native smoke tests on the minimum and current Windows/macOS releases
  before making support claims.
