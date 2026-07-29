# ssh-transport

Stages 15 and 16 implement both sides of the restricted system OpenSSH
boundary. The receiver validates the exact forced session, reads one bounded
canonical event from stdin, returns a compact structured acknowledgement, and
provides host-key and authorized-file diagnostics.

The relay adapter starts the system `ssh` executable with fixed arguments,
sends event bytes only through stdin, bounds both output streams, validates the
matching acknowledgement, and classifies timeout, network, authentication,
host-key, process, and remote-rejection failures for the durable agent. It does
not embed an SSH client/server or open a listener. Setup and forced-command
templates are documented in
[`docs/restricted-ssh.md`](../../docs/restricted-ssh.md); relay operation and
recovery are in [`docs/relay-ssh.md`](../../docs/relay-ssh.md).

Stage 17 adds a client-availability check and an empty-stdin receiver probe.
The latter accepts only the forced receiver's fixed non-retryable
`malformed_json` acknowledgement and therefore verifies authentication,
host-key policy, and reachability without submitting an event.
