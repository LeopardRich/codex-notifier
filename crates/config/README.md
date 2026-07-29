# config

Implements deterministic configuration layering in this order: built-in
defaults, user TOML, profile TOML, then explicit CLI overrides. The crate also
provides host-independent Windows, macOS, and XDG path resolution, version 0 to
version 1 migration, bounded validation, stable safe errors, prohibited
sensitive-field checks, and redacted diagnostic summaries.

The state-directory writability check is an injected interface so validation
tests do not depend on the test host's accounts or filesystem permissions.
Relay settings include a fixed OpenSSH alias and connection timeout plus
bounded initial delay, maximum delay, and attempt-count controls for durable
exponential retry.
