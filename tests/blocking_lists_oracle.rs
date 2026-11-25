//! Sources: [main recording](https://github.com/Ming-Research/Firn-wf/actions/runs/38453128755)
//! and [negative recording](https://github.com/Ming-Research/Firn-wf/actions/runs/38453804021).
//! Recorded wire/AOF bytes and INFO values cite pinned Redis 7.2.16 CI artifacts.
//! Every expectation cites an exact recording; none remains ORACLE-pending.
//! The 144 added later come from the 20-batch remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json).
//! Failed or incomplete recorder batches supply no expected bytes.
//! Fractional timeout acceptance also awaits the identified Whitefoot gap.

use super::{Action, Scenario};
use Action::*;

pub(super) const SCENARIOS: &[Scenario] = &[
    Scenario {
        name: "immediate",
        clients: &["p"],
        aof: false,
        actions: &[
            // Redis 7.2.16 run 38453128755, ORACLE-pending-aof-immediate-timeout-exec-script-and-restart, reply 0.
            Command("p", &["RPUSH", "l", "a", "b"], b":2\r\n"),
            // Redis 7.2.16 run 38453128755, ORACLE-pending-aof-immediate-timeout-exec-script-and-restart, reply 1.
            Command("p", &["BLPOP", "l", "0"], b"*2\r\n$1\r\nl\r\n$1\r\na\r\n"),
            // Redis 7.2.16 run 38453128755, ORACLE-pending-aof-immediate-timeout-exec-script-and-restart, reply 2.
            Command("p", &["BRPOP", "l", "0"], b"*2\r\n$1\r\nl\r\n$1\r\nb\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-immediate, operation 8.
            Command("p", &["EXISTS", "l"], b":0\r\n"),
            // Redis 7.2.16 run 38453128755, ORACLE-pending-brpoplpush-errors-typecheck-timeout-and-empty-context, reply 8.
            Command("p", &["RPUSH", "s", "a", "b", "c"], b":3\r\n"),
            // Redis 7.2.16 run 38453128755, ORACLE-pending-brpoplpush-errors-typecheck-timeout-and-empty-context, reply 11.
            Command("p", &["BRPOPLPUSH", "s", "d", "0"], b"$1\r\nc\r\n"),
            // Redis 7.2.16 run 38453128755, ORACLE-pending-brpoplpush-errors-typecheck-timeout-and-empty-context, reply 13.
            Command("p", &["LRANGE", "d", "0", "-1"], b"*1\r\n$1\r\nc\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-immediate, operation 16.
            Command("p", &["RPUSH", "from-LEFT-LEFT", "a", "b"], b":2\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-immediate, operation 18.
            Command("p", &["BLMOVE", "from-LEFT-LEFT", "d", "LEFT", "LEFT", "0"], b"$1\r\na\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-immediate, operation 20.
            Command("p", &["LRANGE", "from-LEFT-LEFT", "0", "-1"], b"*1\r\n$1\r\nb\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-immediate, operation 22.
            Command("p", &["LRANGE", "d", "0", "-1"], b"*2\r\n$1\r\na\r\n$1\r\nc\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-immediate, operation 24.
            Command("p", &["RPUSH", "from-LEFT-RIGHT", "a", "b"], b":2\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-immediate, operation 26.
            Command("p", &["BLMOVE", "from-LEFT-RIGHT", "d", "LEFT", "RIGHT", "0"], b"$1\r\na\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-immediate, operation 28.
            Command("p", &["LRANGE", "from-LEFT-RIGHT", "0", "-1"], b"*1\r\n$1\r\nb\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-immediate, operation 30.
            Command("p", &["LRANGE", "d", "0", "-1"], b"*3\r\n$1\r\na\r\n$1\r\nc\r\n$1\r\na\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-immediate, operation 32.
            Command("p", &["RPUSH", "from-RIGHT-LEFT", "a", "b"], b":2\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-immediate, operation 34.
            Command("p", &["BLMOVE", "from-RIGHT-LEFT", "d", "RIGHT", "LEFT", "0"], b"$1\r\nb\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-immediate, operation 36.
            Command("p", &["LRANGE", "from-RIGHT-LEFT", "0", "-1"], b"*1\r\n$1\r\na\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-immediate, operation 38.
            Command("p", &["LRANGE", "d", "0", "-1"], b"*4\r\n$1\r\nb\r\n$1\r\na\r\n$1\r\nc\r\n$1\r\na\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-immediate, operation 40.
            Command("p", &["RPUSH", "from-RIGHT-RIGHT", "a", "b"], b":2\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-immediate, operation 42.
            Command("p", &["BLMOVE", "from-RIGHT-RIGHT", "d", "RIGHT", "RIGHT", "0"], b"$1\r\nb\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-immediate, operation 44.
            Command("p", &["LRANGE", "from-RIGHT-RIGHT", "0", "-1"], b"*1\r\n$1\r\na\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-immediate, operation 46.
            Command("p", &["LRANGE", "d", "0", "-1"], b"*5\r\n$1\r\nb\r\n$1\r\na\r\n$1\r\nc\r\n$1\r\na\r\n$1\r\nb\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-immediate, operation 48.
            Command("p", &["BRPOPLPUSH", "d", "d", "0"], b"$1\r\nb\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-immediate, operation 50.
            Command("p", &["LRANGE", "d", "0", "-1"], b"*5\r\n$1\r\nb\r\n$1\r\nb\r\n$1\r\na\r\n$1\r\nc\r\n$1\r\na\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s10-destination-error-brpoplpush-resp2, operation 4.
            Command("p", &["SET", "bad", "string"], b"+OK\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-immediate, operation 54.
            Command("p", &["BLPOP", "bad", "0"], b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-immediate, operation 56.
            Command("p", &["BLMOVE", "s", "bad", "LEFT", "RIGHT", "0"], b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-immediate, operation 58.
            Command("p", &["LRANGE", "s", "0", "-1"], b"*2\r\n$1\r\na\r\n$1\r\nb\r\n"),
            // Redis 7.2.16 run 38453128755, ORACLE-pending-blpop-exec-and-script, reply 14.
            Command("p", &["PING"], b"+PONG\r\n"),
        ],
    },
    Scenario {
        name: "fifo",
        clients: &["p", "a", "b", "c"],
        aof: false,
        actions: &[
            Send("a", &["BLPOP", "l", "0"]),
            WaitBlocked(1),
            Send("b", &["BRPOP", "l", "0"]),
            WaitBlocked(2),
            Send("c", &["BLPOP", "l", "0"]),
            WaitBlocked(3),
            Quiet("a"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s03-fifo-rpush-resp2, operation 11.
            Command("p", &["RPUSH", "l", "a", "b", "c", "d"], b":4\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s03-fifo-rpush-resp2, operation 12.
            Read("a", b"*2\r\n$1\r\nl\r\n$1\r\na\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s03-fifo-rpush-resp2, operation 13.
            Read("b", b"*2\r\n$1\r\nl\r\n$1\r\nd\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s03-fifo-rpush-resp2, operation 14.
            Read("c", b"*2\r\n$1\r\nl\r\n$1\r\nb\r\n"),
            WaitBlocked(0),
            // Redis 7.2.16 run 38453128755, blocking-mc-s03-fifo-rpush-resp2, operation 19.
            Command("p", &["LRANGE", "l", "0", "-1"], b"*1\r\n$1\r\nc\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s07-exec-push-delete-string-resp2, operation 9.
            Command("p", &["DEL", "l"], b":1\r\n"),
            Send("a", &["BLPOP", "l", "0"]),
            WaitBlocked(1),
            Send("b", &["BRPOP", "l", "0"]),
            WaitBlocked(2),
            Send("c", &["BLPOP", "l", "0"]),
            WaitBlocked(3),
            // Redis 7.2.16 run 38453128755, blocking-mc-s03-fifo-lpush-resp2, operation 11.
            Command("p", &["LPUSH", "l", "a", "b", "c", "d"], b":4\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s03-fifo-lpush-resp2, operation 12.
            Read("a", b"*2\r\n$1\r\nl\r\n$1\r\nd\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s03-fifo-lpush-resp2, operation 13.
            Read("b", b"*2\r\n$1\r\nl\r\n$1\r\na\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s03-fifo-lpush-resp2, operation 14.
            Read("c", b"*2\r\n$1\r\nl\r\n$1\r\nc\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s03-fifo-lpush-resp2, operation 19.
            Command("p", &["LRANGE", "l", "0", "-1"], b"*1\r\n$1\r\nb\r\n"),
        ],
    },
    Scenario {
        name: "multikey",
        clients: &["p", "a", "b"],
        aof: false,
        actions: &[
            Send("a", &["BLPOP", "x", "y", "y", "x", "0"]),
            WaitBlocked(1),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-multikey, operation 6.
            Info("total_blocking_keys", "2"),
            Send("b", &["BRPOP", "y", "0"]),
            WaitBlocked(2),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-multikey, operation 10.
            Info("blocked_clients", "2"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-multikey, operation 12.
            Info("total_blocking_keys", "2"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-multikey, operation 14.
            Command("p", &["RPUSH", "y", "one", "two"], b":2\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-multikey, operation 15.
            Read("a", b"*2\r\n$1\r\ny\r\n$3\r\none\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-multikey, operation 16.
            Read("b", b"*2\r\n$1\r\ny\r\n$3\r\ntwo\r\n"),
            WaitBlocked(0),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-multikey, operation 19.
            Info("total_blocking_keys", "0"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-multikey, operation 21.
            Command("p", &["RPUSH", "x", "retained"], b":1\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-multikey, operation 23.
            Command("p", &["LRANGE", "x", "0", "-1"], b"*1\r\n$8\r\nretained\r\n"),
        ],
    },
    Scenario {
        name: "ready-rescan",
        clients: &["p", "a", "b"],
        aof: false,
        actions: &[
            Send("a", &["BLPOP", "x", "y", "0"]),
            WaitBlocked(1),
            Send("b", &["BLPOP", "y", "0"]),
            WaitBlocked(2),
            // Redis 7.2.16 run 38453128755, blocking-mc-s05-blpop-rescan-xy-resp2, operation 8 frame 0.
            Command("p", &["MULTI"], b"+OK\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s05-blpop-rescan-xy-resp2, operation 8 frame 1.
            Command("p", &["RPUSH", "y", "Y"], b"+QUEUED\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s05-blpop-rescan-xy-resp2, operation 8 frame 2.
            Command("p", &["RPUSH", "x", "X"], b"+QUEUED\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s05-blpop-rescan-xy-resp2, operation 8 frame 3.
            Command("p", &["EXEC"], b"*2\r\n:1\r\n:1\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s05-blpop-rescan-xy-resp2, operation 9.
            Read("a", b"*2\r\n$1\r\nx\r\n$1\r\nX\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s05-blpop-rescan-xy-resp2, operation 10.
            Read("b", b"*2\r\n$1\r\ny\r\n$1\r\nY\r\n"),
            WaitBlocked(0),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-ready-rescan, operation 19.
            Command("p", &["EXISTS", "x", "y"], b":0\r\n"),
        ],
    },
    Scenario {
        name: "execution-unit",
        clients: &["p", "a"],
        aof: false,
        actions: &[
            Send("a", &["BLPOP", "l", "0"]),
            WaitBlocked(1),
            // Redis 7.2.16 run 38453128755, blocking-mc-s07-exec-three-pushes-resp2, operation 5 frame 0.
            Command("p", &["MULTI"], b"+OK\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s07-exec-three-pushes-resp2, operation 5 frame 1.
            Command("p", &["LPUSH", "l", "a"], b"+QUEUED\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s07-exec-three-pushes-resp2, operation 5 frame 2.
            Command("p", &["LPUSH", "l", "b"], b"+QUEUED\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s07-exec-three-pushes-resp2, operation 5 frame 3.
            Command("p", &["LPUSH", "l", "c"], b"+QUEUED\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s07-exec-three-pushes-resp2, operation 5 frame 4.
            Command("p", &["EXEC"], b"*3\r\n:1\r\n:2\r\n:3\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s07-exec-three-pushes-resp2, operation 6.
            Read("a", b"*2\r\n$1\r\nl\r\n$1\r\nc\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s07-exec-three-pushes-resp2, operation 11.
            Command("p", &["LRANGE", "l", "0", "-1"], b"*2\r\n$1\r\nb\r\n$1\r\na\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s07-exec-push-delete-string-resp2, operation 9.
            Command("p", &["DEL", "l"], b":1\r\n"),
            Send("a", &["BLPOP", "l", "0"]),
            WaitBlocked(1),
            // Redis 7.2.16 run 38453128755, blocking-mc-s07-exec-push-delete-string-resp2, operation 5 frame 0.
            Command("p", &["MULTI"], b"+OK\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-execution-unit, operation 24.
            Command("p", &["LPUSH", "l", "transient"], b"+QUEUED\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s07-exec-push-delete-string-resp2, operation 5 frame 2.
            Command("p", &["DEL", "l"], b"+QUEUED\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s07-exec-push-delete-string-resp2, operation 5 frame 3.
            Command("p", &["SET", "l", "string"], b"+QUEUED\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s07-exec-push-delete-string-resp2, operation 5 frame 4.
            Command("p", &["EXEC"], b"*3\r\n:1\r\n:1\r\n+OK\r\n"),
            Quiet("a"),
            WaitBlocked(1),
            // Redis 7.2.16 run 38453128755, blocking-mc-s07-exec-push-delete-string-resp2, operation 9.
            Command("p", &["DEL", "l"], b":1\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-execution-unit, operation 36.
            Command("p", &["EVAL", "redis.call('LPUSH','l','first'); redis.call('LPUSH','l','last'); return 'done'", "0"], b"$4\r\ndone\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-execution-unit, operation 37.
            Read("a", b"*2\r\n$1\r\nl\r\n$4\r\nlast\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-execution-unit, operation 39.
            Command("p", &["LRANGE", "l", "0", "-1"], b"*1\r\n$5\r\nfirst\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-execution-unit, operation 41.
            Command("p", &["MULTI"], b"+OK\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-execution-unit, operation 43.
            Command("p", &["BLPOP", "empty", "0"], b"+QUEUED\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-execution-unit, operation 45.
            Command("p", &["BRPOP", "empty", "0"], b"+QUEUED\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-execution-unit, operation 47.
            Command("p", &["BRPOPLPUSH", "empty", "d", "0"], b"+QUEUED\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-execution-unit, operation 49.
            Command("p", &["BLMOVE", "empty", "d", "LEFT", "RIGHT", "0"], b"+QUEUED\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-execution-unit, operation 51.
            Command("p", &["EXEC"], b"*4\r\n*-1\r\n*-1\r\n$-1\r\n$-1\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-execution-unit, operation 53.
            Command("p", &["EVAL", "return {redis.call('BLPOP','empty','0'),redis.call('BRPOP','empty','0'),redis.call('BRPOPLPUSH','empty','d','0'),redis.call('BLMOVE','empty','d','LEFT','RIGHT','0')}", "0"], b"*4\r\n$-1\r\n$-1\r\n$-1\r\n$-1\r\n"),
            // Redis 7.2.16 run 38453128755, ORACLE-pending-blpop-exec-and-script, reply 14.
            Command("p", &["PING"], b"+PONG\r\n"),
        ],
    },
    Scenario {
        name: "moves",
        clients: &["p", "a", "b"],
        aof: false,
        actions: &[
            // Redis 7.2.16 run 38453128755, blocking-mc-s10-destination-error-brpoplpush-resp2, operation 4.
            Command("p", &["SET", "bad", "string"], b"+OK\r\n"),
            Send("a", &["BRPOPLPUSH", "s", "bad", "0"]),
            WaitBlocked(1),
            Send("b", &["BRPOPLPUSH", "s", "d", "0"]),
            WaitBlocked(2),
            // Redis 7.2.16 run 38453128755, blocking-mc-s10-destination-error-brpoplpush-resp2, operation 10.
            Command("p", &["RPUSH", "s", "a", "b", "c"], b":3\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s10-destination-error-brpoplpush-resp2, operation 11.
            Read("a", b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-moves, operation 12.
            Read("b", b"$1\r\nc\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s10-destination-error-brpoplpush-resp2, operation 19.
            Command("p", &["LRANGE", "s", "0", "-1"], b"*2\r\n$1\r\na\r\n$1\r\nb\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-moves, operation 16.
            Command("p", &["LRANGE", "d", "0", "-1"], b"*1\r\n$1\r\nc\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-moves, operation 18.
            Command("p", &["DEL", "s", "d"], b":2\r\n"),
            Send("a", &["BLMOVE", "s", "d", "RIGHT", "LEFT", "0"]),
            WaitBlocked(1),
            Send("b", &["BLMOVE", "d", "e", "LEFT", "RIGHT", "0"]),
            WaitBlocked(2),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-moves, operation 24.
            Command("p", &["RPUSH", "s", "linked"], b":1\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-moves, operation 25.
            Read("a", b"$6\r\nlinked\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-moves, operation 26.
            Read("b", b"$6\r\nlinked\r\n"),
            WaitBlocked(0),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-moves, operation 29.
            Command("p", &["EXISTS", "s", "d"], b":0\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-moves, operation 31.
            Command("p", &["LRANGE", "e", "0", "-1"], b"*1\r\n$6\r\nlinked\r\n"),
        ],
    },
    Scenario {
        name: "type-change",
        clients: &["p", "a"],
        aof: false,
        actions: &[
            Send("a", &["BLPOP", "x", "y", "0"]),
            WaitBlocked(1),
            // Redis 7.2.16 run 38453128755, blocking-mc-s09-wrongtype-earlier-key-rescan-resp2, operation 5.
            Command("p", &["SET", "x", "string"], b"+OK\r\n"),
            Quiet("a"),
            WaitBlocked(1),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-type-change, operation 9.
            Command("p", &["RPUSH", "y", "untouched"], b":1\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s09-wrongtype-earlier-key-rescan-resp2, operation 10.
            Read("a", b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n"),
            WaitBlocked(0),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-type-change, operation 13.
            Command("p", &["LRANGE", "y", "0", "-1"], b"*1\r\n$9\r\nuntouched\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-type-change, operation 15.
            Command("p", &["DEL", "x", "y"], b":2\r\n"),
            Send("a", &["BLPOP", "x", "0"]),
            WaitBlocked(1),
            // Redis 7.2.16 run 38453128755, blocking-mc-s09-wrongtype-replaced-with-list-resp2, operation 5.
            Command("p", &["SET", "x", "string"], b"+OK\r\n"),
            Quiet("a"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s09-wrongtype-replaced-with-list-resp2, operation 9.
            Command("p", &["DEL", "x"], b":1\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-type-change, operation 24.
            Command("p", &["RPUSH", "x", "final"], b":1\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-type-change, operation 25.
            Read("a", b"*2\r\n$1\r\nx\r\n$5\r\nfinal\r\n"),
        ],
    },
    Scenario {
        name: "resp3-pubsub",
        clients: &["p", "a"],
        aof: false,
        actions: &[
            // Redis 7.2.16 run 38453128755, blocking-mc-s18-subscribed-blpop-resp3, operation 1 HELLO proto field.
            Resp3("a", b":3\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s18-subscribed-blpop-resp3, operation 3.
            Command("a", &["SUBSCRIBE", "news"], b">3\r\n$9\r\nsubscribe\r\n$4\r\nnews\r\n:1\r\n"),
            Send("a", &["BLPOP", "l", "0"]),
            WaitBlocked(1),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-resp3-pubsub, operation 9.
            Command("p", &["PUBLISH", "news", "while-blocked"], b":1\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-resp3-pubsub, operation 10.
            Read("a", b">3\r\n$7\r\nmessage\r\n$4\r\nnews\r\n$13\r\nwhile-blocked\r\n"),
            WaitBlocked(1),
            // Redis 7.2.16 run 38453128755, blocking-mc-s18-subscribed-blpop-resp3, operation 11.
            Command("p", &["RPUSH", "l", "value"], b":1\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s18-subscribed-blpop-resp3, operation 12.
            Read("a", b"*2\r\n$1\r\nl\r\n$5\r\nvalue\r\n"),
            WaitBlocked(0),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-resp3-pubsub, operation 17.
            Command("a", &["UNSUBSCRIBE", "news"], b">3\r\n$11\r\nunsubscribe\r\n$4\r\nnews\r\n:0\r\n"),
            // Redis 7.2.16 run 38453128755, ORACLE-pending-blpop-exec-and-script, reply 14.
            Command("a", &["PING"], b"+PONG\r\n"),
        ],
    },
    Scenario {
        name: "disconnect",
        clients: &["p", "a", "b"],
        aof: false,
        actions: &[
            Send("a", &["BLPOP", "x", "y", "0"]),
            WaitBlocked(1),
            Send("b", &["BLPOP", "y", "0"]),
            WaitBlocked(2),
            Close("a"),
            WaitBlocked(1),
            // Redis 7.2.16 run 38453128755, blocking-mc-s16-disconnect-multikey-cleanup-resp2, operation 8 final INFO total_blocking_keys.
            Info("total_blocking_keys", "1"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s16-disconnect-multikey-cleanup-resp2, operation 10.
            Command("p", &["RPUSH", "y", "survivor"], b":1\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s16-disconnect-multikey-cleanup-resp2, operation 11.
            Read("b", b"*2\r\n$1\r\ny\r\n$8\r\nsurvivor\r\n"),
            WaitBlocked(0),
            // Redis 7.2.16 run 38453128755, blocking-mc-s16-disconnect-multikey-cleanup-resp2, operation 14.
            Command("p", &["RPUSH", "x", "retained"], b":1\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s16-disconnect-multikey-cleanup-resp2, operation 18.
            Command("p", &["LRANGE", "x", "0", "-1"], b"*1\r\n$8\r\nretained\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s16-disconnect-multikey-cleanup-resp2, operation 12 final INFO blocked_clients.
            Info("blocked_clients", "0"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s16-disconnect-multikey-cleanup-resp2, operation 12 final INFO total_blocking_keys.
            Info("total_blocking_keys", "0"),
        ],
    },
    Scenario {
        name: "client-unblock",
        clients: &["p", "a"],
        aof: false,
        actions: &[
            CaptureId("a"),
            Send("a", &["BLPOP", "queued", "0"]),
            WaitBlocked(1),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-client-unblock, operation 7.
            Command("p", &["MULTI"], b"+OK\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-client-unblock, operation 9.
            Command("p", &["CLIENT", "UNBLOCK", "@id:a", "TIMEOUT"], b"+QUEUED\r\n"),
            Quiet("a"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-client-unblock, operation 12.
            Command("p", &["EXEC"], b"*1\r\n:1\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-client-unblock, operation 13.
            Read("a", b"*-1\r\n"),
            WaitBlocked(0),
            Send("a", &["BLMOVE", "queued", "destination", "LEFT", "RIGHT", "0"]),
            WaitBlocked(1),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-client-unblock, operation 18.
            Command("p", &["MULTI"], b"+OK\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-client-unblock, operation 20.
            Command("p", &["CLIENT", "UNBLOCK", "@id:a", "ERROR"], b"+QUEUED\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-client-unblock, operation 22.
            Command("p", &["EXEC"], b"*1\r\n:1\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-client-unblock, operation 23.
            Read("a", b"-UNBLOCKED client unblocked via CLIENT UNBLOCK\r\n"),
            WaitBlocked(0),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-client-unblock, operation 26.
            Command("p", &["EXISTS", "queued", "destination"], b":0\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-client-unblock, operation 28.
            Command("p", &["MULTI"], b"+OK\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-client-unblock, operation 30.
            Command("p", &["CLIENT", "UNBLOCK", "@id:a"], b"+QUEUED\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-client-unblock, operation 32.
            Command("p", &["EXEC"], b"*1\r\n:0\r\n"),
            Send("a", &["BLPOP", "x", "y", "y", "0"]),
            WaitBlocked(1),
            // Redis 7.2.16 run 38453128755, blocking-mc-s15-client-unblock-lifecycle-resp2, operation 48 final INFO blocked_clients.
            Info("blocked_clients", "1"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s15-client-unblock-lifecycle-resp2, operation 48 final INFO total_blocking_keys.
            Info("total_blocking_keys", "2"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s15-client-unblock-lifecycle-resp2, operation 48 final INFO total_blocking_keys_on_nokey.
            Info("total_blocking_keys_on_nokey", "0"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s15-client-unblock-lifecycle-resp2, operation 10.
            Command("p", &["CLIENT", "UNBLOCK", "@id:a"], b":1\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s15-client-unblock-lifecycle-resp2, operation 11.
            Read("a", b"*-1\r\n"),
            WaitBlocked(0),
            // Redis 7.2.16 run 38453128755, blocking-mc-s15-client-unblock-lifecycle-resp2, operation 14.
            Command("p", &["CLIENT", "UNBLOCK", "@id:a"], b":0\r\n"),
            Send("a", &["BRPOPLPUSH", "s", "d", "0"]),
            WaitBlocked(1),
            // Redis 7.2.16 run 38453128755, blocking-mc-s15-client-unblock-lifecycle-resp2, operation 22.
            Command("p", &["CLIENT", "UNBLOCK", "@id:a", "TIMEOUT"], b":1\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s15-client-unblock-lifecycle-resp2, operation 23.
            Read("a", b"*-1\r\n"),
            Send("a", &["BLMOVE", "s", "d", "LEFT", "RIGHT", "0"]),
            WaitBlocked(1),
            // Redis 7.2.16 run 38453128755, blocking-mc-s15-client-unblock-lifecycle-resp2, operation 34.
            Command("p", &["CLIENT", "UNBLOCK", "@id:a", "ERROR"], b":1\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s15-client-unblock-lifecycle-resp2, operation 35.
            Read("a", b"-UNBLOCKED client unblocked via CLIENT UNBLOCK\r\n"),
            WaitBlocked(0),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-client-unblock, operation 59.
            Command("a", &["PING", "tail"], b"$4\r\ntail\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-client-unblock, operation 61.
            Command("p", &["RPUSH", "x", "one"], b":1\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-client-unblock, operation 63.
            Command("p", &["RPUSH", "y", "two"], b":1\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-client-unblock, operation 65.
            Command("p", &["EXISTS", "s", "d"], b":0\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s15-client-unblock-lifecycle-resp2, operation 52 final INFO blocked_clients.
            Info("blocked_clients", "0"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s15-client-unblock-lifecycle-resp2, operation 52 final INFO total_blocking_keys.
            Info("total_blocking_keys", "0"),
        ],
    },
    Scenario {
        name: "aof",
        clients: &["p", "a", "b"],
        aof: true,
        actions: &[
            Send("a", &["BLPOP", "l", "0"]),
            WaitBlocked(1),
            // Redis 7.2.16 run 38453128755, blocking-mc-s19-aof-delayed-blpop-resp2, operation 4 incremental files.
            Aof(b""),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-aof, operation 7.
            Command("p", &["RPUSH", "l", "popped", "left"], b":2\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-aof, operation 8.
            Read("a", b"*2\r\n$1\r\nl\r\n$6\r\npopped\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-aof, operation 10.
            Command("p", &["BRPOP", "l", "0"], b"*2\r\n$1\r\nl\r\n$4\r\nleft\r\n"),
            Send("a", &["BRPOPLPUSH", "s", "d", "0"]),
            WaitBlocked(1),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-aof, operation 14.
            Command("p", &["RPUSH", "s", "moved"], b":1\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-aof, operation 15.
            Read("a", b"$5\r\nmoved\r\n"),
            Send("a", &["BLMOVE", "s", "d", "LEFT", "RIGHT", "0"]),
            WaitBlocked(1),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-aof, operation 19.
            Command("p", &["RPUSH", "s", "second"], b":1\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-aof, operation 20.
            Read("a", b"$6\r\nsecond\r\n"),
            Send("a", &["BLMOVE", "from", "mid", "LEFT", "RIGHT", "0"]),
            WaitBlocked(1),
            Send("b", &["BLMOVE", "mid", "last", "RIGHT", "LEFT", "0"]),
            WaitBlocked(2),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-aof, operation 26.
            Command("p", &["RPUSH", "from", "chain"], b":1\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-aof, operation 27.
            Read("a", b"$5\r\nchain\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-aof, operation 28.
            Read("b", b"$5\r\nchain\r\n"),
            WaitBlocked(0),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-aof, operation 30.
            Aof(b"*2\r\n$6\r\nSELECT\r\n$1\r\n0\r\n*4\r\n$5\r\nRPUSH\r\n$1\r\nl\r\n$6\r\npopped\r\n$4\r\nleft\r\n*2\r\n$4\r\nLPOP\r\n$1\r\nl\r\n*2\r\n$4\r\nRPOP\r\n$1\r\nl\r\n*3\r\n$5\r\nRPUSH\r\n$1\r\ns\r\n$5\r\nmoved\r\n*3\r\n$9\r\nRPOPLPUSH\r\n$1\r\ns\r\n$1\r\nd\r\n*3\r\n$5\r\nRPUSH\r\n$1\r\ns\r\n$6\r\nsecond\r\n*5\r\n$5\r\nLMOVE\r\n$1\r\ns\r\n$1\r\nd\r\n$4\r\nLEFT\r\n$5\r\nRIGHT\r\n*3\r\n$5\r\nRPUSH\r\n$4\r\nfrom\r\n$5\r\nchain\r\n*5\r\n$5\r\nLMOVE\r\n$4\r\nfrom\r\n$3\r\nmid\r\n$4\r\nLEFT\r\n$5\r\nRIGHT\r\n*5\r\n$5\r\nLMOVE\r\n$3\r\nmid\r\n$4\r\nlast\r\n$5\r\nRIGHT\r\n$4\r\nLEFT\r\n"),
            Restart,
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-aof, operation 34.
            Command("p", &["LRANGE", "d", "0", "-1"], b"*2\r\n$5\r\nmoved\r\n$6\r\nsecond\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-aof, operation 36.
            Command("p", &["LRANGE", "last", "0", "-1"], b"*1\r\n$5\r\nchain\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-aof, operation 38.
            Command("p", &["EXISTS", "l", "s", "from", "mid"], b":0\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-aof, operation 39.
            Aof(b"*2\r\n$6\r\nSELECT\r\n$1\r\n0\r\n*4\r\n$5\r\nRPUSH\r\n$1\r\nl\r\n$6\r\npopped\r\n$4\r\nleft\r\n*2\r\n$4\r\nLPOP\r\n$1\r\nl\r\n*2\r\n$4\r\nRPOP\r\n$1\r\nl\r\n*3\r\n$5\r\nRPUSH\r\n$1\r\ns\r\n$5\r\nmoved\r\n*3\r\n$9\r\nRPOPLPUSH\r\n$1\r\ns\r\n$1\r\nd\r\n*3\r\n$5\r\nRPUSH\r\n$1\r\ns\r\n$6\r\nsecond\r\n*5\r\n$5\r\nLMOVE\r\n$1\r\ns\r\n$1\r\nd\r\n$4\r\nLEFT\r\n$5\r\nRIGHT\r\n*3\r\n$5\r\nRPUSH\r\n$4\r\nfrom\r\n$5\r\nchain\r\n*5\r\n$5\r\nLMOVE\r\n$4\r\nfrom\r\n$3\r\nmid\r\n$4\r\nLEFT\r\n$5\r\nRIGHT\r\n*5\r\n$5\r\nLMOVE\r\n$3\r\nmid\r\n$4\r\nlast\r\n$5\r\nRIGHT\r\n$4\r\nLEFT\r\n"),
        ],
    },
    Scenario {
        name: "fractional-timeout",
        clients: &["p"],
        aof: false,
        actions: &[
            // Redis 7.2.16 run 38453804021, ORACLE-pending-blpop-errors-and-finite-timeout, reply 12.
            Command("p", &["BLPOP", "missing", "0.001"], b"*-1\r\n"),
            // Redis 7.2.16 run 38453804021, ORACLE-pending-brpop-errors-and-finite-timeout, reply 14.
            Command("p", &["BRPOP", "missing", "0.02"], b"*-1\r\n"),
            // Redis 7.2.16 run 38453128755, ORACLE-pending-brpoplpush-errors-typecheck-timeout-and-empty-context, reply 14.
            Command("p", &["BRPOPLPUSH", "missing", "d", "0.001"], b"*-1\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-fractional-timeout, operation 8.
            Command("p", &["BLMOVE", "missing", "d", "LEFT", "RIGHT", "0.001"], b"*-1\r\n"),
            // Redis rejects nan before any key lookup, so the recorded absent key k and missing give the same error (t_list.c:1218).
            // Redis 7.2.16 run 38453804021, ORACLE-pending-blpop-errors-and-finite-timeout, reply 4.
            Command("p", &["BLPOP", "missing", "nan"], b"-ERR timeout is not a float or out of range\r\n"),
            // Redis 7.2.16 run 38453128755, ORACLE-pending-blpop-exec-and-script, reply 14.
            Command("p", &["PING"], b"+PONG\r\n"),
        ],
    },
    Scenario {
        name: "waiter-pipeline",
        clients: &["p", "a"],
        aof: false,
        actions: &[
            Pipeline("a", &[&["BLPOP", "l", "0"], &["PING", "tail"], &["RPUSH", "after", "value"]]),
            WaitBlocked(1),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-waiter-pipeline, operation 5.
            Command("p", &["EXISTS", "after"], b":0\r\n"),
            Quiet("a"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s02-waiter-pipeline-producer-resp2, operation 10.
            Command("p", &["RPUSH", "l", "first"], b":1\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s02-waiter-pipeline-producer-resp2, operation 11 frame 0.
            Read("a", b"*2\r\n$1\r\nl\r\n$5\r\nfirst\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s02-waiter-pipeline-producer-resp2, operation 11 frame 1.
            Read("a", b"$4\r\ntail\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s02-waiter-pipeline-producer-resp2, operation 11 frame 2.
            Read("a", b":1\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-waiter-pipeline, operation 13.
            Command("p", &["LRANGE", "after", "0", "-1"], b"*1\r\n$5\r\nvalue\r\n"),
        ],
    },
    Scenario {
        name: "completion-reuse",
        clients: &["p", "a"],
        aof: false,
        actions: &[
            CaptureId("a"),
            Pipeline("a", &[&["BLPOP", "x", "0"], &["BRPOPLPUSH", "y", "d", "0"], &["PING"]]),
            WaitBlocked(1),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-completion-reuse, operation 7.
            Command("p", &["RPUSH", "x", "first"], b":1\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-completion-reuse, operation 8.
            Read("a", b"*2\r\n$1\r\nx\r\n$5\r\nfirst\r\n"),
            WaitBlocked(1),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-completion-reuse, operation 11.
            Command("p", &["CLIENT", "UNBLOCK", "@id:a", "ERROR"], b":1\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-completion-reuse, operation 12.
            Read("a", b"-UNBLOCKED client unblocked via CLIENT UNBLOCK\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-completion-reuse, operation 13.
            Read("a", b"+PONG\r\n"),
            WaitBlocked(0),
            Quiet("a"),
            Send("a", &["BLPOP", "z", "0"]),
            WaitBlocked(1),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-completion-reuse, operation 19.
            Command("p", &["RPUSH", "z", "final"], b":1\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-completion-reuse, operation 20.
            Read("a", b"*2\r\n$1\r\nz\r\n$5\r\nfinal\r\n"),
            WaitBlocked(0),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-completion-reuse, operation 23.
            Command("p", &["LRANGE", "d", "0", "-1"], b"*0\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-completion-reuse, operation 25.
            Command("a", &["PING"], b"+PONG\r\n"),
        ],
    },
    Scenario {
        name: "producer-pipeline",
        clients: &["p", "a", "b"],
        aof: false,
        actions: &[
            Send("a", &["BLPOP", "l", "0"]),
            WaitBlocked(1),
            Pipeline("b", &[&["LPUSH", "l", "first"], &["BLPOP", "l", "0"]]),
            // Redis 7.2.16 run 38453128755, blocking-mc-s06-producer-pipeline-fairness-resp2, operation 6.
            Read("a", b"*2\r\n$1\r\nl\r\n$5\r\nfirst\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s06-producer-pipeline-fairness-resp2, operation 7.
            Read("b", b":1\r\n"),
            WaitBlocked(1),
            Quiet("b"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s06-producer-pipeline-fairness-resp2, operation 11.
            Command("p", &["LPUSH", "l", "second"], b":1\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s06-producer-pipeline-fairness-resp2, operation 12.
            Read("b", b"*2\r\n$1\r\nl\r\n$6\r\nsecond\r\n"),
            WaitBlocked(0),
        ],
    },
    Scenario {
        name: "other-producers",
        clients: &["p", "a"],
        aof: false,
        actions: &[
            Send("a", &["BLPOP", "target", "0"]),
            WaitBlocked(1),
            // Redis 7.2.16 run 38453128755, blocking-mc-s13-producer-rename-resp2, operation 3.
            Command("p", &["RPUSH", "source", "a", "b"], b":2\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s13-producer-rename-resp2, operation 7.
            Command("p", &["RENAME", "source", "target"], b"+OK\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s13-producer-rename-resp2, operation 8.
            Read("a", b"*2\r\n$6\r\ntarget\r\n$1\r\na\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s13-producer-rename-resp2, operation 17.
            Command("p", &["LRANGE", "target", "0", "-1"], b"*1\r\n$1\r\nb\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-other-producers, operation 12.
            Command("p", &["DEL", "target"], b":1\r\n"),
            Send("a", &["BRPOP", "target", "0"]),
            WaitBlocked(1),
            // Redis 7.2.16 run 38453128755, blocking-mc-s13-producer-rename-resp2, operation 3.
            Command("p", &["RPUSH", "source", "a", "b"], b":2\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s13-producer-copy-resp2, operation 7.
            Command("p", &["COPY", "source", "target"], b":1\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-other-producers, operation 19.
            Read("a", b"*2\r\n$6\r\ntarget\r\n$1\r\nb\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s13-producer-copy-resp2, operation 13.
            Command("p", &["LRANGE", "source", "0", "-1"], b"*2\r\n$1\r\na\r\n$1\r\nb\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-other-producers, operation 23.
            Command("p", &["DEL", "target"], b":1\r\n"),
            Send("a", &["BLPOP", "target", "0"]),
            WaitBlocked(1),
            // Redis 7.2.16 run 38453128755, blocking-mc-s13-producer-lmove-resp2, operation 7.
            Command("p", &["LMOVE", "source", "target", "LEFT", "RIGHT"], b"$1\r\na\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s13-producer-lmove-resp2, operation 8.
            Read("a", b"*2\r\n$6\r\ntarget\r\n$1\r\na\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-other-producers, operation 30.
            Command("p", &["EXISTS", "target"], b":0\r\n"),
        ],
    },
    Scenario {
        name: "nested-fifo",
        clients: &["p", "a", "b", "c"],
        aof: false,
        actions: &[
            Send("a", &["BRPOPLPUSH", "l1", "l3", "0"]),
            WaitBlocked(1),
            Send("b", &["BLPOP", "l2", "0"]),
            WaitBlocked(2),
            Send("c", &["BLPOP", "l2", "l3", "0"]),
            WaitBlocked(3),
            // Redis 7.2.16 run 38453128755, blocking-mc-s12-nested-fifo-brpoplpush-resp2, operation 11 frame 0.
            Command("p", &["MULTI"], b"+OK\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s12-nested-fifo-brpoplpush-resp2, operation 11 frame 1.
            Command("p", &["LPUSH", "l1", "one"], b"+QUEUED\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s12-nested-fifo-brpoplpush-resp2, operation 11 frame 2.
            Command("p", &["LPUSH", "l2", "two"], b"+QUEUED\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s12-nested-fifo-brpoplpush-resp2, operation 11 frame 3.
            Command("p", &["EXEC"], b"*2\r\n:1\r\n:1\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s12-nested-fifo-brpoplpush-resp2, operation 12.
            Read("a", b"$3\r\none\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s12-nested-fifo-brpoplpush-resp2, operation 13.
            Read("b", b"*2\r\n$2\r\nl2\r\n$3\r\ntwo\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s12-nested-fifo-brpoplpush-resp2, operation 14.
            Read("c", b"*2\r\n$2\r\nl3\r\n$3\r\none\r\n"),
            WaitBlocked(0),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-nested-fifo, operation 23.
            Command("p", &["EXISTS", "l1", "l2", "l3"], b":0\r\n"),
        ],
    },
    Scenario {
        name: "expiry-before-ready",
        clients: &["p", "a"],
        aof: true,
        actions: &[
            Send("a", &["BLPOP", "l", "0"]),
            WaitBlocked(1),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-expiry-before-ready, operation 5.
            Command("p", &["MULTI"], b"+OK\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-expiry-before-ready, operation 7.
            Command("p", &["RPUSH", "l", "transient"], b"+QUEUED\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-expiry-before-ready, operation 9.
            Command("p", &["PEXPIREAT", "l", "1"], b"+QUEUED\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-expiry-before-ready, operation 11.
            Command("p", &["EXEC"], b"*2\r\n:1\r\n:1\r\n"),
            Quiet("a"),
            WaitBlocked(1),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-expiry-before-ready, operation 15.
            Command("p", &["EXISTS", "l"], b":0\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-expiry-before-ready, operation 16.
            Aof(b"*1\r\n$5\r\nMULTI\r\n*2\r\n$6\r\nSELECT\r\n$1\r\n0\r\n*3\r\n$5\r\nRPUSH\r\n$1\r\nl\r\n$9\r\ntransient\r\n*2\r\n$3\r\nDEL\r\n$1\r\nl\r\n*1\r\n$4\r\nEXEC\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-expiry-before-ready, operation 18.
            Command("p", &["RPUSH", "l", "final"], b":1\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-expiry-before-ready, operation 19.
            Read("a", b"*2\r\n$1\r\nl\r\n$5\r\nfinal\r\n"),
            WaitBlocked(0),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-expiry-before-ready, operation 21.
            Aof(b"*1\r\n$5\r\nMULTI\r\n*2\r\n$6\r\nSELECT\r\n$1\r\n0\r\n*3\r\n$5\r\nRPUSH\r\n$1\r\nl\r\n$9\r\ntransient\r\n*2\r\n$3\r\nDEL\r\n$1\r\nl\r\n*1\r\n$4\r\nEXEC\r\n*3\r\n$5\r\nRPUSH\r\n$1\r\nl\r\n$5\r\nfinal\r\n*2\r\n$4\r\nLPOP\r\n$1\r\nl\r\n"),
        ],
    },
    Scenario {
        name: "resp2-subscriber-refusal",
        clients: &["p"],
        aof: false,
        actions: &[
            // Redis 7.2.16 run 38453128755, ORACLE-pending-resp2-subscribed-blocking-command-refusals, reply 0.
            Command("p", &["SUBSCRIBE", "news"], b"*3\r\n$9\r\nsubscribe\r\n$4\r\nnews\r\n:1\r\n"),
            // Redis 7.2.16 run 38453128755, ORACLE-pending-resp2-subscribed-blocking-command-refusals, reply 1.
            Command("p", &["BLPOP", "l", "0"], b"-ERR Can't execute 'blpop': only (P|S)SUBSCRIBE / (P|S)UNSUBSCRIBE / PING / QUIT / RESET are allowed in this context\r\n"),
            // Redis 7.2.16 run 38453128755, ORACLE-pending-resp2-subscribed-blocking-command-refusals, reply 2.
            Command("p", &["BRPOP", "l", "0"], b"-ERR Can't execute 'brpop': only (P|S)SUBSCRIBE / (P|S)UNSUBSCRIBE / PING / QUIT / RESET are allowed in this context\r\n"),
            // Redis 7.2.16 run 38453128755, ORACLE-pending-resp2-subscribed-blocking-command-refusals, reply 3.
            Command("p", &["BRPOPLPUSH", "s", "d", "0"], b"-ERR Can't execute 'brpoplpush': only (P|S)SUBSCRIBE / (P|S)UNSUBSCRIBE / PING / QUIT / RESET are allowed in this context\r\n"),
            // Redis 7.2.16 run 38453128755, ORACLE-pending-resp2-subscribed-blocking-command-refusals, reply 4.
            Command("p", &["BLMOVE", "s", "d", "LEFT", "RIGHT", "0"], b"-ERR Can't execute 'blmove': only (P|S)SUBSCRIBE / (P|S)UNSUBSCRIBE / PING / QUIT / RESET are allowed in this context\r\n"),
            // Redis 7.2.16 run 38453128755, ORACLE-pending-resp2-subscribed-blocking-command-refusals, reply 5.
            Command("p", &["BLPOP"], b"-ERR wrong number of arguments for 'blpop' command\r\n"),
            // Redis 7.2.16 run 38453128755, ORACLE-pending-resp2-subscribed-blocking-command-refusals, reply 6.
            Command("p", &["UNSUBSCRIBE", "news"], b"*3\r\n$11\r\nunsubscribe\r\n$4\r\nnews\r\n:0\r\n"),
            // Redis 7.2.16 run 38453128755, ORACLE-pending-resp2-subscribed-blocking-command-refusals, reply 8.
            Command("p", &["PING"], b"+PONG\r\n"),
        ],
    },
    Scenario {
        name: "existing-list-producers-in-exec",
        clients: &["p", "a"],
        aof: false,
        actions: &[
            Send("a", &["BLPOP", "l", "0"]),
            WaitBlocked(1),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-existing-list-producers-in-exec, operation 5.
            Command("p", &["LPUSHX", "l", "absent"], b":0\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-existing-list-producers-in-exec, operation 7.
            Command("p", &["RPUSHX", "l", "absent"], b":0\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-existing-list-producers-in-exec, operation 9.
            Command("p", &["LINSERT", "l", "AFTER", "absent", "value"], b":0\r\n"),
            Quiet("a"),
            WaitBlocked(1),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-existing-list-producers-in-exec, operation 13.
            Command("p", &["MULTI"], b"+OK\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-existing-list-producers-in-exec, operation 15.
            Command("p", &["RPUSH", "l", "first"], b"+QUEUED\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-existing-list-producers-in-exec, operation 17.
            Command("p", &["LPUSHX", "l", "left"], b"+QUEUED\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-existing-list-producers-in-exec, operation 19.
            Command("p", &["RPUSHX", "l", "right"], b"+QUEUED\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-existing-list-producers-in-exec, operation 21.
            Command("p", &["LINSERT", "l", "AFTER", "first", "inserted"], b"+QUEUED\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-existing-list-producers-in-exec, operation 23.
            Command("p", &["EXEC"], b"*4\r\n:1\r\n:2\r\n:3\r\n:4\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-existing-list-producers-in-exec, operation 24.
            Read("a", b"*2\r\n$1\r\nl\r\n$4\r\nleft\r\n"),
            WaitBlocked(0),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-existing-list-producers-in-exec, operation 27.
            Command("p", &["LRANGE", "l", "0", "-1"], b"*3\r\n$5\r\nfirst\r\n$8\r\ninserted\r\n$5\r\nright\r\n"),
        ],
    },
    Scenario {
        name: "promoted-unblock-order",
        clients: &["p", "a", "b"],
        aof: false,
        actions: &[
            CaptureId("a"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s18-subscribed-blpop-resp3, operation 1.
            Resp3("b", b":3\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s18-subscribed-blpop-resp3, operation 3.
            Command("b", &["SUBSCRIBE", "news"], b">3\r\n$9\r\nsubscribe\r\n$4\r\nnews\r\n:1\r\n"),
            Pipeline("a", &[&["BLPOP", "l", "0"], &["PUBLISH", "news", "after-unblock"]]),
            WaitBlocked(1),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-promoted-unblock-order, operation 12.
            Command("b", &["CLIENT", "UNBLOCK", "@id:a", "TIMEOUT"], b":1\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-promoted-unblock-order, operation 13.
            Read("a", b"*-1\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-promoted-unblock-order, operation 14.
            Read("a", b":1\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-promoted-unblock-order, operation 15.
            Read("b", b">3\r\n$7\r\nmessage\r\n$4\r\nnews\r\n$13\r\nafter-unblock\r\n"),
            WaitBlocked(0),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-promoted-unblock-order, operation 18.
            Command("b", &["UNSUBSCRIBE", "news"], b">3\r\n$11\r\nunsubscribe\r\n$4\r\nnews\r\n:0\r\n"),
            // Redis 7.2.16 run 38453128755, ORACLE-pending-blpop-exec-and-script, reply 14.
            Command("b", &["PING"], b"+PONG\r\n"),
        ],
    },
    Scenario {
        name: "script-error-and-declared-fallback",
        clients: &["p", "a"],
        aof: false,
        actions: &[
            Send("a", &["BLPOP", "l", "0"]),
            WaitBlocked(1),
            // Redis 7.2.16 run 38453128755, blocking-mc-s08-script-push-explicit-error-resp2, operation 5.
            Command("p", &["EVAL", "redis.call('RPUSH','l','kept'); error('after push')", "0"], b"-ERR user_script:1: after push script: d6274faf496d9a17c55556675937350c0259875f, on @user_script:1.\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s08-script-push-explicit-error-resp2, operation 6.
            Read("a", b"*2\r\n$1\r\nl\r\n$4\r\nkept\r\n"),
            WaitBlocked(0),
            // Redis 7.2.16 run 38453128755, blocking-mc-s08-script-push-explicit-error-resp2, operation 11.
            Command("p", &["LRANGE", "l", "0", "-1"], b"*0\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s08-script-declared-counter-retry-resp2, operation 3.
            Command("p", &["SET", "counter", "0"], b"+OK\r\n"),
            Send("a", &["BLPOP", "fallback-list", "0"]),
            WaitBlocked(1),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-script-error-and-declared-fallback, operation 15.
            Command("p", &["EVAL", "redis.call('INCR',KEYS[1]); redis.call('RPUSH',KEYS[2],'once'); return redis.call('GET',KEYS[1])", "2", "counter", "fallback-list"], b"$1\r\n1\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-script-error-and-declared-fallback, operation 16.
            Read("a", b"*2\r\n$13\r\nfallback-list\r\n$4\r\nonce\r\n"),
            WaitBlocked(0),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-script-error-and-declared-fallback, operation 19.
            Command("p", &["GET", "counter"], b"$1\r\n1\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-script-error-and-declared-fallback, operation 21.
            Command("p", &["LRANGE", "fallback-list", "0", "-1"], b"*0\r\n"),
            // Redis 7.2.16 run 38453128755, ORACLE-pending-blpop-exec-and-script, reply 14.
            Command("p", &["PING"], b"+PONG\r\n"),
        ],
    },
    Scenario {
        name: "blocking-argument-precedence",
        clients: &["p"],
        aof: false,
        actions: &[
            // Redis rejects the first bad direction before reading the other direction or timeout (t_list.c:1309-1318).
            // Redis 7.2.16 run 38453128755, ORACLE-pending-blmove-errors-typecheck-timeout-and-empty-context, reply 2.
            Command("p", &["BLMOVE", "s", "d", "bad", "LEFT", "invalid"], b"-ERR syntax error\r\n"),
            // Redis 7.2.16 run 38453128755, ORACLE-pending-blpop-exec-and-script, reply 14.
            Command("p", &["PING"], b"+PONG\r\n"),
        ],
    },
    Scenario {
        name: "rounded-negative-zero-blpop-timeout--0.001-resp2",
        clients: &["p", "a"],
        aof: false,
        actions: &[
            Send("a", &["BLPOP", "l", "-0.001"]),
            WaitBlocked(1),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.001-resp2, operation 3.
            Info("blocked_clients", "1"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.001-resp2, operation 3.
            Info("total_blocking_keys", "1"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.001-resp2, operation 3.
            Info("total_blocking_keys_on_nokey", "0"),
            Quiet("a"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.001-resp2, operation 6.
            Command("p", &["PING"], b"+PONG\r\n"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.001-resp2, operation 8.
            Command("p", &["RPUSH", "l", "x"], b":1\r\n"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.001-resp2, operation 9.
            Read("a", b"*2\r\n$1\r\nl\r\n$1\r\nx\r\n"),
            WaitBlocked(0),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.001-resp2, operation 10.
            Info("blocked_clients", "0"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.001-resp2, operation 10.
            Info("total_blocking_keys", "0"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.001-resp2, operation 10.
            Info("total_blocking_keys_on_nokey", "0"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.001-resp2, operation 12.
            Command("p", &["TYPE", "l"], b"+none\r\n"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.001-resp2, operation 14.
            Command("p", &["LRANGE", "l", "0", "-1"], b"*0\r\n"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.001-resp2, operation 16.
            Command("p", &["PING"], b"+PONG\r\n"),
        ],
    },
    Scenario {
        name: "rounded-negative-zero-blpop-timeout--0.0005-resp2",
        clients: &["p", "a"],
        aof: false,
        actions: &[
            Send("a", &["BLPOP", "l", "-0.0005"]),
            WaitBlocked(1),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.0005-resp2, operation 3.
            Info("blocked_clients", "1"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.0005-resp2, operation 3.
            Info("total_blocking_keys", "1"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.0005-resp2, operation 3.
            Info("total_blocking_keys_on_nokey", "0"),
            Quiet("a"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.0005-resp2, operation 6.
            Command("p", &["PING"], b"+PONG\r\n"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.0005-resp2, operation 8.
            Command("p", &["RPUSH", "l", "x"], b":1\r\n"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.0005-resp2, operation 9.
            Read("a", b"*2\r\n$1\r\nl\r\n$1\r\nx\r\n"),
            WaitBlocked(0),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.0005-resp2, operation 10.
            Info("blocked_clients", "0"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.0005-resp2, operation 10.
            Info("total_blocking_keys", "0"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.0005-resp2, operation 10.
            Info("total_blocking_keys_on_nokey", "0"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.0005-resp2, operation 12.
            Command("p", &["TYPE", "l"], b"+none\r\n"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.0005-resp2, operation 14.
            Command("p", &["LRANGE", "l", "0", "-1"], b"*0\r\n"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.0005-resp2, operation 16.
            Command("p", &["PING"], b"+PONG\r\n"),
        ],
    },
    Scenario {
        name: "rounded-negative-zero-blpop-timeout--0.001-resp3",
        clients: &["p", "a"],
        aof: false,
        actions: &[
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.001-resp3, operation 1 HELLO proto field.
            Resp3("a", b":3\r\n"),
            Send("a", &["BLPOP", "l", "-0.001"]),
            WaitBlocked(1),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.001-resp3, operation 3.
            Info("blocked_clients", "1"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.001-resp3, operation 3.
            Info("total_blocking_keys", "1"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.001-resp3, operation 3.
            Info("total_blocking_keys_on_nokey", "0"),
            Quiet("a"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.001-resp3, operation 6.
            Command("p", &["PING"], b"+PONG\r\n"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.001-resp3, operation 8.
            Command("p", &["RPUSH", "l", "x"], b":1\r\n"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.001-resp3, operation 9.
            Read("a", b"*2\r\n$1\r\nl\r\n$1\r\nx\r\n"),
            WaitBlocked(0),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.001-resp3, operation 10.
            Info("blocked_clients", "0"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.001-resp3, operation 10.
            Info("total_blocking_keys", "0"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.001-resp3, operation 10.
            Info("total_blocking_keys_on_nokey", "0"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.001-resp3, operation 12.
            Command("p", &["TYPE", "l"], b"+none\r\n"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.001-resp3, operation 14.
            Command("p", &["LRANGE", "l", "0", "-1"], b"*0\r\n"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.001-resp3, operation 16.
            Command("p", &["PING"], b"+PONG\r\n"),
        ],
    },
    Scenario {
        name: "rounded-negative-zero-blpop-timeout--0.0005-resp3",
        clients: &["p", "a"],
        aof: false,
        actions: &[
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.0005-resp3, operation 1 HELLO proto field.
            Resp3("a", b":3\r\n"),
            Send("a", &["BLPOP", "l", "-0.0005"]),
            WaitBlocked(1),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.0005-resp3, operation 3.
            Info("blocked_clients", "1"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.0005-resp3, operation 3.
            Info("total_blocking_keys", "1"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.0005-resp3, operation 3.
            Info("total_blocking_keys_on_nokey", "0"),
            Quiet("a"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.0005-resp3, operation 6.
            Command("p", &["PING"], b"+PONG\r\n"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.0005-resp3, operation 8.
            Command("p", &["RPUSH", "l", "x"], b":1\r\n"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.0005-resp3, operation 9.
            Read("a", b"*2\r\n$1\r\nl\r\n$1\r\nx\r\n"),
            WaitBlocked(0),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.0005-resp3, operation 10.
            Info("blocked_clients", "0"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.0005-resp3, operation 10.
            Info("total_blocking_keys", "0"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.0005-resp3, operation 10.
            Info("total_blocking_keys_on_nokey", "0"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.0005-resp3, operation 12.
            Command("p", &["TYPE", "l"], b"+none\r\n"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.0005-resp3, operation 14.
            Command("p", &["LRANGE", "l", "0", "-1"], b"*0\r\n"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.0005-resp3, operation 16.
            Command("p", &["PING"], b"+PONG\r\n"),
        ],
    },
    Scenario {
        name: "rounded-negative-zero-brpop-timeout--0.001-resp2",
        clients: &["p", "a"],
        aof: false,
        actions: &[
            Send("a", &["BRPOP", "l", "-0.001"]),
            WaitBlocked(1),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-brpop-timeout--0.001-resp2, operation 3.
            Info("blocked_clients", "1"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-brpop-timeout--0.001-resp2, operation 3.
            Info("total_blocking_keys", "1"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-brpop-timeout--0.001-resp2, operation 3.
            Info("total_blocking_keys_on_nokey", "0"),
            Quiet("a"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-brpop-timeout--0.001-resp2, operation 6.
            Command("p", &["PING"], b"+PONG\r\n"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-brpop-timeout--0.001-resp2, operation 8.
            Command("p", &["RPUSH", "l", "x"], b":1\r\n"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-brpop-timeout--0.001-resp2, operation 9.
            Read("a", b"*2\r\n$1\r\nl\r\n$1\r\nx\r\n"),
            WaitBlocked(0),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-brpop-timeout--0.001-resp2, operation 10.
            Info("blocked_clients", "0"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-brpop-timeout--0.001-resp2, operation 10.
            Info("total_blocking_keys", "0"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-brpop-timeout--0.001-resp2, operation 10.
            Info("total_blocking_keys_on_nokey", "0"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-brpop-timeout--0.001-resp2, operation 12.
            Command("p", &["TYPE", "l"], b"+none\r\n"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-brpop-timeout--0.001-resp2, operation 14.
            Command("p", &["LRANGE", "l", "0", "-1"], b"*0\r\n"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-brpop-timeout--0.001-resp2, operation 16.
            Command("p", &["PING"], b"+PONG\r\n"),
        ],
    },
    Scenario {
        name: "rounded-negative-zero-brpop-timeout--0.0005-resp2",
        clients: &["p", "a"],
        aof: false,
        actions: &[
            Send("a", &["BRPOP", "l", "-0.0005"]),
            WaitBlocked(1),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-brpop-timeout--0.0005-resp2, operation 3.
            Info("blocked_clients", "1"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-brpop-timeout--0.0005-resp2, operation 3.
            Info("total_blocking_keys", "1"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-brpop-timeout--0.0005-resp2, operation 3.
            Info("total_blocking_keys_on_nokey", "0"),
            Quiet("a"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-brpop-timeout--0.0005-resp2, operation 6.
            Command("p", &["PING"], b"+PONG\r\n"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-brpop-timeout--0.0005-resp2, operation 8.
            Command("p", &["RPUSH", "l", "x"], b":1\r\n"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-brpop-timeout--0.0005-resp2, operation 9.
            Read("a", b"*2\r\n$1\r\nl\r\n$1\r\nx\r\n"),
            WaitBlocked(0),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-brpop-timeout--0.0005-resp2, operation 10.
            Info("blocked_clients", "0"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-brpop-timeout--0.0005-resp2, operation 10.
            Info("total_blocking_keys", "0"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-brpop-timeout--0.0005-resp2, operation 10.
            Info("total_blocking_keys_on_nokey", "0"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-brpop-timeout--0.0005-resp2, operation 12.
            Command("p", &["TYPE", "l"], b"+none\r\n"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-brpop-timeout--0.0005-resp2, operation 14.
            Command("p", &["LRANGE", "l", "0", "-1"], b"*0\r\n"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-brpop-timeout--0.0005-resp2, operation 16.
            Command("p", &["PING"], b"+PONG\r\n"),
        ],
    },
    Scenario {
        name: "rounded-negative-zero-brpop-timeout--0.001-resp3",
        clients: &["p", "a"],
        aof: false,
        actions: &[
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-brpop-timeout--0.001-resp3, operation 1 HELLO proto field.
            Resp3("a", b":3\r\n"),
            Send("a", &["BRPOP", "l", "-0.001"]),
            WaitBlocked(1),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-brpop-timeout--0.001-resp3, operation 3.
            Info("blocked_clients", "1"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-brpop-timeout--0.001-resp3, operation 3.
            Info("total_blocking_keys", "1"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-brpop-timeout--0.001-resp3, operation 3.
            Info("total_blocking_keys_on_nokey", "0"),
            Quiet("a"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-brpop-timeout--0.001-resp3, operation 6.
            Command("p", &["PING"], b"+PONG\r\n"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-brpop-timeout--0.001-resp3, operation 8.
            Command("p", &["RPUSH", "l", "x"], b":1\r\n"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-brpop-timeout--0.001-resp3, operation 9.
            Read("a", b"*2\r\n$1\r\nl\r\n$1\r\nx\r\n"),
            WaitBlocked(0),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-brpop-timeout--0.001-resp3, operation 10.
            Info("blocked_clients", "0"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-brpop-timeout--0.001-resp3, operation 10.
            Info("total_blocking_keys", "0"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-brpop-timeout--0.001-resp3, operation 10.
            Info("total_blocking_keys_on_nokey", "0"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-brpop-timeout--0.001-resp3, operation 12.
            Command("p", &["TYPE", "l"], b"+none\r\n"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-brpop-timeout--0.001-resp3, operation 14.
            Command("p", &["LRANGE", "l", "0", "-1"], b"*0\r\n"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-brpop-timeout--0.001-resp3, operation 16.
            Command("p", &["PING"], b"+PONG\r\n"),
        ],
    },
    Scenario {
        name: "rounded-negative-zero-brpop-timeout--0.0005-resp3",
        clients: &["p", "a"],
        aof: false,
        actions: &[
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-brpop-timeout--0.0005-resp3, operation 1 HELLO proto field.
            Resp3("a", b":3\r\n"),
            Send("a", &["BRPOP", "l", "-0.0005"]),
            WaitBlocked(1),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-brpop-timeout--0.0005-resp3, operation 3.
            Info("blocked_clients", "1"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-brpop-timeout--0.0005-resp3, operation 3.
            Info("total_blocking_keys", "1"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-brpop-timeout--0.0005-resp3, operation 3.
            Info("total_blocking_keys_on_nokey", "0"),
            Quiet("a"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-brpop-timeout--0.0005-resp3, operation 6.
            Command("p", &["PING"], b"+PONG\r\n"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-brpop-timeout--0.0005-resp3, operation 8.
            Command("p", &["RPUSH", "l", "x"], b":1\r\n"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-brpop-timeout--0.0005-resp3, operation 9.
            Read("a", b"*2\r\n$1\r\nl\r\n$1\r\nx\r\n"),
            WaitBlocked(0),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-brpop-timeout--0.0005-resp3, operation 10.
            Info("blocked_clients", "0"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-brpop-timeout--0.0005-resp3, operation 10.
            Info("total_blocking_keys", "0"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-brpop-timeout--0.0005-resp3, operation 10.
            Info("total_blocking_keys_on_nokey", "0"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-brpop-timeout--0.0005-resp3, operation 12.
            Command("p", &["TYPE", "l"], b"+none\r\n"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-brpop-timeout--0.0005-resp3, operation 14.
            Command("p", &["LRANGE", "l", "0", "-1"], b"*0\r\n"),
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-brpop-timeout--0.0005-resp3, operation 16.
            Command("p", &["PING"], b"+PONG\r\n"),
        ],
    },
    Scenario {
        name: "negative-finite-errors-resp2",
        clients: &["p", "a"],
        aof: false,
        actions: &[
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-negative-finite-errors-resp2, operation 3.
            Command("a", &["BLPOP", "l", "-0.0015"], b"-ERR timeout is negative\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-negative-finite-errors-resp2, operation 5.
            Command("a", &["BLPOP", "l", "-0.002"], b"-ERR timeout is negative\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-negative-finite-errors-resp2, operation 7.
            Command("a", &["BRPOP", "l", "-0.0015"], b"-ERR timeout is negative\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-negative-finite-errors-resp2, operation 9.
            Command("a", &["BRPOP", "l", "-0.002"], b"-ERR timeout is negative\r\n"),
            // Redis 7.2.16 run 38453128755, ORACLE-pending-blpop-exec-and-script, reply 14.
            Command("p", &["PING"], b"+PONG\r\n"),
        ],
    },
    Scenario {
        name: "negative-finite-errors-resp3",
        clients: &["p", "a"],
        aof: false,
        actions: &[
            // Redis 7.2.16 run 38453804021, blocking-mc-s01-blpop-timeout--0.001-resp3, operation 1 HELLO proto field.
            Resp3("a", b":3\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-negative-finite-errors-resp3, operation 5.
            Command("a", &["BLPOP", "l", "-0.0015"], b"-ERR timeout is negative\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-negative-finite-errors-resp3, operation 7.
            Command("a", &["BLPOP", "l", "-0.002"], b"-ERR timeout is negative\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-negative-finite-errors-resp3, operation 9.
            Command("a", &["BRPOP", "l", "-0.0015"], b"-ERR timeout is negative\r\n"),
            // Redis 7.2.16 remaining recording ([run 38464000185](https://github.com/Ming-Research/Firn-wf/actions/runs/38464000185), oracle-replies.json), blocking-fixture-exact-negative-finite-errors-resp3, operation 11.
            Command("a", &["BRPOP", "l", "-0.002"], b"-ERR timeout is negative\r\n"),
            // Redis 7.2.16 run 38453128755, ORACLE-pending-blpop-exec-and-script, reply 14.
            Command("p", &["PING"], b"+PONG\r\n"),
        ],
    },
    Scenario {
        name: "aof-expired-top-level-producers",
        clients: &["p"],
        aof: true,
        actions: &[
            // Redis 7.2.16 run 39899773861, blocking-expired-keys-recording.json, top-level-producers-over-expired-key, step 0 replies.
            Pipeline("p", &[&["SET", "e1", "v", "PXAT", "1"], &["RPUSH", "e1", "b"]]),
            Read("p", b"+OK\r\n"),
            Read("p", b":1\r\n"),
            // Redis 7.2.16 run 39899773861, blocking-expired-keys-recording.json, top-level-producers-over-expired-key, step 1 replies.
            Pipeline("p", &[&["SET", "e2", "v", "PXAT", "1"], &["LPUSH", "e2", "b"]]),
            Read("p", b"+OK\r\n"),
            Read("p", b":1\r\n"),
            // Redis 7.2.16 run 39899773861, blocking-expired-keys-recording.json, top-level-producers-over-expired-key, step 2 replies.
            Pipeline("p", &[&["SET", "e3", "v", "PXAT", "1"], &["RPUSHX", "e3", "b"]]),
            Read("p", b"+OK\r\n"),
            Read("p", b":0\r\n"),
            // Redis 7.2.16 run 39899773861, blocking-expired-keys-recording.json, top-level-producers-over-expired-key, step 3 replies.
            Pipeline("p", &[&["SET", "e4", "v", "PXAT", "1"], &["LINSERT", "e4", "BEFORE", "a", "b"]]),
            Read("p", b"+OK\r\n"),
            Read("p", b":0\r\n"),
            // Redis 7.2.16 run 39899773861, blocking-expired-keys-recording.json, top-level-producers-over-expired-key, step 4.
            Command("p", &["RPUSH", "s5", "a"], b":1\r\n"),
            // Redis 7.2.16 run 39899773861, blocking-expired-keys-recording.json, top-level-producers-over-expired-key, step 5 replies.
            Pipeline("p", &[&["SET", "e5", "v", "PXAT", "1"], &["LMOVE", "s5", "e5", "LEFT", "LEFT"]]),
            Read("p", b"+OK\r\n"),
            Read("p", b"$1\r\na\r\n"),
            // Redis 7.2.16 run 39899773861, blocking-expired-keys-recording.json, top-level-producers-over-expired-key, step 6.
            Command("p", &["RPUSH", "s6", "a"], b":1\r\n"),
            // Redis 7.2.16 run 39899773861, blocking-expired-keys-recording.json, top-level-producers-over-expired-key, step 7 replies.
            Pipeline("p", &[&["SET", "e6", "v", "PXAT", "1"], &["RPOPLPUSH", "s6", "e6"]]),
            Read("p", b"+OK\r\n"),
            Read("p", b"$1\r\na\r\n"),
            // Redis 7.2.16 run 39899773861, blocking-expired-keys-recording.json, top-level-producers-over-expired-key, step 8.
            Command("p", &["RPUSH", "r1", "a"], b":1\r\n"),
            // Redis 7.2.16 run 39899773861, blocking-expired-keys-recording.json, top-level-producers-over-expired-key, step 9 replies.
            Pipeline("p", &[&["SET", "e7", "v", "PXAT", "1"], &["RENAME", "r1", "e7"]]),
            Read("p", b"+OK\r\n"),
            Read("p", b"+OK\r\n"),
            // Redis 7.2.16 run 39899773861, blocking-expired-keys-recording.json, top-level-producers-over-expired-key, step 10.
            Command("p", &["RPUSH", "c1", "a"], b":1\r\n"),
            // Redis 7.2.16 run 39899773861, blocking-expired-keys-recording.json, top-level-producers-over-expired-key, step 11 replies.
            Pipeline("p", &[&["SET", "e8", "v", "PXAT", "1"], &["COPY", "c1", "e8"]]),
            Read("p", b"+OK\r\n"),
            Read("p", b":1\r\n"),
            // Redis 7.2.16 run 39899773861, blocking-expired-keys-recording.json, top-level-producers-over-expired-key, incremental file.
            Aof(b"*2\r\n$6\r\nSELECT\r\n$1\r\n0\r\n*5\r\n$3\r\nSET\r\n$2\r\ne1\r\n$1\r\nv\r\n$4\r\nPXAT\r\n$1\r\n1\r\n*1\r\n$5\r\nMULTI\r\n*2\r\n$3\r\nDEL\r\n$2\r\ne1\r\n*3\r\n$5\r\nRPUSH\r\n$2\r\ne1\r\n$1\r\nb\r\n*1\r\n$4\r\nEXEC\r\n*5\r\n$3\r\nSET\r\n$2\r\ne2\r\n$1\r\nv\r\n$4\r\nPXAT\r\n$1\r\n1\r\n*1\r\n$5\r\nMULTI\r\n*2\r\n$3\r\nDEL\r\n$2\r\ne2\r\n*3\r\n$5\r\nLPUSH\r\n$2\r\ne2\r\n$1\r\nb\r\n*1\r\n$4\r\nEXEC\r\n*5\r\n$3\r\nSET\r\n$2\r\ne3\r\n$1\r\nv\r\n$4\r\nPXAT\r\n$1\r\n1\r\n*2\r\n$3\r\nDEL\r\n$2\r\ne3\r\n*5\r\n$3\r\nSET\r\n$2\r\ne4\r\n$1\r\nv\r\n$4\r\nPXAT\r\n$1\r\n1\r\n*2\r\n$3\r\nDEL\r\n$2\r\ne4\r\n*3\r\n$5\r\nRPUSH\r\n$2\r\ns5\r\n$1\r\na\r\n*5\r\n$3\r\nSET\r\n$2\r\ne5\r\n$1\r\nv\r\n$4\r\nPXAT\r\n$1\r\n1\r\n*1\r\n$5\r\nMULTI\r\n*2\r\n$3\r\nDEL\r\n$2\r\ne5\r\n*5\r\n$5\r\nLMOVE\r\n$2\r\ns5\r\n$2\r\ne5\r\n$4\r\nLEFT\r\n$4\r\nLEFT\r\n*1\r\n$4\r\nEXEC\r\n*3\r\n$5\r\nRPUSH\r\n$2\r\ns6\r\n$1\r\na\r\n*5\r\n$3\r\nSET\r\n$2\r\ne6\r\n$1\r\nv\r\n$4\r\nPXAT\r\n$1\r\n1\r\n*1\r\n$5\r\nMULTI\r\n*2\r\n$3\r\nDEL\r\n$2\r\ne6\r\n*3\r\n$9\r\nRPOPLPUSH\r\n$2\r\ns6\r\n$2\r\ne6\r\n*1\r\n$4\r\nEXEC\r\n*3\r\n$5\r\nRPUSH\r\n$2\r\nr1\r\n$1\r\na\r\n*5\r\n$3\r\nSET\r\n$2\r\ne7\r\n$1\r\nv\r\n$4\r\nPXAT\r\n$1\r\n1\r\n*1\r\n$5\r\nMULTI\r\n*2\r\n$3\r\nDEL\r\n$2\r\ne7\r\n*3\r\n$6\r\nRENAME\r\n$2\r\nr1\r\n$2\r\ne7\r\n*1\r\n$4\r\nEXEC\r\n*3\r\n$5\r\nRPUSH\r\n$2\r\nc1\r\n$1\r\na\r\n*5\r\n$3\r\nSET\r\n$2\r\ne8\r\n$1\r\nv\r\n$4\r\nPXAT\r\n$1\r\n1\r\n*1\r\n$5\r\nMULTI\r\n*2\r\n$3\r\nDEL\r\n$2\r\ne8\r\n*3\r\n$4\r\nCOPY\r\n$2\r\nc1\r\n$2\r\ne8\r\n*1\r\n$4\r\nEXEC\r\n"),
        ],
    },
    Scenario {
        name: "aof-expired-immediate-blocking-pops",
        clients: &["p"],
        aof: true,
        actions: &[
            // Redis 7.2.16 run 39899773861, blocking-expired-keys-recording.json, immediate-blocking-pops-over-expired-key, step 0.
            Command("p", &["RPUSH", "l1", "x"], b":1\r\n"),
            // Redis 7.2.16 run 39899773861, blocking-expired-keys-recording.json, immediate-blocking-pops-over-expired-key, step 1 replies.
            Pipeline("p", &[&["SET", "e1", "v", "PXAT", "1"], &["BLPOP", "e1", "l1", "0"]]),
            Read("p", b"+OK\r\n"),
            Read("p", b"*2\r\n$2\r\nl1\r\n$1\r\nx\r\n"),
            // Redis 7.2.16 run 39899773861, blocking-expired-keys-recording.json, immediate-blocking-pops-over-expired-key, step 2.
            Command("p", &["RPUSH", "s1", "a"], b":1\r\n"),
            // Redis 7.2.16 run 39899773861, blocking-expired-keys-recording.json, immediate-blocking-pops-over-expired-key, step 3 replies.
            Pipeline("p", &[&["SET", "e2", "v", "PXAT", "1"], &["BRPOPLPUSH", "s1", "e2", "0"]]),
            Read("p", b"+OK\r\n"),
            Read("p", b"$1\r\na\r\n"),
            // Redis 7.2.16 run 39899773861, blocking-expired-keys-recording.json, immediate-blocking-pops-over-expired-key, step 4.
            Command("p", &["RPUSH", "s2", "a"], b":1\r\n"),
            // Redis 7.2.16 run 39899773861, blocking-expired-keys-recording.json, immediate-blocking-pops-over-expired-key, step 5 replies.
            Pipeline("p", &[&["SET", "e3", "v", "PXAT", "1"], &["BLMOVE", "s2", "e3", "LEFT", "RIGHT", "0"]]),
            Read("p", b"+OK\r\n"),
            Read("p", b"$1\r\na\r\n"),
            // Redis 7.2.16 run 39899773861, blocking-expired-keys-recording.json, immediate-blocking-pops-over-expired-key, incremental file.
            Aof(b"*2\r\n$6\r\nSELECT\r\n$1\r\n0\r\n*3\r\n$5\r\nRPUSH\r\n$2\r\nl1\r\n$1\r\nx\r\n*5\r\n$3\r\nSET\r\n$2\r\ne1\r\n$1\r\nv\r\n$4\r\nPXAT\r\n$1\r\n1\r\n*1\r\n$5\r\nMULTI\r\n*2\r\n$3\r\nDEL\r\n$2\r\ne1\r\n*2\r\n$4\r\nLPOP\r\n$2\r\nl1\r\n*1\r\n$4\r\nEXEC\r\n*3\r\n$5\r\nRPUSH\r\n$2\r\ns1\r\n$1\r\na\r\n*5\r\n$3\r\nSET\r\n$2\r\ne2\r\n$1\r\nv\r\n$4\r\nPXAT\r\n$1\r\n1\r\n*1\r\n$5\r\nMULTI\r\n*2\r\n$3\r\nDEL\r\n$2\r\ne2\r\n*3\r\n$9\r\nRPOPLPUSH\r\n$2\r\ns1\r\n$2\r\ne2\r\n*1\r\n$4\r\nEXEC\r\n*3\r\n$5\r\nRPUSH\r\n$2\r\ns2\r\n$1\r\na\r\n*5\r\n$3\r\nSET\r\n$2\r\ne3\r\n$1\r\nv\r\n$4\r\nPXAT\r\n$1\r\n1\r\n*1\r\n$5\r\nMULTI\r\n*2\r\n$3\r\nDEL\r\n$2\r\ne3\r\n*5\r\n$5\r\nLMOVE\r\n$2\r\ns2\r\n$2\r\ne3\r\n$4\r\nLEFT\r\n$5\r\nRIGHT\r\n*1\r\n$4\r\nEXEC\r\n"),
        ],
    },
    Scenario {
        name: "aof-expired-served-waiter",
        clients: &["p", "a"],
        aof: true,
        actions: &[
            Send("a", &["BLMOVE", "s", "d", "LEFT", "RIGHT", "0"]),
            WaitBlocked(1),
            // Redis 7.2.16 run 39899773861, blocking-expired-keys-recording.json, served-waiter-over-expired-destination, step 1 replies.
            Pipeline("p", &[&["SET", "d", "v", "PXAT", "1"], &["RPUSH", "s", "w"]]),
            Read("p", b"+OK\r\n"),
            Read("p", b":1\r\n"),
            // Redis 7.2.16 run 39899773861, blocking-expired-keys-recording.json, served-waiter-over-expired-destination, step 2.
            Read("a", b"$1\r\nw\r\n"),
            // Redis 7.2.16 run 39899773861, blocking-expired-keys-recording.json, served-waiter-over-expired-destination, incremental file.
            Aof(b"*2\r\n$6\r\nSELECT\r\n$1\r\n0\r\n*5\r\n$3\r\nSET\r\n$1\r\nd\r\n$1\r\nv\r\n$4\r\nPXAT\r\n$1\r\n1\r\n*3\r\n$5\r\nRPUSH\r\n$1\r\ns\r\n$1\r\nw\r\n*1\r\n$5\r\nMULTI\r\n*2\r\n$3\r\nDEL\r\n$1\r\nd\r\n*5\r\n$5\r\nLMOVE\r\n$1\r\ns\r\n$1\r\nd\r\n$4\r\nLEFT\r\n$5\r\nRIGHT\r\n*1\r\n$4\r\nEXEC\r\n"),
        ],
    },
    Scenario {
        name: "narrow-expired-string-then-push",
        clients: &["p", "a"],
        aof: true,
        actions: &[
            Send("a", &["BLPOP", "k", "0"]),
            WaitBlocked(1),
            // Redis 7.2.16 run 40057972246, blocking-waits-on-strings-recording.json, waiter-survives-an-expired-string-then-push, step 2 replies.
            Pipeline("p", &[&["SET", "k", "v", "PXAT", "1"], &["LPUSH", "k", "x"]]),
            Read("p", b"+OK\r\n"),
            Read("p", b":1\r\n"),
            // Redis 7.2.16 run 40057972246, blocking-waits-on-strings-recording.json, waiter-survives-an-expired-string-then-push, step 3, the waiter's reply.
            Read("a", b"*2\r\n$1\r\nk\r\n$1\r\nx\r\n"),
            // Redis 7.2.16 run 40057972246, blocking-waits-on-strings-recording.json, waiter-survives-an-expired-string-then-push, step 4, incremental file.
            Aof(b"*2\r\n$6\r\nSELECT\r\n$1\r\n0\r\n*5\r\n$3\r\nSET\r\n$1\r\nk\r\n$1\r\nv\r\n$4\r\nPXAT\r\n$1\r\n1\r\n*1\r\n$5\r\nMULTI\r\n*2\r\n$3\r\nDEL\r\n$1\r\nk\r\n*3\r\n$5\r\nLPUSH\r\n$1\r\nk\r\n$1\r\nx\r\n*1\r\n$4\r\nEXEC\r\n*2\r\n$4\r\nLPOP\r\n$1\r\nk\r\n"),
        ],
    },
    Scenario {
        name: "narrow-expired-string-then-move",
        clients: &["p", "a"],
        aof: true,
        actions: &[
            Send("a", &["BLPOP", "d", "0"]),
            WaitBlocked(1),
            // Redis 7.2.16 run 40057972246, blocking-waits-on-strings-recording.json, waiter-survives-an-expired-string-then-move, step 2 replies.
            Pipeline("p", &[&["RPUSH", "s", "w"], &["SET", "d", "v", "PXAT", "1"], &["LMOVE", "s", "d", "LEFT", "LEFT"]]),
            Read("p", b":1\r\n"),
            Read("p", b"+OK\r\n"),
            Read("p", b"$1\r\nw\r\n"),
            // Redis 7.2.16 run 40057972246, blocking-waits-on-strings-recording.json, waiter-survives-an-expired-string-then-move, step 3, the waiter's reply.
            Read("a", b"*2\r\n$1\r\nd\r\n$1\r\nw\r\n"),
            // Redis 7.2.16 run 40057972246, blocking-waits-on-strings-recording.json, waiter-survives-an-expired-string-then-move, step 4, incremental file.
            Aof(b"*2\r\n$6\r\nSELECT\r\n$1\r\n0\r\n*3\r\n$5\r\nRPUSH\r\n$1\r\ns\r\n$1\r\nw\r\n*5\r\n$3\r\nSET\r\n$1\r\nd\r\n$1\r\nv\r\n$4\r\nPXAT\r\n$1\r\n1\r\n*1\r\n$5\r\nMULTI\r\n*2\r\n$3\r\nDEL\r\n$1\r\nd\r\n*5\r\n$5\r\nLMOVE\r\n$1\r\ns\r\n$1\r\nd\r\n$4\r\nLEFT\r\n$4\r\nLEFT\r\n*1\r\n$4\r\nEXEC\r\n*2\r\n$4\r\nLPOP\r\n$1\r\nd\r\n"),
        ],
    },
    Scenario {
        name: "narrow-live-string-refused-pushes",
        clients: &["p", "a"],
        aof: true,
        actions: &[
            Send("a", &["BLPOP", "k", "0"]),
            WaitBlocked(1),
            // Redis 7.2.16 run 40057972246, blocking-waits-on-strings-recording.json, waiter-survives-a-live-string-and-a-refused-push, step 2 replies.
            Pipeline("p", &[&["SET", "k", "v"], &["LPUSH", "k", "x"], &["LPUSHX", "k", "x"], &["RPUSH", "k", "x"]]),
            Read("p", b"+OK\r\n"),
            Read("p", b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n"),
            Read("p", b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n"),
            Read("p", b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n"),
            Quiet("a"),
            // Redis 7.2.16 run 40057972246, blocking-waits-on-strings-recording.json, waiter-survives-a-live-string-and-a-refused-push, step 3 replies.
            Pipeline("p", &[&["DEL", "k"], &["LPUSH", "k", "y"]]),
            Read("p", b":1\r\n"),
            Read("p", b":1\r\n"),
            // Redis 7.2.16 run 40057972246, blocking-waits-on-strings-recording.json, waiter-survives-a-live-string-and-a-refused-push, step 4, the waiter's reply.
            Read("a", b"*2\r\n$1\r\nk\r\n$1\r\ny\r\n"),
            // Redis 7.2.16 run 40057972246, blocking-waits-on-strings-recording.json, waiter-survives-a-live-string-and-a-refused-push, step 5, incremental file.
            Aof(b"*2\r\n$6\r\nSELECT\r\n$1\r\n0\r\n*3\r\n$3\r\nSET\r\n$1\r\nk\r\n$1\r\nv\r\n*2\r\n$3\r\nDEL\r\n$1\r\nk\r\n*3\r\n$5\r\nLPUSH\r\n$1\r\nk\r\n$1\r\ny\r\n*2\r\n$4\r\nLPOP\r\n$1\r\nk\r\n"),
        ],
    },
    Scenario {
        name: "narrow-live-list-writes-with-waiter-elsewhere",
        clients: &["p", "a"],
        aof: true,
        actions: &[
            Send("a", &["BLPOP", "w", "0"]),
            WaitBlocked(1),
            // Redis 7.2.16 run 40057972246, blocking-waits-on-strings-recording.json, pushes-onto-a-live-list-with-a-waiter-elsewhere, step 2 replies.
            Pipeline("p", &[&["RPUSH", "l", "a"], &["LPUSH", "l", "b"], &["LPUSHX", "l", "c"], &["RPUSHX", "l", "d"], &["LINSERT", "l", "BEFORE", "a", "e"], &["LMOVE", "l", "l", "LEFT", "RIGHT"], &["RPOPLPUSH", "l", "l"], &["LRANGE", "l", "0", "-1"]]),
            Read("p", b":1\r\n"),
            Read("p", b":2\r\n"),
            Read("p", b":3\r\n"),
            Read("p", b":4\r\n"),
            Read("p", b":5\r\n"),
            Read("p", b"$1\r\nc\r\n"),
            Read("p", b"$1\r\nc\r\n"),
            Read("p", b"*5\r\n$1\r\nc\r\n$1\r\nb\r\n$1\r\ne\r\n$1\r\na\r\n$1\r\nd\r\n"),
            // Redis 7.2.16 run 40057972246, blocking-waits-on-strings-recording.json, pushes-onto-a-live-list-with-a-waiter-elsewhere, step 3 replies.
            Pipeline("p", &[&["LPUSH", "w", "z"]]),
            Read("p", b":1\r\n"),
            // Redis 7.2.16 run 40057972246, blocking-waits-on-strings-recording.json, pushes-onto-a-live-list-with-a-waiter-elsewhere, step 4, the waiter's reply.
            Read("a", b"*2\r\n$1\r\nw\r\n$1\r\nz\r\n"),
            // Redis 7.2.16 run 40057972246, blocking-waits-on-strings-recording.json, pushes-onto-a-live-list-with-a-waiter-elsewhere, step 5, incremental file.
            Aof(b"*2\r\n$6\r\nSELECT\r\n$1\r\n0\r\n*3\r\n$5\r\nRPUSH\r\n$1\r\nl\r\n$1\r\na\r\n*3\r\n$5\r\nLPUSH\r\n$1\r\nl\r\n$1\r\nb\r\n*3\r\n$6\r\nLPUSHX\r\n$1\r\nl\r\n$1\r\nc\r\n*3\r\n$6\r\nRPUSHX\r\n$1\r\nl\r\n$1\r\nd\r\n*5\r\n$7\r\nLINSERT\r\n$1\r\nl\r\n$6\r\nBEFORE\r\n$1\r\na\r\n$1\r\ne\r\n*5\r\n$5\r\nLMOVE\r\n$1\r\nl\r\n$1\r\nl\r\n$4\r\nLEFT\r\n$5\r\nRIGHT\r\n*3\r\n$9\r\nRPOPLPUSH\r\n$1\r\nl\r\n$1\r\nl\r\n*3\r\n$5\r\nLPUSH\r\n$1\r\nw\r\n$1\r\nz\r\n*2\r\n$4\r\nLPOP\r\n$1\r\nw\r\n"),
        ],
    },
];
