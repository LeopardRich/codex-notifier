# codex-source

Implements fixture-gated Codex event source adapters. The adapters accept the
exact Codex CLI 0.144.5 CLI `Stop` hook and app-server command-approval request
shapes verified in Stage 01, hash the source session/thread ID, and emit fixed
private presentation text.

Raw working directories, transcript paths, model names, assistant messages,
prompts, responses, commands, command actions, approval decisions, environment
values, action URLs, and unknown fields never enter the canonical event.
Unsupported versions and interfaces fail closed.

`CodexCapabilityReport` is the shared read-only source for adapter selection,
installation behavior, and `doctor codex`. For 0.144.5 it reports task
completion as supported only through the CLI hook and approval requests as
supported only through app-server; the CLI approval hook remains unverified.
