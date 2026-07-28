# application

Defines application use cases and ports without depending on concrete platform
adapters. Its current implementation establishes the structured event-log
contract, fixed safe diagnostics, level filtering, bounded rotation/retention
policy, and a thread-safe in-memory log sink for deterministic tests.

Event log records can contain only the canonical event ID and kind, typed
status, bounded duration, validated correlation ID, and validated safe error
code. Display text, source labels, paths, commands, and raw payloads are not
fields in the model at any log level.
