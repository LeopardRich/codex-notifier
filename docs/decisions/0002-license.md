# ADR-0002: MIT license

- Status: Accepted
- Date: 2026-07-29
- Owners: Project maintainers

## Context

The repository previously contained a no-rights placeholder. A real license is
required before accepting source contributions or distributing binaries.

## Decision

Use the MIT License for project-authored source, documentation, and release
artifacts. Third-party dependencies and bundled assets retain their own
licenses and must be included in the release notices and SBOM.

## Alternatives

Apache-2.0 was considered for its explicit patent grant but rejected to keep
the initial project license and redistribution obligations minimal. A
proprietary license conflicts with the intended public distribution model.

## Consequences

Users may reuse and redistribute the software subject to preserving the
copyright and license notice. The project provides no warranty.

## Security Impact

License choice does not weaken runtime controls. Dependency-license review is
a release gate so incompatible or missing notices do not enter artifacts.

## Compatibility Impact

All packages use the workspace license metadata. Dependencies do not need to
use MIT but must be legally redistributable under the selected packaging.

## Verification

- `LICENSE` contains the canonical MIT text.
- Cargo package metadata declares `license = "MIT"` once Stage 03 exists.
- CI audits dependency licenses before Stage 19 artifacts are published.
