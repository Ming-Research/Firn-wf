//! Sources: [main recording](https://github.com/Ming-Research/Firn-wf/actions/runs/38453128755)
//! and [negative recording](https://github.com/Ming-Research/Firn-wf/actions/runs/38453804021).
//! Recorded wire/AOF bytes and INFO values cite pinned Redis 7.2.16 CI artifacts.
//! Remaining ORACLE-pending expectations require an exact source recording.
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
            Command("p", &["EXISTS", "l"], b"ORACLE-pending"),
            // Redis 7.2.16 run 38453128755, ORACLE-pending-brpoplpush-errors-typecheck-timeout-and-empty-context, reply 8.
            Command("p", &["RPUSH", "s", "a", "b", "c"], b":3\r\n"),
            // Redis 7.2.16 run 38453128755, ORACLE-pending-brpoplpush-errors-typecheck-timeout-and-empty-context, reply 11.
            Command("p", &["BRPOPLPUSH", "s", "d", "0"], b"$1\r\nc\r\n"),
            // Redis 7.2.16 run 38453128755, ORACLE-pending-brpoplpush-errors-typecheck-timeout-and-empty-context, reply 13.
            Command("p", &["LRANGE", "d", "0", "-1"], b"*1\r\n$1\r\nc\r\n"),
            Command("p", &["RPUSH", "from-LEFT-LEFT", "a", "b"], b"ORACLE-pending"),
            Command("p", &["BLMOVE", "from-LEFT-LEFT", "d", "LEFT", "LEFT", "0"], b"ORACLE-pending"),
            Command("p", &["LRANGE", "from-LEFT-LEFT", "0", "-1"], b"ORACLE-pending"),
            Command("p", &["LRANGE", "d", "0", "-1"], b"ORACLE-pending"),
            Command("p", &["RPUSH", "from-LEFT-RIGHT", "a", "b"], b"ORACLE-pending"),
            Command("p", &["BLMOVE", "from-LEFT-RIGHT", "d", "LEFT", "RIGHT", "0"], b"ORACLE-pending"),
            Command("p", &["LRANGE", "from-LEFT-RIGHT", "0", "-1"], b"ORACLE-pending"),
            Command("p", &["LRANGE", "d", "0", "-1"], b"ORACLE-pending"),
            Command("p", &["RPUSH", "from-RIGHT-LEFT", "a", "b"], b"ORACLE-pending"),
            Command("p", &["BLMOVE", "from-RIGHT-LEFT", "d", "RIGHT", "LEFT", "0"], b"ORACLE-pending"),
            Command("p", &["LRANGE", "from-RIGHT-LEFT", "0", "-1"], b"ORACLE-pending"),
            Command("p", &["LRANGE", "d", "0", "-1"], b"ORACLE-pending"),
            Command("p", &["RPUSH", "from-RIGHT-RIGHT", "a", "b"], b"ORACLE-pending"),
            Command("p", &["BLMOVE", "from-RIGHT-RIGHT", "d", "RIGHT", "RIGHT", "0"], b"ORACLE-pending"),
            Command("p", &["LRANGE", "from-RIGHT-RIGHT", "0", "-1"], b"ORACLE-pending"),
            Command("p", &["LRANGE", "d", "0", "-1"], b"ORACLE-pending"),
            Command("p", &["BRPOPLPUSH", "d", "d", "0"], b"ORACLE-pending"),
            Command("p", &["LRANGE", "d", "0", "-1"], b"ORACLE-pending"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s10-destination-error-brpoplpush-resp2, operation 4.
            Command("p", &["SET", "bad", "string"], b"+OK\r\n"),
            Command("p", &["BLPOP", "bad", "0"], b"ORACLE-pending"),
            Command("p", &["BLMOVE", "s", "bad", "LEFT", "RIGHT", "0"], b"ORACLE-pending"),
            Command("p", &["LRANGE", "s", "0", "-1"], b"ORACLE-pending"),
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
            Info("total_blocking_keys", "ORACLE-pending"),
            Send("b", &["BRPOP", "y", "0"]),
            WaitBlocked(2),
            Info("blocked_clients", "ORACLE-pending"),
            Info("total_blocking_keys", "ORACLE-pending"),
            Command("p", &["RPUSH", "y", "one", "two"], b"ORACLE-pending"),
            Read("a", b"ORACLE-pending"),
            Read("b", b"ORACLE-pending"),
            WaitBlocked(0),
            Info("total_blocking_keys", "ORACLE-pending"),
            Command("p", &["RPUSH", "x", "retained"], b"ORACLE-pending"),
            Command("p", &["LRANGE", "x", "0", "-1"], b"ORACLE-pending"),
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
            Command("p", &["EXISTS", "x", "y"], b"ORACLE-pending"),
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
            Command("p", &["LPUSH", "l", "transient"], b"ORACLE-pending"),
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
            Command("p", &["EVAL", "redis.call('LPUSH','l','first'); redis.call('LPUSH','l','last'); return 'done'", "0"], b"ORACLE-pending"),
            Read("a", b"ORACLE-pending"),
            Command("p", &["LRANGE", "l", "0", "-1"], b"ORACLE-pending"),
            Command("p", &["MULTI"], b"ORACLE-pending"),
            Command("p", &["BLPOP", "empty", "0"], b"ORACLE-pending"),
            Command("p", &["BRPOP", "empty", "0"], b"ORACLE-pending"),
            Command("p", &["BRPOPLPUSH", "empty", "d", "0"], b"ORACLE-pending"),
            Command("p", &["BLMOVE", "empty", "d", "LEFT", "RIGHT", "0"], b"ORACLE-pending"),
            Command("p", &["EXEC"], b"ORACLE-pending"),
            Command("p", &["EVAL", "return {redis.call('BLPOP','empty','0'),redis.call('BRPOP','empty','0'),redis.call('BRPOPLPUSH','empty','d','0'),redis.call('BLMOVE','empty','d','LEFT','RIGHT','0')}", "0"], b"ORACLE-pending"),
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
            Read("b", b"ORACLE-pending"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s10-destination-error-brpoplpush-resp2, operation 19.
            Command("p", &["LRANGE", "s", "0", "-1"], b"*2\r\n$1\r\na\r\n$1\r\nb\r\n"),
            Command("p", &["LRANGE", "d", "0", "-1"], b"ORACLE-pending"),
            Command("p", &["DEL", "s", "d"], b"ORACLE-pending"),
            Send("a", &["BLMOVE", "s", "d", "RIGHT", "LEFT", "0"]),
            WaitBlocked(1),
            Send("b", &["BLMOVE", "d", "e", "LEFT", "RIGHT", "0"]),
            WaitBlocked(2),
            Command("p", &["RPUSH", "s", "linked"], b"ORACLE-pending"),
            Read("a", b"ORACLE-pending"),
            Read("b", b"ORACLE-pending"),
            WaitBlocked(0),
            Command("p", &["EXISTS", "s", "d"], b"ORACLE-pending"),
            Command("p", &["LRANGE", "e", "0", "-1"], b"ORACLE-pending"),
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
            Command("p", &["RPUSH", "y", "untouched"], b"ORACLE-pending"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s09-wrongtype-earlier-key-rescan-resp2, operation 10.
            Read("a", b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n"),
            WaitBlocked(0),
            Command("p", &["LRANGE", "y", "0", "-1"], b"ORACLE-pending"),
            Command("p", &["DEL", "x", "y"], b"ORACLE-pending"),
            Send("a", &["BLPOP", "x", "0"]),
            WaitBlocked(1),
            // Redis 7.2.16 run 38453128755, blocking-mc-s09-wrongtype-replaced-with-list-resp2, operation 5.
            Command("p", &["SET", "x", "string"], b"+OK\r\n"),
            Quiet("a"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s09-wrongtype-replaced-with-list-resp2, operation 9.
            Command("p", &["DEL", "x"], b":1\r\n"),
            Command("p", &["RPUSH", "x", "final"], b"ORACLE-pending"),
            Read("a", b"ORACLE-pending"),
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
            Command("p", &["PUBLISH", "news", "while-blocked"], b"ORACLE-pending"),
            Read("a", b"ORACLE-pending"),
            WaitBlocked(1),
            // Redis 7.2.16 run 38453128755, blocking-mc-s18-subscribed-blpop-resp3, operation 11.
            Command("p", &["RPUSH", "l", "value"], b":1\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s18-subscribed-blpop-resp3, operation 12.
            Read("a", b"*2\r\n$1\r\nl\r\n$5\r\nvalue\r\n"),
            WaitBlocked(0),
            Command("a", &["UNSUBSCRIBE", "news"], b"ORACLE-pending"),
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
            Command("p", &["MULTI"], b"ORACLE-pending"),
            Command("p", &["CLIENT", "UNBLOCK", "@id:a", "TIMEOUT"], b"ORACLE-pending"),
            Quiet("a"),
            Command("p", &["EXEC"], b"ORACLE-pending"),
            Read("a", b"ORACLE-pending"),
            WaitBlocked(0),
            Send("a", &["BLMOVE", "queued", "destination", "LEFT", "RIGHT", "0"]),
            WaitBlocked(1),
            Command("p", &["MULTI"], b"ORACLE-pending"),
            Command("p", &["CLIENT", "UNBLOCK", "@id:a", "ERROR"], b"ORACLE-pending"),
            Command("p", &["EXEC"], b"ORACLE-pending"),
            Read("a", b"ORACLE-pending"),
            WaitBlocked(0),
            Command("p", &["EXISTS", "queued", "destination"], b"ORACLE-pending"),
            Command("p", &["MULTI"], b"ORACLE-pending"),
            Command("p", &["CLIENT", "UNBLOCK", "@id:a"], b"ORACLE-pending"),
            Command("p", &["EXEC"], b"ORACLE-pending"),
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
            Command("a", &["PING", "tail"], b"ORACLE-pending"),
            Command("p", &["RPUSH", "x", "one"], b"ORACLE-pending"),
            Command("p", &["RPUSH", "y", "two"], b"ORACLE-pending"),
            Command("p", &["EXISTS", "s", "d"], b"ORACLE-pending"),
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
            Command("p", &["RPUSH", "l", "popped", "left"], b"ORACLE-pending"),
            Read("a", b"ORACLE-pending"),
            Command("p", &["BRPOP", "l", "0"], b"ORACLE-pending"),
            Send("a", &["BRPOPLPUSH", "s", "d", "0"]),
            WaitBlocked(1),
            Command("p", &["RPUSH", "s", "moved"], b"ORACLE-pending"),
            Read("a", b"ORACLE-pending"),
            Send("a", &["BLMOVE", "s", "d", "LEFT", "RIGHT", "0"]),
            WaitBlocked(1),
            Command("p", &["RPUSH", "s", "second"], b"ORACLE-pending"),
            Read("a", b"ORACLE-pending"),
            Send("a", &["BLMOVE", "from", "mid", "LEFT", "RIGHT", "0"]),
            WaitBlocked(1),
            Send("b", &["BLMOVE", "mid", "last", "RIGHT", "LEFT", "0"]),
            WaitBlocked(2),
            Command("p", &["RPUSH", "from", "chain"], b"ORACLE-pending"),
            Read("a", b"ORACLE-pending"),
            Read("b", b"ORACLE-pending"),
            WaitBlocked(0),
            Aof(b"ORACLE-pending"),
            Restart,
            Command("p", &["LRANGE", "d", "0", "-1"], b"ORACLE-pending"),
            Command("p", &["LRANGE", "last", "0", "-1"], b"ORACLE-pending"),
            Command("p", &["EXISTS", "l", "s", "from", "mid"], b"ORACLE-pending"),
            Aof(b"ORACLE-pending"),
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
            Command("p", &["BLMOVE", "missing", "d", "LEFT", "RIGHT", "0.001"], b"ORACLE-pending"),
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
            Command("p", &["EXISTS", "after"], b"ORACLE-pending"),
            Quiet("a"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s02-waiter-pipeline-producer-resp2, operation 10.
            Command("p", &["RPUSH", "l", "first"], b":1\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s02-waiter-pipeline-producer-resp2, operation 11 frame 0.
            Read("a", b"*2\r\n$1\r\nl\r\n$5\r\nfirst\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s02-waiter-pipeline-producer-resp2, operation 11 frame 1.
            Read("a", b"$4\r\ntail\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s02-waiter-pipeline-producer-resp2, operation 11 frame 2.
            Read("a", b":1\r\n"),
            Command("p", &["LRANGE", "after", "0", "-1"], b"ORACLE-pending"),
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
            Command("p", &["RPUSH", "x", "first"], b"ORACLE-pending"),
            Read("a", b"ORACLE-pending"),
            WaitBlocked(1),
            Command("p", &["CLIENT", "UNBLOCK", "@id:a", "ERROR"], b"ORACLE-pending"),
            Read("a", b"ORACLE-pending"),
            Read("a", b"ORACLE-pending"),
            WaitBlocked(0),
            Quiet("a"),
            Send("a", &["BLPOP", "z", "0"]),
            WaitBlocked(1),
            Command("p", &["RPUSH", "z", "final"], b"ORACLE-pending"),
            Read("a", b"ORACLE-pending"),
            WaitBlocked(0),
            Command("p", &["LRANGE", "d", "0", "-1"], b"ORACLE-pending"),
            Command("a", &["PING"], b"ORACLE-pending"),
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
            Command("p", &["DEL", "target"], b"ORACLE-pending"),
            Send("a", &["BRPOP", "target", "0"]),
            WaitBlocked(1),
            // Redis 7.2.16 run 38453128755, blocking-mc-s13-producer-rename-resp2, operation 3.
            Command("p", &["RPUSH", "source", "a", "b"], b":2\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s13-producer-copy-resp2, operation 7.
            Command("p", &["COPY", "source", "target"], b":1\r\n"),
            Read("a", b"ORACLE-pending"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s13-producer-copy-resp2, operation 13.
            Command("p", &["LRANGE", "source", "0", "-1"], b"*2\r\n$1\r\na\r\n$1\r\nb\r\n"),
            Command("p", &["DEL", "target"], b"ORACLE-pending"),
            Send("a", &["BLPOP", "target", "0"]),
            WaitBlocked(1),
            // Redis 7.2.16 run 38453128755, blocking-mc-s13-producer-lmove-resp2, operation 7.
            Command("p", &["LMOVE", "source", "target", "LEFT", "RIGHT"], b"$1\r\na\r\n"),
            // Redis 7.2.16 run 38453128755, blocking-mc-s13-producer-lmove-resp2, operation 8.
            Read("a", b"*2\r\n$6\r\ntarget\r\n$1\r\na\r\n"),
            Command("p", &["EXISTS", "target"], b"ORACLE-pending"),
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
            Command("p", &["EXISTS", "l1", "l2", "l3"], b"ORACLE-pending"),
        ],
    },
    Scenario {
        name: "expiry-before-ready",
        clients: &["p", "a"],
        aof: true,
        actions: &[
            Send("a", &["BLPOP", "l", "0"]),
            WaitBlocked(1),
            Command("p", &["MULTI"], b"ORACLE-pending"),
            Command("p", &["RPUSH", "l", "transient"], b"ORACLE-pending"),
            Command("p", &["PEXPIREAT", "l", "1"], b"ORACLE-pending"),
            Command("p", &["EXEC"], b"ORACLE-pending"),
            Quiet("a"),
            WaitBlocked(1),
            Command("p", &["EXISTS", "l"], b"ORACLE-pending"),
            Aof(b"ORACLE-pending"),
            Command("p", &["RPUSH", "l", "final"], b"ORACLE-pending"),
            Read("a", b"ORACLE-pending"),
            WaitBlocked(0),
            Aof(b"ORACLE-pending"),
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
            Command("p", &["LPUSHX", "l", "absent"], b"ORACLE-pending"),
            Command("p", &["RPUSHX", "l", "absent"], b"ORACLE-pending"),
            Command("p", &["LINSERT", "l", "AFTER", "absent", "value"], b"ORACLE-pending"),
            Quiet("a"),
            WaitBlocked(1),
            Command("p", &["MULTI"], b"ORACLE-pending"),
            Command("p", &["RPUSH", "l", "first"], b"ORACLE-pending"),
            Command("p", &["LPUSHX", "l", "left"], b"ORACLE-pending"),
            Command("p", &["RPUSHX", "l", "right"], b"ORACLE-pending"),
            Command("p", &["LINSERT", "l", "AFTER", "first", "inserted"], b"ORACLE-pending"),
            Command("p", &["EXEC"], b"ORACLE-pending"),
            Read("a", b"ORACLE-pending"),
            WaitBlocked(0),
            Command("p", &["LRANGE", "l", "0", "-1"], b"ORACLE-pending"),
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
            Command("b", &["CLIENT", "UNBLOCK", "@id:a", "TIMEOUT"], b"ORACLE-pending"),
            Read("a", b"ORACLE-pending"),
            Read("a", b"ORACLE-pending"),
            Read("b", b"ORACLE-pending"),
            WaitBlocked(0),
            Command("b", &["UNSUBSCRIBE", "news"], b"ORACLE-pending"),
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
            Command("p", &["EVAL", "redis.call('INCR',KEYS[1]); redis.call('RPUSH',KEYS[2],'once'); return redis.call('GET',KEYS[1])", "2", "counter", "fallback-list"], b"ORACLE-pending"),
            Read("a", b"ORACLE-pending"),
            WaitBlocked(0),
            Command("p", &["GET", "counter"], b"ORACLE-pending"),
            Command("p", &["LRANGE", "fallback-list", "0", "-1"], b"ORACLE-pending"),
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
            Command("a", &["BLPOP", "l", "-0.0015"], b"ORACLE-pending"),
            Command("a", &["BLPOP", "l", "-0.002"], b"ORACLE-pending"),
            Command("a", &["BRPOP", "l", "-0.0015"], b"ORACLE-pending"),
            Command("a", &["BRPOP", "l", "-0.002"], b"ORACLE-pending"),
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
            Command("a", &["BLPOP", "l", "-0.0015"], b"ORACLE-pending"),
            Command("a", &["BLPOP", "l", "-0.002"], b"ORACLE-pending"),
            Command("a", &["BRPOP", "l", "-0.0015"], b"ORACLE-pending"),
            Command("a", &["BRPOP", "l", "-0.002"], b"ORACLE-pending"),
            // Redis 7.2.16 run 38453128755, ORACLE-pending-blpop-exec-and-script, reply 14.
            Command("p", &["PING"], b"+PONG\r\n"),
        ],
    },
];
