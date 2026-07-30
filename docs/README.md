# Architecture documents

This directory owns architecture decisions, the frozen event protocol, the
threat model, and the tested Codex/platform compatibility matrix. Public product
behavior remains documented in both root README files.

- [`compatibility.md`](compatibility.md): evidence-gated Codex interface matrix.
- [`event-protocol-v1.md`](event-protocol-v1.md): canonical event and
  acknowledgement wire contract.
- [`threat-model.md`](threat-model.md): assets, boundaries, mitigations, and
  residual risks.
- [`restricted-ssh.md`](restricted-ssh.md): dedicated-key desktop receiver
  setup, security checks, and revocation.
- [`relay-ssh.md`](relay-ssh.md): source-built relay configuration, delivery,
  retry recovery, and removal.
- [`personal-deployment.md`](personal-deployment.md): verified engineering
  builds on personally administered Windows, macOS, and Ubuntu devices.
- [`personal-deployment-zh.md`](personal-deployment-zh.md): Chinese personal
  deployment procedure.
- [`diagnostics.md`](diagnostics.md): read-only health/status contracts,
  delivery-aware local/remote self-tests, and stable exit codes.
- [`reliability.md`](reliability.md): Stage 18 crash, duplicate, fault, load,
  and four-path platform evidence contract.
- [`release.md`](release.md): archives, protected signing/notarization,
  checksums, SBOM/licenses, verification, and package lifecycle.
- [`release-checklist.md`](release-checklist.md): release-candidate decision,
  platform reruns, reproducible evidence, rollback, and withdrawal procedure.
- [`decisions/`](decisions/): accepted architecture decisions.
- [`verification/`](verification/): stage evidence and explicitly unverified
  platform checks.
