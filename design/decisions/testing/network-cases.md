# Network cases

How the network cases' harness runs its test process and the programs it starts.

### decision network-test-process-blocks-sigchld
The network cases' test process blocks SIGCHLD in every thread, and every child it starts gets the signal mask the test process started with.
- rejected: the owner proposed retrying a socket read that fails with EINTR; the owner rejected it, because a retry on failure covers the failure up instead of removing its cause.
