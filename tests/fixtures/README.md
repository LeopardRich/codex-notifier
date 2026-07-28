# Test fixtures

This directory is reserved for sanitized, version-labelled Codex event
payloads. Fixtures must not contain prompts, responses, credentials, usernames,
absolute paths, or other machine-identifying data.

Stage 01 accepted two real, sanitized payload fixtures for Codex CLI 0.144.5 on
Windows 10 Pro 22H2:

- `codex-0.144.5-windows-cli-task-completed.json` records a real external
  `Stop` hook invocation from `codex exec`.
- `codex-0.144.5-windows-cli-approval-requested.json` records a real
  `item/commandExecution/requestApproval` request from `codex app-server`; the
  probe declined the request and did not execute the proposed command.

Each future fixture must include its Codex CLI version, interface, operating
system, capture date, and evidence that an external process was invoked. Before
committing, replace session and turn identifiers, transcript and working paths,
model names, commands and arguments, prompts, assistant messages, credentials,
usernames, and machine identifiers. Preserve only the field names, value types,
and non-sensitive enums required to implement the adapter contract.
