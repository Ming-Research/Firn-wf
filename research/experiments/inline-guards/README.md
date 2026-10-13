# Inline guard measurement branch

This branch is measurement-only and must never merge. It estimates what Firn
could gain after Whitefoot fixes `firn-req-helper-guard-wake-filter`: waiting
guards expressed through source helpers currently receive no selective wake
filter. It uses direct Meta fields for the writer and rebuilding waits and a
reply-ready sequence comparison for the connection wait, maintaining their
summaries in the same atomic statements as the original inputs.

The reply summary is the maximum written-through or failure-release sequence.
At the maximum u64 value it is clamped one below maximum while either buffer
has bytes and no append error is saved. A reachable saturated reply retains
bytes until a complete append or failure publishes that maximum; buffer drain
and watermark publication remain atomic. The original helper still governs
nonwaiting reply coalescing. Reply ordering, append/retry/sync behavior and
atomic boundaries are unchanged. The compiler pin and submodules are unchanged.

Network and persistence tests must pass in CI before measurements are used.
No build, test or measurement runs locally, and this README claims no result.
