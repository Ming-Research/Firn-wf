# Redis suite

The reference Redis's own test suite as firn's gate: what makes the gate fail, which tests it skips, how the suite is adjusted, and how its lists are recorded.

### decision redis-suite-gate-is-a-per-test-ratchet
The gate runs Redis 7.2.16's own suite against firn, without tolerating the framework's commands, and fails unless every test in `tests/redis-suite/passing.tsv` passes.
- rejected: an agent proposed gating on an aggregate count of passing tests; the owner rejected it, because a lost pass must fail the gate even when another test starts passing.
- rejected: an agent proposed requiring the entire suite to pass; the owner rejected it, because the gate could then not protect the compatibility firn already has until firn passed every test.

### decision redis-suite-gate-skips-known-hung-tests
The gate skips from the start each test that `tests/redis-suite/hung.tsv` lists with its evidence, a test the suite's own timeout has seen making no progress.
- rejected: an agent proposed raising the gate's time limit, or shortening the timeout every test gets; the owner rejected it, because each hung test costs the timeout and a rerun of its unit on every run, which had pushed the gate past its time limit.

### decision redis-suite-pubsub-barrier-patch
Before each run, for every target, a zero-fuzz forward-only patch makes the suite's five argument-free pub/sub removals drain and check their known acknowledgments.
- rejected: an agent proposed excluding the affected tests, or changing their wire commands or expected recipient counts; the owner rejected it, because the patch keeps every original command and recipient-count assertion, and firn and Redis run the same patched tests.

### decision redis-suite-blocked-client-fixture-patch
The same patch makes the suite's two BRPOPLPUSH/WATCH fixtures close their deferred clients even when their body raises, and wait until the blocked-client count is back to its start.
- rejected: an agent proposed changing server list semantics, weakening the later assertion or making WATCH succeed; the owner rejected it, because the fixtures otherwise leave two infinite list waiters that consume the setup values of a later RPOPLPUSH case.

### decision redis-suite-recording-skips-hangs-and-reports-them-apart
The recording runs skip the listed hung tests as the gate does; one more run of their units without the list names a test a candidate to leave it only if it passes.
- replaced: recording every run without the list was on main; the owner changed it, because a listed test run anyway can fail instead of hanging and leave its connection's state, such as an open MULTI, to the tests after it, so those runs lose passes the gate keeps.
- rejected: an agent proposed reading an error or a failure in the run without the list as a test that stopped hanging; the owner rejected it, because a test left inside MULTI by an earlier test errors at once, whether or not it would still hang.