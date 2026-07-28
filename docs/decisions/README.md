# Architecture Decision Records

Use one numbered Markdown file per decision. Each ADR must record its status,
context, decision, alternatives, consequences, security impact, compatibility
impact, and verification plan. Accepted decisions are immutable; superseding
decisions link to the ADR they replace.

| ADR | Status | Decision |
| --- | --- | --- |
| [0001](0001-supported-versions.md) | Accepted | Codex 0.144.5, Windows 10 22H2, and macOS 14 initial floors with evidence-gated support. |
| [0002](0002-license.md) | Accepted | MIT license. |
| [0003](0003-notification-privacy.md) | Accepted | Private generic notifications and OS-respected quiet behavior by default. |
| [0004](0004-ssh-topology.md) | Accepted | Direct system OpenSSH; no reverse tunnel in version 1. |
| [0005](0005-release-channel.md) | Accepted | Signed GitHub Release artifacts with checksums and SBOM. |
| [0006](0006-event-protocol-v1.md) | Accepted | Strict, bounded canonical JSON event protocol version 1. |
