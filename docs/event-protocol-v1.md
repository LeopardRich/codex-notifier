# Canonical Event Protocol v1

Status: Frozen by ADR-0006 on 2026-07-29.

## Encoding

An event is one UTF-8 JSON object. Serialized size is measured on the complete
UTF-8 byte sequence and must not exceed 16,384 bytes. JSON numbers are integers;
floating-point values are forbidden. Strings must be valid Unicode, contain no
NUL or C0/C1 control characters, and are normalized to NFC before scalar-count
limits are applied.

Canonical serialization uses compact JSON, the field order shown below,
lowercase canonical UUID text, RFC 3339 UTC timestamps with millisecond
precision, and lexicographically sorted extension keys. Parsers do not rely on
object order. Unknown fields outside the `extensions` value are rejected.

## Envelope

| Field | Type | Required | Limit and rule |
| --- | --- | --- | --- |
| `schema_version` | integer | yes | Exactly `1`. |
| `event_id` | string | yes | Lowercase hyphenated UUIDv7, 36 ASCII bytes. Stable across retries. |
| `kind` | string enum | yes | `approval_requested` or `task_completed`. |
| `occurred_at` | string | yes | RFC 3339 UTC; at initial ingestion no more than 5 minutes in the future or 7 days in the past. Accepted queued events are not time-rejected again during retry. |
| `source` | object | yes | Exact fields defined below. |
| `presentation` | object | yes | Exact fields defined below. |
| `routing` | object or null | yes | Exact fields defined below. |
| `extensions` | object | yes | At most 32 entries and 4,096 encoded bytes. |

`source` fields:

| Field | Type | Required | Limit and rule |
| --- | --- | --- | --- |
| `host_label` | string | yes | 1-64 scalars; display label only, never a hostname used for transport. |
| `project_label` | string or null | yes | Null or 1-80 scalars; never an absolute path. |
| `session_id` | string or null | yes | Null or 1-128 ASCII characters. Adapters hash source identifiers when their raw value contains user or machine information. |

`presentation` fields:

| Field | Type | Required | Limit and rule |
| --- | --- | --- | --- |
| `title` | string | yes | 1-120 scalars. |
| `body` | string | yes | 1-512 scalars. |
| `urgency` | string enum | yes | `normal` or `high`. |
| `privacy` | string enum | yes | `private` or `public`; private is the configuration default. |

`routing` is null for the default desktop profile. Otherwise it contains only
`profile`, an ASCII string matching `[A-Za-z0-9][A-Za-z0-9_-]{0,63}`. It is
never interpreted as an address, hostname, command, URL, or path.

Every extension key is 3-64 ASCII characters and must match
`[a-z0-9][a-z0-9_-]*(\.[a-z0-9][a-z0-9_-]*)+`. Values may be null, booleans,
integers, strings, arrays, or objects with a maximum nesting depth of 4. Each
string is at most 256 scalars and each array/object is at most 16 elements.
Extensions are preserved across transport but ignored unless their namespace
is explicitly supported. They cannot override core fields or routing.

## Forbidden content

The envelope must not contain prompts, model output, environment variables,
credentials, private keys, access tokens, raw commands or arguments, absolute
working directories, action URLs, notification actions, or arbitrary transport
destinations. Source adapters use allowlisted mapping and discard all other raw
fields before constructing the envelope.

## Acknowledgement

An acknowledgement is a compact UTF-8 JSON object no larger than 2,048 bytes:

| Field | Type | Rule |
| --- | --- | --- |
| `schema_version` | integer | Exactly `1`. |
| `event_id` | string | Canonical UUIDv7 matching the request. |
| `status` | string enum | `accepted`, `duplicate`, `delivered`, or `rejected`. |
| `error` | object or null | Null unless rejected; exact fields `code`, `retryable`, and `message`. |

`error.code` is one of the stable codes below. `error.message` is a safe,
single-line explanation of at most 160 scalars and never echoes payload data,
paths, credentials, or stack traces.

## Validation errors

| Code | Meaning |
| --- | --- |
| `malformed_json` | Input is not one complete JSON object. |
| `payload_too_large` | Event or acknowledgement exceeds its byte limit. |
| `unsupported_schema` | `schema_version` is missing or not supported. |
| `unknown_field` | An object contains a field not defined by version 1. |
| `invalid_event_id` | `event_id` is not canonical UUIDv7. |
| `unknown_kind` | `kind` is not one of the two version 1 values. |
| `timestamp_out_of_range` | `occurred_at` is invalid or outside the initial-ingestion window. |
| `field_too_long` | A bounded string exceeds its scalar or ASCII limit. |
| `invalid_routing` | Routing is not null or a valid profile. |
| `invalid_extension` | Extension namespace, type, depth, count, or size is invalid. |

Validation order is total bytes, JSON syntax and duplicate keys, object shape,
schema version, required fields, scalar/type checks, semantic limits, then
extension limits. Implementations may stop at the first failure but must return
the corresponding stable code.
