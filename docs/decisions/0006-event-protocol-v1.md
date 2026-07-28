# ADR-0006: Canonical event protocol version 1

- Status: Accepted
- Date: 2026-07-29
- Owners: Project maintainers

## Context

Local hooks, IPC, SQLite, SSH, and native adapters need one stable event model.
Unbounded or loosely versioned JSON would expand every trust boundary.

## Decision

Adopt the canonical JSON envelope and acknowledgement defined in
[`../event-protocol-v1.md`](../event-protocol-v1.md). The essential rules are:

- `schema_version` is exactly `1` and `event_id` is a canonical UUIDv7.
- Only `approval_requested` and `task_completed` are valid kinds.
- The UTF-8 encoded envelope is at most 16,384 bytes.
- Unknown top-level fields, schema versions, and event kinds are rejected.
- Forward metadata is allowed only under bounded, namespaced `extensions`.
- Serialization is compact UTF-8 JSON with deterministic field ordering.
- Validation errors use stable safe codes and never echo the input.

## Alternatives

Protobuf and MessagePack were rejected because JSON is directly supported by
Codex hooks, stdin, diagnostics, and fixtures. Permissive unknown top-level
fields were rejected because they obscure compatibility and size accounting.

## Consequences

Adapters must map raw events through a strict allowlist. Adding a required
field or event kind requires a new schema version. Extensions can carry small
optional metadata without changing core behavior.

## Security Impact

All untrusted ingress validates bytes before persistence or platform APIs.
Limits apply after UTF-8 encoding, preventing Unicode-length ambiguity.

## Compatibility Impact

Receivers accept only version 1 until explicitly upgraded. Persisted version 1
events remain byte- and semantic-compatible across retries.

## Verification

- Contract snapshots cover both event kinds and acknowledgements.
- Boundary tests cover every byte, scalar, count, and time limit.
- Rejection tests cover malformed JSON, unknown fields/version/kind, invalid
  UUID/time, oversized extensions, and oversized envelopes.
