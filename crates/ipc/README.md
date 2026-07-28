# ipc

Implements per-user Windows named-pipe and macOS/Linux Unix-domain-socket
transport using `interprocess`. Windows listeners use an owner-only security
descriptor. Unix endpoints use an owned `0700` directory, a `0600` socket, and
effective-user peer credential checks.

The protocol carries exactly one bounded length-prefixed canonical event and
one bounded structured acknowledgement per connection. Connect, read, and
write deadlines, a connection semaphore, bidirectional peer checks, secure
stale endpoint recovery, and strict endpoint names keep the local path bounded.
No proxy library or proxy environment variable is consulted.
