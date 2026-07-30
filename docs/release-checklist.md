# Release Candidate Checklist and Withdrawal Plan

This checklist is blocking for every formal `vVERSION` tag. It complements the
mechanical package contract in [`release.md`](release.md). A verification
archive from a branch or `main` run is not a candidate, even when every
unsigned/ad-hoc package test passes.

## 1. Freeze candidate identity

- Record the exact workspace version, 40-character commit, intended tag, and
  clean source-tree state.
- Require the tag to be `vVERSION`, point to that commit, and enter a protected
  `release` GitHub Environment restricted to version tags and maintainer review.
- Record target OS builds, Codex version/interface, package names, SHA-256
  values, CI run, test operator, and UTC time. Do not replace artifacts under
  an existing version.

## 2. Evidence and security preflight

- Confirm the complete Stage 01-19 evidence map in
  [`verification/stage-20.md`](verification/stage-20.md), with every
  unavailable platform run still labelled unverified.
- Recheck [`compatibility.md`](compatibility.md),
  [`threat-model.md`](threat-model.md), known limitations, release notes, and
  both root READMEs against the candidate version.
- Require formatting, strict all-target/all-feature Clippy, all normal tests,
  real OpenSSH gates, headless native diagnostics, dependency policy, SBOM,
  license notices, and the checksum-pinned full-history Gitleaks scan to pass.
- Confirm no private key, certificate, token, password, raw event payload,
  prompt/model output, user-specific path, or application proxy default exists
  in source, history, generated documents, metadata, or extracted archives.

## 3. Production trust and artifact verification

- Windows: import the protected certificate only in the runner temporary
  directory; require Authenticode validation and the configured signer
  thumbprint on the packaged executable.
- macOS: require the exact Developer ID Application identity, hardened runtime,
  successful `notarytool --wait`, a stapled/validated ticket, `codesign --deep
  --strict`, and Gatekeeper acceptance on the extracted app.
- Linux: require the protected tag's aggregate checksum and GitHub provenance
  for both relay archives. Do not claim Linux desktop support.
- Run each archive sidecar and aggregate `SHA256SUMS` before extraction. Require
  the independent verifier to match version, commit, target, layout, SBOM, and
  production trust mode.

## 4. Candidate platform reruns

Use the exact signed/notarized candidate artifacts, not a source build or a
verification archive.

| Path | Required run |
| --- | --- |
| Windows local | Clean per-user install, Codex 0.144.5 task-completion path, explicit approval test, real WinRT acceptance, status/doctor, idempotent reinstall, login/startup restart, upgrade/rollback where applicable, and external uninstall with state preservation. |
| macOS local | Clean signed-app install, first authorization, both event kinds, Focus behavior, status/doctor, idempotent reinstall, login LaunchAgent restart, upgrade/rollback where applicable, and uninstall with state preservation. |
| Remote to Windows | Linux relay archive through real system OpenSSH and the restricted Windows receiver to the same candidate's real WinRT adapter; exercise both event kinds, offline recovery, deduplication, and revocation. |
| Remote to macOS | Linux relay archive through real system OpenSSH and the restricted macOS receiver to the same candidate's UserNotifications adapter; exercise both event kinds, offline recovery, deduplication, and revocation. |

Component tests or separate local/transport runs cannot be combined to satisfy
a remote row. For `0.1.0`, record the approved first-release exception because
no previous stable package exists. Starting with the next minor release, a
literal previous-stable upgrade with readable configuration and pending queue
state is mandatory.

## 5. Decision and publication

The decision is **go** only when every applicable item above is evidenced and
no blocking issue remains. Otherwise record **no-go**, keep the tag absent, and
retain only explicitly labelled engineering artifacts. A go run may create the
GitHub Release only through the protected workflow; manual asset replacement
or an unsigned desktop fallback is prohibited.

## Rollback and withdrawal

Before publication, reject the candidate, remove temporary signing material,
and fix forward on a new commit. Never weaken a gate to preserve a version.

After publication:

1. Freeze the `release` environment and further publication while identifying
   affected versions and SHA-256 values. Preserve a restricted incident copy
   of evidence; do not keep compromised assets publicly downloadable.
2. Mark the GitHub Release withdrawn and remove affected public assets. Do not
   overwrite the same version with different bytes. Remove or revoke the tag
   only under the project's incident policy, with the withdrawal notice
   retaining the affected identifiers.
3. Revoke/rotate a compromised Windows certificate, Apple certificate/notary
   key, GitHub credential, or SSH key through its owning service. Host-key or
   relay-key incidents also follow the revocation steps in
   [`restricted-ssh.md`](restricted-ssh.md).
4. Publish a fixed higher version after the complete checklist passes again.
   Update compatibility, checksums, SBOM, release notes, and diagnostics.
5. Direct desktop users to run `uninstall` from a trusted external package.
   Linux users run `uninstall.sh`. These removals retain configuration and
   SQLite state; deleting retained data requires a separate explicit user
   decision.

Version `0.1.0` has no previous stable binary to restore, so its safe response
is withdrawal/uninstall followed by a fixed higher version. Later releases may
roll back only to a still-trusted previous stable package whose schema contract
can read the retained configuration and pending queue.
