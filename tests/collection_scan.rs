//! Redis 7.2.16 src/db.c scanGenericCommand/parseScanCursorOrReply and
//! t_set.c/t_hash.c/t_zset.c command wrappers define these expectations.
//! Collection order and the values of dict cursors are deliberately not
//! compared: design/decisions/commands/enumeration.md selects firn's order,
//! whole replies for compact collections and Whitefoot's hash-position cursor
//! for larger ones.
//! Recorder inputs for the literal Redis transcripts below live in
//! collection-scan-oracle.json; RESP selection is outside each transcript.

use super::*;

const EMPTY_SCAN: &[u8] = b"*2\r\n$1\r\n0\r\n*0\r\n";
const INVALID_CURSOR: &[u8] = b"-ERR invalid cursor\r\n";
const BAD_INTEGER: &[u8] = b"-ERR value is not an integer or out of range\r\n";
const SYNTAX: &[u8] = b"-ERR syntax error\r\n";
const WRONGTYPE: &[u8] = b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n";

// Raw concatenated Redis 7.2.16 replies to the named recorder batches,
// excluding HELLO. Recording: actions/runs/38398911733; command arrays
// were checked against collection-scan-oracle.json before importing bytes.
const DIRECT_RESP2: &str = ":1\r\n:1\r\n:1\r\n*2\r\n$1\r\n0\r\n*1\r\n$6\r\nmember\r\n*2\r\n$1\r\n0\r\n*2\r\n$5\r\nfield\r\n$5\r\nvalue\r\n*2\r\n$1\r\n0\r\n*2\r\n$6\r\nmember\r\n$1\r\n3\r\n*2\r\n$1\r\n0\r\n*0\r\n*2\r\n$1\r\n0\r\n*0\r\n+OK\r\n-WRONGTYPE Operation against a key holding the wrong kind of value\r\n-ERR invalid cursor\r\n-ERR syntax error\r\n-ERR value is not an integer or out of range\r\n-ERR syntax error\r\n+PONG\r\n";
const DIRECT_RESP3: &str = ":1\r\n:1\r\n:1\r\n*2\r\n$1\r\n0\r\n*1\r\n$6\r\nmember\r\n*2\r\n$1\r\n0\r\n*2\r\n$5\r\nfield\r\n$5\r\nvalue\r\n*2\r\n$1\r\n0\r\n*2\r\n$6\r\nmember\r\n$1\r\n3\r\n*2\r\n$1\r\n0\r\n*0\r\n*2\r\n$1\r\n0\r\n*0\r\n+OK\r\n-WRONGTYPE Operation against a key holding the wrong kind of value\r\n-ERR invalid cursor\r\n-ERR syntax error\r\n-ERR value is not an integer or out of range\r\n-ERR syntax error\r\n+PONG\r\n";
const MULTI_RESP2: &str = ":1\r\n:1\r\n:1\r\n+OK\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n*6\r\n*2\r\n$1\r\n0\r\n*1\r\n$6\r\nmember\r\n*2\r\n$1\r\n0\r\n*2\r\n$5\r\nfield\r\n$5\r\nvalue\r\n*2\r\n$1\r\n0\r\n*2\r\n$6\r\nmember\r\n$1\r\n3\r\n-ERR invalid cursor\r\n-ERR syntax error\r\n*2\r\n$1\r\n0\r\n*0\r\n+PONG\r\n";
const MULTI_RESP3: &str = ":1\r\n:1\r\n:1\r\n+OK\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n*6\r\n*2\r\n$1\r\n0\r\n*1\r\n$6\r\nmember\r\n*2\r\n$1\r\n0\r\n*2\r\n$5\r\nfield\r\n$5\r\nvalue\r\n*2\r\n$1\r\n0\r\n*2\r\n$6\r\nmember\r\n$1\r\n3\r\n-ERR invalid cursor\r\n-ERR syntax error\r\n*2\r\n$1\r\n0\r\n*0\r\n+PONG\r\n";
const SCRIPT_RESP2: &str = ":1\r\n:1\r\n:1\r\n*3\r\n*2\r\n$1\r\n0\r\n*1\r\n$6\r\nmember\r\n*2\r\n$1\r\n0\r\n*2\r\n$5\r\nfield\r\n$5\r\nvalue\r\n*2\r\n$1\r\n0\r\n*2\r\n$6\r\nmember\r\n$1\r\n3\r\n-ERR invalid cursor\r\n$16\r\nERR syntax error\r\n$6\r\ncaught\r\n*2\r\n$1\r\n0\r\n*1\r\n$6\r\nmember\r\n$1\r\n1\r\n+PONG\r\n";
const SCRIPT_RESP3: &str = ":1\r\n:1\r\n:1\r\n*3\r\n*2\r\n$1\r\n0\r\n*1\r\n$6\r\nmember\r\n*2\r\n$1\r\n0\r\n*2\r\n$5\r\nfield\r\n$5\r\nvalue\r\n*2\r\n$1\r\n0\r\n*2\r\n$6\r\nmember\r\n$1\r\n3\r\n-ERR invalid cursor\r\n$16\r\nERR syntax error\r\n$6\r\ncaught\r\n*2\r\n$1\r\n0\r\n*1\r\n$6\r\nmember\r\n$1\r\n1\r\n+PONG\r\n";
const LISTPACK_SCORES_RESP2: &str = ":5\r\n:1\r\n*2\r\n$1\r\n0\r\n*2\r\n$5\r\ntenth\r\n$3\r\n0.1\r\n*2\r\n$1\r\n0\r\n*2\r\n$5\r\nlarge\r\n$18\r\n100000000000000000\r\n*2\r\n$1\r\n0\r\n*2\r\n$4\r\nhalf\r\n$3\r\n1.5\r\n*2\r\n$1\r\n0\r\n*2\r\n$5\r\nthree\r\n$1\r\n3\r\n*2\r\n$1\r\n0\r\n*2\r\n$8\r\ninfinity\r\n$3\r\ninf\r\n*2\r\n$1\r\n0\r\n*2\r\n$8\r\nfraction\r\n$18\r\n0.6666666666666666\r\n";
const LISTPACK_SCORES_RESP3: &str = ":5\r\n:1\r\n*2\r\n$1\r\n0\r\n*2\r\n$5\r\ntenth\r\n$3\r\n0.1\r\n*2\r\n$1\r\n0\r\n*2\r\n$5\r\nlarge\r\n$18\r\n100000000000000000\r\n*2\r\n$1\r\n0\r\n*2\r\n$4\r\nhalf\r\n$3\r\n1.5\r\n*2\r\n$1\r\n0\r\n*2\r\n$5\r\nthree\r\n$1\r\n3\r\n*2\r\n$1\r\n0\r\n*2\r\n$8\r\ninfinity\r\n$3\r\ninf\r\n*2\r\n$1\r\n0\r\n*2\r\n$8\r\nfraction\r\n$18\r\n0.6666666666666666\r\n";

// Exact single-member probes from the listpack and forced-skiplist
// recordings above; RESP2 and RESP3 emitted identical bulk frames. Firn
// selects the former again after shrinking, unlike Redis encoding history.
const SCORE_MEMBERS: [&str; 6] = ["tenth", "large", "half", "three", "infinity", "fraction"];
const COMPACT_SCORE_REPLIES: [&[u8]; 6] = [
    b"*2\r\n$1\r\n0\r\n*2\r\n$5\r\ntenth\r\n$3\r\n0.1\r\n",
    b"*2\r\n$1\r\n0\r\n*2\r\n$5\r\nlarge\r\n$18\r\n100000000000000000\r\n",
    b"*2\r\n$1\r\n0\r\n*2\r\n$4\r\nhalf\r\n$3\r\n1.5\r\n",
    b"*2\r\n$1\r\n0\r\n*2\r\n$5\r\nthree\r\n$1\r\n3\r\n",
    b"*2\r\n$1\r\n0\r\n*2\r\n$8\r\ninfinity\r\n$3\r\ninf\r\n",
    b"*2\r\n$1\r\n0\r\n*2\r\n$8\r\nfraction\r\n$18\r\n0.6666666666666666\r\n",
];
const DICT_SCORE_REPLIES: [&[u8]; 6] = [
    b"*2\r\n$1\r\n0\r\n*2\r\n$5\r\ntenth\r\n$19\r\n0.10000000000000001\r\n",
    b"*2\r\n$1\r\n0\r\n*2\r\n$5\r\nlarge\r\n$5\r\n1e+17\r\n",
    b"*2\r\n$1\r\n0\r\n*2\r\n$4\r\nhalf\r\n$3\r\n1.5\r\n",
    b"*2\r\n$1\r\n0\r\n*2\r\n$5\r\nthree\r\n$1\r\n3\r\n",
    b"*2\r\n$1\r\n0\r\n*2\r\n$8\r\ninfinity\r\n$3\r\ninf\r\n",
    b"*2\r\n$1\r\n0\r\n*2\r\n$8\r\nfraction\r\n$19\r\n0.66666666666666663\r\n",
];

fn start(protocol: u8) -> (ProgramChild, TcpStream) {
    start_with(protocol, &[])
}

/// Starts firn with `extra` arguments after the port and the client count.
fn start_with(protocol: u8, extra: &[&[u8]]) -> (ProgramChild, TcpStream) {
    let lease = support::lease_port();
    let port = lease.port();
    let text = port.to_string();
    let mut arguments: Vec<&[u8]> = vec![b"--port", text.as_bytes(), b"--firn-clients", b"1"];
    arguments.extend_from_slice(extra);
    let child = firn().spawn_on_route(true, &arguments).with_port(&lease);
    let mut client = connect_when_ready(port, &child);
    if protocol == 3 {
        let reply = request(&mut client, &["HELLO", "3"]);
        assert!(reply.starts_with(b"%"), "HELLO 3: {reply:?}");
    }
    (child, client)
}

/// Binary-safe RESP2/RESP3 frame reader, including HELLO's RESP3 map.
fn frame(client: &mut TcpStream) -> Vec<u8> {
    let header = reply_line(client, "collection scan frame").into_bytes();
    let mut result = header.clone();
    let count = std::str::from_utf8(&header[1..header.len() - 2])
        .ok().and_then(|digits| digits.parse::<i64>().ok()).unwrap_or(-1);
    match header[0] {
        b'$' | b'!' | b'=' if count >= 0 => {
            let mut body = vec![0; count as usize + 2];
            client.read_exact(&mut body).expect("collection scan bulk body");
            assert!(body.ends_with(b"\r\n"));
            result.extend(body);
        }
        b'*' | b'~' | b'>' | b'%' if count >= 0 => {
            let children = if header[0] == b'%' { count * 2 } else { count };
            for _ in 0..children { result.extend(frame(client)); }
        }
        _ => {}
    }
    result
}

fn request(client: &mut TcpStream, args: &[&str]) -> Vec<u8> {
    client.write_all(&resp(args)).expect("collection scan request");
    frame(client)
}

fn line<'a>(bytes: &mut &'a [u8]) -> &'a [u8] {
    let end = bytes.windows(2).position(|pair| pair == b"\r\n").expect("RESP line");
    let result = &bytes[..end];
    *bytes = &bytes[end + 2..];
    result
}

fn bulk(bytes: &mut &[u8]) -> Vec<u8> {
    let header = line(bytes);
    assert_eq!(header[0], b'$', "SCAN elements must be bulk strings even in RESP3");
    let length = std::str::from_utf8(&header[1..]).unwrap().parse::<usize>().unwrap();
    let value = bytes[..length].to_vec();
    assert_eq!(&bytes[length..length + 2], b"\r\n");
    *bytes = &bytes[length + 2..];
    value
}

/// Splits one scan reply into its cursor and its items.
fn parts(reply: &[u8]) -> (Vec<u8>, Vec<Vec<u8>>) {
    let mut remaining = reply;
    assert_eq!(line(&mut remaining), b"*2", "SCAN outer reply: {reply:?}");
    let cursor = bulk(&mut remaining);
    let header = line(&mut remaining);
    assert_eq!(header[0], b'*', "SCAN inner array in both protocols");
    let length = std::str::from_utf8(&header[1..]).unwrap().parse::<usize>().unwrap();
    let result = (0..length).map(|_| bulk(&mut remaining)).collect();
    assert!(remaining.is_empty(), "trailing bytes: {remaining:?}");
    (cursor, result)
}

/// The items of a reply that must be the whole collection: cursor zero.
fn items(reply: &[u8]) -> Vec<Vec<u8>> {
    let (cursor, result) = parts(reply);
    assert_eq!(cursor, b"0", "a compact collection comes back whole, with cursor zero");
    result
}

/// Follows a scan from cursor zero to cursor zero, with `options` after the
/// cursor; returns every item in the order received and each call's item count.
fn scan_all(client: &mut TcpStream, scan: &str, key: &str, options: &[&str]) -> (Vec<Vec<u8>>, Vec<usize>) {
    let mut cursor = b"0".to_vec();
    let mut all = Vec::new();
    let mut sizes = Vec::new();
    for _ in 0..200_000 {
        let text = String::from_utf8(cursor.clone()).expect("decimal cursor");
        let mut args = vec![scan, key, text.as_str()];
        args.extend_from_slice(options);
        let (next, got) = parts(&request(client, &args));
        sizes.push(got.len());
        all.extend(got);
        if next == b"0" {
            return (all, sizes);
        }
        cursor = next;
    }
    panic!("{scan} {key} did not reach cursor zero");
}

/// Items grouped by `width` (1 for a set, 2 for a hash or sorted set) and
/// sorted, so that two scans compare without regard to order.
fn grouped(flat: &[Vec<u8>], width: usize) -> Vec<Vec<Vec<u8>>> {
    assert_eq!(flat.len() % width, 0, "items come in groups of {width}");
    let mut groups: Vec<Vec<Vec<u8>>> = flat.chunks(width).map(|group| group.to_vec()).collect();
    groups.sort();
    groups
}

fn assert_members(reply: &[u8], expected: &[Vec<u8>]) {
    let mut actual = items(reply);
    let mut expected = expected.to_vec();
    actual.sort(); expected.sort();
    assert_eq!(actual, expected, "every member exactly once");
}

fn assert_pairs(reply: &[u8], expected: &[(Vec<u8>, Vec<u8>)]) {
    let actual = items(reply);
    assert_eq!(actual.len() % 2, 0, "field/member followed by its value/score");
    let mut actual: Vec<_> = actual.chunks_exact(2).map(|pair| (pair[0].clone(), pair[1].clone())).collect();
    let mut expected = expected.to_vec();
    actual.sort(); expected.sort();
    assert_eq!(actual, expected, "sorting pairs must preserve their association");
}

fn direct_batch() -> Vec<Vec<&'static str>> {
    vec![
        vec!["SADD", "scan:set", "member"],
        vec!["HSET", "scan:hash", "field", "value"],
        vec!["ZADD", "scan:zset", "3", "member"],
        vec!["SSCAN", "scan:set", "0", "COUNT", "1"],
        vec!["HSCAN", "scan:hash", "0", "MATCH", "f*"],
        vec!["ZSCAN", "scan:zset", "0", "MATCH", "m*"],
        vec!["SSCAN", "scan:set", "0", "MATCH", "absent"],
        vec!["HSCAN", "scan:missing", "0", "COUNT", "not-a-number"],
        vec!["SET", "scan:wrong", "string"],
        vec!["ZSCAN", "scan:wrong", "0", "COUNT", "not-a-number"],
        vec!["SSCAN", "scan:missing", "bad"],
        vec!["SSCAN", "scan:set", "0", "COUNT", "0"],
        vec!["HSCAN", "scan:hash", "0", "COUNT", "not-a-number"],
        vec!["ZSCAN", "scan:zset", "0", "NOVALUES"],
        vec!["PING"],
    ]
}

fn multi_batch() -> Vec<Vec<&'static str>> {
    vec![
        vec!["SADD", "scan:set", "member"],
        vec!["HSET", "scan:hash", "field", "value"],
        vec!["ZADD", "scan:zset", "3", "member"],
        vec!["MULTI"],
        vec!["SSCAN", "scan:set", "0"],
        vec!["HSCAN", "scan:hash", "0"],
        vec!["ZSCAN", "scan:zset", "0"],
        vec!["SSCAN", "scan:set", "bad"],
        vec!["HSCAN", "scan:hash", "0", "COUNT", "0"],
        vec!["ZSCAN", "scan:missing", "0", "NOVALUES"],
        vec!["EXEC"],
        vec!["PING"],
    ]
}

fn script_batch() -> Vec<Vec<&'static str>> {
    vec![
        vec!["SADD", "scan:set", "member"],
        vec!["HSET", "scan:hash", "field", "value"],
        vec!["ZADD", "scan:zset", "3", "member"],
        vec!["EVAL", "return {redis.call('SSCAN',KEYS[1],0),redis.call('HSCAN',KEYS[2],0),redis.call('ZSCAN',KEYS[3],0)}", "3", "scan:set", "scan:hash", "scan:zset"],
        vec!["EVAL", "return redis.pcall('SSCAN',KEYS[1],'bad')", "1", "scan:set"],
        vec!["EVAL", "local r=redis.pcall('HSCAN',KEYS[1],0,'COUNT',0); return r.err", "1", "scan:hash"],
        vec!["EVAL", "local ok,r=pcall(function() return redis.call('ZSCAN',KEYS[1],0,'NOVALUES') end); return ok and 'unexpected success' or 'caught'", "1", "scan:zset"],
        vec!["EVAL", "redis.call('INCR',KEYS[1]); return redis.call('SSCAN','scan:set',0)", "1", "scan:counter"],
        vec!["GET", "scan:counter"],
        vec!["PING"],
    ]
}

fn score_batch() -> Vec<Vec<&'static str>> {
    vec![
        vec!["ZADD", "scan:scores", "0.1", "tenth", "1e17", "large", "1.5", "half", "3", "three", "inf", "infinity"],
        vec!["EVAL", "return redis.call('ZADD',KEYS[1],2/3,'fraction')", "1", "scan:scores"],
        vec!["ZSCAN", "scan:scores", "0", "COUNT", "1000", "MATCH", "tenth"],
        vec!["ZSCAN", "scan:scores", "0", "COUNT", "1000", "MATCH", "large"],
        vec!["ZSCAN", "scan:scores", "0", "COUNT", "1000", "MATCH", "half"],
        vec!["ZSCAN", "scan:scores", "0", "COUNT", "1000", "MATCH", "three"],
        vec!["ZSCAN", "scan:scores", "0", "COUNT", "1000", "MATCH", "infinity"],
        vec!["ZSCAN", "scan:scores", "0", "COUNT", "1000", "MATCH", "fraction"],
    ]
}

fn oracle(protocol: u8, name: &str, commands: Vec<Vec<&str>>, expected: &str) {
    let (child, mut client) = start(protocol);
    let mut actual = Vec::new();
    for command in commands { actual.extend(request(&mut client, &command)); }
    drop(client);
    assert_eq!(finished(child).0, 0);
    assert_eq!(actual, expected.as_bytes(), "Redis 7.2.16 batch {name}");
}

#[test] fn direct_resp2_oracle() { oracle(2, "direct-resp2", direct_batch(), DIRECT_RESP2); }
#[test] fn direct_resp3_oracle() { oracle(3, "direct-resp3", direct_batch(), DIRECT_RESP3); }
#[test] fn multi_resp2_oracle() { oracle(2, "multi-resp2", multi_batch(), MULTI_RESP2); }
#[test] fn multi_resp3_oracle() { oracle(3, "multi-resp3", multi_batch(), MULTI_RESP3); }
#[test] fn script_resp2_oracle() { oracle(2, "script-resp2", script_batch(), SCRIPT_RESP2); }
#[test] fn script_resp3_oracle() { oracle(3, "script-resp3", script_batch(), SCRIPT_RESP3); }
#[test] fn listpack_scores_resp2_oracle() { oracle(2, "listpack-scores-resp2", score_batch(), LISTPACK_SCORES_RESP2); }
#[test] fn listpack_scores_resp3_oracle() { oracle(3, "listpack-scores-resp3", score_batch(), LISTPACK_SCORES_RESP3); }

fn seed_scores(client: &mut TcpStream) {
    let commands = score_batch();
    assert_eq!(request(client, &commands[0]), b":5\r\n");
    assert_eq!(request(client, &commands[1]), b":1\r\n");
}

/// Every direct probe checks the recorded bulk frame, including all six score
/// spellings. EXEC and Lua also check the three spellings that distinguish the
/// writers. MATCH hides each boundary trigger, so classification must inspect
/// the whole current zset before filtering. A compact sorted set ignores COUNT;
/// above the compact bound COUNT must reach every bucket for the one call to
/// hold the whole answer, so each probe passes the COUNT its form needs.
fn assert_score_probes(client: &mut TcpStream, expected: &[&[u8]; 6], count: &str) {
    for (member, reply) in SCORE_MEMBERS.iter().zip(expected) {
        assert_eq!(request(client, &["ZSCAN", "scan:scores", "0", "MATCH", member, "COUNT", count]), *reply, "score for {member}");
    }
    assert_eq!(request(client, &["MULTI"]), b"+OK\r\n");
    let mut replies = b"*3\r\n".to_vec();
    for index in [0, 1, 5] {
        assert_eq!(request(client, &["ZSCAN", "scan:scores", "0", "MATCH", SCORE_MEMBERS[index], "COUNT", count]), b"+QUEUED\r\n");
        replies.extend_from_slice(expected[index]);
    }
    assert_eq!(request(client, &["EXEC"]), replies);
    for index in [0, 1, 5] {
        assert_eq!(request(client, &["EVAL", "return redis.call('ZSCAN',KEYS[1],0,'MATCH',ARGV[1],'COUNT',ARGV[2])", "1", "scan:scores", SCORE_MEMBERS[index], count]), expected[index]);
    }
}

#[test]
fn zscan_score_text_uses_current_cardinality_before_match() {
    for protocol in [2, 3] {
        let (child, mut client) = start(protocol);
        seed_scores(&mut client);
        // Six score probes plus 122 fillers reach the compact limit exactly.
        for index in 0..122 {
            let member = format!("padding:{index}");
            assert_eq!(request(&mut client, &["ZADD", "scan:scores", "3", &member]), b":1\r\n");
        }
        assert_eq!(request(&mut client, &["ZCARD", "scan:scores"]), b":128\r\n");
        assert_score_probes(&mut client, &COMPACT_SCORE_REPLIES, "1");
        assert_eq!(request(&mut client, &["ZADD", "scan:scores", "3", "overflow"]), b":1\r\n");
        assert_eq!(request(&mut client, &["ZCARD", "scan:scores"]), b":129\r\n");
        assert_score_probes(&mut client, &DICT_SCORE_REPLIES, "100000");
        // This is the proposed divergence from Redis's persistent skiplist:
        // firn switches back at 128 without retaining any encoding history.
        assert_eq!(request(&mut client, &["ZREM", "scan:scores", "overflow"]), b":1\r\n");
        assert_score_probes(&mut client, &COMPACT_SCORE_REPLIES, "1");
        drop(client); assert_eq!(finished(child).0, 0);
    }
}

#[test]
fn zscan_score_text_uses_current_member_length_before_match() {
    for protocol in [2, 3] {
        let (child, mut client) = start(protocol);
        seed_scores(&mut client);
        let within = "m".repeat(64);
        let beyond = "é".repeat(32) + "x"; // 65 bytes, only 33 characters.
        assert_eq!(request(&mut client, &["ZADD", "scan:scores", "3", &within]), b":1\r\n");
        assert_score_probes(&mut client, &COMPACT_SCORE_REPLIES, "1");
        assert_eq!(request(&mut client, &["ZADD", "scan:scores", "3", &beyond]), b":1\r\n");
        assert_score_probes(&mut client, &DICT_SCORE_REPLIES, "100000");
        assert_eq!(request(&mut client, &["ZREM", "scan:scores", &beyond]), b":1\r\n");
        assert_score_probes(&mut client, &COMPACT_SCORE_REPLIES, "1");
        drop(client); assert_eq!(finished(child).0, 0);
    }
}

/// Cursor uses strtoul, unlike COUNT's canonical signed-long parser. The
/// wrappers parse cursor before lookup, but lookup precedes option parsing.
/// A generic argument validator before lookup would fail missing/wrongtype
/// precedence; a shared signed cursor parser would reject Redis's -1/max.
#[test]
fn cursor_options_and_lookup_precedence() {
    for protocol in [2, 3] {
        let (child, mut client) = start(protocol);
        assert_eq!(request(&mut client, &["SET", "wrong", "value"]), b"+OK\r\n");
        for (scan, create) in [
            ("SSCAN", vec!["SADD", "good", "member"]),
            ("HSCAN", vec!["HSET", "good", "field", "value"]),
            ("ZSCAN", vec!["ZADD", "good", "3", "member"]),
        ] {
            request(&mut client, &["DEL", "good"]);
            assert_eq!(request(&mut client, &create), b":1\r\n");
            for cursor in ["", "0", "+0", "-0", "00", "-1", "18446744073709551615", "+18446744073709551615", "-18446744073709551615", "0\0ignored"] {
                assert_eq!(request(&mut client, &[scan, "missing", cursor]), EMPTY_SCAN, "{scan}: {cursor:?}");
                assert!(!items(&request(&mut client, &[scan, "good", cursor])).is_empty(), "compact collection ignores cursor position");
            }
            for cursor in [" ", " 0", "\t0", "0 ", "+", "-", "x", "1x", "18446744073709551616", "-18446744073709551616"] {
                for key in ["missing", "wrong", "good"] {
                    assert_eq!(request(&mut client, &[scan, key, cursor, "COUNT", "0"]), INVALID_CURSOR, "{scan}: {cursor:?}, {key}");
                }
            }
            for options in [vec!["COUNT", "bad"], vec!["COUNT", "0"], vec!["MATCH"], vec!["NOVALUES"], vec!["TYPE", "set"]] {
                let mut args = vec![scan, "missing", "0"]; args.extend(&options);
                assert_eq!(request(&mut client, &args), EMPTY_SCAN, "missing key ignores options");
                args[1] = "wrong";
                assert_eq!(request(&mut client, &args), WRONGTYPE, "wrongtype precedes options");
            }
            for count in ["", "+1", "01", "1.0", "bad", "9223372036854775808", "-9223372036854775809", "1\0ignored"] {
                assert_eq!(request(&mut client, &[scan, "good", "0", "COUNT", count]), BAD_INTEGER, "COUNT {count:?}");
            }
            for count in ["0", "-1", "-9223372036854775808"] {
                assert_eq!(request(&mut client, &[scan, "good", "0", "COUNT", count]), SYNTAX, "COUNT {count:?}");
            }
            for options in [vec!["COUNT"], vec!["MATCH"], vec!["TYPE", "set"], vec!["NOVALUES"], vec!["UNKNOWN"]] {
                let mut args = vec![scan, "good", "0"]; args.extend(options);
                assert_eq!(request(&mut client, &args), SYNTAX, "collection-only options");
            }
            for count in ["1", "9223372036854775807"] {
                assert!(!items(&request(&mut client, &[scan, "good", "0", "count", count])).is_empty());
            }
            // Option names use strcasecmp on their C string; the pattern
            // itself uses sdslen, so bytes after NUL remain significant.
            assert!(!items(&request(&mut client, &[scan, "good", "0", "COUNT\0ignored", "1"])).is_empty());
            assert!(!items(&request(&mut client, &[scan, "good", "0", "MATCH\0ignored", "*"])).is_empty());
            assert_eq!(request(&mut client, &[scan, "good", "0", "MATCH\0ignored", "*\0ignored"]), EMPTY_SCAN);
            assert_eq!(request(&mut client, &[scan, "good", "0", "COUNT", "1", "COUNT", "0"]), SYNTAX);
            assert_eq!(request(&mut client, &[scan, "good", "0", "COUNT", "bad", "COUNT", "1"]), BAD_INTEGER);
            assert_eq!(request(&mut client, &[scan, "good", "0", "MATCH", "*", "MATCH", "nothing"]), EMPTY_SCAN);
            assert!(!items(&request(&mut client, &[scan, "good", "0", "MATCH", "nothing", "MATCH", "*", "COUNT", "1", "COUNT", "2"])).is_empty());
            for args in [vec![scan], vec![scan, "good"]] {
                assert_eq!(request(&mut client, &args), format!("-ERR wrong number of arguments for '{}' command\r\n", scan.to_ascii_lowercase()).as_bytes());
            }
            assert_eq!(request(&mut client, &["PING"]), b"+PONG\r\n", "errors leave the connection usable");
        }
        drop(client); assert_eq!(finished(child).0, 0);
    }
}

/// MATCH tests bytes of members/fields only, with complete value associations.
/// NUL and UTF-8 bytes must not terminate or change the glob match.
#[test]
fn binary_match_filters_members_and_fields_not_values() {
    for protocol in [2, 3] {
        let (child, mut client) = start(protocol);
        let names = ["a\0b", "a\0c", "é", "star*", "other", ""];
        for name in names {
            assert_eq!(request(&mut client, &["SADD", "binary:s", name]), b":1\r\n");
            assert_eq!(request(&mut client, &["HSET", "binary:h", name, "a\0b"]), b":1\r\n");
            assert_eq!(request(&mut client, &["ZADD", "binary:z", "3", name]), b":1\r\n");
        }
        for (pattern, selected) in [
            ("a\0?", vec!["a\0b", "a\0c"]), ("a\0[b]", vec!["a\0b"]),
            ("a\0[^b]", vec!["a\0c"]), ("star\\*", vec!["star*"]),
            // Redis util.c stringmatchlen_impl returns true for zero pattern
            // length and zero string length. Exact "*" bypasses matching in
            // scanGenericCommand and includes the empty member too.
            ("??", vec!["é"]), ("", vec![""]), ("*", names.to_vec()),
        ] {
            let members: Vec<_> = selected.iter().map(|name| name.as_bytes().to_vec()).collect();
            assert_members(&request(&mut client, &["SSCAN", "binary:s", "0", "MATCH", pattern]), &members);
            let hashes: Vec<_> = members.iter().map(|name| (name.clone(), b"a\0b".to_vec())).collect();
            assert_pairs(&request(&mut client, &["HSCAN", "binary:h", "0", "MATCH", pattern]), &hashes);
            let scores: Vec<_> = members.iter().map(|name| (name.clone(), b"3".to_vec())).collect();
            assert_pairs(&request(&mut client, &["ZSCAN", "binary:z", "0", "MATCH", pattern]), &scores);
        }
        // Exactly one matching field, despite every value matching too.
        assert_eq!(request(&mut client, &["HSCAN", "binary:h", "0", "MATCH", "a\0b"]),
            b"*2\r\n$1\r\n0\r\n*2\r\n$3\r\na\0b\r\n$3\r\na\0b\r\n");
        drop(client); assert_eq!(finished(child).0, 0);
    }
}

/// One collection at one size. A compact collection is one call with cursor
/// zero even at COUNT 1. A larger one cannot finish at COUNT 1, so its first
/// reply carries a nonzero cursor, and following the cursor returns every
/// member exactly once while nothing changes.
fn assert_scan(client: &mut TcpStream, scan: &str, key: &str, compact: bool, expected: &[Vec<u8>], width: usize) {
    let (cursor, first) = parts(&request(client, &[scan, key, "0", "COUNT", "1"]));
    if compact {
        assert_eq!(cursor, b"0", "{scan} {key}: a compact collection ignores COUNT");
        assert_eq!(grouped(&first, width), grouped(expected, width), "{scan} {key}: compact whole");
    } else {
        assert_ne!(cursor, b"0".to_vec(), "{scan} {key}: a larger collection continues");
        if expected.len() > 16 {
            assert!(first.len() < expected.len(), "{scan} {key}: one step must not hold the collection");
        }
    }
    let (all, _) = scan_all(client, scan, key, &["COUNT", "16"]);
    assert_eq!(grouped(&all, width), grouped(expected, width), "{scan} {key}: every member once");
}

/// Compact bounds are Redis's default listpack and intset limits: 128 members of
/// a set or sorted set, 512 fields of a hash, each at most 64 bytes, a hash's
/// values too, and 512 members of a set of integers. firn applies them to the
/// current content, with no encoding history.
#[test]
fn compact_collections_return_whole_and_larger_ones_scan_in_steps() {
    for protocol in [2, 3] {
        let (child, mut client) = start(protocol);
        let mut members: Vec<Vec<u8>> = Vec::new();
        let mut integers: Vec<Vec<u8>> = Vec::new();
        let mut pairs: Vec<Vec<u8>> = Vec::new();
        let mut scored: Vec<Vec<u8>> = Vec::new();
        for index in 0..513 {
            let member = format!("m{index:03}");
            let integer = index.to_string();
            let value = format!("v{index:03}");
            request(&mut client, &["SADD", "many:s", &member]);
            request(&mut client, &["SADD", "many:i", &integer]);
            request(&mut client, &["HSET", "many:h", &member, &value]);
            request(&mut client, &["ZADD", "many:z", "3", &member]);
            members.push(member.as_bytes().to_vec());
            integers.push(integer.into_bytes());
            pairs.push(member.as_bytes().to_vec());
            pairs.push(value.into_bytes());
            scored.push(member.as_bytes().to_vec());
            scored.push(b"3".to_vec());
            let size = index + 1;
            if [1, 128, 129, 512, 513].contains(&size) {
                assert_scan(&mut client, "SSCAN", "many:s", size <= 128, &members, 1);
                assert_scan(&mut client, "SSCAN", "many:i", size <= 512, &integers, 1);
                assert_scan(&mut client, "HSCAN", "many:h", size <= 512, &pairs, 2);
                assert_scan(&mut client, "ZSCAN", "many:z", size <= 128, &scored, 2);
            }
        }
        // 64 bytes keeps a collection compact, 65 does not, in a member or
        // field and also in a hash's value. The collection holds 100 members, so
        // that one that is not compact cannot be finished by a call at COUNT 1.
        for length in [64, 65] {
            let member = "m".repeat(length);
            let value = "v".repeat(length);
            request(&mut client, &["DEL", "long:s", "long:h", "long:v", "long:z"]);
            let mut short: Vec<Vec<u8>> = Vec::new();
            for index in 0..99 {
                let name = format!("s{index:02}");
                request(&mut client, &["SADD", "long:s", &name]);
                request(&mut client, &["HSET", "long:h", &name, "v"]);
                request(&mut client, &["HSET", "long:v", &name, "v"]);
                request(&mut client, &["ZADD", "long:z", "3", &name]);
                short.push(name.into_bytes());
            }
            request(&mut client, &["SADD", "long:s", &member]);
            request(&mut client, &["HSET", "long:h", &member, "v"]);
            request(&mut client, &["HSET", "long:v", "f", &value]);
            request(&mut client, &["ZADD", "long:z", "3", &member]);
            let compact = length == 64;
            let mut set = short.clone();
            set.push(member.as_bytes().to_vec());
            let mut hash: Vec<Vec<u8>> = short.iter().flat_map(|name| [name.clone(), b"v".to_vec()]).collect();
            let mut valued = hash.clone();
            hash.extend([member.as_bytes().to_vec(), b"v".to_vec()]);
            valued.extend([b"f".to_vec(), value.into_bytes()]);
            let mut sorted: Vec<Vec<u8>> = short.iter().flat_map(|name| [name.clone(), b"3".to_vec()]).collect();
            sorted.extend([member.into_bytes(), b"3".to_vec()]);
            assert_scan(&mut client, "SSCAN", "long:s", compact, &set, 1);
            assert_scan(&mut client, "HSCAN", "long:h", compact, &hash, 2);
            assert_scan(&mut client, "HSCAN", "long:v", compact, &valued, 2);
            assert_scan(&mut client, "ZSCAN", "long:z", compact, &sorted, 2);
        }
        drop(client); assert_eq!(finished(child).0, 0);
    }
}

/// A large collection is followed in many small steps, each as large as COUNT
/// says and no larger, and MATCH filters every step.
#[test]
fn large_collections_scan_in_bounded_steps() {
    for protocol in [2, 3] {
        let (child, mut client) = start(protocol);
        let mut members = Vec::new();
        let mut pairs = Vec::new();
        let mut scored = Vec::new();
        for index in 0..2000 {
            let member = format!("m{index:04}");
            let value = format!("v{index:04}");
            request(&mut client, &["SADD", "big:s", &member]);
            request(&mut client, &["HSET", "big:h", &member, &value]);
            request(&mut client, &["ZADD", "big:z", "0.1", &member]);
            members.push(member.as_bytes().to_vec());
            pairs.push(member.as_bytes().to_vec());
            pairs.push(value.into_bytes());
            scored.push(member.into_bytes());
            // Above the compact bound a sorted set writes scores as %.17g.
            scored.push(b"0.10000000000000001".to_vec());
        }
        for (scan, key, width, expected) in [("SSCAN", "big:s", 1, &members), ("HSCAN", "big:h", 2, &pairs), ("ZSCAN", "big:z", 2, &scored)] {
            let (all, sizes) = scan_all(&mut client, scan, key, &["COUNT", "20"]);
            assert_eq!(grouped(&all, width), grouped(expected, width), "{scan}: every member once");
            assert!(sizes.len() > 20, "{scan}: {} calls is too few for 2000 members at COUNT 20", sizes.len());
            let most = sizes.iter().map(|size| size / width).max().unwrap();
            assert!(most < 200, "{scan}: a step of COUNT 20 returned {most} members");
            // MATCH applies to every step and the cursor still ends at zero.
            let (matched, _) = scan_all(&mut client, scan, key, &["MATCH", "m19*", "COUNT", "50"]);
            let want: Vec<Vec<u8>> = expected.chunks(width).filter(|group| group[0].starts_with(b"m19")).flatten().cloned().collect();
            assert_eq!(grouped(&matched, width), grouped(&want, width), "{scan}: MATCH over steps");
        }
        drop(client); assert_eq!(finished(child).0, 0);
    }
}

/// Any cursor is accepted; COUNT sizes the step, and a COUNT beyond the table
/// holds the whole collection with cursor zero, as it does in Redis.
#[test]
fn any_cursor_is_accepted_and_count_sizes_the_step() {
    for protocol in [2, 3] {
        let (child, mut client) = start(protocol);
        for index in 0..1000 {
            let member = format!("m{index:04}");
            request(&mut client, &["SADD", "any:s", &member]);
            request(&mut client, &["HSET", "any:h", &member, "v"]);
            request(&mut client, &["ZADD", "any:z", "3", &member]);
        }
        for (scan, key, width) in [("SSCAN", "any:s", 1), ("HSCAN", "any:h", 2), ("ZSCAN", "any:z", 2)] {
            for cursor in ["1", "12345", "9223372036854775808", "18446744073709551615"] {
                let (_, got) = parts(&request(&mut client, &[scan, key, cursor, "COUNT", "10"]));
                assert!(got.len() / width < 100, "{scan} from {cursor}: {} members for COUNT 10", got.len() / width);
            }
            let (cursor, got) = parts(&request(&mut client, &[scan, key, "0", "COUNT", "1000000"]));
            assert_eq!(cursor, b"0", "{scan}: COUNT beyond the table finishes in one call");
            assert_eq!(got.len(), 1000 * width, "{scan}: the whole collection");
            // Nothing changed between two calls from one cursor, so they agree.
            let (cursor, _) = parts(&request(&mut client, &[scan, key, "0", "COUNT", "10"]));
            assert_ne!(cursor, b"0".to_vec(), "{scan}: the first of many steps");
            let text = String::from_utf8(cursor).unwrap();
            assert_eq!(request(&mut client, &[scan, key, &text, "COUNT", "10"]), request(&mut client, &[scan, key, &text, "COUNT", "10"]), "{scan}: one cursor, one step");
        }
        drop(client); assert_eq!(finished(child).0, 0);
    }
}

/// Adds or removes one member of the named kind of collection.
fn change(client: &mut TcpStream, scan: &str, key: &str, add: bool, member: &str) {
    let command = match (scan, add) {
        ("SSCAN", true) => vec!["SADD", key, member],
        ("SSCAN", false) => vec!["SREM", key, member],
        ("HSCAN", true) => vec!["HSET", key, member, "v"],
        ("HSCAN", false) => vec!["HDEL", key, member],
        ("ZSCAN", true) => vec!["ZADD", key, "1", member],
        _ => vec!["ZREM", key, member],
    };
    request(client, &command);
}

/// The guarantee Redis gives a scan: every member present from the first call
/// to the last comes back at least once, whatever else is added and removed
/// between calls, even a burst that makes the table grow and a removal that
/// leaves vacated buckets for a rebuild. New and removed members may or may not
/// appear. Each step is as small as COUNT 4 says, so the scan crosses the burst.
#[test]
fn members_present_throughout_are_returned_while_the_collection_changes() {
    for protocol in [2, 3] {
        let (child, mut client) = start(protocol);
        for (scan, key, width) in [("SSCAN", "churn:s", 1), ("HSCAN", "churn:h", 2), ("ZSCAN", "churn:z", 2)] {
            let kept: Vec<String> = (0..700).map(|index| format!("keep{index:04}")).collect();
            for member in &kept {
                change(&mut client, scan, key, true, member);
            }
            let mut seen = std::collections::HashSet::new();
            let mut cursor = "0".to_string();
            let mut fresh = 0;
            let mut oldest = 0;
            let mut burst = Vec::new();
            for step in 0..200_000 {
                let (next, got) = parts(&request(&mut client, &[scan, key, &cursor, "COUNT", "4"]));
                for group in got.chunks(width) {
                    seen.insert(group[0].clone());
                }
                if next == b"0" {
                    break;
                }
                cursor = String::from_utf8(next).unwrap();
                assert!(step < 199_999, "{scan}: no end in sight");
                for _ in 0..5 {
                    change(&mut client, scan, key, true, &format!("fresh{fresh:06}"));
                    fresh += 1;
                }
                for _ in 0..5 {
                    change(&mut client, scan, key, false, &format!("fresh{oldest:06}"));
                    oldest += 1;
                }
                if step == 3 {
                    for index in 0..1500 {
                        let member = format!("burst{index:04}");
                        change(&mut client, scan, key, true, &member);
                        burst.push(member);
                    }
                }
                if step == 8 {
                    for member in burst.drain(..) {
                        change(&mut client, scan, key, false, &member);
                    }
                }
            }
            for member in &kept {
                assert!(seen.contains(member.as_bytes()), "{scan}: {member} was present throughout and never returned");
            }
        }
        drop(client); assert_eq!(finished(child).0, 0);
    }
}

/// Scripts follow a large scan across calls in one attempt, for a declared key
/// and, through the retry with the whole keyspace held, an undeclared one.
#[test]
fn scripts_follow_a_large_scan_to_cursor_zero() {
    for protocol in [2, 3] {
        let (child, mut client) = start(protocol);
        for index in 0..1000 {
            let member = format!("m{index:04}");
            request(&mut client, &["SADD", "script:s", &member]);
            request(&mut client, &["HSET", "script:h", &member, "v"]);
            request(&mut client, &["ZADD", "script:z", "3", &member]);
        }
        let source = "local c='0'; local n=0; repeat local r=redis.call(ARGV[1],ARGV[2],c,'COUNT',20); c=r[1]; n=n+#r[2] until c=='0'; return n";
        for (scan, key, count) in [("SSCAN", "script:s", 1000), ("HSCAN", "script:h", 2000), ("ZSCAN", "script:z", 2000)] {
            for declared in [key, "script:other"] {
                assert_eq!(request(&mut client, &["EVAL", source, "1", declared, scan, key]), format!(":{count}\r\n").as_bytes(), "{scan} with KEYS[1] {declared}");
            }
        }
        drop(client); assert_eq!(finished(child).0, 0);
    }
}

/// Reading an undeclared collection retries an attempt after its earlier
/// write. Redis executes that write once and keeps it on an ordinary error;
/// fallback must be invisible to call/pcall and not become WRONGTYPE.
#[test]
fn script_collection_reads_retry_once_and_keep_writes_on_errors() {
    for protocol in [2, 3] {
        let (child, mut client) = start(protocol);
        for args in [vec!["SADD", "outside:s", "member"], vec!["HSET", "outside:h", "field", "value"], vec!["ZADD", "outside:z", "3", "member"]] {
            assert_eq!(request(&mut client, &args), b":1\r\n");
        }
        for (scan, key) in [("SSCAN", "outside:s"), ("HSCAN", "outside:h"), ("ZSCAN", "outside:z")] {
            for (api, declared) in [("call", true), ("call", false), ("pcall", false)] {
                let counter = format!("counter:{scan}:{api}:{declared}");
                let source = format!("redis.call('INCR',KEYS[1]); local r=redis.{api}('{scan}','{key}',0); if r.err then return r.err end; return {{r[1],#r[2]}}");
                let args = if declared { vec!["EVAL", source.as_str(), "2", counter.as_str(), key] } else { vec!["EVAL", source.as_str(), "1", counter.as_str()] };
                let length = if scan == "SSCAN" { 1 } else { 2 };
                assert_eq!(request(&mut client, &args), format!("*2\r\n$1\r\n0\r\n:{length}\r\n").as_bytes());
                assert_eq!(request(&mut client, &["GET", &counter]), b"$1\r\n1\r\n", "earlier write must not be replayed twice");
            }
            let counter = format!("error:{scan}");
            let source = format!("redis.call('INCR',KEYS[1]); return redis.pcall('{scan}','{key}',0,'COUNT',0)");
            assert_eq!(request(&mut client, &["EVAL", &source, "1", &counter]), SYNTAX);
            assert_eq!(request(&mut client, &["GET", &counter]), b"$1\r\n1\r\n", "accepted Lua errors retain earlier writes");
        }
        drop(client); assert_eq!(finished(child).0, 0);
    }
}

/// Expired collections behave as missing before options are parsed in direct,
/// transaction and script calls; cursor errors still win. Active expiry may
/// already remove these keys, so this does not isolate lazy deletion's branch.
#[test]
fn expired_collections_are_missing_in_direct_exec_and_script_reads() {
    for protocol in [2, 3] {
        let (child, mut client) = start(protocol);
        for (key, create) in [
            ("expired:s", vec!["SADD", "expired:s", "member"]),
            ("expired:h", vec!["HSET", "expired:h", "field", "value"]),
            ("expired:z", vec!["ZADD", "expired:z", "3", "member"]),
        ] {
            assert_eq!(request(&mut client, &create), b":1\r\n");
            assert_eq!(request(&mut client, &["PEXPIRE", key, "1"]), b":1\r\n");
        }
        std::thread::sleep(Duration::from_millis(20));
        let scans = [("SSCAN", "expired:s"), ("HSCAN", "expired:h"), ("ZSCAN", "expired:z")];
        for (scan, key) in scans {
            assert_eq!(request(&mut client, &[scan, key, "0", "COUNT", "bad"]), EMPTY_SCAN);
            assert_eq!(request(&mut client, &[scan, key, "bad", "COUNT", "0"]), INVALID_CURSOR);
        }
        assert_eq!(request(&mut client, &["MULTI"]), b"+OK\r\n");
        let mut expected = b"*6\r\n".to_vec();
        for (scan, key) in scans {
            assert_eq!(request(&mut client, &[scan, key, "0", "COUNT", "bad"]), b"+QUEUED\r\n");
            assert_eq!(request(&mut client, &[scan, key, "bad", "COUNT", "0"]), b"+QUEUED\r\n");
            expected.extend_from_slice(EMPTY_SCAN);
            expected.extend_from_slice(INVALID_CURSOR);
        }
        assert_eq!(request(&mut client, &["EXEC"]), expected);
        for (scan, key) in scans {
            let source = format!("return redis.call('{scan}',KEYS[1],0,'COUNT','bad')");
            assert_eq!(request(&mut client, &["EVAL", &source, "1", key]), EMPTY_SCAN);
            let source = format!("return redis.pcall('{scan}',KEYS[1],'bad','COUNT',0)");
            assert_eq!(request(&mut client, &["EVAL", &source, "1", key]), INVALID_CURSOR);
        }
        drop(client); assert_eq!(finished(child).0, 0);
    }
}

/// SADD/HSET/ZADD (or SREM/HDEL/ZREM) of the members `{prefix}{index:05}` for
/// `range`, 500 to a command so that a collection of 20000 takes few requests.
fn change_many(client: &mut TcpStream, scan: &str, key: &str, add: bool, prefix: &str, range: std::ops::Range<usize>) {
    let indexes: Vec<usize> = range.collect();
    for chunk in indexes.chunks(500) {
        let mut args: Vec<String> = Vec::new();
        match (scan, add) {
            ("SSCAN", true) => args.push("SADD".into()),
            ("SSCAN", false) => args.push("SREM".into()),
            ("HSCAN", true) => args.push("HSET".into()),
            ("HSCAN", false) => args.push("HDEL".into()),
            ("ZSCAN", true) => args.push("ZADD".into()),
            _ => args.push("ZREM".into()),
        }
        args.push(key.into());
        for index in chunk {
            let name = format!("{prefix}{index:05}");
            match (scan, add) {
                ("HSCAN", true) => { args.push(name); args.push("v".into()); }
                ("ZSCAN", true) => { args.push("1".into()); args.push(name); }
                _ => args.push(name),
            }
        }
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        request(client, &refs);
    }
}

/// Follows a scan from cursor zero to cursor zero and gives the number of
/// items each call returned, in order, the cursor having ended at zero.
fn call_sizes(client: &mut TcpStream, scan: &str, key: &str, options: &[&str]) -> Vec<usize> {
    scan_all(client, scan, key, options).1
}

/// Redis 7.2.16's scanGenericCommand keeps stepping a hash table until it has
/// sampled COUNT entries (counting before MATCH), the cursor reaches zero, or it
/// has made 10*COUNT+1 steps. So every call but the last returns at least COUNT
/// entries when nothing is filtered, an unfiltered scan takes about
/// members/COUNT calls, and a scan whose MATCH takes nothing still takes at most
/// members/COUNT+1 calls, each with an empty array and a nonzero cursor.
#[test]
fn a_step_samples_at_least_count_entries_before_match() {
    for protocol in [2, 3] {
        let (child, mut client) = start(protocol);
        for (scan, width, prefix) in [("SSCAN", 1, "m"), ("HSCAN", 2, "m"), ("ZSCAN", 2, "m")] {
            for size in [1000usize, 10000] {
                let key = format!("sampled:{scan}:{size}");
                change_many(&mut client, scan, &key, true, prefix, 0..size);
                for count in [10usize, 100] {
                    let count_text = count.to_string();
                    let sizes = call_sizes(&mut client, scan, &key, &["COUNT", &count_text]);
                    let steps = sizes.len();
                    assert!(steps >= size / (2 * count), "{scan} {size} COUNT {count}: only {steps} calls, a step must stay near COUNT");
                    let last = steps - 1;
                    let counts: Vec<usize> = sizes[..last].iter().map(|items| items / width).collect();
                    assert!(counts.iter().all(|&members| members >= count), "{scan} {size} COUNT {count}: a step before the last returned fewer than COUNT: {counts:?}");
                    assert!(counts.iter().all(|&members| members < 4 * count), "{scan} {size} COUNT {count}: a step returned far more than COUNT");
                    let mean = counts.iter().sum::<usize>() as f64 / counts.len() as f64;
                    assert!(mean < 2.0 * count as f64, "{scan} {size} COUNT {count}: mean step {mean}");
                    // Nothing matches: every call still samples COUNT entries.
                    let nothing = call_sizes(&mut client, scan, &key, &["MATCH", "zzz*", "COUNT", &count_text]);
                    assert!(nothing.iter().all(|&items| items == 0), "{scan} {size}: MATCH took nothing");
                    assert!(nothing.len() <= size / count + 1, "{scan} {size} COUNT {count}: {} calls for a scan that samples COUNT entries each", nothing.len());
                    assert!(nothing.len() >= size / (2 * count), "{scan} {size} COUNT {count}: only {} calls", nothing.len());
                }
            }
        }
        drop(client); assert_eq!(finished(child).0, 0);
    }
}

/// A table that was once large and has been emptied to a few members keeps its
/// buckets (Whitefoot's hash map does not shrink). A call stops at Redis's
/// budget of 10*COUNT+1 steps, with fewer than COUNT entries and a nonzero
/// cursor, so the scan takes at least peak/(10*COUNT+1) calls, the peak being a
/// number the test itself filled, and still ends with exactly the members left.
#[test]
fn a_sparse_table_ends_each_call_at_the_bucket_budget() {
    for protocol in [2, 3] {
        let (child, mut client) = start(protocol);
        let (peak, kept, count) = (20_000usize, 600usize, 10usize);
        for (scan, width) in [("SSCAN", 1), ("HSCAN", 2), ("ZSCAN", 2)] {
            let key = format!("sparse:{scan}");
            change_many(&mut client, scan, &key, true, "p", 0..peak);
            change_many(&mut client, scan, &key, false, "p", kept..peak);
            let mut cursor = "0".to_string();
            let mut seen = Vec::new();
            let mut calls = 0usize;
            loop {
                let (next, got) = parts(&request(&mut client, &[scan, &key, &cursor, "COUNT", &count.to_string()]));
                calls += 1;
                assert!(got.len() / width < count, "{scan}: a call on a sparse table returned {} entries", got.len() / width);
                for group in got.chunks(width) { seen.push(group[0].clone()); }
                if next == b"0" { break; }
                cursor = String::from_utf8(next).unwrap();
                assert!(calls < 100_000, "{scan}: no end in sight");
            }
            assert!(calls >= peak / (10 * count + 1), "{scan}: {calls} calls cover at most {} buckets", calls * (10 * count + 1));
            seen.sort();
            seen.dedup();
            let expected: Vec<Vec<u8>> = { let mut names: Vec<Vec<u8>> = (0..kept).map(|index| format!("p{index:05}").into_bytes()).collect(); names.sort(); names };
            assert_eq!(seen, expected, "{scan}: the members left, each at least once");
        }
        drop(client); assert_eq!(finished(child).0, 0);
    }
}

/// Redis keeps a set as an intset while every member is an integer string2ll
/// reads (an optional minus, no leading zero, within 64 bits) and there are at
/// most 512; such a set comes back whole with cursor zero. One member that is
/// not such an integer, or the 513th, puts it in steps.
#[test]
fn a_set_of_integers_is_whole_up_to_512_members() {
    for protocol in [2, 3] {
        let (child, mut client) = start(protocol);
        let integers = ["-1", "0", "9223372036854775807", "-9223372036854775808"];
        let not_integers = ["+5", "05", "-0", "-05", "5 ", " 5", "0x5", "1.0", "9223372036854775808", "-9223372036854775809", "x", ""];
        for (name, extra, whole) in integers.iter().map(|text| ("int", *text, true)).chain(not_integers.iter().map(|text| ("not", *text, false))) {
            let key = format!("ints:{name}:{extra}");
            let numbers: Vec<String> = (1000..1199).map(|number| number.to_string()).collect();
            let mut add = vec!["SADD", key.as_str()];
            add.extend(numbers.iter().map(String::as_str));
            assert_eq!(request(&mut client, &add), b":199\r\n");
            assert_eq!(request(&mut client, &["SADD", &key, extra]), b":1\r\n");
            let (cursor, got) = parts(&request(&mut client, &["SSCAN", &key, "0", "COUNT", "1"]));
            // 200 integers; the compact test must see every member as one.
            if whole {
                assert_eq!(cursor, b"0", "{extra:?} is an integer: the set is whole");
                assert_eq!(got.len(), 200);
            } else {
                assert_ne!(cursor, b"0".to_vec(), "{extra:?} is not an integer: the set is in steps");
                assert!(got.len() < 200);
            }
        }
        drop(client); assert_eq!(finished(child).0, 0);
    }
}

/// The sequence of the AOF's rewrite commits from INFO persistence, which is a
/// bulk string in RESP2 and a verbatim string in RESP3.
#[cfg(target_os = "linux")]
fn rewrite_commit_sequence(client: &mut TcpStream, protocol: u8) -> u64 {
    client.write_all(&resp(&["INFO", "persistence"])).expect("INFO request");
    info_field(&info_reply(client, protocol), "firn_aof_rewrite_commit_seq")
        .expect("rewrite sequence field").parse().expect("u64 sequence")
}

/// A script that writes a large collection and then reads an undeclared key is
/// run again after a fallback that restores the collection from a logical copy.
/// A client's scan that spans such scripts still returns every member present
/// throughout, and the collection ends exactly as the accepted runs leave it.
/// Each script's write is not idempotent (SREM returns 1 only if the member was
/// put back, HINCRBY and ZINCRBY return 1 only if the counter was restored to
/// absent), so a rerun that kept the first attempt's write shows in the reply.
/// The sequence of the AOF's rewrite commits moves once for each script, as it
/// does for the fallback cases of the network tests.
#[cfg(target_os = "linux")]
#[test]
fn a_script_fallback_between_scan_calls_loses_no_member() {
    for protocol in [2, 3] {
        // The cases of this binary share one working directory, so each protocol
        // gets its own file: a second server must not replay the first one's writes.
        let file = format!("scan-fallback-{protocol}.aof");
        let (child, mut client) = start_with(protocol, &[b"--appendonly", b"yes", b"--appendfilename", file.as_bytes()]);
        for (scan, width) in [("SSCAN", 1), ("HSCAN", 2), ("ZSCAN", 2)] {
            let key = format!("fallback:{scan}");
            change_many(&mut client, scan, &key, true, "m", 0..1000);
            let source = match scan {
                "SSCAN" => "local r=redis.call('SREM',KEYS[1],ARGV[1]); redis.call('GET','undeclared:'..KEYS[1]); return r",
                "HSCAN" => "local r=redis.call('HINCRBY',KEYS[1],ARGV[1],1); redis.call('GET','undeclared:'..KEYS[1]); return r",
                _ => "local r=redis.call('ZINCRBY',KEYS[1],1,ARGV[1]); redis.call('GET','undeclared:'..KEYS[1]); return r",
            };
            let before = rewrite_commit_sequence(&mut client, protocol);
            let mut cursor = "0".to_string();
            let mut seen = std::collections::HashSet::new();
            let (mut step, mut scripts) = (0, 0usize);
            loop {
                let (next, got) = parts(&request(&mut client, &[scan, &key, &cursor, "COUNT", "10"]));
                for group in got.chunks(width) { seen.insert(group[0].clone()); }
                if next == b"0" { break; }
                cursor = String::from_utf8(next).unwrap();
                step += 1;
                if step % 5 == 0 {
                    // The set loses m00900.. one at a time; the others gain a counter.
                    let argument = if scan == "SSCAN" { format!("m{:05}", 900 + scripts) } else { format!("c{scripts:05}") };
                    let reply = request(&mut client, &["EVAL", source, "1", &key, &argument]);
                    assert!(reply == b":1\r\n" || reply == b"$1\r\n1\r\n" || reply == b",1\r\n", "{scan}: script {scripts} left the first attempt's write in place: {reply:?}");
                    scripts += 1;
                }
                assert!(step < 100_000, "{scan}: no end in sight");
            }
            assert!(scripts >= 10, "{scan}: {scripts} scripts ran between calls");
            assert_eq!(rewrite_commit_sequence(&mut client, protocol), before + scripts as u64, "{scan}: one sequence for each script");
            for index in 0..1000 {
                let name = format!("m{index:05}");
                // A member the scripts removed may or may not have been returned.
                if scan != "SSCAN" || index < 900 {
                    assert!(seen.contains(name.as_bytes()), "{scan}: {name} was present throughout and never returned");
                }
            }
            // Unchanged now, the collection comes back exactly as the scripts left it.
            let mut expected: Vec<Vec<u8>> = Vec::new();
            for index in 0..1000 {
                if scan == "SSCAN" && index >= 900 && index < 900 + scripts { continue; }
                expected.push(format!("m{index:05}").into_bytes());
                if scan == "HSCAN" { expected.push(b"v".to_vec()); }
                if scan == "ZSCAN" { expected.push(b"1".to_vec()); }
            }
            if scan != "SSCAN" {
                for number in 0..scripts {
                    expected.push(format!("c{number:05}").into_bytes());
                    expected.push(b"1".to_vec());
                }
            }
            let (all, _) = scan_all(&mut client, scan, &key, &["COUNT", "100"]);
            assert_eq!(grouped(&all, width), grouped(&expected, width), "{scan}: the collection after the scripts");
        }
        drop(client); assert_eq!(finished(child).0, 0);
    }
}

/// The flat reply a whole scan of `m{index:05}` for `range` gives: a set's
/// members, or a hash's fields with the value `v`, or a sorted set's members
/// with the score `1` as `change_many` filled them.
fn filled(scan: &str, range: std::ops::Range<usize>) -> Vec<Vec<u8>> {
    let mut flat = Vec::new();
    for index in range {
        flat.push(format!("m{index:05}").into_bytes());
        match scan {
            "HSCAN" => flat.push(b"v".to_vec()),
            "ZSCAN" => flat.push(b"1".to_vec()),
            _ => {}
        }
    }
    flat
}

/// Redis 7.2.16 scanGenericCommand reads COUNT as a long, runs
/// `dictScan` until the cursor is zero or `sampled >= count`, and bounds the
/// loop by `count*10`, which for these COUNTs wraps in C and never stops the
/// loop early. The reference therefore answers a table of 1000 entries whole,
/// with cursor zero, from any starting cursor. A multiplication that overflowed
/// in firn's budget would cut the call short or never end.
#[test]
fn a_count_near_the_arithmetic_limits_returns_the_rest_in_one_call() {
    let (child, mut client) = start(2);
    for (scan, width) in [("SSCAN", 1), ("HSCAN", 2), ("ZSCAN", 2)] {
        let key = format!("huge:{scan}");
        change_many(&mut client, scan, &key, true, "m", 0..1000);
        let expected = filled(scan, 0..1000);
        for count in ["9223372036854775807", "4611686018427387904", "1844674407370955161", "1844674407370955160", "1000000000000000000", "4294967296"] {
            let (cursor, got) = parts(&request(&mut client, &[scan, &key, "0", "COUNT", count]));
            assert_eq!(cursor, b"0", "{scan} COUNT {count}: one call from cursor zero");
            assert_eq!(grouped(&got, width), grouped(&expected, width), "{scan} COUNT {count}: every member once");
            let (first_cursor, first) = parts(&request(&mut client, &[scan, &key, "0", "COUNT", "10"]));
            assert_ne!(first_cursor, b"0".to_vec(), "{scan}: the first of many steps");
            let text = String::from_utf8(first_cursor).unwrap();
            let (end, rest) = parts(&request(&mut client, &[scan, &key, &text, "COUNT", count]));
            assert_eq!(end, b"0", "{scan} COUNT {count} from cursor {text}: the rest in one call");
            let mut both = first;
            both.extend(rest);
            assert_eq!(grouped(&both, width), grouped(&expected, width), "{scan} COUNT {count} from cursor {text}: first step and rest, once each");
        }
    }
    drop(client); assert_eq!(finished(child).0, 0);
}

/// A call decides compact or stepped from the collection as it is, with no
/// memory of an earlier call or of an earlier size: Redis returns a listpack or
/// intset whole with cursor zero (limits 128 members of a set or sorted set, 512
/// fields of a hash, 64 bytes for each name and hash value) and any larger table
/// in steps, and its guarantee for a scan under change is that what is present
/// throughout comes back. Here a scan in steps whose collection shrinks below
/// the bound ends with the whole remainder, one that grows past the bound steps
/// again, and a collection sitting on either side of each bound answers
/// accordingly, one more or one fewer member or one overlong byte flipping it.
#[test]
fn a_collection_that_crosses_a_compact_bound_between_calls_answers_by_its_content() {
    for protocol in [2, 3] {
        let (child, mut client) = start(protocol);
        for (scan, width, bound) in [("SSCAN", 1, 128usize), ("HSCAN", 2, 512), ("ZSCAN", 2, 128)] {
            let key = format!("cross:{scan}");
            change_many(&mut client, scan, &key, true, "m", 0..1000);
            let (cursor, first) = parts(&request(&mut client, &[scan, &key, "0", "COUNT", "10"]));
            assert_ne!(cursor, b"0".to_vec(), "{scan}: a table of 1000 steps");
            // The collection shrinks to the bound while the client holds a cursor.
            change_many(&mut client, scan, &key, false, "m", bound..1000);
            let text = String::from_utf8(cursor).unwrap();
            let (next, rest) = parts(&request(&mut client, &[scan, &key, &text, "COUNT", "10"]));
            assert_eq!(next, b"0", "{scan}: {bound} members are compact, whole with cursor zero from any cursor");
            assert_eq!(grouped(&rest, width), grouped(&filled(scan, 0..bound), width), "{scan}: the whole remainder");
            let remaining: std::collections::HashSet<Vec<u8>> = rest.chunks(width).map(|group| group[0].clone()).collect();
            for group in first.chunks(width) {
                if group[0] < format!("m{bound:05}").into_bytes() {
                    assert!(remaining.contains(&group[0]), "{scan}: a member present throughout is in the last reply");
                }
            }
            // One more member and it steps again.
            change_many(&mut client, scan, &key, true, "m", bound..bound + 1);
            let (cursor, step) = parts(&request(&mut client, &[scan, &key, "0", "COUNT", "1"]));
            assert_ne!(cursor, b"0".to_vec(), "{scan}: {} members step", bound + 1);
            assert!(step.len() / width < bound + 1, "{scan}: the step does not hold the collection");
            let (all, _) = scan_all(&mut client, scan, &key, &["COUNT", "20"]);
            assert_eq!(grouped(&all, width), grouped(&filled(scan, 0..bound + 1), width), "{scan}: the stepped scan finds each once");
            // Back to the bound: whole again.
            change_many(&mut client, scan, &key, false, "m", bound..bound + 1);
            let (cursor, again) = parts(&request(&mut client, &[scan, &key, "0", "COUNT", "1"]));
            assert_eq!(cursor, b"0", "{scan}: {bound} members are whole again");
            assert_eq!(grouped(&again, width), grouped(&filled(scan, 0..bound), width));
        }
        // One byte past 64 in a name takes the collection out of the compact
        // form for as long as it is there, and the value of a hash does the same.
        for (scan, width) in [("SSCAN", 1), ("HSCAN", 2), ("ZSCAN", 2)] {
            let key = format!("overlong:{scan}");
            change_many(&mut client, scan, &key, true, "m", 0..100);
            let compact = filled(scan, 0..100);
            let (cursor, whole) = parts(&request(&mut client, &[scan, &key, "0", "COUNT", "1"]));
            assert_eq!(cursor, b"0");
            assert_eq!(grouped(&whole, width), grouped(&compact, width));
            let long = "L".repeat(65);
            let (name, big) = (key.as_str(), long.as_str());
            let add: Vec<&str> = match scan { "SSCAN" => vec!["SADD", name, big], "HSCAN" => vec!["HSET", name, big, "v"], _ => vec!["ZADD", name, "1", big] };
            request(&mut client, &add);
            let (cursor, _) = parts(&request(&mut client, &[scan, &key, "0", "COUNT", "1"]));
            assert_ne!(cursor, b"0".to_vec(), "{scan}: a 65-byte name steps");
            let remove: Vec<&str> = match scan { "SSCAN" => vec!["SREM", name, big], "HSCAN" => vec!["HDEL", name, big], _ => vec!["ZREM", name, big] };
            request(&mut client, &remove);
            let (cursor, whole) = parts(&request(&mut client, &[scan, &key, "0", "COUNT", "1"]));
            assert_eq!(cursor, b"0", "{scan}: whole again without it");
            assert_eq!(grouped(&whole, width), grouped(&compact, width));
        }
        request(&mut client, &["HSET", "overlong:HSCAN", "m00000", &"V".repeat(65)]);
        let (cursor, _) = parts(&request(&mut client, &["HSCAN", "overlong:HSCAN", "0", "COUNT", "1"]));
        assert_ne!(cursor, b"0".to_vec(), "a 65-byte hash value steps");
        request(&mut client, &["HSET", "overlong:HSCAN", "m00000", "v"]);
        let (cursor, whole) = parts(&request(&mut client, &["HSCAN", "overlong:HSCAN", "0", "COUNT", "1"]));
        assert_eq!(cursor, b"0", "whole again when the value is short");
        assert_eq!(grouped(&whole, 2), grouped(&filled("HSCAN", 0..100), 2));
        drop(client); assert_eq!(finished(child).0, 0);
    }
}

/// A member of 70000 characters, one with CR, LF, NUL and multi-byte text and the
/// empty one, and hash values of 100000 bytes, come back byte for byte through
/// the reply buffer a step collects before its header: Redis writes each as a
/// length-prefixed bulk string (addReplyBulkCBuffer), so no byte of a name or
/// value ends a frame. The reply of the hash is several megabytes in one call.
#[test]
fn stepped_replies_carry_binary_and_large_elements_unchanged() {
    let (child, mut client) = start(2);
    let text = |seed: usize, length: usize| -> String {
        format!("{seed:03}\r\n\0é$*").chars().cycle().take(length).collect()
    };
    let long_member = text(7, 70_000);
    let mut set_members: Vec<String> = vec![String::new(), long_member.clone()];
    for index in 0..40 { set_members.push(format!("x\r\n\0é{index}")); }
    let mut hash_pairs: Vec<(String, String)> = Vec::new();
    for index in 0..30 { hash_pairs.push((format!("f{index}\r\n\0"), text(index, 100_000))); }
    for member in &set_members { request(&mut client, &["SADD", "bin:s", member]); }
    for (field, value) in &hash_pairs { request(&mut client, &["HSET", "bin:h", field, value]); }
    let sorted: Vec<(String, &str, &[u8])> = vec![
        (long_member.clone(), "1.5", &b"1.5"[..]), ("a\r\nb".to_string(), "0.1", &b"0.10000000000000001"[..]), (String::new(), "2", &b"2"[..]),
    ];
    for entry in &sorted { request(&mut client, &["ZADD", "bin:z", entry.1, &entry.0]); }

    let want: Vec<Vec<u8>> = set_members.iter().map(|member| member.as_bytes().to_vec()).collect();
    let hashed: Vec<Vec<u8>> = hash_pairs.iter().flat_map(|(field, value)| [field.as_bytes().to_vec(), value.as_bytes().to_vec()]).collect();
    let scored: Vec<Vec<u8>> = sorted.iter().flat_map(|(member, _, shown)| [member.as_bytes().to_vec(), shown.to_vec()]).collect();
    for (scan, key, width, expected, count) in [
        ("SSCAN", "bin:s", 1, &want, "1000"), ("HSCAN", "bin:h", 2, &hashed, "1000"), ("ZSCAN", "bin:z", 2, &scored, "1000"),
        ("SSCAN", "bin:s", 1, &want, "3"), ("HSCAN", "bin:h", 2, &hashed, "3"), ("ZSCAN", "bin:z", 2, &scored, "1"),
    ] {
        let (all, _) = scan_all(&mut client, scan, key, &["COUNT", count]);
        assert_eq!(grouped(&all, width), grouped(expected, width), "{scan} COUNT {count}: every byte of every element");
    }
    drop(client); assert_eq!(finished(child).0, 0);
}

/// Above the compact bound ZSCAN writes each score as C's %.17g of the double
/// (Redis 7.2.16 scanCallback: ld2string(..., LD_STR_AUTO) of the dictionary's
/// double, and "inf" or "-inf" for the infinities), so the text has 17
/// significant digits with trailing zeros dropped, an exponent form from 1e17
/// up and below 1e-4. Each expected text is the %.17g of the double the decimal
/// rounds to, worked out digit by digit. Negative zero is not offered: a sorted
/// set of firn keeps it as zero whatever its size (kept_score), which Redis does
/// only for a listpack, so a stored score is never -0, whichever command reads
/// it (research/investigations/firn/DESIGN.md).
#[test]
fn stepped_zscan_writes_17_significant_digit_scores() {
    for protocol in [2, 3] {
        let (child, mut client) = start(protocol);
        let special: [(&str, &str, &str); 11] = [
            ("tenth", "0.1", "0.10000000000000001"), ("e16", "1e16", "10000000000000000"), ("e17", "1e17", "1e+17"),
            ("tiny", "1e-5", "1.0000000000000001e-05"), ("normalmin", "2.2250738585072014e-308", "2.2250738585072014e-308"), ("huge", "1e308", "1e+308"),
            ("pinf", "inf", "inf"), ("ninf", "-inf", "-inf"),
            ("whole", "100", "100"), ("negative", "-1.5", "-1.5"), ("wide", "123456789012345678", "1.2345678901234568e+17"),
        ];
        for index in 0..200 {
            request(&mut client, &["ZADD", "scores", "1", &format!("s{index:03}")]);
        }
        for (member, score, _) in special {
            assert_eq!(request(&mut client, &["ZADD", "scores", score, member]), b":1\r\n");
        }
        let (all, sizes) = scan_all(&mut client, "ZSCAN", "scores", &["COUNT", "7"]);
        assert!(sizes.len() > 10, "a sorted set of 211 members steps at COUNT 7");
        let found: std::collections::HashMap<Vec<u8>, Vec<u8>> = all.chunks(2).map(|pair| (pair[0].clone(), pair[1].clone())).collect();
        assert_eq!(found.len(), 211);
        for (member, _, shown) in special {
            assert_eq!(String::from_utf8_lossy(&found[member.as_bytes()]), shown, "score of {member}");
        }
        assert_eq!(found[&b"s000".to_vec()], b"1");
        drop(client); assert_eq!(finished(child).0, 0);
    }
}

/// COUNT 1 and COUNT 3: Redis stops a call once it has sampled COUNT entries, so
/// every call but the last returns at least COUNT (HSCAN and ZSCAN counted in
/// pairs), never the whole collection, and the scan from zero to zero returns
/// each entry exactly once while nothing changes. MATCH is applied to a hash's
/// field names only, so a pattern that only the values satisfy matches nothing
/// in any step, though it still samples COUNT fields each call.
#[test]
fn small_counts_step_by_entries_and_match_reads_field_names_only() {
    let (child, mut client) = start(2);
    for (scan, width, size) in [("SSCAN", 1, 300usize), ("HSCAN", 2, 600), ("ZSCAN", 2, 300)] {
        let key = format!("small:{scan}");
        change_many(&mut client, scan, &key, true, "m", 0..size);
        for count in [1usize, 3] {
            let text = count.to_string();
            let (all, sizes) = scan_all(&mut client, scan, &key, &["COUNT", &text]);
            assert_eq!(grouped(&all, width), grouped(&filled(scan, 0..size), width), "{scan} COUNT {count}: each entry once");
            let last = sizes.len() - 1;
            assert!(sizes[..last].iter().all(|items| items / width >= count), "{scan} COUNT {count}: a call returned fewer than COUNT entries: {sizes:?}");
            assert!(sizes.iter().all(|items| items / width < size), "{scan} COUNT {count}: a call held the collection");
            assert!(sizes.len() >= size / (4 * count) , "{scan} COUNT {count}: only {} calls", sizes.len());
        }
    }
    let (all, sizes) = scan_all(&mut client, "HSCAN", "small:HSCAN", &["MATCH", "v*", "COUNT", "5"]);
    assert!(all.is_empty(), "MATCH tests field names, not values: {} items", all.len());
    assert!(sizes.len() >= 600 / 20, "the scan still samples fields: {} calls", sizes.len());
    drop(client); assert_eq!(finished(child).0, 0);
}
