# ssh-transport

Stage 15 implements the restricted receive boundary: exact forced-session
validation, one bounded canonical event from stdin, compact structured
acknowledgements, host-key enrollment checks through the system OpenSSH tools,
and platform permission checks for authorized-key files.

It does not embed an SSH server or implement relay delivery. The Stage 16
sender will remain a separate adapter. Setup and forced-command templates are
documented in [`docs/restricted-ssh.md`](../../docs/restricted-ssh.md).
