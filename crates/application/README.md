# application

Defines application use cases and ports without depending on concrete platform
adapters. Its current implementation establishes the role-aware agent
lifecycle, durable queue and asynchronous delivery ports, fixed worker and
shutdown bounds, cooperative cancellation, lease-safe forced cancellation, and
the structured event-log contract.

Event log records can contain only the canonical event ID and kind, typed
status, bounded duration, validated correlation ID, and validated safe error
code. Display text, source labels, paths, commands, and raw payloads are not
fields in the model at any log level.

The runtime initializes exactly one desktop or relay delivery graph, accepts
events only while ready, and uses the durable queue as its backpressure boundary
instead of creating an in-memory task per event. Queue adapters can report the
next durable availability time, so delayed retries and expired leases wake the
worker set without a new submission. Retry transitions distinguish a scheduled
attempt from attempt exhaustion that has already produced a dead letter.
