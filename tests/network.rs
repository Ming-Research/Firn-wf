//! firn's Redis compatibility cases over loopback TCP.

#![cfg(target_os = "linux")]

mod support;

use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::time::{Duration, Instant};

use support::{CompiledProgram, ProgramChild, fixture_directory};

/// One port the host is not using, released before the program binds it.
///
/// A listening socket that never accepted leaves no connection in `TIME_WAIT`,
/// so the port is free the moment this drops, and the program's own `bind`
/// would take it even on a host where the runtime set no `SO_REUSEADDR`.
fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("reserve a loopback port");
    listener
        .local_addr()
        .expect("the reserved port's address")
        .port()
}

/// Connects to a program that is still starting.
///
/// The program binds its listener some time after the harness spawned it, so
/// the first attempts are refused. This retries for a bounded wall-clock span
/// and fails the case if the program never listened; nothing about the
/// program's own acceptance depends on it.
fn connect_when_ready(port: u16) -> TcpStream {
    connect_to_when_ready(SocketAddr::from(([127, 0, 0, 1], port)))
}

/// Connects to a program that is still starting, at the address it listens on.
fn connect_to_when_ready(address: SocketAddr) -> TcpStream {
    let port = address.port();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match TcpStream::connect_timeout(&address, Duration::from_millis(100)) {
            Ok(stream) => return bounded_stream(stream),
            Err(error) if Instant::now() < deadline => {
                let _ = error;
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("the program never listened on {port}: {error}"),
        }
    }
}

fn bounded_stream(stream: TcpStream) -> TcpStream {
    stream.set_nonblocking(false).expect("blocking peer socket");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("bound peer reads");
    stream
        .set_write_timeout(Some(Duration::from_secs(10)))
        .expect("bound peer writes");
    stream
}

/// The exit code one finished child reported, with its diagnostics on failure.
fn finished(child: ProgramChild) -> (i32, Vec<u8>) {
    let output = child.wait_with_output().expect("wait for compiled program");
    assert!(
        output.stderr.is_empty(),
        "native program diagnostics: {output:?}"
    );
    (output.status.code().unwrap_or(-1), output.stdout)
}

/// One RESP2 request of bulk strings.
#[cfg(target_os = "linux")]
fn resp(arguments: &[&str]) -> Vec<u8> {
    let mut bytes = format!("*{}\r\n", arguments.len()).into_bytes();
    for argument in arguments {
        bytes.extend_from_slice(format!("${}\r\n{argument}\r\n", argument.len()).as_bytes());
    }
    bytes
}

/// firn, the Redis-compatible server supplied through `FIRN`, shared by every
/// case that runs it; each case names its own append-only file, so the shared
/// working directory holds no state one case leaves for another.
#[cfg(target_os = "linux")]
fn firn() -> &'static CompiledProgram {
    static PROGRAM: std::sync::OnceLock<CompiledProgram> = std::sync::OnceLock::new();
    PROGRAM.get_or_init(CompiledProgram::from_environment)
}

/// Reads one reply line through its CR LF.
#[cfg(target_os = "linux")]
fn reply_line(stream: &mut TcpStream, what: &str) -> String {
    let mut byte = [0_u8; 1];
    let mut line = Vec::new();
    while !line.ends_with(b"\r\n") {
        stream
            .read_exact(&mut byte)
            .unwrap_or_else(|error| panic!("{what}: {error}"));
        line.push(byte[0]);
    }
    String::from_utf8_lossy(&line).into_owned()
}

/// Reads one RESP integer reply.
#[cfg(target_os = "linux")]
fn integer_reply(stream: &mut TcpStream, what: &str) -> i64 {
    let line = reply_line(stream, what);
    line.strip_prefix(':')
        .and_then(|rest| rest.strip_suffix("\r\n"))
        .and_then(|digits| digits.parse().ok())
        .unwrap_or_else(|| panic!("{what}: not an integer reply: {line:?}"))
}

/// Reads one RESP bulk string reply whole.
#[cfg(target_os = "linux")]
fn bulk_reply(stream: &mut TcpStream, what: &str) -> String {
    let header = reply_line(stream, what);
    let length = header
        .strip_prefix('$')
        .and_then(|rest| rest.strip_suffix("\r\n"))
        .and_then(|digits| digits.parse::<usize>().ok())
        .unwrap_or_else(|| panic!("{what}: not a bulk string reply: {header:?}"));
    let mut body = vec![0_u8; length + 2];
    stream
        .read_exact(&mut body)
        .unwrap_or_else(|error| panic!("{what}: {error}"));
    assert!(body.ends_with(b"\r\n"), "{what}: {body:?}");
    body.truncate(length);
    String::from_utf8(body).unwrap_or_else(|error| panic!("{what}: {error}"))
}

/// The section names of an INFO reply in order, checking Redis's form: each
/// section a `# ` header and `name:value` lines, every line ending in CR LF
/// and holding no other CR or LF, one blank line between two sections.
#[cfg(target_os = "linux")]
fn info_sections(info: &str, what: &str) -> Vec<String> {
    assert!(
        info.is_empty() || info.ends_with("\r\n"),
        "{what}: {info:?}"
    );
    info.split("\r\n\r\n")
        .filter(|section| !section.is_empty())
        .map(|section| {
            let body = section.strip_suffix("\r\n").unwrap_or(section);
            let mut lines = body.split("\r\n");
            let header = lines.next().unwrap_or_default();
            assert!(header.starts_with("# "), "{what}: {section:?}");
            assert!(!header.contains(['\r', '\n']), "{what}: {header:?}");
            for line in lines {
                let named = line
                    .split_once(':')
                    .is_some_and(|(name, _)| !name.is_empty());
                assert!(named && !line.contains(['\r', '\n']), "{what}: {line:?}");
            }
            header[2..].to_owned()
        })
        .collect()
}

/// The value of one field of an INFO reply.
#[cfg(target_os = "linux")]
fn info_field(info: &str, name: &str) -> Option<String> {
    info.split("\r\n")
        .find_map(|line| line.strip_prefix(&format!("{name}:")).map(str::to_owned))
}

/// Reads one `TIME` reply as calendar milliseconds.
#[cfg(target_os = "linux")]
fn time_reply(stream: &mut TcpStream, what: &str) -> u64 {
    assert_eq!(reply_line(stream, what), "*2\r\n", "{what}: TIME");
    let mut parts = [0_u64; 2];
    for part in &mut parts {
        reply_line(stream, what);
        let digits = reply_line(stream, what);
        *part = digits
            .trim_end()
            .parse()
            .unwrap_or_else(|_| panic!("{what}: TIME answered {digits:?}"));
    }
    parts[0] * 1000 + parts[1] / 1000
}

/// Reads one whole RESP reply, a bulk string's or an array's elements
/// included, and returns its bytes.
#[cfg(target_os = "linux")]
fn whole_reply(stream: &mut TcpStream, what: &str) -> String {
    let mut reply = reply_line(stream, what);
    let count = reply[1..reply.len() - 2].parse::<i64>().unwrap_or(-1);
    if reply.starts_with('$') && count >= 0 {
        let mut body = vec![0_u8; count as usize + 2];
        stream
            .read_exact(&mut body)
            .unwrap_or_else(|error| panic!("{what}: {error}"));
        reply.push_str(&String::from_utf8_lossy(&body));
    } else if reply.starts_with('*') {
        for _ in 0..count.max(0) {
            reply.push_str(&whole_reply(stream, what));
        }
    }
    reply
}

/// Reads exactly the bytes of the expected replies and compares them.
#[cfg(target_os = "linux")]
fn expect_replies(stream: &mut TcpStream, expected: &[u8], what: &str) {
    let mut returned = vec![0_u8; expected.len()];
    stream
        .read_exact(&mut returned)
        .unwrap_or_else(|error| panic!("{what}: {error}"));
    assert_eq!(
        String::from_utf8_lossy(&returned),
        String::from_utf8_lossy(expected),
        "{what}"
    );
}

/// [SHARE-1, SHARE-3] firn serves every client over one
/// keyspace: the commands answer as Redis does, a pipelined batch and a
/// command split across two sends are answered whole, and clients that
/// increment one key from four drivers at once lose no increment, which a
/// keyspace not held alone by each atomic statement would.
#[cfg(target_os = "linux")]
#[test]
fn firn_serves_every_client_over_one_keyspace() {
    const CLIENTS: usize = 8;
    const INCREMENTS: usize = 250;
    let program = firn();
    let port = free_port();
    let text = port.to_string();
    let count = (CLIENTS + 1).to_string();
    let child = program.spawn_on_route_with(
        true,
        &[("WF_DRIVERS", "4")],
        &[text.as_bytes(), count.as_bytes()],
    );
    let mut first = connect_when_ready(port);
    first
        .set_read_timeout(Some(Duration::from_secs(20)))
        .expect("bound the first client's waits");
    let mut batch = Vec::new();
    for request in [
        vec!["PING"],
        vec!["SET", "fruit", "apple"],
        vec!["GET", "fruit"],
        vec!["GET", "absent"],
        vec!["INCR", "fruit"],
        vec!["DEL", "fruit"],
        vec!["DEL", "fruit"],
        vec!["INCR", "total"],
        vec!["CONFIG", "GET", "save"],
    ] {
        batch.extend(resp(&request));
    }
    first.write_all(&batch).expect("send a pipelined batch");
    expect_replies(
        &mut first,
        b"+PONG\r\n+OK\r\n$5\r\napple\r\n$-1\r\n-ERR value is not an integer or out of range\r\n:1\r\n:0\r\n:1\r\n*2\r\n$4\r\nsave\r\n$0\r\n\r\n",
        "the pipelined batch",
    );
    let split = resp(&["SET", "split", "across two sends"]);
    let (head, tail) = split.split_at(11);
    first.write_all(head).expect("send the first part");
    first.flush().expect("flush the first part");
    std::thread::sleep(Duration::from_millis(50));
    first.write_all(tail).expect("send the rest");
    first
        .write_all(&resp(&["GET", "split"]))
        .expect("read it back");
    expect_replies(
        &mut first,
        b"+OK\r\n$16\r\nacross two sends\r\n",
        "a command split across two sends",
    );
    let clients = (0..CLIENTS)
        .map(|client| {
            let mut stream = connect_when_ready(port);
            std::thread::spawn(move || {
                stream
                    .set_read_timeout(Some(Duration::from_secs(20)))
                    .expect("bound this client's waits");
                for _ in 0..INCREMENTS {
                    stream
                        .write_all(&resp(&["INCR", "hits"]))
                        .expect("send an increment");
                    let mut reply = [0_u8; 1];
                    let mut line = Vec::new();
                    loop {
                        stream
                            .read_exact(&mut reply)
                            .unwrap_or_else(|error| panic!("client {client}: {error}"));
                        line.push(reply[0]);
                        if line.ends_with(b"\r\n") {
                            break;
                        }
                    }
                    assert_eq!(line[0], b':', "client {client}: {line:?}");
                }
            })
        })
        .collect::<Vec<_>>();
    for client in clients {
        client.join().expect("a client finished");
    }
    let total = CLIENTS * INCREMENTS;
    first
        .write_all(&resp(&["GET", "hits"]))
        .expect("read the counter");
    expect_replies(
        &mut first,
        format!("${}\r\n{total}\r\n", total.to_string().len()).as_bytes(),
        "every increment",
    );
    drop(first);
    let (status, _) = finished(child);
    assert_eq!(status, 0);
}

/// [PRE-2] firn expires keys as Redis does. In one pipelined
/// batch, whose commands all see one reading of the clocks, `TTL` answers -1
/// for a key without an expiry and -2 for an absent one, `EXPIRE` gives one,
/// `PERSIST` removes it once, and `SET` refuses a zero expiry, an unknown
/// option and a count that is not a number. An expiry of 10^11 seconds, past
/// what nanoseconds in 64 bits hold, is kept to the second, as Redis keeps
/// expiries in calendar milliseconds; a negative `EXPIRE` removes the key at
/// once, so that `DBSIZE` no longer counts it, and
/// one whose milliseconds leave the range of i64, alone or added to the time,
/// is refused, also when they would wrap to a small number. `EXPIRE` takes NX,
/// XX, GT and LT as Redis 7.0.15 does, a key without an expiry failing GT and
/// passing LT, refuses NX beside another option and GT beside LT, and echoes
/// an unknown option up to a zero byte, its trailing line ends dropped and any
/// others written as spaces; `EXPIREAT` and `PEXPIREAT` set calendar times
/// that `EXPIRETIME`, rounded to the second as Redis rounds it, and
/// `PEXPIRETIME` give back exactly, and a name only sharing their first eight
/// letters is unknown. A key set with `PX 100` answers a
/// positive `PTTL` and is absent 200 milliseconds later. A key read 5
/// milliseconds after its expiry is absent too, which the command's own check
/// answers: the expiring context wakes only every 100 milliseconds, so without
/// that check the value would usually still be returned. A key lives through
/// the millisecond its expiry names, as Redis's `keyIsExpired` keeps it: of
/// keys set to expire at each of a hundred milliseconds from a reading of the
/// clock, each read back at once in a batch whose commands share the reading a
/// `TIME` before and after them reports, those expiring at that reading or
/// later answer their value and the earlier ones nil.
#[cfg(target_os = "linux")]
#[test]
fn firn_expires_keys_on_both_routes() {
    let program = firn();
    for native_ring in [true, false] {
        let what = format!("native ring: {native_ring}");
        let port = free_port();
        let text = port.to_string();
        let child = program.spawn_on_route(native_ring, &[text.as_bytes(), b"1"]);
        let mut client = connect_when_ready(port);
        client
            .set_read_timeout(Some(Duration::from_secs(20)))
            .expect("bound the client's waits");
        let mut batch = Vec::new();
        for request in [
            vec!["SET", "kept", "1"],
            vec!["SET", "brief", "hello", "PX", "100"],
            vec!["TTL", "kept"],
            vec!["TTL", "absent"],
            vec!["EXPIRE", "kept", "100"],
            vec!["TTL", "kept"],
            vec!["PERSIST", "kept"],
            vec!["TTL", "kept"],
            vec!["PERSIST", "kept"],
            vec!["SET", "other", "v", "EX", "0"],
            vec!["SET", "other", "v", "XX", "1"],
            vec!["SET", "other", "v", "PX", "soon"],
            vec!["DBSIZE"],
            vec!["SET", "far", "v", "EX", "100000000000"],
            vec!["TTL", "far"],
            vec!["EXPIRE", "far", "-1"],
            vec!["DBSIZE"],
            vec!["EXPIRE", "absent", "-1"],
            vec!["SET", "other", "v", "EX", "9223372036854776"],
            vec!["SET", "other", "v", "PX", "9223372036854775807"],
            vec!["SET", "other", "v", "EXAT", "9223372036854776"],
            vec!["PEXPIRE", "kept", "9223372036854775000"],
            vec!["EXPIRE", "kept", "18446744073709552"],
            vec!["EXPIRE", "kept", "-18446744073709552"],
            vec!["SET", "e", "v"],
            vec!["EXPIRE", "e", "100", "nx"],
            vec!["EXPIRE", "e", "200", "NX"],
            vec!["EXPIRE", "e", "50", "gt"],
            vec!["EXPIRE", "e", "300", "GT"],
            vec!["TTL", "e"],
            vec!["EXPIRE", "e", "400", "lt"],
            vec!["EXPIRE", "e", "100", "LT"],
            vec!["EXPIRE", "e", "100", "XX"],
            vec!["PERSIST", "e"],
            vec!["EXPIRE", "e", "100", "XX"],
            vec!["EXPIRE", "e", "100", "GT"],
            vec!["EXPIRE", "e", "100", "LT"],
            vec!["EXPIRE", "e", "100", "NX", "GT"],
            vec!["EXPIRE", "e", "100", "GT", "LT"],
            vec!["EXPIRE", "e", "abc", "FOO"],
            vec!["EXPIRE", "e", "100", "\r\nab\nc\r"],
            vec!["EXPIRE", "e", "100", "fo\0o"],
            vec!["EXPIRE", "e", "100", "nx\0garbage"],
            vec!["EXPIREAT", "e", "99999999999"],
            vec!["EXPIRETIME", "e"],
            vec!["PEXPIRETIME", "e"],
            vec!["PEXPIREAT", "e", "99999999999500"],
            vec!["EXPIRETIME", "e"],
            vec!["PEXPIREAT", "e", "99999999999499"],
            vec!["EXPIRETIME", "e"],
            vec!["PEXPIREAT", "e", "99999999999499", "GT"],
            vec!["PEXPIREAT", "e", "99999999999499", "LT"],
            vec!["EXPIRE", "e", "100", "ab\r\n\0x"],
            vec!["EXPIREAT", "e", "9223372036854776"],
            vec!["EXPIRETIME", "kept"],
            vec!["PEXPIRETIME", "absent"],
            vec!["EXPIRE", "e", "-1", "GT"],
            vec!["EXPIRE", "e", "-1", "XX"],
            vec!["EXISTS", "e"],
            vec!["PEXPIRETIMEX", "e"],
            vec!["EXPIRETIMEX", "e"],
        ] {
            batch.extend(resp(&request));
        }
        client.write_all(&batch).expect("send the expiry batch");
        expect_replies(
            &mut client,
            b"+OK\r\n+OK\r\n:-1\r\n:-2\r\n:1\r\n:100\r\n:1\r\n:-1\r\n:0\r\n-ERR invalid expire time in 'set' command\r\n-ERR syntax error\r\n-ERR value is not an integer or out of range\r\n:2\r\n+OK\r\n:100000000000\r\n:1\r\n:2\r\n:0\r\n-ERR invalid expire time in 'set' command\r\n-ERR invalid expire time in 'set' command\r\n-ERR invalid expire time in 'set' command\r\n-ERR invalid expire time in 'pexpire' command\r\n-ERR invalid expire time in 'expire' command\r\n-ERR invalid expire time in 'expire' command\r\n+OK\r\n:1\r\n:0\r\n:0\r\n:1\r\n:300\r\n:0\r\n:1\r\n:1\r\n:1\r\n:0\r\n:0\r\n:1\r\n-ERR NX and XX, GT or LT options at the same time are not compatible\r\n-ERR GT and LT options at the same time are not compatible\r\n-ERR Unsupported option FOO\r\n-ERR Unsupported option   ab c\r\n-ERR Unsupported option fo\r\n:0\r\n:1\r\n:99999999999\r\n:99999999999000\r\n:1\r\n:100000000000\r\n:1\r\n:99999999999\r\n:0\r\n:0\r\n-ERR Unsupported option ab\r\n-ERR invalid expire time in 'expireat' command\r\n:-1\r\n:-2\r\n:0\r\n:1\r\n:0\r\n-ERR unknown command 'PEXPIRETIMEX', with args beginning with: 'e' \r\n-ERR unknown command 'EXPIRETIMEX', with args beginning with: 'e' \r\n",
            &what,
        );
        client
            .write_all(&resp(&["PTTL", "brief"]))
            .expect("ask the time left");
        let left = integer_reply(&mut client, &what);
        assert!((1..=100).contains(&left), "{what}: {left}");
        std::thread::sleep(Duration::from_millis(200));
        let mut after = resp(&["GET", "brief"]);
        after.extend(resp(&["PTTL", "brief"]));
        after.extend(resp(&["DBSIZE"]));
        client.write_all(&after).expect("read the expired key");
        expect_replies(&mut client, b"$-1\r\n:-2\r\n:1\r\n", &what);
        client
            .write_all(&resp(&["SET", "flash", "v", "PX", "1"]))
            .expect("set a key that expires at once");
        expect_replies(&mut client, b"+OK\r\n", &what);
        std::thread::sleep(Duration::from_millis(5));
        client
            .write_all(&resp(&["GET", "flash"]))
            .expect("read it after its expiry");
        expect_replies(&mut client, b"$-1\r\n", &what);
        // A batch the server read in two parts, or later than the hundredth
        // millisecond, cannot place the boundary and is sent again.
        let mut placed = false;
        for _ in 0..10 {
            client.write_all(&resp(&["TIME"])).expect("ask the time");
            let start = time_reply(&mut client, &what);
            let mut edge = resp(&["TIME"]);
            for offset in 0..100_u64 {
                let key = format!("edge:{offset}");
                let at = (start + offset).to_string();
                edge.extend(resp(&["SET", &key, "v", "PXAT", &at]));
                edge.extend(resp(&["GET", &key]));
            }
            edge.extend(resp(&["TIME"]));
            client
                .write_all(&edge)
                .expect("set keys expiring around the time");
            let now = time_reply(&mut client, &what);
            let mut alive = Vec::new();
            for _ in 0..100 {
                assert_eq!(reply_line(&mut client, &what), "+OK\r\n", "{what}");
                let header = reply_line(&mut client, &what);
                let live = header == "$1\r\n";
                if live {
                    assert_eq!(reply_line(&mut client, &what), "v\r\n", "{what}");
                } else {
                    assert_eq!(header, "$-1\r\n", "{what}");
                }
                alive.push(live);
            }
            let later = time_reply(&mut client, &what);
            if later != now || now < start || now >= start + 100 {
                continue;
            }
            let expected = (0..100)
                .map(|offset| start + offset >= now)
                .collect::<Vec<_>>();
            assert_eq!(
                alive, expected,
                "{what}: keys expiring from {start} on, read at {now}"
            );
            placed = true;
            break;
        }
        assert!(
            placed,
            "{what}: no batch was read at one reading within its keys"
        );
        drop(client);
        let (status, _) = finished(child);
        assert_eq!(status, 0, "{what}");
    }
}

/// [PRE-2] firn's expiring context removes keys no command reads:
/// a thousand keys set with `PX 50` leave `DBSIZE`, which reads no key, at
/// zero within three seconds, as do a key `RENAME` moved and one `COPY` made,
/// whose expiries are queued under their new names.
#[cfg(target_os = "linux")]
#[test]
fn firn_removes_expired_keys_no_command_reads_on_both_routes() {
    const KEYS: usize = 1000;
    let program = firn();
    for native_ring in [true, false] {
        let what = format!("native ring: {native_ring}");
        let port = free_port();
        let text = port.to_string();
        let child = program.spawn_on_route(native_ring, &[text.as_bytes(), b"1"]);
        let mut client = connect_when_ready(port);
        client
            .set_read_timeout(Some(Duration::from_secs(20)))
            .expect("bound the client's waits");
        let mut batch = Vec::new();
        for index in 0..KEYS {
            let key = format!("key:{index}");
            batch.extend(resp(&["SET", &key, "v", "PX", "50"]));
        }
        for request in [
            vec!["SET", "mover", "v", "PX", "200"],
            vec!["RENAME", "mover", "moved"],
            vec!["SET", "copier", "v", "PX", "200"],
            vec!["COPY", "copier", "copied"],
        ] {
            batch.extend(resp(&request));
        }
        client.write_all(&batch).expect("send the keys");
        let mut expected = b"+OK\r\n".repeat(KEYS);
        expected.extend_from_slice(b"+OK\r\n+OK\r\n+OK\r\n:1\r\n");
        expect_replies(&mut client, &expected, &what);
        let started = Instant::now();
        loop {
            client
                .write_all(&resp(&["DBSIZE"]))
                .expect("count the keys");
            let size = integer_reply(&mut client, &what);
            if size == 0 {
                break;
            }
            assert!(
                started.elapsed() < Duration::from_secs(3),
                "{what}: {size} keys left"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        drop(client);
        let (status, _) = finished(child);
        assert_eq!(status, 0, "{what}");
    }
}

/// [PRE-2] firn replays its append-only file after a restart:
/// every change the first run made, set, removed, incremented, given an expiry
/// or made persistent, holds in the second, and a key whose expiry passed
/// while firn was stopped is absent. As in Redis, the replay applies the
/// file's commands in order without expiring anything, so a key made
/// persistent before its expiry holds its value, and a key incremented before
/// its expiry passed is absent rather than counting again from one. A key a
/// command finds expired is removed, and the file records the removal, as
/// Redis propagates it, so that the commands after it replay as they ran: a
/// `SET` with NX, one with KEEPTTL, an `INCR`, `APPEND`, `SETRANGE`, `MSETNX`
/// and `INCRBYFLOAT` on a key set already expired, a `SET` with NX after
/// `EXISTS`, `GET`, `TTL`, `TYPE`, `DEL`, `PERSIST`, `EXPIRE`, a negative
/// `EXPIRE`, `GETDEL`, `GETEX`, `STRLEN`, `GETRANGE`, `MGET`, or `RENAME` or
/// `COPY` from it, found it so, and a `RENAMENX` and a `COPY` onto it hold
/// their values after the restart, the `INCR` counting from zero and KEEPTTL,
/// as `INCRBYFLOAT`'s record has it, keeping no expiry. Each expiry given
/// relative to the time, by `SET` with EX or PX,
/// `SETEX`, `PSETEX`, `GETEX`, `EXPIRE` with an option and `PEXPIRE`, keeps
/// the very calendar millisecond `PEXPIRETIME` gave before the restart, which
/// a file recording the relative amount would replay later; a string
/// `APPEND` and `SETRANGE` edited holds its bytes; an expiry moves with its
/// key under `RENAME` and is copied with it by `COPY`; and a sum `INCRBYFLOAT`
/// wrote keeps its text and the key its expiry. The first run ends once its
/// one client has closed, after its writer appended and synced the last
/// changes.
#[cfg(target_os = "linux")]
#[test]
fn firn_replays_its_append_only_file_after_a_restart_on_both_routes() {
    let program = firn();
    for native_ring in [true, false] {
        let what = format!("native ring: {native_ring}");
        let name = format!("replay-{native_ring}.aof");
        let port = free_port();
        let text = port.to_string();
        let child = program.spawn_on_route(native_ring, &[text.as_bytes(), b"1", name.as_bytes()]);
        let mut client = connect_when_ready(port);
        client
            .set_read_timeout(Some(Duration::from_secs(20)))
            .expect("bound the first client's waits");
        let mut batch = Vec::new();
        for request in [
            vec!["SET", "gone", "v"],
            vec!["SET", "brief", "v", "PX", "300"],
            vec!["SET", "long", "v", "EX", "100"],
            vec!["INCR", "count"],
            vec!["INCR", "count"],
            vec!["INCRBY", "count", "10"],
            vec!["DECRBY", "count", "3"],
            vec!["DECR", "count"],
            vec!["DEL", "gone"],
            vec!["SET", "kept", "v", "PX", "60000"],
            vec!["PERSIST", "kept"],
            vec!["SET", "later", "v", "PX", "60000"],
            vec!["SET", "persisted", "v", "PX", "300"],
            vec!["PERSIST", "persisted"],
            vec!["SET", "bumped", "5", "PX", "300"],
            vec!["INCR", "bumped"],
            vec!["SET", "lazy:a", "old", "PXAT", "1"],
            vec!["SET", "lazy:a", "new", "NX"],
            vec!["SET", "lazy:b", "5", "PXAT", "1"],
            vec!["INCR", "lazy:b"],
            vec!["SET", "lazy:c", "old", "PXAT", "1"],
            vec!["EXISTS", "lazy:c"],
            vec!["SET", "lazy:c", "new", "NX"],
            vec!["SET", "lazy:d", "old", "PXAT", "1"],
            vec!["GET", "lazy:d"],
            vec!["SET", "lazy:d", "new", "NX"],
            vec!["SET", "lazy:e", "old", "PXAT", "1"],
            vec!["SET", "lazy:e", "new", "KEEPTTL"],
            vec!["SET", "lazy:f", "old", "PXAT", "1"],
            vec!["TTL", "lazy:f"],
            vec!["SET", "lazy:f", "new", "NX"],
            vec!["SET", "lazy:g", "old", "PXAT", "1"],
            vec!["TYPE", "lazy:g"],
            vec!["SET", "lazy:g", "new", "NX"],
            vec!["SET", "lazy:h", "old", "PXAT", "1"],
            vec!["DEL", "lazy:h"],
            vec!["SET", "lazy:h", "new", "NX"],
            vec!["SET", "lazy:i", "old", "PXAT", "1"],
            vec!["PERSIST", "lazy:i"],
            vec!["SET", "lazy:i", "new", "NX"],
            vec!["SET", "lazy:j", "old", "PXAT", "1"],
            vec!["EXPIRE", "lazy:j", "100"],
            vec!["SET", "lazy:j", "new", "NX"],
            vec!["SET", "lazy:k", "old", "PXAT", "1"],
            vec!["GETDEL", "lazy:k"],
            vec!["SET", "lazy:k", "new", "NX"],
            vec!["SET", "lazy:l", "old", "PXAT", "1"],
            vec!["GETEX", "lazy:l", "PERSIST"],
            vec!["SET", "lazy:l", "new", "NX"],
            vec!["SET", "lazy:m", "old", "PXAT", "1"],
            vec!["EXPIRE", "lazy:m", "-1"],
            vec!["SET", "lazy:m", "new", "NX"],
            vec!["SET", "lazy:n", "old", "PXAT", "1"],
            vec!["APPEND", "lazy:n", "x"],
            vec!["SET", "lazy:o", "old", "PXAT", "1"],
            vec!["SETRANGE", "lazy:o", "0", "x"],
            vec!["SET", "lazy:p", "old", "PXAT", "1"],
            vec!["STRLEN", "lazy:p"],
            vec!["SET", "lazy:p", "new", "NX"],
            vec!["SET", "lazy:q", "old", "PXAT", "1"],
            vec!["GETRANGE", "lazy:q", "0", "-1"],
            vec!["SET", "lazy:q", "new", "NX"],
            vec!["SET", "lazy:r", "old", "PXAT", "1"],
            vec!["MGET", "lazy:r"],
            vec!["SET", "lazy:r", "new", "NX"],
            vec!["SET", "lazy:s", "old", "PXAT", "1"],
            vec!["MSETNX", "lazy:s", "new"],
            vec!["SET", "lazy:t", "5", "PXAT", "1"],
            vec!["INCRBYFLOAT", "lazy:t", "0.5"],
            vec!["SET", "lazy:u", "old", "PXAT", "1"],
            vec!["RENAME", "lazy:u", "lazy:u2"],
            vec!["SET", "lazy:u", "new", "NX"],
            vec!["SET", "lazy:v", "old", "PXAT", "1"],
            vec!["SET", "lazy:vs", "v"],
            vec!["RENAMENX", "lazy:vs", "lazy:v"],
            vec!["SET", "lazy:w", "old", "PXAT", "1"],
            vec!["COPY", "lazy:w", "lazy:w2"],
            vec!["SET", "lazy:w", "new", "NX"],
            vec!["SET", "lazy:x", "old", "PXAT", "1"],
            vec!["SET", "lazy:xs", "v"],
            vec!["COPY", "lazy:xs", "lazy:x"],
            vec!["SETEX", "at:setex", "100", "v"],
            vec!["PSETEX", "at:psetex", "100000", "v"],
            vec!["SET", "at:getex", "v"],
            vec!["GETEX", "at:getex", "EX", "100"],
            vec!["SET", "at:expire", "v"],
            vec!["EXPIRE", "at:expire", "100", "NX"],
            vec!["SET", "at:pexpire", "v"],
            vec!["PEXPIRE", "at:pexpire", "100000"],
            vec!["SET", "edited", "ab"],
            vec!["APPEND", "edited", "cd"],
            vec!["SETRANGE", "edited", "1", "XY"],
            vec!["MSETNX", "pair:a", "1", "pair:b", "2"],
            vec!["SET", "mv:src", "v", "PXAT", "99999999999999"],
            vec!["RENAME", "mv:src", "mv:dst"],
            vec!["COPY", "mv:dst", "mv:copy"],
            vec!["SET", "float", "10.5", "PXAT", "99999999999999"],
            vec!["INCRBYFLOAT", "float", "0.1"],
        ] {
            batch.extend(resp(&request));
        }
        client.write_all(&batch).expect("send the changes");
        expect_replies(
            &mut client,
            b"+OK\r\n+OK\r\n+OK\r\n:1\r\n:2\r\n:12\r\n:9\r\n:8\r\n:1\r\n+OK\r\n:1\r\n+OK\r\n+OK\r\n:1\r\n+OK\r\n:6\r\n+OK\r\n+OK\r\n+OK\r\n:1\r\n+OK\r\n:0\r\n+OK\r\n+OK\r\n$-1\r\n+OK\r\n+OK\r\n+OK\r\n+OK\r\n:-2\r\n+OK\r\n+OK\r\n+none\r\n+OK\r\n+OK\r\n:0\r\n+OK\r\n+OK\r\n:0\r\n+OK\r\n+OK\r\n:0\r\n+OK\r\n+OK\r\n$-1\r\n+OK\r\n+OK\r\n$-1\r\n+OK\r\n+OK\r\n:0\r\n+OK\r\n+OK\r\n:1\r\n+OK\r\n:1\r\n+OK\r\n:0\r\n+OK\r\n+OK\r\n$0\r\n\r\n+OK\r\n+OK\r\n*1\r\n$-1\r\n+OK\r\n+OK\r\n:1\r\n+OK\r\n$3\r\n0.5\r\n+OK\r\n-ERR no such key\r\n+OK\r\n+OK\r\n+OK\r\n:1\r\n+OK\r\n:0\r\n+OK\r\n+OK\r\n+OK\r\n:1\r\n+OK\r\n+OK\r\n+OK\r\n$1\r\nv\r\n+OK\r\n:1\r\n+OK\r\n:1\r\n+OK\r\n:4\r\n:4\r\n:1\r\n+OK\r\n+OK\r\n:1\r\n+OK\r\n$4\r\n10.6\r\n",
            &what,
        );
        let timed = [
            "long",
            "later",
            "at:setex",
            "at:psetex",
            "at:getex",
            "at:expire",
            "at:pexpire",
        ];
        let mut expiries = Vec::new();
        for key in timed {
            client
                .write_all(&resp(&["PEXPIRETIME", key]))
                .expect("ask the expiry");
            expiries.push(integer_reply(&mut client, &what));
        }
        drop(client);
        let (status, _) = finished(child);
        assert_eq!(status, 0, "{what}: the first run");
        std::thread::sleep(Duration::from_millis(400));
        let port = free_port();
        let text = port.to_string();
        let child = program.spawn_on_route(native_ring, &[text.as_bytes(), b"1", name.as_bytes()]);
        let mut client = connect_when_ready(port);
        client
            .set_read_timeout(Some(Duration::from_secs(20)))
            .expect("bound the second client's waits");
        let mut batch = Vec::new();
        for request in [
            vec!["GET", "gone"],
            vec!["GET", "brief"],
            vec!["GET", "long"],
            vec!["GET", "count"],
            vec!["GET", "kept"],
            vec!["TTL", "kept"],
            vec!["GET", "persisted"],
            vec!["TTL", "persisted"],
            vec!["GET", "bumped"],
            vec!["DBSIZE"],
            vec!["GET", "lazy:a"],
            vec!["GET", "lazy:b"],
            vec!["TTL", "lazy:b"],
            vec!["GET", "lazy:c"],
            vec!["GET", "lazy:d"],
            vec!["GET", "lazy:e"],
            vec!["TTL", "lazy:e"],
            vec!["GET", "lazy:f"],
            vec!["GET", "lazy:g"],
            vec!["GET", "lazy:h"],
            vec!["GET", "lazy:i"],
            vec!["GET", "lazy:j"],
            vec!["GET", "lazy:k"],
            vec!["GET", "lazy:l"],
            vec!["GET", "lazy:m"],
            vec!["GET", "lazy:n"],
            vec!["GET", "lazy:o"],
            vec!["GET", "lazy:p"],
            vec!["GET", "lazy:q"],
            vec!["GET", "lazy:r"],
            vec!["GET", "lazy:s"],
            vec!["GET", "lazy:t"],
            vec!["GET", "lazy:u"],
            vec!["GET", "lazy:v"],
            vec!["GET", "lazy:w"],
            vec!["GET", "lazy:x"],
            vec!["GET", "edited"],
            vec!["MGET", "pair:a", "pair:b"],
            vec!["PEXPIRETIME", "mv:dst"],
            vec!["PEXPIRETIME", "mv:copy"],
            vec!["EXISTS", "mv:src"],
            vec!["GET", "float"],
            vec!["PEXPIRETIME", "float"],
        ] {
            batch.extend(resp(&request));
        }
        client.write_all(&batch).expect("read the replayed keys");
        expect_replies(
            &mut client,
            b"$-1\r\n$-1\r\n$1\r\nv\r\n$1\r\n8\r\n$1\r\nv\r\n:-1\r\n$1\r\nv\r\n:-1\r\n$-1\r\n:41\r\n$3\r\nnew\r\n$1\r\n1\r\n:-1\r\n$3\r\nnew\r\n$3\r\nnew\r\n$3\r\nnew\r\n:-1\r\n$3\r\nnew\r\n$3\r\nnew\r\n$3\r\nnew\r\n$3\r\nnew\r\n$3\r\nnew\r\n$3\r\nnew\r\n$3\r\nnew\r\n$3\r\nnew\r\n$1\r\nx\r\n$1\r\nx\r\n$3\r\nnew\r\n$3\r\nnew\r\n$3\r\nnew\r\n$3\r\nnew\r\n$3\r\n0.5\r\n$3\r\nnew\r\n$1\r\nv\r\n$3\r\nnew\r\n$1\r\nv\r\n$4\r\naXYd\r\n*2\r\n$1\r\n1\r\n$1\r\n2\r\n:99999999999999\r\n:99999999999999\r\n:0\r\n$4\r\n10.6\r\n:99999999999999\r\n",
            &what,
        );
        for (key, expiry) in timed.iter().zip(&expiries) {
            client
                .write_all(&resp(&["PEXPIRETIME", key]))
                .expect("ask the replayed expiry");
            let replayed = integer_reply(&mut client, &what);
            assert_eq!(replayed, *expiry, "{what}: the expiry of {key}");
        }
        client
            .write_all(&resp(&["TTL", "long"]))
            .expect("ask the time left");
        let long = integer_reply(&mut client, &what);
        assert!((98..=100).contains(&long), "{what}: {long}");
        client
            .write_all(&resp(&["PTTL", "later"]))
            .expect("ask the time left");
        let later = integer_reply(&mut client, &what);
        assert!((58_000..=60_000).contains(&later), "{what}: {later}");
        drop(client);
        let (status, _) = finished(child);
        assert_eq!(status, 0, "{what}: the second run");
    }
}

/// [PRE-2] firn replays list and set changes after a restart on both routes.
/// A list pushed to or moved to and a set added to or moved to after expiry
/// retain their new values. Keys removed on expiry before RPUSH by list pops,
/// updates, removals, trims, inserts or moves, or set removals, pops, moves or
/// stores replay with their new list values. The first run closes after its
/// writer appends and syncs the changes; the second checks their values and
/// the database size alongside seeded string keys.
#[cfg(target_os = "linux")]
#[test]
fn firn_replays_its_append_only_file_after_a_restart_on_both_routes_with_lists_and_sets() {
    let program = firn();
    for native_ring in [true, false] {
        let what = format!("native ring: {native_ring}");
        let name = format!("replay-lists-sets-{native_ring}.aof");
        let port = free_port();
        let text = port.to_string();
        let child = program.spawn_on_route(native_ring, &[text.as_bytes(), b"1", name.as_bytes()]);
        let mut client = connect_when_ready(port);
        client
            .set_read_timeout(Some(Duration::from_secs(20)))
            .expect("bound the first client's waits");
        // The distinct GET count and DBSIZE replies need the base case's 18 live
        // keys, but none of its expiry or lazy-removal operations.
        client
            .write_all(&resp(&[
                "MSET",
                "long",
                "v",
                "count",
                "2",
                "kept",
                "v",
                "later",
                "v",
                "persisted",
                "v",
                "lazy:a",
                "new",
                "lazy:b",
                "1",
                "lazy:c",
                "new",
                "lazy:d",
                "new",
                "lazy:e",
                "new",
                "lazy:f",
                "new",
                "lazy:g",
                "new",
                "lazy:h",
                "new",
                "lazy:i",
                "new",
                "lazy:j",
                "new",
                "lazy:k",
                "new",
                "lazy:l",
                "new",
                "lazy:m",
                "new",
            ]))
            .expect("seed the live string keys");
        expect_replies(&mut client, b"+OK\r\n", &what);
        // Keys a list or set command finds expired: each is given an expiry of one
        // millisecond and written five milliseconds later, by a command that
        // writes it or by one that only removes it, then by RPUSH.
        let mut batch = Vec::new();
        for request in [
            vec!["RPUSH", "list:push", "old"],
            vec!["RPUSH", "list:pop", "old"],
            vec!["RPUSH", "list:set", "old"],
            vec!["RPUSH", "list:rem", "old"],
            vec!["RPUSH", "list:trim", "old"],
            vec!["RPUSH", "list:insert", "old"],
            vec!["RPUSH", "list:from", "old"],
            vec!["RPUSH", "list:to", "old"],
            vec!["RPUSH", "list:source", "m"],
            vec!["SADD", "set:add", "old"],
            vec!["SADD", "set:rem", "old"],
            vec!["SADD", "set:pop", "old"],
            vec!["SADD", "set:from", "old"],
            vec!["SADD", "set:to", "old"],
            vec!["SADD", "set:store", "old"],
            vec!["SADD", "set:moving", "m"],
            vec!["SADD", "set:other", "old"],
        ] {
            batch.extend(resp(&request));
        }
        for key in [
            "list:push",
            "list:pop",
            "list:set",
            "list:rem",
            "list:trim",
            "list:insert",
            "list:from",
            "list:to",
            "set:add",
            "set:rem",
            "set:pop",
            "set:from",
            "set:to",
            "set:store",
        ] {
            batch.extend(resp(&["PEXPIRE", key, "1"]));
        }
        client.write_all(&batch).expect("give keys a brief expiry");
        expect_replies(&mut client, &b":1\r\n".repeat(31), &what);
        std::thread::sleep(Duration::from_millis(5));
        let mut batch = Vec::new();
        for request in [
            vec!["LPUSH", "list:push", "new"],
            vec!["LPOP", "list:pop"],
            vec!["LSET", "list:set", "0", "v"],
            vec!["LREM", "list:rem", "0", "old"],
            vec!["LTRIM", "list:trim", "0", "0"],
            vec!["LINSERT", "list:insert", "BEFORE", "old", "v"],
            vec!["LMOVE", "list:from", "list:nowhere", "LEFT", "LEFT"],
            vec!["LMOVE", "list:source", "list:to", "LEFT", "RIGHT"],
            vec!["SADD", "set:add", "new"],
            vec!["SREM", "set:rem", "old"],
            vec!["SPOP", "set:pop"],
            vec!["SMOVE", "set:from", "set:nowhere", "old"],
            vec!["SMOVE", "set:moving", "set:to", "m"],
            vec!["SINTERSTORE", "set:stored", "set:store", "set:other"],
        ] {
            batch.extend(resp(&request));
        }
        for key in [
            "list:pop",
            "list:set",
            "list:rem",
            "list:trim",
            "list:insert",
            "list:from",
            "set:rem",
            "set:pop",
            "set:from",
            "set:store",
        ] {
            batch.extend(resp(&["RPUSH", key, "new"]));
        }
        client
            .write_all(&batch)
            .expect("write the keys found expired");
        let mut expected = b":1\r\n$-1\r\n-ERR no such key\r\n:0\r\n+OK\r\n:0\r\n$-1\r\n$1\r\nm\r\n:1\r\n:0\r\n$-1\r\n:0\r\n:1\r\n:0\r\n".to_vec();
        expected.extend_from_slice(&b":1\r\n".repeat(10));
        expect_replies(&mut client, &expected, &what);
        drop(client);
        let (status, _) = finished(child);
        assert_eq!(status, 0, "{what}: the first run");
        std::thread::sleep(Duration::from_millis(400));
        let port = free_port();
        let text = port.to_string();
        let child = program.spawn_on_route(native_ring, &[text.as_bytes(), b"1", name.as_bytes()]);
        let mut client = connect_when_ready(port);
        client
            .set_read_timeout(Some(Duration::from_secs(20)))
            .expect("bound the second client's waits");
        let mut batch = Vec::new();
        for request in [vec!["GET", "count"], vec!["DBSIZE"]] {
            batch.extend(resp(&request));
        }
        client
            .write_all(&batch)
            .expect("read the replayed key count");
        expect_replies(&mut client, b"$1\r\n2\r\n:33\r\n", &what);
        let reads = [
            (
                vec!["LRANGE", "list:push", "0", "-1"],
                "*1\r\n$3\r\nnew\r\n",
            ),
            (vec!["LRANGE", "list:pop", "0", "-1"], "*1\r\n$3\r\nnew\r\n"),
            (vec!["LRANGE", "list:set", "0", "-1"], "*1\r\n$3\r\nnew\r\n"),
            (vec!["LRANGE", "list:rem", "0", "-1"], "*1\r\n$3\r\nnew\r\n"),
            (
                vec!["LRANGE", "list:trim", "0", "-1"],
                "*1\r\n$3\r\nnew\r\n",
            ),
            (
                vec!["LRANGE", "list:insert", "0", "-1"],
                "*1\r\n$3\r\nnew\r\n",
            ),
            (
                vec!["LRANGE", "list:from", "0", "-1"],
                "*1\r\n$3\r\nnew\r\n",
            ),
            (vec!["LRANGE", "list:to", "0", "-1"], "*1\r\n$1\r\nm\r\n"),
            (vec!["LRANGE", "set:rem", "0", "-1"], "*1\r\n$3\r\nnew\r\n"),
            (vec!["LRANGE", "set:pop", "0", "-1"], "*1\r\n$3\r\nnew\r\n"),
            (vec!["LRANGE", "set:from", "0", "-1"], "*1\r\n$3\r\nnew\r\n"),
            (
                vec!["LRANGE", "set:store", "0", "-1"],
                "*1\r\n$3\r\nnew\r\n",
            ),
            (vec!["SMEMBERS", "set:add"], "*1\r\n$3\r\nnew\r\n"),
            (vec!["SMEMBERS", "set:to"], "*1\r\n$1\r\nm\r\n"),
        ];
        let mut batch = Vec::new();
        for (request, _) in &reads {
            batch.extend(resp(request));
        }
        client
            .write_all(&batch)
            .expect("read the keys found expired");
        let mut wrong = Vec::new();
        for (request, expected) in &reads {
            let reply = whole_reply(&mut client, &what);
            if reply != *expected {
                wrong.push(format!("{}: {reply:?}", request[1]));
            }
        }
        assert!(
            wrong.is_empty(),
            "{what}: keys replayed against the values that had expired: {wrong:?}"
        );
        drop(client);
        let (status, _) = finished(child);
        assert_eq!(status, 0, "{what}: the second run");
    }
}

/// [PRE-2] firn records its writes in its append-only file as Redis 7.0.15
/// propagates them, which a replay does not show where two forms replay to the
/// same state: a key `MSET` or `GET` finds expired is recorded as its `DEL`
/// before the command, and so is each key `MSETNX` finds expired before its
/// first live one, and each key `DEL` or `EXISTS` finds expired, once, in the
/// order named; `GETDEL` is recorded as `DEL`,
/// `GETSET` as `SET`, `INCRBYFLOAT` as `SET` with `KEEPTTL`, `GETEX` as
/// `PEXPIREAT` or `PERSIST`, `EXAT` as `PXAT` and `EXPIREAT` as `PEXPIREAT`,
/// both in milliseconds, a `PEXPIREAT` its LT refused not at all, and the
/// other commands as they were sent. The expected file is redis-server
/// 7.0.15's for the same requests but for the `SELECT 0` it writes first and
/// the `MULTI` and `EXEC` it brackets one command's records in. The expiring
/// context, which records the same `DEL` when it removes an expired key
/// first, could change the file only by removing `zz` or `aa` between their
/// `SET` and `MSETNX`, a window of microseconds.
#[cfg(target_os = "linux")]
#[test]
fn firn_records_its_writes_as_redis_propagates_them() {
    let program = firn();
    let fixture = fixture_directory();
    let port = free_port();
    let text = port.to_string();
    let client = std::thread::spawn(move || {
        let mut client = connect_when_ready(port);
        let mut batch = Vec::new();
        for request in [
            vec!["SET", "mset:k", "old", "PXAT", "1"],
            vec!["MSET", "mset:k", "new"],
            vec!["SET", "read", "v", "PXAT", "1"],
            vec!["GET", "read"],
            vec!["SET", "gd", "v"],
            vec!["GETDEL", "gd"],
            vec!["SET", "gs", "a"],
            vec!["GETSET", "gs", "b"],
            vec!["SET", "f", "10.5"],
            vec!["INCRBYFLOAT", "f", "0.1"],
            vec!["SET", "ge", "v"],
            vec!["GETEX", "ge", "PXAT", "99999999999999"],
            vec!["GETEX", "ge", "PERSIST"],
            vec!["SET", "at", "v", "EXAT", "99999999999"],
            vec!["EXPIREAT", "at", "99999999998"],
            vec!["PEXPIREAT", "at", "99999999999999", "LT"],
            vec!["SETNX", "nx", "v"],
            vec!["APPEND", "ap", "x"],
            vec!["SETRANGE", "ap", "3", "y"],
            vec!["INCRBY", "ib", "5"],
            vec!["DECR", "ib"],
            vec!["SET", "zz", "1", "PXAT", "1"],
            vec!["SET", "aa", "1", "PXAT", "1"],
            vec!["SET", "live", "v"],
            vec!["MSETNX", "zz", "1", "aa", "2", "live", "3"],
            vec!["MSETNX", "m1", "a", "m2", "b"],
            vec!["RENAME", "m1", "m3"],
            vec!["COPY", "m3", "m4"],
            vec!["UNLINK", "m4"],
            vec!["SET", "y2", "1", "PXAT", "1"],
            vec!["SET", "y1", "1", "PXAT", "1"],
            vec!["DEL", "y2", "y1", "y2"],
            vec!["SET", "x2", "1", "PXAT", "1"],
            vec!["SET", "x1", "1", "PXAT", "1"],
            vec!["EXISTS", "x2", "x1"],
        ] {
            batch.extend(resp(&request));
        }
        client.write_all(&batch).expect("send the writes");
        expect_replies(
            &mut client,
            b"+OK\r\n+OK\r\n+OK\r\n$-1\r\n+OK\r\n$1\r\nv\r\n+OK\r\n$1\r\na\r\n+OK\r\n$4\r\n10.6\r\n+OK\r\n$1\r\nv\r\n$1\r\nv\r\n+OK\r\n:1\r\n:0\r\n:1\r\n:1\r\n:4\r\n:5\r\n:4\r\n+OK\r\n+OK\r\n+OK\r\n:0\r\n:1\r\n+OK\r\n:1\r\n:1\r\n+OK\r\n+OK\r\n:0\r\n+OK\r\n+OK\r\n:0\r\n",
            "the writes",
        );
    });
    let output = program.run(fixture.path(), &[text.as_bytes(), b"1", b"forms.aof"]);
    client.join().expect("the client's exchange");
    assert!(output.status.success(), "firn: {:?}", output.status);
    let file = std::fs::read(fixture.path().join("forms.aof")).expect("read firn's file");
    assert_eq!(
        String::from_utf8_lossy(&file),
        String::from_utf8_lossy(
            b"*5\r\n$3\r\nSET\r\n$6\r\nmset:k\r\n$3\r\nold\r\n$4\r\nPXAT\r\n$1\r\n1\r\n*2\r\n$3\r\nDEL\r\n$6\r\nmset:k\r\n*3\r\n$4\r\nMSET\r\n$6\r\nmset:k\r\n$3\r\nnew\r\n*5\r\n$3\r\nSET\r\n$4\r\nread\r\n$1\r\nv\r\n$4\r\nPXAT\r\n$1\r\n1\r\n*2\r\n$3\r\nDEL\r\n$4\r\nread\r\n*3\r\n$3\r\nSET\r\n$2\r\ngd\r\n$1\r\nv\r\n*2\r\n$3\r\nDEL\r\n$2\r\ngd\r\n*3\r\n$3\r\nSET\r\n$2\r\ngs\r\n$1\r\na\r\n*3\r\n$3\r\nSET\r\n$2\r\ngs\r\n$1\r\nb\r\n*3\r\n$3\r\nSET\r\n$1\r\nf\r\n$4\r\n10.5\r\n*4\r\n$3\r\nSET\r\n$1\r\nf\r\n$4\r\n10.6\r\n$7\r\nKEEPTTL\r\n*3\r\n$3\r\nSET\r\n$2\r\nge\r\n$1\r\nv\r\n*3\r\n$9\r\nPEXPIREAT\r\n$2\r\nge\r\n$14\r\n99999999999999\r\n*2\r\n$7\r\nPERSIST\r\n$2\r\nge\r\n*5\r\n$3\r\nSET\r\n$2\r\nat\r\n$1\r\nv\r\n$4\r\nPXAT\r\n$14\r\n99999999999000\r\n*3\r\n$9\r\nPEXPIREAT\r\n$2\r\nat\r\n$14\r\n99999999998000\r\n*3\r\n$5\r\nSETNX\r\n$2\r\nnx\r\n$1\r\nv\r\n*3\r\n$6\r\nAPPEND\r\n$2\r\nap\r\n$1\r\nx\r\n*4\r\n$8\r\nSETRANGE\r\n$2\r\nap\r\n$1\r\n3\r\n$1\r\ny\r\n*3\r\n$6\r\nINCRBY\r\n$2\r\nib\r\n$1\r\n5\r\n*2\r\n$4\r\nDECR\r\n$2\r\nib\r\n*5\r\n$3\r\nSET\r\n$2\r\nzz\r\n$1\r\n1\r\n$4\r\nPXAT\r\n$1\r\n1\r\n*5\r\n$3\r\nSET\r\n$2\r\naa\r\n$1\r\n1\r\n$4\r\nPXAT\r\n$1\r\n1\r\n*3\r\n$3\r\nSET\r\n$4\r\nlive\r\n$1\r\nv\r\n*2\r\n$3\r\nDEL\r\n$2\r\nzz\r\n*2\r\n$3\r\nDEL\r\n$2\r\naa\r\n*5\r\n$6\r\nMSETNX\r\n$2\r\nm1\r\n$1\r\na\r\n$2\r\nm2\r\n$1\r\nb\r\n*3\r\n$6\r\nRENAME\r\n$2\r\nm1\r\n$2\r\nm3\r\n*3\r\n$4\r\nCOPY\r\n$2\r\nm3\r\n$2\r\nm4\r\n*2\r\n$6\r\nUNLINK\r\n$2\r\nm4\r\n*5\r\n$3\r\nSET\r\n$2\r\ny2\r\n$1\r\n1\r\n$4\r\nPXAT\r\n$1\r\n1\r\n*5\r\n$3\r\nSET\r\n$2\r\ny1\r\n$1\r\n1\r\n$4\r\nPXAT\r\n$1\r\n1\r\n*2\r\n$3\r\nDEL\r\n$2\r\ny2\r\n*2\r\n$3\r\nDEL\r\n$2\r\ny1\r\n*5\r\n$3\r\nSET\r\n$2\r\nx2\r\n$1\r\n1\r\n$4\r\nPXAT\r\n$1\r\n1\r\n*5\r\n$3\r\nSET\r\n$2\r\nx1\r\n$1\r\n1\r\n$4\r\nPXAT\r\n$1\r\n1\r\n*2\r\n$3\r\nDEL\r\n$2\r\nx2\r\n*2\r\n$3\r\nDEL\r\n$2\r\nx1\r\n"
        ),
        "firn's append-only file"
    );
}

/// [PRE-2] firn applies append-only blocks only after their EXEC, including
/// blocks crossing its read buffer, and passes over EXEC outside a block,
/// as Redis 7.0.15 does. A clean end inside a block cuts before its latest
/// MULTI; an end inside a command cuts after the last whole command, even
/// inside a block. The retained bytes and a second replay show why that
/// distinction matters: a write appended inside a retained unfinished block
/// is dropped on restart, as Redis drops it.
#[cfg(target_os = "linux")]
#[test]
fn firn_replays_blocks_and_cuts_an_unloaded_end_as_redis_does() {
    let program = firn();
    let fixture = fixture_directory();
    let a = resp(&["SET", "a", "1"]);
    let m = resp(&["MULTI"]);
    let e = resp(&["EXEC"]);
    let b = resp(&["SET", "b", "2"]);
    let c = resp(&["SET", "c", "3"]);
    let value = "v".repeat(100);
    let blocks: Vec<u8> = (0..2000)
        .flat_map(|i| resp(&["SET", &format!("k{i}"), &value]))
        .collect();
    let whole = [
        a.clone(),
        m.clone(),
        b.clone(),
        resp(&["INCR", "a"]),
        e.clone(),
        c.clone(),
    ]
    .concat();
    let crossing = [
        a.clone(),
        m.clone(),
        blocks.clone(),
        e.clone(),
        resp(&["SET", "z", "9"]),
    ]
    .concat();
    let unfinished = [a.clone(), m.clone(), b.clone()].concat();
    let outside_exec = [a.clone(), e, b.clone()].concat();
    let cases = [
        (
            "block-applies.aof",
            whole.clone(),
            vec![
                (resp(&["GET", "a"]), b"$1\r\n2\r\n".to_vec()),
                (resp(&["GET", "b"]), b"$1\r\n2\r\n".to_vec()),
                (resp(&["GET", "c"]), b"$1\r\n3\r\n".to_vec()),
            ],
            whole,
            b"$1\r\n1\r\n:4\r\n".to_vec(),
        ),
        (
            "block-crosses-buffer.aof",
            crossing.clone(),
            vec![
                (resp(&["DBSIZE"]), b":2002\r\n".to_vec()),
                (
                    resp(&["GET", "k1999"]),
                    format!("$100\r\n{value}\r\n").into_bytes(),
                ),
            ],
            crossing,
            b"$1\r\n1\r\n:2003\r\n".to_vec(),
        ),
        (
            "clean-end-in-block.aof",
            [a.clone(), m.clone(), blocks].concat(),
            vec![(resp(&["DBSIZE"]), b":1\r\n".to_vec())],
            a.clone(),
            b"$1\r\n1\r\n:2\r\n".to_vec(),
        ),
        (
            "partial-command-in-block.aof",
            [unfinished.clone(), c[..9].to_vec()].concat(),
            vec![(resp(&["DBSIZE"]), b":1\r\n".to_vec())],
            unfinished.clone(),
            b"$-1\r\n:1\r\n".to_vec(),
        ),
        (
            "partial-command-outside-block.aof",
            [a.clone(), b[..12].to_vec()].concat(),
            vec![(resp(&["DBSIZE"]), b":1\r\n".to_vec())],
            a,
            b"$1\r\n1\r\n:2\r\n".to_vec(),
        ),
        (
            "nested-multi.aof",
            [unfinished.clone(), m, c].concat(),
            vec![(resp(&["DBSIZE"]), b":1\r\n".to_vec())],
            unfinished,
            b"$-1\r\n:1\r\n".to_vec(),
        ),
        (
            "exec-outside-block.aof",
            outside_exec.clone(),
            vec![(resp(&["DBSIZE"]), b":2\r\n".to_vec())],
            outside_exec,
            b"$1\r\n1\r\n:3\r\n".to_vec(),
        ),
    ];
    for (name, input, checks, mut kept, reload) in cases {
        let path = fixture.path().join(name);
        std::fs::write(&path, input)
            .unwrap_or_else(|error| panic!("{name}: write the fixture: {error}"));
        let mut batch = Vec::new();
        let mut expected = Vec::new();
        for (request, reply) in checks {
            batch.extend(request);
            expected.extend(reply);
        }
        let after = resp(&["SET", "after", "1"]);
        batch.extend_from_slice(&after);
        expected.extend_from_slice(b"+OK\r\n");
        kept.extend(after);
        let port = free_port();
        let text = port.to_string();
        let client = std::thread::spawn(move || {
            let mut client = connect_when_ready(port);
            client
                .write_all(&batch)
                .unwrap_or_else(|error| panic!("{name}: send the checks and write: {error}"));
            expect_replies(&mut client, &expected, &format!("{name}: first replay"));
        });
        let output = program.run(fixture.path(), &[text.as_bytes(), b"1", name.as_bytes()]);
        client
            .join()
            .unwrap_or_else(|error| panic!("{name}: first client: {error:?}"));
        assert!(output.status.success(), "{name}: first run: {output:?}");
        let file = std::fs::read(&path)
            .unwrap_or_else(|error| panic!("{name}: read the retained file: {error}"));
        assert_eq!(file, kept, "{name}: retained bytes and appended write");

        let port = free_port();
        let text = port.to_string();
        let client = std::thread::spawn(move || {
            let mut client = connect_when_ready(port);
            let batch = [resp(&["GET", "after"]), resp(&["DBSIZE"])].concat();
            client
                .write_all(&batch)
                .unwrap_or_else(|error| panic!("{name}: send the reload checks: {error}"));
            expect_replies(&mut client, &reload, &format!("{name}: second replay"));
        });
        let output = program.run(fixture.path(), &[text.as_bytes(), b"1", name.as_bytes()]);
        client
            .join()
            .unwrap_or_else(|error| panic!("{name}: second client: {error:?}"));
        assert!(output.status.success(), "{name}: second run: {output:?}");
    }
}

/// [PRE-2] firn stops with status 4 before it listens when its append-only
/// file holds a record that is not a well-formed command, here an argument
/// line that does not start with `$` between two whole commands, where
/// Redis 7.0.15's loader takes its format-error path and exits; the file
/// stays as it was, neither cut nor appended to.
#[cfg(target_os = "linux")]
#[test]
fn firn_stops_on_an_append_only_file_that_does_not_parse() {
    let program = firn();
    let fixture = fixture_directory();
    let name = "malformed.aof";
    let path = fixture.path().join(name);
    let content = [
        resp(&["SET", "a", "1"]),
        b"*1\r\n:3\r\nfoo\r\n".to_vec(),
        resp(&["SET", "b", "2"]),
    ]
    .concat();
    std::fs::write(&path, &content).unwrap_or_else(|error| panic!("write the fixture: {error}"));
    let port = free_port();
    let text = port.to_string();
    let output = program.run(fixture.path(), &[text.as_bytes(), b"1", name.as_bytes()]);
    assert_eq!(
        output.status.code(),
        Some(4),
        "firn must stop with status 4: {output:?}"
    );
    let after = std::fs::read(&path).unwrap_or_else(|error| panic!("read the file: {error}"));
    assert_eq!(after, content, "the file must stay as it was");
}

/// [PRE-2] a deadline on `receive_next` closes a client silent past
/// firn's idle limit: with a limit of one second, the connection ends after
/// at least 0.9 and at most two seconds of silence. A limit CONFIG SET
/// removes reaches a client already waiting under the old one, which reads the
/// limit again when its deadline passes, and a client that connects after it:
/// both stay open through 1.5 seconds of silence. INFO then reports an uptime
/// of at least the whole seconds since firn listened. On the first route the
/// client that removed the limit sets one of five seconds, under which another
/// client waits, then one of a second, and keeps sending: it reads the limit
/// within a second, as every sending client does, and is closed once it falls
/// silent, after at least 0.9 and at most two seconds, while the waiting
/// client, whose waits under a limit last a second at most, is closed within
/// 2.5 seconds of its last request rather than five. A third client there
/// leaves the replies to its requests unread for 1.5 seconds, more than the
/// limit, and sends again half a second after reading them: its silence counts
/// from the replies, as Redis counts it from its last write, so it stays open.
#[cfg(target_os = "linux")]
#[test]
fn firn_closes_a_client_silent_past_its_idle_limit_on_both_routes() {
    let program = firn();
    for native_ring in [true, false] {
        let what = format!("native ring: {native_ring}");
        let port = free_port();
        let text = port.to_string();
        let clients: &[u8] = if native_ring { b"5" } else { b"3" };
        let child = program.spawn_on_route(native_ring, &[text.as_bytes(), clients, b"-", b"1"]);
        let mut client = connect_when_ready(port);
        let ready = Instant::now();
        client
            .set_read_timeout(Some(Duration::from_secs(10)))
            .expect("bound the client's waits");
        client.write_all(&resp(&["PING"])).expect("send a ping");
        expect_replies(&mut client, b"+PONG\r\n", &what);
        let started = Instant::now();
        let mut rest = [0_u8; 16];
        let read = client
            .read(&mut rest)
            .unwrap_or_else(|error| panic!("{what}: the connection stayed open: {error}"));
        let silent = started.elapsed();
        assert_eq!(read, 0, "{what}: {:?}", &rest[..read]);
        assert!(
            silent >= Duration::from_millis(900) && silent <= Duration::from_secs(2),
            "{what}: closed after {silent:?}"
        );
        drop(client);
        let mut changer = connect_when_ready(port);
        changer
            .write_all(&resp(&["CONFIG", "SET", "timeout", "0"]))
            .expect("remove the limit");
        expect_replies(&mut changer, b"+OK\r\n", &what);
        let mut later = connect_when_ready(port);
        later.write_all(&resp(&["PING"])).expect("send a ping");
        expect_replies(&mut later, b"+PONG\r\n", &what);
        expect_silence_for(&mut changer, Duration::from_millis(1500), &what);
        expect_silence(&mut later, &what);
        changer
            .write_all(&resp(&["CONFIG", "GET", "timeout"]))
            .expect("read the limit");
        expect_replies(&mut changer, b"*2\r\n$7\r\ntimeout\r\n$1\r\n0\r\n", &what);
        let listened = ready.elapsed().as_secs();
        changer
            .write_all(&resp(&["INFO", "server"]))
            .expect("ask for the uptime");
        let info = bulk_reply(&mut changer, &what);
        let uptime = info_field(&info, "uptime_in_seconds")
            .and_then(|value| value.parse::<u64>().ok())
            .expect("INFO's uptime_in_seconds");
        assert!(
            listened >= 2 && uptime >= listened,
            "{what}: {uptime} seconds up, {listened} since firn listened"
        );
        later.write_all(&resp(&["PING"])).expect("send a ping");
        expect_replies(&mut later, b"+PONG\r\n", &what);
        drop(later);
        if native_ring {
            changer
                .write_all(&resp(&["CONFIG", "SET", "timeout", "5"]))
                .expect("set a longer limit");
            expect_replies(&mut changer, b"+OK\r\n", &what);
            let mut waiter = connect_when_ready(port);
            waiter.write_all(&resp(&["PING"])).expect("send a ping");
            expect_replies(&mut waiter, b"+PONG\r\n", &what);
            let waited = Instant::now();
            changer
                .write_all(&resp(&["CONFIG", "SET", "timeout", "1"]))
                .expect("set the limit again");
            expect_replies(&mut changer, b"+OK\r\n", &what);
            let reader = what.clone();
            let late = std::thread::spawn(move || read_replies_late(port, &reader));
            let sending = Instant::now();
            let mut last_ping = sending;
            while sending.elapsed() < Duration::from_millis(1300) {
                std::thread::sleep(Duration::from_millis(200));
                changer.write_all(&resp(&["PING"])).expect("keep sending");
                expect_replies(&mut changer, b"+PONG\r\n", &what);
                last_ping = Instant::now();
            }
            expect_closed(&mut waiter, &what);
            let silent = waited.elapsed();
            assert!(
                silent <= Duration::from_millis(2500),
                "{what}: the waiting client closed after {silent:?}"
            );
            expect_closed(&mut changer, &what);
            let silent = last_ping.elapsed();
            assert!(
                silent >= Duration::from_millis(900) && silent <= Duration::from_secs(2),
                "{what}: closed after {silent:?}"
            );
            late.join().expect("the client reading its replies late");
        }
        drop(changer);
        let (status, _) = finished(child);
        assert_eq!(status, 0, "{what}");
    }
}

/// Connects under an idle limit of one second, asks for 24 copies of a
/// 1 MiB value, more than the sockets' buffers hold, so that firn's sends
/// wait while the replies stay unread for 1.5 seconds, then reads them all
/// and, half a second later, expects an answer to PING.
#[cfg(target_os = "linux")]
fn read_replies_late(port: u16, what: &str) {
    const SIZE: usize = 1 << 20;
    const COPIES: usize = 24;
    let mut stream = connect_when_ready(port);
    let mut set = format!("*3\r\n$3\r\nSET\r\n$3\r\nbig\r\n${SIZE}\r\n").into_bytes();
    set.resize(set.len() + SIZE, b'v');
    set.extend_from_slice(b"\r\n");
    stream.write_all(&set).expect("set a large value");
    expect_replies(&mut stream, b"+OK\r\n", what);
    stream
        .write_all(&resp(&["GET", "big"]).repeat(COPIES))
        .expect("ask for the value's copies");
    std::thread::sleep(Duration::from_millis(1500));
    let header = format!("${SIZE}\r\n");
    let mut replies = vec![0_u8; COPIES * (header.len() + SIZE + 2)];
    stream
        .read_exact(&mut replies)
        .unwrap_or_else(|error| panic!("{what}: the copies: {error}"));
    std::thread::sleep(Duration::from_millis(500));
    stream.write_all(&resp(&["PING"])).expect("send a ping");
    expect_replies(&mut stream, b"+PONG\r\n", what);
}

/// firn answers the value types as Redis does. One pipelined batch pushes,
/// ranges and pops a list until it is empty, which removes its key; adds and
/// removes set members; sets and reads hash fields; adds, rescores and pops
/// sorted-set members; refuses a list command on a string and a string command
/// on a hash; sets ten keys in one MSET, a command of eleven arguments; empties
/// the keyspace, keys of every kind and one with an expiry, with FLUSHALL and
/// an option read up to a zero byte, after refusing an unknown option and two
/// options, and FLUSHDB empties it again; and names an unknown command and a
/// command short of arguments as Redis does. Two inline commands close the
/// batch. The expected bytes are those redis-server 7.0.15 returns for the
/// same bytes.
#[cfg(target_os = "linux")]
#[test]
fn firn_answers_the_value_types_as_redis_does() {
    let program = firn();
    let port = free_port();
    let text = port.to_string();
    let child = program.spawn_on_route(true, &[text.as_bytes(), b"1"]);
    let mut client = connect_when_ready(port);
    client
        .set_read_timeout(Some(Duration::from_secs(20)))
        .expect("bound the client's waits");
    let mut batch = Vec::new();
    for request in [
        vec!["RPUSH", "l", "a", "b", "c"],
        vec!["LPUSH", "l", "z"],
        vec!["LRANGE", "l", "0", "-1"],
        vec!["LRANGE", "l", "1", "-2"],
        vec!["LPOP", "l"],
        vec!["RPOP", "l", "2"],
        vec!["LLEN", "l"],
        vec!["RPOP", "l"],
        vec!["EXISTS", "l"],
        vec!["SADD", "s", "a", "b", "c", "a"],
        vec!["SREM", "s", "a", "q"],
        vec!["SCARD", "s"],
        vec!["HSET", "h", "f", "1", "g", "2"],
        vec!["HSET", "h", "f", "3"],
        vec!["HGET", "h", "f"],
        vec!["HGET", "h", "nope"],
        vec!["ZADD", "z", "3", "c", "1", "a", "2", "b"],
        vec!["ZADD", "z", "0", "c"],
        vec!["ZSCORE", "z", "c"],
        vec!["ZPOPMIN", "z", "2"],
        vec!["ZCARD", "z"],
        vec!["SET", "str", "v"],
        vec!["LPUSH", "str", "x"],
        vec!["SADD", "l", "x"],
        vec!["TYPE", "l"],
        vec!["GET", "h"],
        vec![
            "MSET", "k1", "1", "k2", "2", "k3", "3", "k4", "4", "k5", "5",
        ],
        vec!["GET", "k5"],
        vec!["SET", "brief", "v", "PX", "60000"],
        vec!["DBSIZE"],
        vec!["FLUSHALL", "x"],
        vec!["FLUSHDB", "SYNC", "ASYNC"],
        vec!["FLUSHALL", "async\0x"],
        vec!["DBSIZE"],
        vec!["EXISTS", "l", "s", "h", "z", "str", "k1", "brief"],
        vec!["TTL", "brief"],
        vec!["FLUSHDB"],
        vec!["NOPE", "a", "b"],
        vec!["LLEN"],
    ] {
        batch.extend(resp(&request));
    }
    batch.extend_from_slice(b"SET inline yes\r\nGET inline\r\n");
    client.write_all(&batch).expect("send the batch");
    expect_replies(
        &mut client,
        b":3\r\n:4\r\n*4\r\n$1\r\nz\r\n$1\r\na\r\n$1\r\nb\r\n$1\r\nc\r\n*2\r\n$1\r\na\r\n$1\r\nb\r\n$1\r\nz\r\n*2\r\n$1\r\nc\r\n$1\r\nb\r\n:1\r\n$1\r\na\r\n:0\r\n:3\r\n:1\r\n:2\r\n:2\r\n:0\r\n$1\r\n3\r\n$-1\r\n:3\r\n:0\r\n$1\r\n0\r\n*4\r\n$1\r\nc\r\n$1\r\n0\r\n$1\r\na\r\n$1\r\n1\r\n:1\r\n+OK\r\n-WRONGTYPE Operation against a key holding the wrong kind of value\r\n:1\r\n+set\r\n-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n$1\r\n5\r\n+OK\r\n:11\r\n-ERR syntax error\r\n-ERR syntax error\r\n+OK\r\n:0\r\n:0\r\n:-2\r\n+OK\r\n-ERR unknown command 'NOPE', with args beginning with: 'a' 'b' \r\n-ERR wrong number of arguments for 'llen' command\r\n+OK\r\n$3\r\nyes\r\n",
        "the value-type batch",
    );
    drop(client);
    let (status, _) = finished(child);
    assert_eq!(status, 0);
}

/// firn answers list updates, moves, searches and set membership, algebra,
/// stores and random draws as Redis does. The expected bytes are those
/// redis-server 7.0.15 returns for the same requests.
#[cfg(target_os = "linux")]
#[test]
fn firn_answers_the_value_types_as_redis_does_with_lists_and_sets() {
    let program = firn();
    let port = free_port();
    let text = port.to_string();
    let child = program.spawn_on_route(true, &[text.as_bytes(), b"1"]);
    let mut client = connect_when_ready(port);
    client
        .set_read_timeout(Some(Duration::from_secs(20)))
        .expect("bound the client's waits");
    let mut batch = Vec::new();
    for request in [
        vec!["RPUSH", "li", "a", "b", "c", "b", "a"],
        vec!["LINDEX", "li", "-2"],
        vec!["LINDEX", "li", "9"],
        vec!["LINDEX", "li", "x"],
        vec!["LINDEX", "noli", "x"],
        vec!["LPOS", "li", "b"],
        vec!["LPOS", "li", "a", "RANK", "-1", "COUNT", "0"],
        vec!["LPOS", "li", "a", "RANK", "0"],
        vec!["LPOS", "li", "a", "COUNT"],
        vec!["RPUSH", "ls", "a", "b", "c"],
        vec!["LSET", "ls", "-1", "C"],
        vec!["LSET", "ls", "9", "x"],
        vec!["LSET", "nols", "0", "x"],
        vec!["LRANGE", "ls", "0", "-1"],
        vec!["RPUSH", "lr", "a", "b", "a", "b", "a"],
        vec!["LREM", "lr", "-2", "a"],
        vec!["LRANGE", "lr", "0", "-1"],
        vec!["RPUSH", "lin", "a", "b"],
        vec!["LINSERT", "lin", "BEFORE", "b", "x"],
        vec!["LINSERT", "lin", "after\x00junk", "b", "y"],
        vec!["LINSERT", "lin", "BEFORE", "zz", "y"],
        vec!["LINSERT", "lin", "MIDDLE", "b", "y"],
        vec!["LRANGE", "lin", "0", "-1"],
        vec!["RPUSH", "lt", "1", "2", "3", "4", "5"],
        vec!["LTRIM", "lt", "1", "-2"],
        vec!["LRANGE", "lt", "0", "-1"],
        vec!["LPUSHX", "nolx", "z"],
        vec!["RPUSH", "lx", "m"],
        vec!["RPUSHX", "lx", "z"],
        vec!["LPUSHX", "lx", "a"],
        vec!["LRANGE", "lx", "0", "-1"],
        vec!["RPUSH", "lp", "a", "b", "c"],
        vec!["LPOP", "lp", "0"],
        vec!["RPOP", "lp", "2"],
        vec!["RPUSH", "lm", "a", "b", "c"],
        vec!["LMOVE", "lm", "lm2", "LEFT", "RIGHT"],
        vec!["LMOVE", "lm", "lm", "RIGHT", "LEFT"],
        vec!["RPOPLPUSH", "lm", "lm2"],
        vec!["LMOVE", "lm", "lm2", "UP", "LEFT"],
        vec!["SET", "mstr", "v"],
        vec!["LMOVE", "lm", "mstr", "LEFT", "LEFT"],
        vec!["LMOVE", "nolm", "mstr", "LEFT", "LEFT"],
        vec!["LRANGE", "lm", "0", "-1"],
        vec!["LRANGE", "lm2", "0", "-1"],
    ] {
        batch.extend(resp(&request));
    }
    client.write_all(&batch).expect("send the list batch");
    expect_replies(
        &mut client,
        b":5\r\n$1\r\nb\r\n$-1\r\n-ERR value is not an integer or out of range\r\n$-1\r\n:1\r\n*2\r\n:4\r\n:0\r\n-ERR RANK can't be zero: use 1 to start from the first match, 2 from the second ... or use negative to start from the end of the list\r\n-ERR syntax error\r\n:3\r\n+OK\r\n-ERR index out of range\r\n-ERR no such key\r\n*3\r\n$1\r\na\r\n$1\r\nb\r\n$1\r\nC\r\n:5\r\n:2\r\n*3\r\n$1\r\na\r\n$1\r\nb\r\n$1\r\nb\r\n:2\r\n:3\r\n:4\r\n:-1\r\n-ERR syntax error\r\n*4\r\n$1\r\na\r\n$1\r\nx\r\n$1\r\nb\r\n$1\r\ny\r\n:5\r\n+OK\r\n*3\r\n$1\r\n2\r\n$1\r\n3\r\n$1\r\n4\r\n:0\r\n:1\r\n:2\r\n:3\r\n*3\r\n$1\r\na\r\n$1\r\nm\r\n$1\r\nz\r\n:3\r\n*0\r\n*2\r\n$1\r\nc\r\n$1\r\nb\r\n:3\r\n$1\r\na\r\n$1\r\nc\r\n$1\r\nb\r\n-ERR syntax error\r\n+OK\r\n-WRONGTYPE Operation against a key holding the wrong kind of value\r\n$-1\r\n*1\r\n$1\r\nc\r\n*2\r\n$1\r\nb\r\n$1\r\na\r\n",
        "the list batch",
    );
    let mut batch = Vec::new();
    for request in [
        vec!["SADD", "sm", "a", "b"],
        vec!["SISMEMBER", "sm", "a"],
        vec!["SMISMEMBER", "sm", "a", "z"],
        vec!["SMISMEMBER", "nosm", "a", "b"],
        vec!["SMEMBERS", "nosm"],
        vec!["SRANDMEMBER", "sm", "0"],
        vec!["SRANDMEMBER", "sm", "1", "2"],
        vec!["SRANDMEMBER", "sm", "-9223372036854775808"],
        vec!["SPOP", "sm", "1", "2"],
        vec!["SCARD", "sm"],
        vec!["SADD", "sc1", "a", "b", "c", "d"],
        vec!["SADD", "sc2", "c", "d", "e"],
        vec!["SINTERCARD", "2", "sc1", "sc2"],
        vec!["SINTERCARD", "2", "sc1", "sc2", "LIMIT", "1"],
        vec!["SINTERCARD", "0", "sc1"],
        vec!["SINTERCARD", "3", "sc1", "sc2"],
        vec!["SINTERSTORE", "si", "sc1", "sc2"],
        vec!["SUNIONSTORE", "su", "sc1", "sc2"],
        vec!["SDIFFSTORE", "sd", "sc1", "sc2"],
        vec!["SDIFF", "sc2", "sc1"],
        vec!["SINTER", "sc1", "nosuch"],
        vec!["SUNION", "nosuch"],
        vec!["SINTERSTORE", "si", "sc1", "nosuch"],
        vec!["EXISTS", "si"],
        vec!["SADD", "sv1", "e", "f"],
        vec!["SMOVE", "sv1", "sv2", "e"],
        vec!["SMEMBERS", "sv2"],
        vec!["SMOVE", "sv1", "sv2", "zz"],
        vec!["SMEMBERS", "sv1"],
        vec!["SET", "sstr", "v"],
        vec!["SINTER", "sc1", "sstr"],
        vec!["SMOVE", "nosuch", "sstr", "a"],
        vec!["SUNIONSTORE", "sstr", "sv2"],
        vec!["TYPE", "sstr"],
    ] {
        batch.extend(resp(&request));
    }
    client.write_all(&batch).expect("send the set batch");
    expect_replies(
        &mut client,
        b":2\r\n:1\r\n*2\r\n:1\r\n:0\r\n*2\r\n:0\r\n:0\r\n*0\r\n*0\r\n-ERR syntax error\r\n-ERR value is out of range, value must between -9223372036854775807 and 9223372036854775807\r\n-ERR syntax error\r\n:2\r\n:4\r\n:3\r\n:2\r\n:1\r\n-ERR numkeys should be greater than 0\r\n-ERR Number of keys can't be greater than number of args\r\n:2\r\n:5\r\n:2\r\n*1\r\n$1\r\ne\r\n*0\r\n*0\r\n:0\r\n:0\r\n:2\r\n:1\r\n*1\r\n$1\r\ne\r\n:0\r\n*1\r\n$1\r\nf\r\n+OK\r\n-WRONGTYPE Operation against a key holding the wrong kind of value\r\n:0\r\n:1\r\n+set\r\n",
        "the set batch",
    );
    // Draws from 60 members: 20 and 59 distinct ones, the two ways firn
    // chooses them, and 90 that must repeat some.
    let members = (0..60)
        .map(|index| format!("m{index:02}"))
        .collect::<Vec<_>>();
    let mut add = vec!["SADD", "mr"];
    add.extend(members.iter().map(String::as_str));
    client.write_all(&resp(&add)).expect("add 60 members");
    expect_replies(&mut client, b":60\r\n", "the members to draw from");
    for (count, distinct) in [(20_i64, true), (59, true), (-90, false)] {
        let what = format!("SRANDMEMBER mr {count}");
        client
            .write_all(&resp(&["SRANDMEMBER", "mr", &count.to_string()]))
            .expect("draw members");
        let length = count.unsigned_abs() as usize;
        assert_eq!(reply_line(&mut client, &what), format!("*{length}\r\n"));
        let mut drawn = Vec::new();
        for _ in 0..length {
            assert_eq!(reply_line(&mut client, &what), "$3\r\n");
            let member = reply_line(&mut client, &what).trim_end().to_owned();
            assert!(members.contains(&member), "{what}: {member}");
            drawn.push(member);
        }
        if distinct {
            drawn.sort();
            drawn.dedup();
            assert_eq!(drawn.len(), length, "{what}: a member repeated");
        }
    }
    drop(client);
    let (status, _) = finished(child);
    assert_eq!(status, 0);
}

/// firn copies and renames value types as Redis does: COPY duplicates a set,
/// hash, sorted set and list, and changing each copy leaves its source intact.
/// REPLACE puts a list in place of a set, and RENAME moves a hash over a list.
/// Further batches cover list updates and moves, set algebra and random draws.
/// The expected bytes are those redis-server 7.0.15 returns for the same requests.
#[cfg(target_os = "linux")]
#[test]
fn firn_copies_and_renames_the_value_types_as_redis_does() {
    let program = firn();
    let port = free_port();
    let text = port.to_string();
    let child = program.spawn_on_route(true, &[text.as_bytes(), b"1"]);
    let mut client = connect_when_ready(port);
    client
        .set_read_timeout(Some(Duration::from_secs(20)))
        .expect("bound the client's waits");
    // COPY needs the final set, hash and sorted-set values from the base case.
    let mut batch = Vec::new();
    for request in [
        vec!["SADD", "s", "b", "c"],
        vec!["HSET", "h", "f", "3", "g", "2"],
        vec!["ZADD", "z", "2", "b"],
    ] {
        batch.extend(resp(&request));
    }
    client.write_all(&batch).expect("seed the values to copy");
    expect_replies(&mut client, b":2\r\n:2\r\n:1\r\n", "the copy sources");
    let mut batch = Vec::new();
    for request in [
        vec!["COPY", "s", "s2"],
        vec!["SADD", "s2", "x"],
        vec!["SCARD", "s"],
        vec!["SCARD", "s2"],
        vec!["COPY", "h", "h2"],
        vec!["HSET", "h2", "f", "9"],
        vec!["HGET", "h", "f"],
        vec!["HGET", "h2", "f"],
        vec!["COPY", "z", "z2"],
        vec!["ZADD", "z2", "5", "q"],
        vec!["ZCARD", "z"],
        vec!["ZPOPMIN", "z2", "2"],
        vec!["RPUSH", "lst", "a", "b", "c"],
        vec!["COPY", "lst", "lst2"],
        vec!["RPOP", "lst2"],
        vec!["LRANGE", "lst", "0", "-1"],
        vec!["COPY", "lst", "s", "REPLACE"],
        vec!["TYPE", "s"],
        vec!["RENAME", "h", "lst"],
        vec!["TYPE", "lst"],
        vec!["EXISTS", "h"],
    ] {
        batch.extend(resp(&request));
    }
    client
        .write_all(&batch)
        .expect("send the copy and rename batch");
    expect_replies(
        &mut client,
        b":1\r\n:1\r\n:2\r\n:3\r\n:1\r\n:0\r\n$1\r\n3\r\n$1\r\n9\r\n:1\r\n:1\r\n:1\r\n*4\r\n$1\r\nb\r\n$1\r\n2\r\n$1\r\nq\r\n$1\r\n5\r\n:3\r\n:1\r\n$1\r\nc\r\n*3\r\n$1\r\na\r\n$1\r\nb\r\n$1\r\nc\r\n:1\r\n+list\r\n+OK\r\n+hash\r\n:0\r\n",
        "the copy and rename batch",
    );
    let mut batch = Vec::new();
    for request in [
        vec!["RPUSH", "li", "a", "b", "c", "b", "a"],
        vec!["LINDEX", "li", "-2"],
        vec!["LINDEX", "li", "9"],
        vec!["LINDEX", "li", "x"],
        vec!["LINDEX", "noli", "x"],
        vec!["LPOS", "li", "b"],
        vec!["LPOS", "li", "a", "RANK", "-1", "COUNT", "0"],
        vec!["LPOS", "li", "a", "RANK", "0"],
        vec!["LPOS", "li", "a", "COUNT"],
        vec!["RPUSH", "ls", "a", "b", "c"],
        vec!["LSET", "ls", "-1", "C"],
        vec!["LSET", "ls", "9", "x"],
        vec!["LSET", "nols", "0", "x"],
        vec!["LRANGE", "ls", "0", "-1"],
        vec!["RPUSH", "lr", "a", "b", "a", "b", "a"],
        vec!["LREM", "lr", "-2", "a"],
        vec!["LRANGE", "lr", "0", "-1"],
        vec!["RPUSH", "lin", "a", "b"],
        vec!["LINSERT", "lin", "BEFORE", "b", "x"],
        vec!["LINSERT", "lin", "after\x00junk", "b", "y"],
        vec!["LINSERT", "lin", "BEFORE", "zz", "y"],
        vec!["LINSERT", "lin", "MIDDLE", "b", "y"],
        vec!["LRANGE", "lin", "0", "-1"],
        vec!["RPUSH", "lt", "1", "2", "3", "4", "5"],
        vec!["LTRIM", "lt", "1", "-2"],
        vec!["LRANGE", "lt", "0", "-1"],
        vec!["LPUSHX", "nolx", "z"],
        vec!["RPUSH", "lx", "m"],
        vec!["RPUSHX", "lx", "z"],
        vec!["LPUSHX", "lx", "a"],
        vec!["LRANGE", "lx", "0", "-1"],
        vec!["RPUSH", "lp", "a", "b", "c"],
        vec!["LPOP", "lp", "0"],
        vec!["RPOP", "lp", "2"],
        vec!["RPUSH", "lm", "a", "b", "c"],
        vec!["LMOVE", "lm", "lm2", "LEFT", "RIGHT"],
        vec!["LMOVE", "lm", "lm", "RIGHT", "LEFT"],
        vec!["RPOPLPUSH", "lm", "lm2"],
        vec!["LMOVE", "lm", "lm2", "UP", "LEFT"],
        vec!["SET", "mstr", "v"],
        vec!["LMOVE", "lm", "mstr", "LEFT", "LEFT"],
        vec!["LMOVE", "nolm", "mstr", "LEFT", "LEFT"],
        vec!["LRANGE", "lm", "0", "-1"],
        vec!["LRANGE", "lm2", "0", "-1"],
    ] {
        batch.extend(resp(&request));
    }
    client.write_all(&batch).expect("send the list batch");
    expect_replies(
        &mut client,
        b":5\r\n$1\r\nb\r\n$-1\r\n-ERR value is not an integer or out of range\r\n$-1\r\n:1\r\n*2\r\n:4\r\n:0\r\n-ERR RANK can't be zero: use 1 to start from the first match, 2 from the second ... or use negative to start from the end of the list\r\n-ERR syntax error\r\n:3\r\n+OK\r\n-ERR index out of range\r\n-ERR no such key\r\n*3\r\n$1\r\na\r\n$1\r\nb\r\n$1\r\nC\r\n:5\r\n:2\r\n*3\r\n$1\r\na\r\n$1\r\nb\r\n$1\r\nb\r\n:2\r\n:3\r\n:4\r\n:-1\r\n-ERR syntax error\r\n*4\r\n$1\r\na\r\n$1\r\nx\r\n$1\r\nb\r\n$1\r\ny\r\n:5\r\n+OK\r\n*3\r\n$1\r\n2\r\n$1\r\n3\r\n$1\r\n4\r\n:0\r\n:1\r\n:2\r\n:3\r\n*3\r\n$1\r\na\r\n$1\r\nm\r\n$1\r\nz\r\n:3\r\n*0\r\n*2\r\n$1\r\nc\r\n$1\r\nb\r\n:3\r\n$1\r\na\r\n$1\r\nc\r\n$1\r\nb\r\n-ERR syntax error\r\n+OK\r\n-WRONGTYPE Operation against a key holding the wrong kind of value\r\n$-1\r\n*1\r\n$1\r\nc\r\n*2\r\n$1\r\nb\r\n$1\r\na\r\n",
        "the list batch",
    );
    let mut batch = Vec::new();
    for request in [
        vec!["SADD", "sm", "a", "b"],
        vec!["SISMEMBER", "sm", "a"],
        vec!["SMISMEMBER", "sm", "a", "z"],
        vec!["SMISMEMBER", "nosm", "a", "b"],
        vec!["SMEMBERS", "nosm"],
        vec!["SRANDMEMBER", "sm", "0"],
        vec!["SRANDMEMBER", "sm", "1", "2"],
        vec!["SRANDMEMBER", "sm", "-9223372036854775808"],
        vec!["SPOP", "sm", "1", "2"],
        vec!["SCARD", "sm"],
        vec!["SADD", "sc1", "a", "b", "c", "d"],
        vec!["SADD", "sc2", "c", "d", "e"],
        vec!["SINTERCARD", "2", "sc1", "sc2"],
        vec!["SINTERCARD", "2", "sc1", "sc2", "LIMIT", "1"],
        vec!["SINTERCARD", "0", "sc1"],
        vec!["SINTERCARD", "3", "sc1", "sc2"],
        vec!["SINTERSTORE", "si", "sc1", "sc2"],
        vec!["SUNIONSTORE", "su", "sc1", "sc2"],
        vec!["SDIFFSTORE", "sd", "sc1", "sc2"],
        vec!["SDIFF", "sc2", "sc1"],
        vec!["SINTER", "sc1", "nosuch"],
        vec!["SUNION", "nosuch"],
        vec!["SINTERSTORE", "si", "sc1", "nosuch"],
        vec!["EXISTS", "si"],
        vec!["SADD", "sv1", "e", "f"],
        vec!["SMOVE", "sv1", "sv2", "e"],
        vec!["SMEMBERS", "sv2"],
        vec!["SMOVE", "sv1", "sv2", "zz"],
        vec!["SMEMBERS", "sv1"],
        vec!["SET", "sstr", "v"],
        vec!["SINTER", "sc1", "sstr"],
        vec!["SMOVE", "nosuch", "sstr", "a"],
        vec!["SUNIONSTORE", "sstr", "sv2"],
        vec!["TYPE", "sstr"],
    ] {
        batch.extend(resp(&request));
    }
    client.write_all(&batch).expect("send the set batch");
    expect_replies(
        &mut client,
        b":2\r\n:1\r\n*2\r\n:1\r\n:0\r\n*2\r\n:0\r\n:0\r\n*0\r\n*0\r\n-ERR syntax error\r\n-ERR value is out of range, value must between -9223372036854775807 and 9223372036854775807\r\n-ERR syntax error\r\n:2\r\n:4\r\n:3\r\n:2\r\n:1\r\n-ERR numkeys should be greater than 0\r\n-ERR Number of keys can't be greater than number of args\r\n:2\r\n:5\r\n:2\r\n*1\r\n$1\r\ne\r\n*0\r\n*0\r\n:0\r\n:0\r\n:2\r\n:1\r\n*1\r\n$1\r\ne\r\n:0\r\n*1\r\n$1\r\nf\r\n+OK\r\n-WRONGTYPE Operation against a key holding the wrong kind of value\r\n:0\r\n:1\r\n+set\r\n",
        "the set batch",
    );
    // Draws from 60 members: 20 and 59 distinct ones, the two ways firn
    // chooses them, and 90 that must repeat some.
    let members = (0..60)
        .map(|index| format!("m{index:02}"))
        .collect::<Vec<_>>();
    let mut add = vec!["SADD", "mr"];
    add.extend(members.iter().map(String::as_str));
    client.write_all(&resp(&add)).expect("add 60 members");
    expect_replies(&mut client, b":60\r\n", "the members to draw from");
    for (count, distinct) in [(20_i64, true), (59, true), (-90, false)] {
        let what = format!("SRANDMEMBER mr {count}");
        client
            .write_all(&resp(&["SRANDMEMBER", "mr", &count.to_string()]))
            .expect("draw members");
        let length = count.unsigned_abs() as usize;
        assert_eq!(reply_line(&mut client, &what), format!("*{length}\r\n"));
        let mut drawn = Vec::new();
        for _ in 0..length {
            assert_eq!(reply_line(&mut client, &what), "$3\r\n");
            let member = reply_line(&mut client, &what).trim_end().to_owned();
            assert!(members.contains(&member), "{what}: {member}");
            drawn.push(member);
        }
        if distinct {
            drawn.sort();
            drawn.dedup();
            assert_eq!(drawn.len(), length, "{what}: a member repeated");
        }
    }
    drop(client);
    let (status, _) = finished(child);
    assert_eq!(status, 0);
}

/// The exact decimal expansion of `m / 2^k`, which has `k` digits after the
/// point: `m * 5^k` with the point placed `k` digits from its end.
#[cfg(target_os = "linux")]
fn dyadic_decimal(m: u64, k: usize) -> String {
    let mut digits: Vec<u8> = m.to_string().bytes().rev().map(|b| b - b'0').collect();
    for _ in 0..k {
        let mut carry = 0;
        for digit in &mut digits {
            let value = *digit * 5 + carry;
            *digit = value % 10;
            carry = value / 10;
        }
        if carry > 0 {
            digits.push(carry);
        }
    }
    digits.resize(digits.len().max(k + 1), 0);
    let text: String = digits
        .iter()
        .rev()
        .map(|digit| char::from(b'0' + digit))
        .collect();
    let (whole, fraction) = text.split_at(text.len() - k);
    format!("{whole}.{fraction}")
}

/// firn reads and writes sorted-set scores as Redis does. Each score is added
/// with `ZADD`, read back with `ZSCORE` and popped with `ZPOPMIN`. Decimal text
/// rounds to the nearest double with ties to even: at the two halfway points
/// past 2^53, at the 752-digit halfway point below the smallest subnormal,
/// which rounds to zero and is refused as Redis refuses an underflow, and at
/// the 768-digit halfway points on either side of the smallest normal double,
/// which round up to it and down to it. A nonzero digit after a halfway point
/// breaks its tie, also when it lies past the 800th significant digit, both
/// after hundreds of digits and after a halfway point of at most 19 digits
/// followed by zeros, and a digit taken away keeps the value below it.
/// Hexadecimal text rounds as decimal text does, a nonzero digit past the bits
/// kept breaking a tie, a subnormal rounding to the smallest one, and a value
/// rounded past the largest double refused, also with an exponent past the
/// range of i64;
/// infinities and arguments of 400 and 1,000 digits are read as strtod reads
/// them; NaN, overflow, a leading or trailing space, an empty argument and a
/// zero byte are refused. Every score is written as %.17g writes it, ties at
/// the seventeenth digit to even, in exponential notation below 10^-4 and from
/// 10^17; negative zero is kept as 0, as Redis keeps it in a sorted set it
/// encodes as a listpack. Members at infinite, subnormal, zero and equal scores
/// pop in Redis's order. An option word before the first score, in either case
/// and compared up to a zero byte as strcasecmp compares it, is a syntax
/// error, while one in a later score's place is an invalid score. The expected
/// replies are redis-server 7.0.15's.
#[cfg(target_os = "linux")]
#[test]
fn firn_reads_and_writes_scores_as_redis_does() {
    let half_smallest = dyadic_decimal(1, 1075);
    let half_smallest_above = format!("{half_smallest}1");
    let half_smallest_far_above = format!("{half_smallest}{}1", "0".repeat(60));
    let below_normal = dyadic_decimal((1 << 53) - 1, 1075);
    let below_normal_below = format!(
        "{}4{}",
        &below_normal[..below_normal.len() - 1],
        "9".repeat(20)
    );
    let above_normal = dyadic_decimal((1 << 53) + 1, 1075);
    let above_normal_far_above = format!("{above_normal}{}1", "0".repeat(40));
    let short_tie_far_above = format!("9007199254740993.{}1", "0".repeat(784));
    let scaled_tie_far_above = format!("5.{}1e22", "0".repeat(799));
    let long_zeros = format!("0.{}1e401", "0".repeat(400));
    let long_thirds = format!("3.{}", "3".repeat(1000));
    let cases: [(&str, Option<&str>); 49] = [
        ("1.5", Some("1.5")),
        ("-2.5e-3", Some("-0.0025000000000000001")),
        ("0.1", Some("0.10000000000000001")),
        ("1e23", Some("9.9999999999999992e+22")),
        ("9007199254740993", Some("9007199254740992")),
        ("9007199254740995", Some("9007199254740996")),
        ("2.2250738585072011e-308", Some("2.2250738585072009e-308")),
        ("4.9e-324", Some("4.9406564584124654e-324")),
        ("2.4703282292062327e-324", None),
        ("2.4703282292062328e-324", Some("4.9406564584124654e-324")),
        (&half_smallest, None),
        (&half_smallest_above, Some("4.9406564584124654e-324")),
        (&half_smallest_far_above, Some("4.9406564584124654e-324")),
        (&below_normal, Some("2.2250738585072014e-308")),
        (&below_normal_below, Some("2.2250738585072009e-308")),
        (&above_normal, Some("2.2250738585072014e-308")),
        (&above_normal_far_above, Some("2.2250738585072019e-308")),
        (&short_tie_far_above, Some("9007199254740994")),
        (&scaled_tie_far_above, Some("5.0000000000000004e+22")),
        ("1.7976931348623158e308", Some("1.7976931348623157e+308")),
        ("1.7976931348623159e308", None),
        ("1e-400", None),
        (&long_zeros, Some("1")),
        (&long_thirds, Some("3.3333333333333335")),
        ("inf", Some("inf")),
        ("-Infinity", Some("-inf")),
        ("nan", None),
        ("infinit", None),
        ("0x1.8p1", Some("3")),
        ("0x1.00000000000008p0", Some("1")),
        ("0x1.000000000000080001p0", Some("1.0000000000000002")),
        ("0x1.fffffffffffff8p1023", None),
        ("0x1.fffffffffffff8p99999999999999999999", None),
        ("0x3p-1076", Some("4.9406564584124654e-324")),
        ("0x1p-1075", None),
        ("1125899906842623.75", Some("1125899906842623.8")),
        ("1125899906842623.25", Some("1125899906842623.2")),
        ("0.0001", Some("0.0001")),
        ("0.00001", Some("1.0000000000000001e-05")),
        ("99999999999999984", Some("99999999999999984")),
        ("1e17", Some("1e+17")),
        ("-0", Some("0")),
        (" 1", None),
        ("1 ", None),
        ("1e", None),
        ("", None),
        ("1\0", None),
        ("+.5", Some("0.5")),
        ("007", Some("7")),
    ];
    let program = firn();
    let port = free_port();
    let text = port.to_string();
    let child = program.spawn_on_route(true, &[text.as_bytes(), b"1"]);
    let mut client = connect_when_ready(port);
    client
        .set_read_timeout(Some(Duration::from_secs(20)))
        .expect("bound the client's waits");
    let mut batch = Vec::new();
    let mut expected = Vec::new();
    for (score, written) in cases {
        batch.extend(resp(&["ZADD", "z", score, "m"]));
        batch.extend(resp(&["ZSCORE", "z", "m"]));
        batch.extend(resp(&["ZPOPMIN", "z"]));
        match written {
            Some(written) => expected.extend(
                format!(
                    ":1\r\n${length}\r\n{written}\r\n*2\r\n$1\r\nm\r\n${length}\r\n{written}\r\n",
                    length = written.len()
                )
                .bytes(),
            ),
            None => expected.extend_from_slice(b"-ERR value is not a valid float\r\n$-1\r\n*0\r\n"),
        }
    }
    batch.extend(resp(&[
        "ZADD", "o", "inf", "top", "-inf", "bottom", "1.5", "b", "1.5", "a", "0", "zero", "-0",
        "negative", "4.9e-324", "tiny", "-1e308", "low", "2.5e-3", "small",
    ]));
    batch.extend(resp(&["ZPOPMIN", "o", "20"]));
    let popped = [
        "bottom",
        "-inf",
        "low",
        "-1e+308",
        "negative",
        "0",
        "zero",
        "0",
        "tiny",
        "4.9406564584124654e-324",
        "small",
        "0.0025000000000000001",
        "a",
        "1.5",
        "b",
        "1.5",
        "top",
        "inf",
    ];
    expected.extend(format!(":9\r\n*{}\r\n", popped.len()).bytes());
    for item in popped {
        expected.extend(format!("${}\r\n{item}\r\n", item.len()).bytes());
    }
    batch.extend(resp(&["ZADD", "q", "nx", "m"]));
    batch.extend(resp(&["ZADD", "q", "Ch", "m"]));
    batch.extend(resp(&["ZADD", "q", "gt\0x", "m"]));
    batch.extend(resp(&["ZADD", "q", "1", "a", "nx", "b"]));
    batch.extend(resp(&["ZCARD", "q"]));
    expected.extend_from_slice(
        b"-ERR syntax error\r\n-ERR syntax error\r\n-ERR syntax error\r\n-ERR value is not a valid float\r\n:0\r\n",
    );
    client.write_all(&batch).expect("send the batch");
    expect_replies(&mut client, &expected, "the score batch");
    drop(client);
    let (status, _) = finished(child);
    assert_eq!(status, 0);
}

/// firn keeps strings of at most 24 bytes inside the keyspace and longer ones
/// in allocations of their own. Keys, members, fields and values of exactly 24
/// bytes and of 25 bytes sharing those 24, and a key differing from them only
/// in its last byte, stay distinct in every kind of value, and a number that
/// `INCR` rewrites to a different length reads back whole. A string `APPEND`
/// or `SETRANGE` grows from 23 or 24 bytes past 24 reads back whole, as does
/// one `SETRANGE` creates on either side of the limit, its gap filled with
/// zero bytes, and `GETRANGE` and `STRLEN` read either form; the expected
/// replies are redis-server 7.0.15's to the same requests.
#[cfg(target_os = "linux")]
#[test]
fn firn_keeps_strings_at_and_past_the_inline_length_apart() {
    let program = firn();
    let port = free_port();
    let text = port.to_string();
    let child = program.spawn_on_route(true, &[text.as_bytes(), b"1"]);
    let mut client = connect_when_ready(port);
    client
        .set_read_timeout(Some(Duration::from_secs(20)))
        .expect("bound the client's waits");
    let at = format!("{}x", "a".repeat(23));
    let past = format!("{at}y");
    let other = format!("{}z", "a".repeat(23));
    let (a, b, c) = (at.as_str(), past.as_str(), other.as_str());
    let short_value = "v".repeat(24);
    let long_value = "v".repeat(25);
    let (v24, v25) = (short_value.as_str(), long_value.as_str());
    let short_run = "a".repeat(23);
    let a23 = short_run.as_str();
    let wide = format!("1{}", "0".repeat(25));
    let mut batch = Vec::new();
    for request in [
        vec!["SET", a, "1"],
        vec!["SET", b, "2"],
        vec!["SET", c, "3"],
        vec!["GET", a],
        vec!["GET", b],
        vec!["GET", c],
        vec!["INCR", a],
        vec!["INCR", b],
        vec!["GET", a],
        vec!["GET", b],
        vec!["SET", "k", v24],
        vec!["GET", "k"],
        vec!["SET", "k", v25],
        vec!["GET", "k"],
        vec!["SET", "k", v24],
        vec!["GET", "k"],
        vec!["SET", "m", "9223372036854775806"],
        vec!["INCR", "m"],
        vec!["INCR", "m"],
        vec!["GET", "m"],
        vec!["SET", "neg", "-9223372036854775807"],
        vec!["INCR", "neg"],
        vec!["GET", "neg"],
        vec!["SET", "wide", wide.as_str()],
        vec!["INCR", "wide"],
        vec!["SADD", "s", a, b, c, a],
        vec!["SCARD", "s"],
        vec!["SREM", "s", a],
        vec!["SCARD", "s"],
        vec!["SREM", "s", a, b],
        vec!["SCARD", "s"],
        vec!["HSET", "h", a, v25, b, v24],
        vec!["HGET", "h", a],
        vec!["HGET", "h", b],
        vec!["HGET", "h", c],
        vec!["ZADD", "z", "1", a, "2", b, "3", c],
        vec!["ZSCORE", "z", b],
        vec!["ZADD", "z", "0", b],
        vec!["ZPOPMIN", "z", "2"],
        vec!["LPUSH", "l", a, b],
        vec!["LRANGE", "l", "0", "-1"],
        vec!["EXISTS", a, b, c],
        vec!["DEL", a, b],
        vec!["EXISTS", a, b, c],
        vec!["MSET", a, v25, b, v24],
        vec!["GET", a],
        vec!["GET", b],
        vec!["SET", "p", a23],
        vec!["APPEND", "p", "b"],
        vec!["APPEND", "p", "c"],
        vec!["GET", "p"],
        vec!["GETRANGE", "p", "22", "24"],
        vec!["STRLEN", "p"],
        vec!["SET", "q", v24],
        vec!["SETRANGE", "q", "24", "w"],
        vec!["GET", "q"],
        vec!["SETRANGE", "q", "0", "W"],
        vec!["GET", "q"],
        vec!["STRLEN", "q"],
        vec!["SETRANGE", "q", "30", "Z"],
        vec!["GET", "q"],
        vec!["APPEND", "q", "d"],
        vec!["SETRANGE", "r", "30", "end"],
        vec!["GET", "r"],
        vec!["SETRANGE", "gap", "3", "x"],
        vec!["GET", "gap"],
        vec!["GETRANGE", "gap", "-1", "-1"],
    ] {
        batch.extend(resp(&request));
    }
    client.write_all(&batch).expect("send the batch");
    expect_replies(
        &mut client,
        b"+OK\r\n+OK\r\n+OK\r\n$1\r\n1\r\n$1\r\n2\r\n$1\r\n3\r\n:2\r\n:3\r\n$1\r\n2\r\n$1\r\n3\r\n+OK\r\n$24\r\nvvvvvvvvvvvvvvvvvvvvvvvv\r\n+OK\r\n$25\r\nvvvvvvvvvvvvvvvvvvvvvvvvv\r\n+OK\r\n$24\r\nvvvvvvvvvvvvvvvvvvvvvvvv\r\n+OK\r\n:9223372036854775807\r\n-ERR increment or decrement would overflow\r\n$19\r\n9223372036854775807\r\n+OK\r\n:-9223372036854775806\r\n$20\r\n-9223372036854775806\r\n+OK\r\n-ERR value is not an integer or out of range\r\n:3\r\n:3\r\n:1\r\n:2\r\n:1\r\n:1\r\n:2\r\n$25\r\nvvvvvvvvvvvvvvvvvvvvvvvvv\r\n$24\r\nvvvvvvvvvvvvvvvvvvvvvvvv\r\n$-1\r\n:3\r\n$1\r\n2\r\n:0\r\n*4\r\n$25\r\naaaaaaaaaaaaaaaaaaaaaaaxy\r\n$1\r\n0\r\n$24\r\naaaaaaaaaaaaaaaaaaaaaaax\r\n$1\r\n1\r\n:2\r\n*2\r\n$25\r\naaaaaaaaaaaaaaaaaaaaaaaxy\r\n$24\r\naaaaaaaaaaaaaaaaaaaaaaax\r\n:3\r\n:2\r\n:1\r\n+OK\r\n$25\r\nvvvvvvvvvvvvvvvvvvvvvvvvv\r\n$24\r\nvvvvvvvvvvvvvvvvvvvvvvvv\r\n+OK\r\n:24\r\n:25\r\n$25\r\naaaaaaaaaaaaaaaaaaaaaaabc\r\n$3\r\nabc\r\n:25\r\n+OK\r\n:25\r\n$25\r\nvvvvvvvvvvvvvvvvvvvvvvvvw\r\n:25\r\n$25\r\nWvvvvvvvvvvvvvvvvvvvvvvvw\r\n:25\r\n:31\r\n$31\r\nWvvvvvvvvvvvvvvvvvvvvvvvw\x00\x00\x00\x00\x00Z\r\n:32\r\n:33\r\n$33\r\n\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00end\r\n:4\r\n$4\r\n\x00\x00\x00x\r\n$1\r\nx\r\n",
        "the strings around the inline length",
    );
    drop(client);
    let (status, _) = finished(child);
    assert_eq!(status, 0);
}

/// firn answers the string commands as Redis does. `SET` reads its options as
/// Redis 7.0.15 reads them: NX and XX store only in place of an absent key or
/// over a live one; GET answers the string the key held, or nil, and refuses a
/// key of another kind, storing nothing; KEEPTTL keeps the key's expiry where a
/// plain `SET` removes it; a later EX replaces an earlier one; a word is
/// compared in either case up to a zero byte; an expiry option needs an amount;
/// an option beside one it excludes, or one only `GETEX` takes, is a syntax
/// error; and an absolute expiry already past leaves the key expired at once.
/// `SETNX`, `SETEX`, `PSETEX`, `GETSET` and `GETDEL` answer as Redis does, and
/// `GETEX` reads its amount only once it has found a live string, and removes
/// the key for an absolute expiry already past, which `DBSIZE` then no longer
/// counts. `APPEND` of nothing creates an empty string; `GETRANGE` and
/// `SUBSTR` count negative indices from the end, clamp both to the string and
/// answer the empty string for an absent key or a range ending before it
/// starts; `SETRANGE` of nothing changes nothing, refuses a negative offset,
/// and refuses a string past 512 MiB, its offset and length summed without
/// overflow; each refuses another kind. `INCRBY`, `DECR` and `DECRBY` add
/// with Redis's overflow checks, `DECRBY` refusing the least i64 before it
/// reads the key, and keep the key's expiry. `INCRBYFLOAT` adds in x87 long
/// double arithmetic and writes the sum as Redis's `%.17Lf` with its trailing
/// zeros dropped: 10.5 plus 0.1 is 10.6, where doubles would give
/// 10.59999999999999964; hexadecimal text reads; text halfway between two
/// long doubles reads to the even one both ways, unless a digit far after the
/// halfway point breaks the tie; a tie at the seventeenth digit after the
/// point writes to even both ways; a subnormal sum, and negative zero, write
/// 0; a sum near the largest double writes all 309 digits, and one past the
/// largest long double is refused, as are an infinity, NaN, a leading space,
/// a zero byte and a value that is not a number, after a key of another kind.
/// The expected replies are redis-server 7.0.15's to the same requests.
#[cfg(target_os = "linux")]
#[test]
fn firn_answers_string_commands_as_redis_does() {
    let program = firn();
    let port = free_port();
    let text = port.to_string();
    let child = program.spawn_on_route(true, &[text.as_bytes(), b"1"]);
    let mut client = connect_when_ready(port);
    client
        .set_read_timeout(Some(Duration::from_secs(20)))
        .expect("bound the client's waits");
    let mut batch = Vec::new();
    for request in [
        vec!["SET", "k", "v", "nx\0zz"],
        vec!["SET", "k", "v2", "xx", "GET"],
        vec!["SET", "k", "v3", "EX", "10", "EX", "20"],
        vec!["TTL", "k"],
        vec!["SET", "k", "v4", "NX", "XX"],
        vec!["SET", "k", "v4", "KEEPTTL", "EX", "5"],
        vec!["SET", "k", "v4", "EX", "5", "KEEPTTL"],
        vec!["SET", "k", "v5", "KEEPTTL"],
        vec!["TTL", "k"],
        vec!["SET", "k", "v6"],
        vec!["TTL", "k"],
        vec!["SET", "k", "v7", "EX"],
        vec!["SET", "k", "v7", "EX", "NX"],
        vec!["SET", "k", "v7", "EX", "0"],
        vec!["SET", "k", "v8", "GET", "GET"],
        vec!["LPUSH", "l", "a"],
        vec!["SET", "l", "x", "GET"],
        vec!["SET", "l", "x", "NX", "GET"],
        vec!["TYPE", "l"],
        vec!["SET", "n", "x", "NX", "GET"],
        vec!["SET", "n", "y", "NX", "GET"],
        vec!["SET", "absent", "y", "XX", "GET"],
        vec!["SET", "absent", "y", "XX"],
        vec!["EXISTS", "absent"],
        vec!["SET", "k", "v", "keepttl\0x"],
        vec!["SET", "k", "v", "GET", "PERSIST"],
        vec!["SET", "k", "v", ""],
        vec!["SET", "k", "v", "PXAT", "1"],
        vec!["GET", "k"],
        vec!["SET", "l", "x"],
        vec!["GET", "l"],
        vec!["SETNX", "k", "x"],
        vec!["SETNX", "k", "y"],
        vec!["SETEX", "s", "100", "v"],
        vec!["TTL", "s"],
        vec!["SETEX", "s", "0", "v"],
        vec!["SETEX", "s", "abc", "v"],
        vec!["PSETEX", "s", "-5", "v"],
        vec!["PSETEX", "s", "100000", "v"],
        vec!["TTL", "s"],
        vec!["GETSET", "s", "w"],
        vec!["TTL", "s"],
        vec!["GETSET", "nope", "w"],
        vec!["LPUSH", "l2", "a"],
        vec!["GETSET", "l2", "w"],
        vec!["GETDEL", "l2"],
        vec!["GETDEL", "nope"],
        vec!["GETDEL", "nope"],
        vec!["SET", "g", "val"],
        vec!["GETEX", "g"],
        vec!["GETEX", "g", "EX", "100"],
        vec!["TTL", "g"],
        vec!["GETEX", "g", "PERSIST"],
        vec!["TTL", "g"],
        vec!["GETEX", "g", "NX"],
        vec!["GETEX", "g", "EX", "abc"],
        vec!["GETEX", "missing", "EX", "abc"],
        vec!["GETEX", "l2", "EX", "abc"],
        vec!["GETEX", "g", "EX", "0"],
        vec!["GETEX", "g", "EX", "10", "PERSIST"],
        vec!["GETEX", "g", "PXAT", "1"],
        vec!["DBSIZE"],
        vec!["GETEX", "g"],
        vec!["SET", "r", "hello"],
        vec!["APPEND", "l2", "x"],
        vec!["APPEND", "newa", ""],
        vec!["EXISTS", "newa"],
        vec!["STRLEN", "nope"],
        vec!["STRLEN", "l2"],
        vec!["GETRANGE", "r", "0", "4"],
        vec!["GETRANGE", "r", "-3", "-1"],
        vec!["GETRANGE", "r", "-1", "-5"],
        vec!["GETRANGE", "r", "3", "1"],
        vec!["GETRANGE", "r", "0", "-100"],
        vec!["GETRANGE", "r", "-100", "100"],
        vec!["GETRANGE", "r", "100", "200"],
        vec!["GETRANGE", "r", "x", "1"],
        vec!["GETRANGE", "r", "1", "x"],
        vec!["GETRANGE", "nope", "0", "1"],
        vec!["GETRANGE", "nope", "x", "1"],
        vec!["GETRANGE", "l2", "0", "1"],
        vec![
            "GETRANGE",
            "r",
            "-9223372036854775808",
            "9223372036854775807",
        ],
        vec!["GETRANGE", "r", "-10", "-20"],
        vec!["GETRANGE", "r", "-20", "-10"],
        vec!["SUBSTR", "r", "1", "3"],
        vec!["SETRANGE", "r", "-1", "x"],
        vec!["SETRANGE", "r", "x", "x"],
        vec!["SETRANGE", "nope", "5", ""],
        vec!["EXISTS", "nope"],
        vec!["SETRANGE", "r", "99999999999", ""],
        vec!["SETRANGE", "nope", "536870911", "ab"],
        vec!["SETRANGE", "nope", "9223372036854775807", "ab"],
        vec!["SETRANGE", "r", "536870911", "ab"],
        vec!["SETRANGE", "l2", "0", ""],
        vec!["GET", "r"],
        vec!["INCRBY", "i", "5"],
        vec!["INCRBY", "i", "x"],
        vec!["INCRBY", "i", "-10"],
        vec!["DECR", "i"],
        vec!["DECR", "fresh"],
        vec!["DECRBY", "i", "-9223372036854775808"],
        vec!["DECRBY", "i", "9223372036854775802"],
        vec!["DECRBY", "i", "1"],
        vec!["INCRBY", "i", "-1"],
        vec!["INCRBY", "i", "9223372036854775807"],
        vec!["INCRBY", "l2", "1"],
        vec!["DECRBY", "l2", "-9223372036854775808"],
        vec!["DECR", "l2"],
        vec!["SET", "t", "5", "PXAT", "99999999999999"],
        vec!["INCRBY", "t", "2"],
        vec!["DECRBY", "t", "3"],
        vec!["PEXPIRETIME", "t"],
        vec!["INCRBY", "i", " 1"],
        vec!["SET", "fl", "10.5"],
        vec!["INCRBYFLOAT", "fl", "0.1"],
        vec!["INCRBYFLOAT", "fl", "-10.6"],
        vec!["SET", "fl2", "3.0e3"],
        vec!["INCRBYFLOAT", "fl2", "1.0e3"],
        vec!["SET", "fl3", "0x1p3"],
        vec!["INCRBYFLOAT", "fl3", "0x1.8p1"],
        vec!["SET", "fl4", "0.000003814697265625"],
        vec!["INCRBYFLOAT", "fl4", "0"],
        vec!["SET", "fl5", "0.000011444091796875"],
        vec!["INCRBYFLOAT", "fl5", "0"],
        vec!["INCRBYFLOAT", "nofl", "1e-4940"],
        vec!["GET", "nofl"],
        vec!["SET", "neg", "-0.0"],
        vec!["INCRBYFLOAT", "neg", "0"],
        vec!["INCRBYFLOAT", "fl", "inf"],
        vec!["INCRBYFLOAT", "fl", "nan"],
        vec!["INCRBYFLOAT", "fl", " 1"],
        vec!["INCRBYFLOAT", "fl", "1.5\0abc"],
        vec!["SET", "bad", "abc"],
        vec!["INCRBYFLOAT", "bad", "1"],
        vec!["INCRBYFLOAT", "l2", "1"],
        vec!["INCRBYFLOAT", "l2", "abc"],
        vec!["SET", "huge", "1.18973149535723176502e4932"],
        vec!["INCRBYFLOAT", "huge", "1.18973149535723176502e4932"],
        vec!["SET", "fe", "5", "PXAT", "99999999999999"],
        vec!["INCRBYFLOAT", "fe", "1.5"],
        vec!["PEXPIRETIME", "fe"],
        vec!["SET", "big", "15.5"],
        vec!["INCRBYFLOAT", "big", "1.7976931348623157e308"],
        vec!["INCRBYFLOAT", "tie1", "18446744073709551617"],
        vec!["INCRBYFLOAT", "tie2", "18446744073709551619"],
        vec![
            "INCRBYFLOAT",
            "tie3",
            "18446744073709551617.0000000000000000000000001",
        ],
    ] {
        batch.extend(resp(&request));
    }
    client.write_all(&batch).expect("send the string batch");
    expect_replies(
        &mut client,
        b"+OK\r\n$1\r\nv\r\n+OK\r\n:20\r\n-ERR syntax error\r\n-ERR syntax error\r\n-ERR syntax error\r\n+OK\r\n:20\r\n+OK\r\n:-1\r\n-ERR syntax error\r\n-ERR value is not an integer or out of range\r\n-ERR invalid expire time in 'set' command\r\n$2\r\nv6\r\n:1\r\n-WRONGTYPE Operation against a key holding the wrong kind of value\r\n-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+list\r\n$-1\r\n$1\r\nx\r\n$-1\r\n$-1\r\n:0\r\n+OK\r\n-ERR syntax error\r\n-ERR syntax error\r\n+OK\r\n$-1\r\n+OK\r\n$1\r\nx\r\n:1\r\n:0\r\n+OK\r\n:100\r\n-ERR invalid expire time in 'setex' command\r\n-ERR value is not an integer or out of range\r\n-ERR invalid expire time in 'psetex' command\r\n+OK\r\n:100\r\n$1\r\nv\r\n:-1\r\n$-1\r\n:1\r\n-WRONGTYPE Operation against a key holding the wrong kind of value\r\n-WRONGTYPE Operation against a key holding the wrong kind of value\r\n$1\r\nw\r\n$-1\r\n+OK\r\n$3\r\nval\r\n$3\r\nval\r\n:100\r\n$3\r\nval\r\n:-1\r\n-ERR syntax error\r\n-ERR value is not an integer or out of range\r\n$-1\r\n-WRONGTYPE Operation against a key holding the wrong kind of value\r\n-ERR invalid expire time in 'getex' command\r\n-ERR syntax error\r\n$3\r\nval\r\n:5\r\n$-1\r\n+OK\r\n-WRONGTYPE Operation against a key holding the wrong kind of value\r\n:0\r\n:1\r\n:0\r\n-WRONGTYPE Operation against a key holding the wrong kind of value\r\n$5\r\nhello\r\n$3\r\nllo\r\n$0\r\n\r\n$0\r\n\r\n$1\r\nh\r\n$5\r\nhello\r\n$0\r\n\r\n-ERR value is not an integer or out of range\r\n-ERR value is not an integer or out of range\r\n$0\r\n\r\n-ERR value is not an integer or out of range\r\n-WRONGTYPE Operation against a key holding the wrong kind of value\r\n$5\r\nhello\r\n$0\r\n\r\n$1\r\nh\r\n$3\r\nell\r\n-ERR offset is out of range\r\n-ERR value is not an integer or out of range\r\n:0\r\n:0\r\n:5\r\n-ERR string exceeds maximum allowed size (proto-max-bulk-len)\r\n-ERR string exceeds maximum allowed size (proto-max-bulk-len)\r\n-ERR string exceeds maximum allowed size (proto-max-bulk-len)\r\n-WRONGTYPE Operation against a key holding the wrong kind of value\r\n$5\r\nhello\r\n:5\r\n-ERR value is not an integer or out of range\r\n:-5\r\n:-6\r\n:-1\r\n-ERR decrement would overflow\r\n:-9223372036854775808\r\n-ERR increment or decrement would overflow\r\n-ERR increment or decrement would overflow\r\n:-1\r\n-WRONGTYPE Operation against a key holding the wrong kind of value\r\n-ERR decrement would overflow\r\n-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n:7\r\n:4\r\n:99999999999999\r\n-ERR value is not an integer or out of range\r\n+OK\r\n$4\r\n10.6\r\n$1\r\n0\r\n+OK\r\n$4\r\n4000\r\n+OK\r\n$2\r\n11\r\n+OK\r\n$19\r\n0.00000381469726562\r\n+OK\r\n$19\r\n0.00001144409179688\r\n$1\r\n0\r\n$1\r\n0\r\n+OK\r\n$1\r\n0\r\n-ERR increment would produce NaN or Infinity\r\n-ERR value is not a valid float\r\n-ERR value is not a valid float\r\n-ERR value is not a valid float\r\n+OK\r\n-ERR value is not a valid float\r\n-WRONGTYPE Operation against a key holding the wrong kind of value\r\n-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n-ERR increment would produce NaN or Infinity\r\n+OK\r\n$3\r\n6.5\r\n:99999999999999\r\n+OK\r\n$309\r\n179769313486231569995921046774104434048386446944329178485314420765176628523124396354701868608387085564428793419425520689308708363451136055357897278026477617073498977686288394835618163294045594434929537447290627480669417384648879122776218333598953852832103282504751611691208117720159985926376802466863339536384\r\n$20\r\n18446744073709551616\r\n$20\r\n18446744073709551620\r\n$20\r\n18446744073709551618\r\n",
        "the string batch",
    );
    drop(client);
    let (status, _) = finished(child);
    assert_eq!(status, 0);
}

/// [SHARE-1, SHARE-2] firn answers `DEL`, `EXISTS` and `MSET` naming a key
/// more than once as Redis does, each holding its keys' entries through a key
/// set, which keeps one element per key: `DEL` removes and counts such a key
/// once, `EXISTS` counts a live key once for each time it is named and an
/// absent one not at all, and `MSET` keeps the last value named for a key.
/// `MGET` answers a key named twice twice, in the order named, though its key
/// set holds the keys in byte order, and nil for an absent key or one of
/// another kind; `MSETNX` keeps the last value named for a key and stores
/// nothing when any key is live, of any kind, looking its keys up in the
/// order named and stopping at the first live one, as Redis does: a key
/// expired before the command stays in place for `DBSIZE` to count when it is
/// named after that one, and is removed when named before it, though it sorts
/// after it. The expiring context, which may remove that key at any time,
/// could upset the comparison only between the first two `DBSIZE` commands,
/// outside the first `MSETNX`'s statement, a window of microseconds, and
/// cannot upset the last count. `RENAME` and `RENAMENX` naming
/// one key twice leave a live key as it is, answering OK and 0, and refuse an
/// absent one; `COPY` refuses one key named twice before it reads the key,
/// and `RENAMENX` and `COPY` without REPLACE leave a live destination alone.
/// `UNLINK` removes and counts a key named twice once, `TOUCH` counts it
/// twice. `COPY` reads its options in order, each up to a zero byte: DB takes
/// an integer of the int range, of which only 0 names firn's one database.
/// The expected replies are redis-server 7.0.15's to the same requests, a
/// server configured with one database for `COPY`'s DB.
#[cfg(target_os = "linux")]
#[test]
fn firn_answers_commands_naming_a_key_twice_as_redis_does() {
    let program = firn();
    let port = free_port();
    let text = port.to_string();
    let child = program.spawn_on_route(true, &[text.as_bytes(), b"1"]);
    let mut client = connect_when_ready(port);
    client
        .set_read_timeout(Some(Duration::from_secs(20)))
        .expect("bound the client's waits");
    let mut batch = Vec::new();
    for request in [
        vec!["SET", "a", "1"],
        vec!["SET", "b", "2"],
        vec!["SET", "c", "3"],
        vec!["EXISTS", "c", "a", "c", "absent", "c"],
        vec!["DEL", "a", "a", "absent", "b"],
        vec!["EXISTS", "a", "b", "a"],
        vec!["MSET", "k", "1", "j", "2", "k", "3", "j", "4", "k", "5"],
        vec!["GET", "k"],
        vec!["GET", "j"],
        vec!["DBSIZE"],
        vec!["MGET", "k", "c", "k", "absent", "j", "c"],
        vec!["LPUSH", "l", "v"],
        vec!["MGET", "l", "k"],
        vec!["MSETNX", "x", "1", "y", "2", "x", "3"],
        vec!["MGET", "x", "y"],
        vec!["MSETNX", "x", "9", "z", "9"],
        vec!["EXISTS", "z"],
        vec!["MSETNX", "l", "1"],
        vec!["RENAME", "c", "c"],
        vec!["RENAMENX", "c", "c"],
        vec!["RENAME", "absent", "absent"],
        vec!["COPY", "c", "c"],
        vec!["RENAME", "c", "k"],
        vec!["MGET", "c", "k"],
        vec!["RENAMENX", "k", "j"],
        vec!["COPY", "k", "j"],
        vec!["COPY", "k", "j", "REPLACE"],
        vec!["MGET", "k", "j"],
        vec!["UNLINK", "j", "j", "absent", "x"],
        vec!["TOUCH", "k", "k", "absent", "y"],
        vec!["COPY", "k", "m", "DB", "1"],
        vec!["COPY", "k", "m", "DB", "-1"],
        vec!["COPY", "k", "m", "DB", "x"],
        vec!["COPY", "k", "m", "DB", "4294967296"],
        vec!["COPY", "k", "m", "DB"],
        vec!["COPY", "k", "m", "FOO"],
        vec!["COPY", "k", "m", "DB", "0", "DB", "1"],
        vec!["COPY", "k", "m", "replace\0x", "db\0", "0"],
        vec!["MGET", "m"],
        vec!["DBSIZE"],
    ] {
        batch.extend(resp(&request));
    }
    client.write_all(&batch).expect("send the batch");
    expect_replies(
        &mut client,
        b"+OK\r\n+OK\r\n+OK\r\n:4\r\n:2\r\n:0\r\n+OK\r\n$1\r\n5\r\n$1\r\n4\r\n:3\r\n*6\r\n$1\r\n5\r\n$1\r\n3\r\n$1\r\n5\r\n$-1\r\n$1\r\n4\r\n$1\r\n3\r\n:1\r\n*2\r\n$-1\r\n$1\r\n5\r\n:1\r\n*2\r\n$1\r\n3\r\n$1\r\n2\r\n:0\r\n:0\r\n:0\r\n+OK\r\n:0\r\n-ERR no such key\r\n-ERR source and destination objects are the same\r\n+OK\r\n*2\r\n$-1\r\n$1\r\n3\r\n:0\r\n:0\r\n:1\r\n*2\r\n$1\r\n3\r\n$1\r\n3\r\n:2\r\n:3\r\n-ERR DB index is out of range\r\n-ERR DB index is out of range\r\n-ERR value is not an integer or out of range\r\n-ERR value is out of range, value must between -2147483648 and 2147483647\r\n-ERR syntax error\r\n-ERR syntax error\r\n-ERR DB index is out of range\r\n:1\r\n*1\r\n$1\r\n3\r\n:4\r\n",
        "the commands naming a key twice",
    );
    let mut late = resp(&["SET", "late", "1", "PXAT", "1"]);
    late.extend(resp(&["DBSIZE"]));
    late.extend(resp(&["MSETNX", "k", "1", "late", "2"]));
    late.extend(resp(&["DBSIZE"]));
    late.extend(resp(&["MSETNX", "late", "1", "k", "2"]));
    late.extend(resp(&["DBSIZE"]));
    client
        .write_all(&late)
        .expect("send MSETNX past a live key");
    expect_replies(&mut client, b"+OK\r\n", "an expired key");
    let before = integer_reply(&mut client, "DBSIZE before MSETNX");
    assert_eq!(integer_reply(&mut client, "MSETNX past a live key"), 0);
    let after = integer_reply(&mut client, "DBSIZE after MSETNX");
    assert_eq!(
        after, before,
        "MSETNX looked up a key named after a live one"
    );
    assert_eq!(integer_reply(&mut client, "MSETNX before a live key"), 0);
    assert_eq!(
        integer_reply(&mut client, "DBSIZE after the second MSETNX"),
        4,
        "MSETNX did not look up a key named before a live one"
    );
    drop(client);
    let (status, _) = finished(child);
    assert_eq!(status, 0);
}

/// firn answers `CONFIG` with too few arguments, an unpaired option or several
/// parameters, and echoes client bytes in its errors, as Redis does: a zero
/// byte ends an echoed name or argument, carriage return and line feed become
/// spaces so that an error stays one line, an unknown subcommand is echoed to
/// 128 bytes and an unknown `CONFIG SET` option whole, and a carriage return a
/// malformed request holds where a dollar sign belongs is echoed as a space
/// before the connection closes. `CONFIG SET` answers the first unknown,
/// immutable or repeated name as Redis does, an alias not repeating its
/// parameter; then the first refused value, read as string2ll or memtoull reads
/// it, the latter's product wrapping modulo 2^64, with Redis's range and the
/// parameter's own name, or a save schedule as Redis splits and reads one, a
/// lone piece that starts with a zero byte being the empty schedule; and sets
/// nothing when any pair is refused. The port and address firn listens on are
/// accepted. `CONFIG RESETSTAT` succeeds. `FUNCTION FLUSH` succeeds with no
/// option, or with `ASYNC` or `SYNC` read up to a zero byte, and refuses
/// another option and two of them; `FUNCTION` alone is short of arguments, and
/// a subcommand whose name holds a zero byte is unknown. `DEBUG LOG` with one
/// message, empty or not, succeeds, its name read up to a zero byte, while
/// `LOG` without exactly one message and an unknown subcommand, its line breaks
/// echoed as spaces, are answered as Redis answers a `DEBUG` subcommand it does
/// not know. The expected replies are redis-server 7.0.15's to the same bytes,
/// its debug command enabled, a `CONFIG GET` of several parameters in
/// alphabetical order, one of the orders Redis answers in, but for the last
/// four `CONFIG SET`s, which ask for a snapshot schedule, the append-only file,
/// another address and another port: Redis would apply them, and firn refuses
/// them in Redis's form for a refused value, with its own reason.
#[cfg(target_os = "linux")]
#[test]
fn firn_answers_config_and_echoes_client_bytes_as_redis_does() {
    let program = firn();
    let port = free_port();
    let text = port.to_string();
    let child = program.spawn_on_route(true, &[text.as_bytes(), b"1"]);
    let mut client = connect_when_ready(port);
    client
        .set_read_timeout(Some(Duration::from_secs(20)))
        .expect("bound the client's waits");
    let mut batch = b"*1\r\n$6\r\nCONFIG\r\n*2\r\n$6\r\nCONFIG\r\n$3\r\nGET\r\n*3\r\n$6\r\nCONFIG\r\n$3\r\nSET\r\n$1\r\nx\r\n*5\r\n$6\r\nCONFIG\r\n$3\r\nSET\r\n$1\r\nx\r\n$1\r\ny\r\n$1\r\nz\r\n*4\r\n$6\r\nCONFIG\r\n$3\r\nSET\r\n$1\r\nx\r\n$1\r\ny\r\n*4\r\n$6\r\nCONFIG\r\n$3\r\nGET\r\n$4\r\nsave\r\n$10\r\nappendonly\r\n*5\r\n$6\r\nCONFIG\r\n$3\r\nGET\r\n$10\r\nappendonly\r\n$4\r\nsave\r\n$4\r\nsave\r\n*4\r\n$6\r\nCONFIG\r\n$3\r\nget\r\n$7\r\nnothing\r\n$4\r\nelse\r\n*3\r\n$6\r\nCONFIG\r\n$3\r\nGET\r\n$4\r\nsave\r\n*3\r\n$6\r\nCONFIG\r\n$3\r\nGET\r\n$10\r\nAPPENDONLY\r\n*3\r\n$6\r\nCONFIG\r\n$3\r\nGET\r\n$4\r\nSave\r\n*2\r\n$4\r\nGET\x00\r\n$1\r\na\r\n*3\r\n$4\r\nG\r\nT\r\n$3\r\nb\nc\r\n$3\r\nd\x00e\r\n*2\r\n$6\r\nCONFIG\r\n$4\r\nNO\nP\r\n".to_vec();
    // An unknown subcommand is echoed to 128 bytes; an unknown CONFIG SET
    // option is echoed whole, up to a zero byte.
    batch.extend_from_slice(b"*2\r\n$6\r\nCONFIG\r\n$200\r\n");
    batch.extend_from_slice(&[b'b'; 200]);
    batch.extend_from_slice(b"\r\n*4\r\n$6\r\nCONFIG\r\n$3\r\nSET\r\n$161\r\n");
    batch.extend_from_slice(&[b'c'; 150]);
    batch.push(0);
    batch.extend_from_slice(&[b'd'; 10]);
    batch.extend_from_slice(b"\r\n$1\r\nv\r\n");
    let addresses = (1..=17)
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(" ");
    let other = port.checked_add(1).unwrap_or(1024).to_string();
    for request in [
        vec!["CONFIG", "SET", "databases", "16"],
        vec!["CONFIG", "SET", "nosuch", "1", "databases", "16"],
        vec!["CONFIG", "SET", "Databases", "16", "nosuch", "1"],
        vec!["CONFIG", "SET", "timeout", "0", "TIMEOUT", "1"],
        vec![
            "CONFIG",
            "SET",
            "hash-max-ziplist-entries",
            "1",
            "hash-max-listpack-entries",
            "2",
        ],
        vec!["CONFIG", "GET", "hash-max-ziplist-entries"],
        vec!["CONFIG", "SET", "TIMEOUT", "-1"],
        vec!["CONFIG", "SET", "timeout", "007"],
        vec!["CONFIG", "SET", "hash-max-listpack-entries", "-1"],
        vec!["CONFIG", "SET", "list-max-ziplist-size", "-2147483649"],
        vec!["CONFIG", "SET", "list-compress-depth", "-1"],
        vec!["CONFIG", "SET", "set-max-intset-entries", "-1"],
        vec!["CONFIG", "SET", "stream-node-max-entries", "-1"],
        vec!["CONFIG", "SET", "zset-max-listpack-entries", "-1"],
        vec![
            "CONFIG",
            "SET",
            "zset-max-ziplist-value",
            "1kb",
            "zset-max-listpack-entries",
            "7",
        ],
        vec!["CONFIG", "SET", "hash-max-listpack-value", "12\0kb"],
        vec![
            "CONFIG",
            "SET",
            "stream-node-max-bytes",
            "18014398509481984kb",
        ],
        vec![
            "CONFIG",
            "GET",
            "zset-max-listpack-value",
            "hash-max-listpack-value",
            "stream-node-max-bytes",
        ],
        vec![
            "CONFIG",
            "SET",
            "stream-node-max-bytes",
            "20000000000000000000",
        ],
        vec!["CONFIG", "SET", "stream-node-max-bytes", "5 kb"],
        vec!["CONFIG", "SET", "appendonly", "maybe"],
        vec!["CONFIG", "SET", "appendonly", "No\0x"],
        vec!["CONFIG", "SET", "save", "1 2 3"],
        vec!["CONFIG", "SET", "save", ""],
        vec!["CONFIG", "SET", "save", "\0zz"],
        vec!["CONFIG", "SET", "save", "\0 1"],
        vec!["CONFIG", "SET", "bind", addresses.as_str()],
        vec!["CONFIG", "SET", "timeout", "9", "port", "70000"],
        vec!["CONFIG", "SET", "port", "70000", "timeout", "abc"],
        vec!["CONFIG", "SET", "appendonly", "yes", "timeout", "abc"],
        vec!["CONFIG", "SET", "port", text.as_str(), "bind", "127.0.0.1"],
        vec!["CONFIG", "GET", "timeout"],
        vec!["CONFIG", "RESETSTAT"],
        vec!["CONFIG", "RESETSTAT", "x"],
        vec!["CONFIG", "SET", "save", "3600 1"],
        vec!["CONFIG", "SET", "appendonly", "yes"],
        vec!["CONFIG", "SET", "timeout", "5", "bind", "0.0.0.0"],
        vec!["CONFIG", "SET", "port", other.as_str()],
        vec!["CONFIG", "GET", "timeout"],
        vec!["FUNCTION"],
        vec!["FUNCTION", "FLUSH"],
        vec!["function", "flush", "async"],
        vec!["FUNCTION", "FLUSH", "SYNC\0x"],
        vec!["FUNCTION", "FLUSH", "x"],
        vec!["FUNCTION", "FLUSH", "a", "b"],
        vec!["FUNCTION", "flush\0"],
        vec!["DEBUG"],
        vec!["DEBUG", "LOG", "a message"],
        vec!["debug", "log\0x", ""],
        vec!["DEBUG", "LOG"],
        vec!["DEBUG", "LOG", "a", "b"],
        vec!["DEBUG", "NO\r\nPE"],
    ] {
        batch.extend(resp(&request));
    }
    batch.extend_from_slice(b"*1\r\n\r\n");
    client.write_all(&batch).expect("send the batch");
    let mut expected = b"-ERR wrong number of arguments for 'config' command\r\n-ERR wrong number of arguments for 'config|get' command\r\n-ERR wrong number of arguments for 'config|set' command\r\n-ERR syntax error\r\n-ERR Unknown option or number of arguments for CONFIG SET - 'x'\r\n*4\r\n$10\r\nappendonly\r\n$2\r\nno\r\n$4\r\nsave\r\n$0\r\n\r\n*4\r\n$10\r\nappendonly\r\n$2\r\nno\r\n$4\r\nsave\r\n$0\r\n\r\n*0\r\n*2\r\n$4\r\nsave\r\n$0\r\n\r\n*2\r\n$10\r\nAPPENDONLY\r\n$2\r\nno\r\n*2\r\n$4\r\nSave\r\n$0\r\n\r\n-ERR unknown command 'GET', with args beginning with: 'a' \r\n-ERR unknown command 'G  T', with args beginning with: 'b c' 'd' \r\n-ERR unknown subcommand 'NO P'. Try CONFIG HELP.\r\n".to_vec();
    expected.extend_from_slice(b"-ERR unknown subcommand '");
    expected.extend_from_slice(&[b'b'; 128]);
    expected.extend_from_slice(
        b"'. Try CONFIG HELP.\r\n-ERR Unknown option or number of arguments for CONFIG SET - '",
    );
    expected.extend_from_slice(&[b'c'; 150]);
    expected.extend_from_slice(b"'\r\n");
    let failed = "-ERR CONFIG SET failed (possibly related to argument";
    let wide = "9223372036854775807";
    expected.extend(
        format!(
            "{failed} 'databases') - can't set immutable config\r\n\
             -ERR Unknown option or number of arguments for CONFIG SET - 'nosuch'\r\n\
             {failed} 'Databases') - can't set immutable config\r\n\
             {failed} 'TIMEOUT') - duplicate parameter\r\n\
             +OK\r\n*2\r\n$24\r\nhash-max-ziplist-entries\r\n$1\r\n2\r\n\
             {failed} 'timeout') - argument must be between 0 and 2147483647 inclusive\r\n\
             {failed} 'timeout') - argument couldn't be parsed into an integer\r\n\
             {failed} 'hash-max-listpack-entries') - argument must be between 0 and {wide} inclusive\r\n\
             {failed} 'list-max-ziplist-size') - argument must be between -2147483648 and 2147483647 inclusive\r\n\
             {failed} 'list-compress-depth') - argument must be between 0 and 2147483647 inclusive\r\n\
             {failed} 'set-max-intset-entries') - argument must be between 0 and {wide} inclusive\r\n\
             {failed} 'stream-node-max-entries') - argument must be between 0 and {wide} inclusive\r\n\
             {failed} 'zset-max-listpack-entries') - argument must be between 0 and {wide} inclusive\r\n\
             +OK\r\n+OK\r\n+OK\r\n\
             *6\r\n$23\r\nhash-max-listpack-value\r\n$2\r\n12\r\n$21\r\nstream-node-max-bytes\r\n$1\r\n0\r\n$23\r\nzset-max-listpack-value\r\n$4\r\n1024\r\n\
             {failed} 'stream-node-max-bytes') - argument must be between 0 and {wide} inclusive\r\n\
             {failed} 'stream-node-max-bytes') - argument must be a memory value\r\n\
             {failed} 'appendonly') - argument must be 'yes' or 'no'\r\n\
             +OK\r\n\
             {failed} 'save') - Invalid save parameters\r\n\
             +OK\r\n\
             +OK\r\n\
             {failed} 'save') - Invalid save parameters\r\n\
             {failed} 'bind') - Too many bind addresses specified.\r\n\
             {failed} 'port') - argument must be between 0 and 65535 inclusive\r\n\
             {failed} 'port') - argument must be between 0 and 65535 inclusive\r\n\
             {failed} 'timeout') - argument couldn't be parsed into an integer\r\n\
             +OK\r\n\
             *2\r\n$7\r\ntimeout\r\n$1\r\n0\r\n\
             +OK\r\n\
             -ERR wrong number of arguments for 'config|resetstat' command\r\n\
             {failed} 'save') - firn saves no snapshot\r\n\
             {failed} 'appendonly') - firn cannot change it while running\r\n\
             {failed} 'bind') - firn cannot change it while running\r\n\
             {failed} 'port') - firn cannot change it while running\r\n\
             *2\r\n$7\r\ntimeout\r\n$1\r\n0\r\n"
        )
        .bytes(),
    );
    expected.extend_from_slice(b"-ERR wrong number of arguments for 'function' command\r\n+OK\r\n+OK\r\n+OK\r\n-ERR FUNCTION FLUSH only supports SYNC|ASYNC option\r\n-ERR unknown subcommand or wrong number of arguments for 'FLUSH'. Try FUNCTION HELP.\r\n-ERR unknown subcommand 'flush'. Try FUNCTION HELP.\r\n");
    expected.extend_from_slice(b"-ERR wrong number of arguments for 'debug' command\r\n+OK\r\n+OK\r\n-ERR unknown subcommand or wrong number of arguments for 'LOG'. Try DEBUG HELP.\r\n-ERR unknown subcommand or wrong number of arguments for 'LOG'. Try DEBUG HELP.\r\n-ERR unknown subcommand or wrong number of arguments for 'NO  PE'. Try DEBUG HELP.\r\n");
    expected.extend_from_slice(b"-ERR Protocol error: expected '$', got ' '\r\n");
    expect_replies(&mut client, &expected, "the CONFIG and echo batch");
    drop(client);
    let (status, _) = finished(child);
    assert_eq!(status, 0);
}

/// firn reads a request larger than its first input window and writes a reply
/// larger than its first reply window: a 100,000-byte value set in one command
/// reads back whole, and a range of 2,000 list elements, a reply of more than
/// 16,384 bytes, arrives whole and in order.
#[cfg(target_os = "linux")]
#[test]
fn firn_carries_requests_and_replies_larger_than_its_windows() {
    const ELEMENTS: usize = 2_000;
    let program = firn();
    let port = free_port();
    let text = port.to_string();
    let child = program.spawn_on_route(true, &[text.as_bytes(), b"1"]);
    let mut client = connect_when_ready(port);
    client
        .set_read_timeout(Some(Duration::from_secs(20)))
        .expect("bound the client's waits");
    let value = (0..100_000)
        .map(|index| char::from(b'a' + (index % 26) as u8))
        .collect::<String>();
    client
        .write_all(&resp(&["SET", "big", &value]))
        .expect("send a large value");
    client
        .write_all(&resp(&["GET", "big"]))
        .expect("read it back");
    let mut expected = b"+OK\r\n".to_vec();
    expected.extend_from_slice(format!("${}\r\n{value}\r\n", value.len()).as_bytes());
    expect_replies(&mut client, &expected, "the large value");
    let elements = (0..ELEMENTS)
        .map(|index| format!("element-{index:05}"))
        .collect::<Vec<_>>();
    let mut push = vec!["RPUSH", "many"];
    push.extend(elements.iter().map(String::as_str));
    client.write_all(&resp(&push)).expect("push the elements");
    client
        .write_all(&resp(&["LRANGE", "many", "0", "-1"]))
        .expect("range every element");
    let mut expected = format!(":{ELEMENTS}\r\n*{ELEMENTS}\r\n").into_bytes();
    for element in &elements {
        expected.extend_from_slice(format!("${}\r\n{element}\r\n", element.len()).as_bytes());
    }
    assert!(expected.len() > 16_384);
    expect_replies(&mut client, &expected, "the large range");
    drop(client);
    let (status, _) = finished(child);
    assert_eq!(status, 0);
}

/// firn replays the value types from its append-only file: after a restart a
/// list keeps the elements its pushes and pops left, a hash its field, a
/// sorted set the member ZPOPMIN left at its score, and a member added at 0.1
/// keeps the double nearest 0.1, which the replay reads again from the score's
/// text as the command read it. The 25 of 50 members SPOP
/// removed stay removed, since the file records the pop as the SREM of the
/// members it chose, as Redis records it; a replay that popped at random, from
/// a generator seeded by the clock at each start, would almost surely remove
/// others. A string set before FLUSHALL and a list pushed before FLUSHDB stay
/// absent, since the file records both commands, and FUNCTION FLUSH after
/// them, as Redis does, and a FLUSHALL of the keyspace still empty before
/// them: the file's first bytes are those redis-server 7.0.15 appends for the
/// same requests, the SELECT it writes first aside. A key the expiring context
/// removed is recorded as removed, as Redis propagates it, so a `SET` with NX
/// that found it absent holds its value after the restart, where a replay
/// keeping the expired key would refuse it. INFO reports the file as kept;
/// CONFIG SET appendonly yes, the file firn keeps, succeeds, and no is refused
/// in Redis's form for a refused value with firn's own reason, where Redis
/// would stop its file.
#[cfg(target_os = "linux")]
#[test]
fn firn_replays_the_value_types_from_its_append_only_file() {
    let program = firn();
    let name = "replay-types.aof";
    let port = free_port();
    let text = port.to_string();
    let child = program.spawn_on_route(true, &[text.as_bytes(), b"1", name.as_bytes()]);
    let mut client = connect_when_ready(port);
    client
        .set_read_timeout(Some(Duration::from_secs(20)))
        .expect("bound the first client's waits");
    let members = (0..50)
        .map(|index| format!("m{index:02}"))
        .collect::<Vec<_>>();
    let mut add = vec!["SADD", "s"];
    add.extend(members.iter().map(String::as_str));
    let mut batch = Vec::new();
    for request in [
        vec!["FLUSHALL"],
        vec!["SET", "flushed", "v"],
        vec!["FLUSHALL"],
        vec!["RPUSH", "flushed-list", "a"],
        vec!["FLUSHDB", "ASYNC"],
        vec!["FUNCTION", "FLUSH", "ASYNC"],
        vec!["RPUSH", "l", "a", "b", "c"],
        vec!["LPOP", "l"],
        vec!["HSET", "h", "f", "v"],
        vec!["ZADD", "z", "1", "a", "2", "b"],
        vec!["ZPOPMIN", "z"],
        vec!["ZADD", "f", "0.1", "m"],
        add,
        vec!["SET", "lapse", "old", "PX", "1"],
    ] {
        batch.extend(resp(&request));
    }
    client.write_all(&batch).expect("send the changes");
    expect_replies(
        &mut client,
        b"+OK\r\n+OK\r\n+OK\r\n:1\r\n+OK\r\n+OK\r\n:3\r\n$1\r\na\r\n:1\r\n:2\r\n*2\r\n$1\r\na\r\n$1\r\n1\r\n:1\r\n:50\r\n+OK\r\n",
        "the first run's changes",
    );
    client
        .write_all(&resp(&["SPOP", "s", "25"]))
        .expect("pop 25 members");
    assert_eq!(
        reply_line(&mut client, "the popped members' header"),
        "*25\r\n"
    );
    let mut popped = Vec::new();
    for _ in 0..25 {
        assert_eq!(
            reply_line(&mut client, "a popped member's header"),
            "$3\r\n"
        );
        let member = reply_line(&mut client, "a popped member");
        let member = member.trim_end().to_owned();
        assert!(members.contains(&member), "{member}");
        assert!(!popped.contains(&member), "{member} popped twice");
        popped.push(member);
    }
    // The expiring context, which wakes every 100 milliseconds, removes the
    // key set with PX 1 before this SET finds it absent.
    std::thread::sleep(Duration::from_millis(250));
    client
        .write_all(&resp(&["SET", "lapse", "new", "NX"]))
        .expect("set the expired key again");
    expect_replies(&mut client, b"+OK\r\n", "the key set again");
    drop(client);
    let (status, _) = finished(child);
    assert_eq!(status, 0, "the first run");
    let file = std::fs::read(program.working_directory().join(name)).expect("read the file");
    let head: &[u8] = b"*1\r\n$8\r\nFLUSHALL\r\n*3\r\n$3\r\nSET\r\n$7\r\nflushed\r\n$1\r\nv\r\n*1\r\n$8\r\nFLUSHALL\r\n*3\r\n$5\r\nRPUSH\r\n$12\r\nflushed-list\r\n$1\r\na\r\n*2\r\n$7\r\nFLUSHDB\r\n$5\r\nASYNC\r\n*3\r\n$8\r\nFUNCTION\r\n$5\r\nFLUSH\r\n$5\r\nASYNC\r\n";
    assert_eq!(
        String::from_utf8_lossy(&file[..head.len().min(file.len())]),
        String::from_utf8_lossy(head),
        "the file's first commands"
    );
    let port = free_port();
    let text = port.to_string();
    let child = program.spawn_on_route(true, &[text.as_bytes(), b"1", name.as_bytes()]);
    let mut client = connect_when_ready(port);
    client
        .set_read_timeout(Some(Duration::from_secs(20)))
        .expect("bound the second client's waits");
    let mut remove = vec!["SREM", "s"];
    remove.extend(popped.iter().map(String::as_str));
    let mut batch = Vec::new();
    for request in [
        vec!["LRANGE", "l", "0", "-1"],
        vec!["HGET", "h", "f"],
        vec!["ZCARD", "z"],
        vec!["ZSCORE", "z", "b"],
        vec!["ZSCORE", "f", "m"],
        vec!["SCARD", "s"],
        remove,
        vec!["EXISTS", "flushed", "flushed-list"],
        vec!["GET", "lapse"],
    ] {
        batch.extend(resp(&request));
    }
    client.write_all(&batch).expect("read the replayed values");
    expect_replies(
        &mut client,
        b"*2\r\n$1\r\nb\r\n$1\r\nc\r\n$1\r\nv\r\n:1\r\n$1\r\n2\r\n$19\r\n0.10000000000000001\r\n:25\r\n:0\r\n:0\r\n$3\r\nnew\r\n",
        "the replayed values",
    );
    let mut batch = resp(&["CONFIG", "SET", "appendonly", "yes"]);
    batch.extend(resp(&["CONFIG", "SET", "appendonly", "no"]));
    batch.extend(resp(&["INFO", "persistence"]));
    client
        .write_all(&batch)
        .expect("ask for the file and whether it is kept");
    expect_replies(
        &mut client,
        b"+OK\r\n-ERR CONFIG SET failed (possibly related to argument 'appendonly') - firn cannot change it while running\r\n",
        "CONFIG SET appendonly with the file kept",
    );
    let info = bulk_reply(&mut client, "INFO persistence");
    assert_eq!(info_field(&info, "aof_enabled").as_deref(), Some("1"));
    drop(client);
    let (status, _) = finished(child);
    assert_eq!(status, 0, "the second run");
}

/// firn replays list updates and moves and set moves and algebra stores from
/// its append-only file. After a restart, the lists retain their elements and
/// the stored sets retain the membership produced before the restart.
#[cfg(target_os = "linux")]
#[test]
fn firn_replays_the_value_types_from_its_append_only_file_with_lists_and_sets() {
    let program = firn();
    let name = "replay-list-set-types.aof";
    let port = free_port();
    let text = port.to_string();
    let child = program.spawn_on_route(true, &[text.as_bytes(), b"1", name.as_bytes()]);
    let mut client = connect_when_ready(port);
    client
        .set_read_timeout(Some(Duration::from_secs(20)))
        .expect("bound the first client's waits");
    let mut batch = Vec::new();
    for request in [
        vec!["RPUSH", "k", "a", "b", "c", "d", "e", "f"],
        vec!["LSET", "k", "0", "A"],
        vec!["LREM", "k", "1", "c"],
        vec!["LTRIM", "k", "0", "3"],
        vec!["LINSERT", "k", "AFTER", "A", "x"],
        vec!["LPUSHX", "k", "p"],
        vec!["RPUSHX", "k", "q"],
        vec!["LMOVE", "k", "k2", "RIGHT", "LEFT"],
        vec!["RPOPLPUSH", "k", "k2"],
        vec!["LPOP", "k", "1"],
        vec!["SADD", "t1", "a", "b", "c"],
        vec!["SADD", "t2", "b", "c", "d"],
        vec!["SMOVE", "t1", "t3", "a"],
        vec!["SINTERSTORE", "ti", "t1", "t2"],
        vec!["SUNIONSTORE", "tu", "t1", "t2"],
        vec!["SDIFFSTORE", "td", "t2", "t1"],
    ] {
        batch.extend(resp(&request));
    }
    client.write_all(&batch).expect("send the changes");
    expect_replies(
        &mut client,
        b":6\r\n+OK\r\n:1\r\n+OK\r\n:5\r\n:6\r\n:7\r\n$1\r\nq\r\n$1\r\ne\r\n*1\r\n$1\r\np\r\n:3\r\n:3\r\n:1\r\n:2\r\n:3\r\n:1\r\n",
        "the first run's changes",
    );
    drop(client);
    let (status, _) = finished(child);
    assert_eq!(status, 0, "the first run");
    let port = free_port();
    let text = port.to_string();
    let child = program.spawn_on_route(true, &[text.as_bytes(), b"1", name.as_bytes()]);
    let mut client = connect_when_ready(port);
    client
        .set_read_timeout(Some(Duration::from_secs(20)))
        .expect("bound the second client's waits");
    let mut batch = Vec::new();
    for request in [
        vec!["LRANGE", "k", "0", "-1"],
        vec!["LRANGE", "k2", "0", "-1"],
        vec!["SCARD", "t1"],
        vec!["SMEMBERS", "t3"],
        vec!["SMISMEMBER", "ti", "b", "c", "d"],
        vec!["SMISMEMBER", "tu", "b", "c", "d"],
        vec!["SMEMBERS", "td"],
    ] {
        batch.extend(resp(&request));
    }
    client.write_all(&batch).expect("read the replayed values");
    expect_replies(
        &mut client,
        b"*4\r\n$1\r\nA\r\n$1\r\nx\r\n$1\r\nb\r\n$1\r\nd\r\n*2\r\n$1\r\ne\r\n$1\r\nq\r\n:2\r\n*1\r\n$1\r\na\r\n*3\r\n:1\r\n:1\r\n:0\r\n*3\r\n:1\r\n:1\r\n:1\r\n*1\r\n$1\r\nd\r\n",
        "the replayed values",
    );
    drop(client);
    let (status, _) = finished(child);
    assert_eq!(status, 0, "the second run");
}

/// Reads until the server closes the connection, failing on anything it sends
/// first.
#[cfg(target_os = "linux")]
fn expect_closed(stream: &mut TcpStream, what: &str) {
    let mut rest = Vec::new();
    stream
        .read_to_end(&mut rest)
        .unwrap_or_else(|error| panic!("{what}: the connection stayed open: {error}"));
    assert!(
        rest.is_empty(),
        "{what}: {:?}",
        String::from_utf8_lossy(&rest)
    );
}

/// Fails when the server sends anything within a third of a second.
#[cfg(target_os = "linux")]
fn expect_silence(stream: &mut TcpStream, what: &str) {
    expect_silence_for(stream, Duration::from_millis(300), what);
}

/// Fails when the server sends anything, or closes the connection, within the
/// wait.
#[cfg(target_os = "linux")]
fn expect_silence_for(stream: &mut TcpStream, wait: Duration, what: &str) {
    stream
        .set_read_timeout(Some(wait))
        .expect("bound the wait for silence");
    let mut byte = [0_u8; 1];
    match stream.read(&mut byte) {
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ) => {}
        other => panic!("{what}: expected no reply, got {other:?} {byte:?}"),
    }
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
        .expect("restore the reply wait");
}

/// firn splits an inline command as Redis's sdssplitargs splits one: double
/// quotes in which \n, \r, \t, \b, \a, \\, \" and \xHH stand for their bytes,
/// a backslash before any other byte for that byte and an incomplete \x for x;
/// single quotes in which only \' stands for a quote; a quote opening in the
/// middle of an argument; a vertical tab that separates arguments only before
/// an argument starts; an empty quoted argument; and a quoted command name,
/// the decoded arguments echoed by an unknown command's error. A closing quote
/// followed by anything but white space is an unbalanced quote, answered with
/// Redis's error before the connection closes, the PING after it unanswered,
/// and so is a double or single quote the line leaves open. The expected bytes
/// are redis-server 7.0.15's for the same bytes.
#[cfg(target_os = "linux")]
#[test]
fn firn_splits_quoted_inline_arguments_as_redis_does() {
    let program = firn();
    let port = free_port();
    let text = port.to_string();
    let child = program.spawn_on_route(true, &[text.as_bytes(), b"3"]);
    let mut client = connect_when_ready(port);
    client
        .write_all(b"ECHO \"hello world\"\r\nECHO \"a\\nb\\r\\t\\b\\a\\\\\\\"\\x41\\x4a\\xzz\\q\"\r\nECHO 'it\\'s'\r\nECHO 'a\\nb'\r\nECHO a\"b c\"\r\nECHO a\x0bb\r\nECHO \x0ba\r\nECHO \"a\"\x0bb\r\nECHO \"\"\r\n\"ECHO\" x\r\nECHO \"\\x4\"\r\nNOPE \"a b\" 'c\\'d' e\r\nECHO \"abc\"x\r\nPING\r\n")
        .expect("send the inline batch");
    expect_replies(
        &mut client,
        b"$11\r\nhello world\r\n$15\r\na\nb\r\t\x08\x07\\\"AJxzzq\r\n$4\r\nit's\r\n$4\r\na\\nb\r\n$4\r\nab c\r\n$3\r\na\x0bb\r\n$1\r\na\r\n-ERR wrong number of arguments for 'echo' command\r\n$0\r\n\r\n$1\r\nx\r\n$2\r\nx4\r\n-ERR unknown command 'NOPE', with args beginning with: 'a b' 'c'd' 'e' \r\n-ERR Protocol error: unbalanced quotes in request\r\n",
        "the inline batch",
    );
    expect_closed(&mut client, "after the unbalanced quote");
    drop(client);
    for line in [
        &b"ECHO \"abc\r\nPING\r\n"[..],
        &b"ECHO 'abc\r\nPING\r\n"[..],
    ] {
        let what = format!("{:?}", String::from_utf8_lossy(line));
        let mut client = connect_when_ready(port);
        client.write_all(line).expect("send a quote left open");
        expect_replies(
            &mut client,
            b"-ERR Protocol error: unbalanced quotes in request\r\n",
            &what,
        );
        expect_closed(&mut client, &what);
    }
    let (status, _) = finished(child);
    assert_eq!(status, 0);
}

/// firn reads a request's count and length lines as Redis 7.0 reads them. A
/// carriage return ends a line whatever byte follows it, which is skipped
/// unread; a negative count is skipped; and an array of 2,147,483,647
/// elements is a request still arriving, the most Redis takes, while
/// 2,147,483,648 elements, a count that is not a number, a negative length
/// and a count or length with a leading zero are malformed and close the
/// connection. A zero byte before a line's carriage return, its marker byte
/// among them, leaves the line incomplete, because Redis finds the
/// carriage return with strchr: with 65,536 bytes held from the line's first
/// byte the connection waits, and one more byte is answered as a count, or a
/// length, line too big before the connection closes. The expected bytes are
/// redis-server 7.0.15's.
#[cfg(target_os = "linux")]
#[test]
fn firn_reads_count_and_length_lines_as_redis_does() {
    let program = firn();
    let port = free_port();
    let text = port.to_string();
    let child = program.spawn_on_route(true, &[text.as_bytes(), b"9"]);
    let mut client = connect_when_ready(port);
    client
        .write_all(b"*1\r\n$4\rXPING\r\n*1\rX$4\r\nPING\r\n*-1\r\n*2147483647\r\n")
        .expect("send lines of every shape");
    expect_replies(&mut client, b"+PONG\r\n+PONG\r\n", "the lines' shapes");
    expect_silence(&mut client, "an array of 2,147,483,647 elements");
    drop(client);
    for (line, error) in [
        (
            &b"*2147483648\r\n"[..],
            &b"-ERR Protocol error: invalid multibulk length\r\n"[..],
        ),
        (
            &b"*-abc\r\n"[..],
            &b"-ERR Protocol error: invalid multibulk length\r\n"[..],
        ),
        (
            &b"*1\r\n$-1\r\n"[..],
            &b"-ERR Protocol error: invalid bulk length\r\n"[..],
        ),
        (
            &b"*01\r\n"[..],
            &b"-ERR Protocol error: invalid multibulk length\r\n"[..],
        ),
        (
            &b"*1\r\n$04\r\n"[..],
            &b"-ERR Protocol error: invalid bulk length\r\n"[..],
        ),
    ] {
        let what = format!("{:?}", String::from_utf8_lossy(line));
        let mut client = connect_when_ready(port);
        client.write_all(line).expect("send a malformed line");
        expect_replies(&mut client, error, &what);
        expect_closed(&mut client, &what);
    }
    for (head, error) in [
        (
            &b"*1\x00\r\n"[..],
            &b"-ERR Protocol error: too big mbulk count string\r\n"[..],
        ),
        (
            &b"*1\r\n$4\x00\r\n"[..],
            &b"-ERR Protocol error: too big bulk count string\r\n"[..],
        ),
        (
            &b"*1\r\n\x004\r\n"[..],
            &b"-ERR Protocol error: too big bulk count string\r\n"[..],
        ),
    ] {
        let what = format!("a zero byte in {:?}", String::from_utf8_lossy(head));
        let line_start = if head.starts_with(b"*1\r\n") { 4 } else { 0 };
        let mut client = connect_when_ready(port);
        let mut held = head.to_vec();
        held.resize(line_start + 65_536, b'x');
        client
            .write_all(&held)
            .expect("send 65,536 bytes from the line");
        expect_silence(&mut client, &what);
        client.write_all(b"x").expect("send one byte more");
        expect_replies(&mut client, error, &what);
        expect_closed(&mut client, &what);
    }
    let (status, _) = finished(child);
    assert_eq!(status, 0);
}

/// firn answers CONFIG GET for the parameters it reports as Redis 7.0 does,
/// matching an argument that holds [, * or ? as Redis's stringmatchlen
/// matches a pattern, in either case: * reaches all 22, the encoding
/// parameters and their aliases among them, each with Redis's default; an
/// argument naming a parameter and a pattern reaching it answer it once,
/// spelled as the first argument spells it; a class with a range, ? and a
/// negated class match; the range [Z-a] holds nothing, because Redis swaps a
/// reversed range's bounds before folding their case; a backslash outside a
/// class folds case and inside one does not; a pattern stops at a zero byte;
/// and an argument with none of the three bytes is a name, \Port among them.
/// The options --timeout and --appendfilename set the values reported. Every
/// reply holds what redis-server 7.0.15 answers for these parameters with the
/// same settings, in alphabetical order, one of the orders Redis answers in.
#[cfg(target_os = "linux")]
#[test]
fn firn_matches_config_get_patterns_as_redis_does() {
    let program = firn();
    let port = free_port();
    let text = port.to_string();
    let child = program.spawn_on_route(
        true,
        &[
            b"--port",
            text.as_bytes(),
            b"--clients",
            b"1",
            b"--timeout",
            b"7",
            b"--appendfilename",
            b"patterns.aof",
        ],
    );
    let mut client = connect_when_ready(port);
    let mut batch = Vec::new();
    for request in [
        vec!["CONFIG", "GET", "*"],
        vec!["CONFIG", "GET", "a*"],
        vec!["CONFIG", "GET", "APPENDONLY", "a*"],
        vec!["CONFIG", "GET", "[b-d]*"],
        vec!["CONFIG", "GET", "p?rt"],
        vec!["CONFIG", "GET", "*o*t"],
        vec!["CONFIG", "GET", "[^a-r]*"],
        vec!["CONFIG", "GET", "[Z-a]*"],
        vec!["CONFIG", "GET", "\\Port"],
        vec!["CONFIG", "GET", "\\Port*"],
        vec!["CONFIG", "GET", "[\\P]ort"],
        vec!["CONFIG", "GET", "s*\0x"],
        vec!["CONFIG", "GET", "maxmemory"],
        vec!["CONFIG", "GET", "save", "SAVE", "s*"],
    ] {
        batch.extend(resp(&request));
    }
    client.write_all(&batch).expect("send the patterns");
    let port_field = format!("$4\r\nport\r\n${}\r\n{text}\r\n", text.len());
    let hash_and_list = "$25\r\nhash-max-listpack-entries\r\n$3\r\n512\r\n$23\r\nhash-max-listpack-value\r\n$2\r\n64\r\n$24\r\nhash-max-ziplist-entries\r\n$3\r\n512\r\n$22\r\nhash-max-ziplist-value\r\n$2\r\n64\r\n$19\r\nlist-compress-depth\r\n$1\r\n0\r\n$22\r\nlist-max-listpack-size\r\n$2\r\n-2\r\n$21\r\nlist-max-ziplist-size\r\n$2\r\n-2\r\n";
    let s_fields = "$4\r\nsave\r\n$0\r\n\r\n$22\r\nset-max-intset-entries\r\n$3\r\n512\r\n$21\r\nstream-node-max-bytes\r\n$4\r\n4096\r\n$23\r\nstream-node-max-entries\r\n$3\r\n100\r\n";
    let zset_fields = "$25\r\nzset-max-listpack-entries\r\n$3\r\n128\r\n$23\r\nzset-max-listpack-value\r\n$2\r\n64\r\n$24\r\nzset-max-ziplist-entries\r\n$3\r\n128\r\n$22\r\nzset-max-ziplist-value\r\n$2\r\n64\r\n";
    let expected = format!(
        "*44\r\n$14\r\nappendfilename\r\n$12\r\npatterns.aof\r\n$10\r\nappendonly\r\n$2\r\nno\r\n$4\r\nbind\r\n$9\r\n127.0.0.1\r\n$9\r\ndatabases\r\n$1\r\n1\r\n{hash_and_list}{port_field}$11\r\nrequirepass\r\n$0\r\n\r\n{s_fields}$7\r\ntimeout\r\n$1\r\n7\r\n{zset_fields}\
         *4\r\n$14\r\nappendfilename\r\n$12\r\npatterns.aof\r\n$10\r\nappendonly\r\n$2\r\nno\r\n\
         *4\r\n$14\r\nappendfilename\r\n$12\r\npatterns.aof\r\n$10\r\nAPPENDONLY\r\n$2\r\nno\r\n\
         *4\r\n$4\r\nbind\r\n$9\r\n127.0.0.1\r\n$9\r\ndatabases\r\n$1\r\n1\r\n\
         *2\r\n{port_field}\
         *4\r\n{port_field}$7\r\ntimeout\r\n$1\r\n7\r\n\
         *18\r\n{s_fields}$7\r\ntimeout\r\n$1\r\n7\r\n{zset_fields}\
         *0\r\n\
         *0\r\n\
         *2\r\n{port_field}\
         *0\r\n\
         *8\r\n{s_fields}\
         *0\r\n\
         *8\r\n{s_fields}"
    );
    expect_replies(&mut client, expected.as_bytes(), "the patterns");
    drop(client);
    let (status, _) = finished(child);
    assert_eq!(status, 0);
}

/// firn takes options by name, in either case, a later value replacing an
/// earlier one: with --bind 127.0.0.2 it listens on that address, so a
/// connection to 127.0.0.1 on the same port is refused, and CONFIG GET
/// reports the address, the port the last --port named, the append-only file
/// --appendonly yes turned on under the name --appendfilename gave, and the
/// idle limit --timeout set. An unknown option, an option without its value, a
/// port past 65,535, an address that is not one, an --appendonly other than
/// yes or no, an idle limit past 2,147,483,647 seconds, an empty file name, a
/// fifth argument by position and an argument by position after one by name
/// each stop firn with status 1 before it listens.
#[cfg(target_os = "linux")]
#[test]
fn firn_listens_where_its_options_by_name_say() {
    let program = firn();
    let port = free_port();
    let text = port.to_string();
    let child = program.spawn_on_route(
        true,
        &[
            b"--port",
            b"1",
            b"--bind",
            b"127.0.0.2",
            b"--PORT",
            text.as_bytes(),
            b"--Clients",
            b"1",
            b"--appendonly",
            b"yes",
            b"--appendfilename",
            b"options.aof",
            b"--timeout",
            b"9",
        ],
    );
    let mut client = connect_to_when_ready(SocketAddr::from(([127, 0, 0, 2], port)));
    let loopback = SocketAddr::from(([127, 0, 0, 1], port));
    assert!(
        TcpStream::connect_timeout(&loopback, Duration::from_millis(500)).is_err(),
        "firn listens on 127.0.0.1 too"
    );
    let mut batch = resp(&["PING"]);
    batch.extend(resp(&[
        "CONFIG",
        "GET",
        "bind",
        "port",
        "appendonly",
        "appendfilename",
        "timeout",
    ]));
    client.write_all(&batch).expect("ask where firn listens");
    expect_replies(
        &mut client,
        format!(
            "+PONG\r\n*10\r\n$14\r\nappendfilename\r\n$11\r\noptions.aof\r\n$10\r\nappendonly\r\n$3\r\nyes\r\n$4\r\nbind\r\n$9\r\n127.0.0.2\r\n$4\r\nport\r\n${}\r\n{text}\r\n$7\r\ntimeout\r\n$1\r\n9\r\n",
            text.len()
        )
        .as_bytes(),
        "the options' values",
    );
    drop(client);
    let (status, _) = finished(child);
    assert_eq!(status, 0);
    let refused: [&[&[u8]]; 9] = [
        &[b"--nosuch", b"1"],
        &[b"--port"],
        &[b"--port", b"65536"],
        &[b"--bind", b"127.0.0"],
        &[b"--appendonly", b"maybe"],
        &[b"--timeout", b"2147483648"],
        &[b"--appendfilename", b""],
        &[b"6379", b"0", b"-", b"0", b"5"],
        &[b"--port", b"6379", b"0"],
    ];
    for arguments in refused {
        let child = program.spawn_on_route(true, arguments);
        let (status, _) = finished(child);
        assert_eq!(status, 1, "{arguments:?}");
    }
}

/// A restarted firn listens on its port while the connection its first run
/// closed waits out TIME_WAIT there, as Redis does, which takes SO_REUSEADDR
/// on the runtime's listening socket: the first run answers QUIT with OK and
/// closes the connection itself, leaving the server's side of it in TIME_WAIT
/// on the port, and a second run started at once on the same port accepts a
/// client and answers it. Without the option the second run's listen fails
/// and firn stops with status 3. The option still refuses a listen of an
/// address and port another socket listens on: a third run started while the
/// second listens stops with status 3.
#[cfg(target_os = "linux")]
#[test]
fn firn_listens_again_on_its_port_after_a_restart() {
    let program = firn();
    for native_ring in [true, false] {
        let what = format!("native ring: {native_ring}");
        let port = free_port();
        let text = port.to_string();
        let child = program.spawn_on_route(native_ring, &[text.as_bytes(), b"1"]);
        let mut client = connect_when_ready(port);
        client.write_all(&resp(&["QUIT"])).expect("send QUIT");
        expect_replies(&mut client, b"+OK\r\n", &what);
        expect_closed(&mut client, &what);
        drop(client);
        let (status, _) = finished(child);
        assert_eq!(status, 0, "{what}: the first run");
        let child = program.spawn_on_route(native_ring, &[text.as_bytes(), b"2"]);
        let mut client = connect_when_ready(port);
        let third = program.spawn_on_route(native_ring, &[text.as_bytes(), b"1"]);
        let (status, _) = finished(third);
        assert_eq!(
            status, 3,
            "{what}: a third run on a port the second listens on"
        );
        client.write_all(&resp(&["PING"])).expect("send a ping");
        expect_replies(&mut client, b"+PONG\r\n", &what);
        drop(client);
        drop(connect_when_ready(port));
        let (status, _) = finished(child);
        assert_eq!(status, 0, "{what}: the second run");
    }
}

/// firn requires the password --requirepass names as Redis does. A connection
/// that has not authenticated is answered NOAUTH for every command but AUTH,
/// HELLO and QUIT, after an unknown command or subcommand or a count outside
/// the command table's arity is answered as such; MSET/MSETNX pair counts and
/// LPOP/RPOP maximum counts are checked only after authentication; a wrong
/// password, one of the right length among them, the user default with a wrong one and another user are answered
/// WRONGPASS and leave it locked; the password
/// unlocks it, and a wrong one afterwards leaves it unlocked. HELLO with AUTH
/// unlocks a connection too and answers with its id. While locked, an array of
/// more than 10 elements or a bulk string of more than 16,384 bytes is a
/// protocol error that closes the connection, and once unlocked an array of
/// 11 elements is a command. CONFIG SET requirepass changes the password: the
/// connection that changed it stays authenticated, a new one is refused the
/// old password, and removing the password lets that one in at once, but only
/// until a password is set again, since it has not authenticated, as Redis's
/// flag of authentication has it, and HELLO, which checks that flag, still
/// refuses it. A transaction it opens while no password is set is checked
/// as Redis checks each command once one is set again: a queued command's
/// arity, then its authentication, WATCH's own arity and authentication
/// before its refusal, and EXEC discards the transaction with EXECABORT and
/// NOAUTH, so that what follows runs outside it and nothing queued ran.
/// AUTH as the user default while no password
/// is set authenticates it, so that a password set afterwards leaves it in.
/// A connection accepted while no password is set has authenticated too, and
/// stays in once it sets one. The expected bytes are redis-server 7.0.15's
/// with the same password, the connection's id aside.
#[cfg(target_os = "linux")]
#[test]
fn firn_requires_its_password_as_redis_does() {
    let program = firn();
    let port = free_port();
    let text = port.to_string();
    let child = program.spawn_on_route(true, &[text.as_bytes(), b"7", b"--requirepass", b"secret"]);
    let mut client = connect_when_ready(port);
    // Redis 7.0.15's src/commands/{mset,msetnx,lpop,rpop}.json admit
    // minimum counts of 3, 3, 2 and 2 before processCommand checks auth.
    // msetGenericCommand checks pairs and popGenericCommand the maximum
    // only afterwards (src/t_string.c and src/t_list.c at that tag).
    for command in ["MSET", "MSETNX", "LPOP", "RPOP"] {
        let pairs = command.starts_with("MSET");
        let minimum = if pairs {
            vec![command, "arity:k"]
        } else {
            vec![command]
        };
        client
            .write_all(&resp(&minimum))
            .expect("send too few arguments while locked");
        let arity = format!(
            "-ERR wrong number of arguments for '{}' command\r\n",
            command.to_ascii_lowercase()
        );
        expect_replies(&mut client, arity.as_bytes(), command);
        client
            .write_all(&resp(&[command, "arity:k", "1", "extra"]))
            .expect("send a command-specific bad count while locked");
        expect_replies(
            &mut client,
            b"-NOAUTH Authentication required.\r\n",
            command,
        );
    }
    let mut batch = Vec::new();
    let mut eleven = vec!["DEL"];
    let keys = (0..10).map(|index| format!("k{index}")).collect::<Vec<_>>();
    eleven.extend(keys.iter().map(String::as_str));
    for request in [
        vec!["PING"],
        vec!["NOPE", "a"],
        vec!["GET"],
        vec!["CONFIG", "GET"],
        vec!["CONFIG", "GET", "save"],
        vec!["CLIENT", "FOO"],
        vec!["CLIENT", "INFO"],
        vec!["CLIENT", "INFO", "x"],
        vec!["FUNCTION", "NOPE"],
        vec!["FUNCTION", "FLUSH"],
        vec!["DEBUG"],
        vec!["DEBUG", "LOG", "x"],
        vec!["INFO"],
        vec!["CONFIG", "RESETSTAT", "x"],
        vec!["HELLO", "2"],
        vec!["AUTH", "wrong"],
        vec!["AUTH", "secreT"],
        vec!["AUTH", "default", "wrong"],
        vec!["AUTH", "bob", "secret"],
        vec!["PING"],
        vec!["PING", "a", "b"],
        vec!["AUTH", "secret"],
        vec!["PING"],
        vec!["PING", "a", "b"],
        vec!["AUTH", "wrong"],
        vec!["PING"],
        vec!["CONFIG", "GET", "requirepass"],
        eleven,
    ] {
        batch.extend(resp(&request));
    }
    client.write_all(&batch).expect("send the locked batch");
    expect_replies(
        &mut client,
        b"-NOAUTH Authentication required.\r\n-ERR unknown command 'NOPE', with args beginning with: 'a' \r\n-ERR wrong number of arguments for 'get' command\r\n-ERR wrong number of arguments for 'config|get' command\r\n-NOAUTH Authentication required.\r\n-ERR unknown subcommand 'FOO'. Try CLIENT HELP.\r\n-NOAUTH Authentication required.\r\n-ERR wrong number of arguments for 'client|info' command\r\n-ERR unknown subcommand 'NOPE'. Try FUNCTION HELP.\r\n-NOAUTH Authentication required.\r\n-ERR wrong number of arguments for 'debug' command\r\n-NOAUTH Authentication required.\r\n-NOAUTH Authentication required.\r\n-ERR wrong number of arguments for 'config|resetstat' command\r\n-NOAUTH HELLO must be called with the client already authenticated, otherwise the HELLO AUTH <user> <pass> option can be used to authenticate the client and select the RESP protocol version at the same time\r\n-WRONGPASS invalid username-password pair or user is disabled.\r\n-WRONGPASS invalid username-password pair or user is disabled.\r\n-WRONGPASS invalid username-password pair or user is disabled.\r\n-WRONGPASS invalid username-password pair or user is disabled.\r\n-NOAUTH Authentication required.\r\n-NOAUTH Authentication required.\r\n+OK\r\n+PONG\r\n-ERR wrong number of arguments for 'ping' command\r\n-WRONGPASS invalid username-password pair or user is disabled.\r\n+PONG\r\n*2\r\n$11\r\nrequirepass\r\n$6\r\nsecret\r\n:0\r\n",
        "the locked batch",
    );
    for command in ["MSET", "MSETNX", "LPOP", "RPOP"] {
        client
            .write_all(&resp(&[command, "arity:k", "1", "extra"]))
            .expect("send a command-specific bad count after authentication");
        let arity = format!(
            "-ERR wrong number of arguments for '{}' command\r\n",
            command.to_ascii_lowercase()
        );
        expect_replies(&mut client, arity.as_bytes(), command);
    }
    client
        .write_all(&resp(&["EXISTS", "arity:k"]))
        .expect("check rejected commands changed no key");
    expect_replies(&mut client, b":0\r\n", "rejected command side effects");
    drop(client);
    let mut client = connect_when_ready(port);
    let mut batch = resp(&["HELLO", "2", "AUTH", "default", "secret"]);
    batch.extend(resp(&["PING"]));
    client.write_all(&batch).expect("authenticate with HELLO");
    expect_replies(
        &mut client,
        b"*14\r\n$6\r\nserver\r\n$5\r\nredis\r\n$7\r\nversion\r\n$6\r\n7.0.15\r\n$5\r\nproto\r\n:2\r\n$2\r\nid\r\n:2\r\n$4\r\nmode\r\n$10\r\nstandalone\r\n$4\r\nrole\r\n$6\r\nmaster\r\n$7\r\nmodules\r\n*0\r\n+PONG\r\n",
        "HELLO with AUTH",
    );
    drop(client);
    let mut client = connect_when_ready(port);
    client
        .write_all(&resp(&[
            "ECHO", "a", "a", "a", "a", "a", "a", "a", "a", "a", "a",
        ]))
        .expect("send 11 elements while locked");
    expect_replies(
        &mut client,
        b"-ERR Protocol error: unauthenticated multibulk length\r\n",
        "11 elements while locked",
    );
    expect_closed(&mut client, "11 elements while locked");
    drop(client);
    let mut client = connect_when_ready(port);
    client
        .write_all(b"*2\r\n$4\r\nECHO\r\n$16385\r\n")
        .expect("send a long bulk string while locked");
    expect_replies(
        &mut client,
        b"-ERR Protocol error: unauthenticated bulk length\r\n",
        "16,385 bytes while locked",
    );
    expect_closed(&mut client, "16,385 bytes while locked");
    drop(client);
    let mut setter = connect_when_ready(port);
    let mut batch = resp(&["AUTH", "secret"]);
    batch.extend(resp(&["CONFIG", "SET", "requirepass", "other"]));
    batch.extend(resp(&["PING"]));
    batch.extend(resp(&["CONFIG", "GET", "requirepass"]));
    setter.write_all(&batch).expect("change the password");
    expect_replies(
        &mut setter,
        b"+OK\r\n+OK\r\n+PONG\r\n*2\r\n$11\r\nrequirepass\r\n$5\r\nother\r\n",
        "the password changed",
    );
    let mut waiting = connect_when_ready(port);
    let mut batch = resp(&["PING"]);
    batch.extend(resp(&["AUTH", "secret"]));
    waiting.write_all(&batch).expect("give the old password");
    expect_replies(
        &mut waiting,
        b"-NOAUTH Authentication required.\r\n-WRONGPASS invalid username-password pair or user is disabled.\r\n",
        "the old password",
    );
    setter
        .write_all(&resp(&["CONFIG", "SET", "requirepass", ""]))
        .expect("remove the password");
    expect_replies(&mut setter, b"+OK\r\n", "the password removed");
    let mut batch = resp(&["PING"]);
    batch.extend(resp(&["HELLO", "3"]));
    batch.extend(resp(&["AUTH", "x"]));
    waiting
        .write_all(&batch)
        .expect("send once the password is removed");
    expect_replies(
        &mut waiting,
        b"+PONG\r\n-NOAUTH HELLO must be called with the client already authenticated, otherwise the HELLO AUTH <user> <pass> option can be used to authenticate the client and select the RESP protocol version at the same time\r\n-ERR AUTH <password> called without any password configured for the default user. Are you sure your configuration is correct?\r\n",
        "once the password is removed",
    );
    waiting
        .write_all(&resp(&["MULTI"]))
        .expect("open a transaction with no password set");
    expect_replies(
        &mut waiting,
        b"+OK\r\n",
        "a transaction with no password set",
    );
    setter
        .write_all(&resp(&["CONFIG", "SET", "requirepass", "again"]))
        .expect("set a password inside the transaction");
    expect_replies(
        &mut setter,
        b"+OK\r\n",
        "a password set inside the transaction",
    );
    let mut batch = Vec::new();
    for request in [
        vec!["SET", "k"],
        vec!["SET", "k", "v"],
        vec!["WATCH"],
        vec!["WATCH", "k"],
        vec!["EXEC"],
    ] {
        batch.extend(resp(&request));
    }
    waiting
        .write_all(&batch)
        .expect("queue while a password is required");
    expect_replies(
        &mut waiting,
        b"-ERR wrong number of arguments for 'set' command\r\n-NOAUTH Authentication required.\r\n-ERR wrong number of arguments for 'watch' command\r\n-NOAUTH Authentication required.\r\n-EXECABORT Transaction discarded because of: NOAUTH Authentication required.\r\n",
        "a transaction a password set inside it refuses",
    );
    setter
        .write_all(&resp(&["CONFIG", "SET", "requirepass", ""]))
        .expect("remove the password after the transaction");
    expect_replies(
        &mut setter,
        b"+OK\r\n",
        "the password removed after the transaction",
    );
    waiting
        .write_all(&resp(&["GET", "k"]))
        .expect("read after the discarded transaction");
    expect_replies(&mut waiting, b"$-1\r\n", "after the discarded transaction");
    setter
        .write_all(&resp(&["CONFIG", "SET", "requirepass", "again"]))
        .expect("set a password again");
    expect_replies(&mut setter, b"+OK\r\n", "a password set again");
    waiting
        .write_all(&resp(&["PING"]))
        .expect("send once a password is set again");
    expect_replies(
        &mut waiting,
        b"-NOAUTH Authentication required.\r\n",
        "once a password is set again",
    );
    setter
        .write_all(&resp(&["CONFIG", "SET", "requirepass", ""]))
        .expect("remove the password again");
    expect_replies(&mut setter, b"+OK\r\n", "the password removed again");
    waiting
        .write_all(&resp(&["AUTH", "default", "x"]))
        .expect("authenticate with no password set");
    expect_replies(&mut waiting, b"+OK\r\n", "AUTH with no password set");
    setter
        .write_all(&resp(&["CONFIG", "SET", "requirepass", "again"]))
        .expect("set a password once more");
    expect_replies(&mut setter, b"+OK\r\n", "a password set once more");
    waiting
        .write_all(&resp(&["PING"]))
        .expect("send once authenticated");
    expect_replies(&mut waiting, b"+PONG\r\n", "once authenticated");
    setter
        .write_all(&resp(&["CONFIG", "SET", "requirepass", ""]))
        .expect("remove the password for the last connection");
    expect_replies(&mut setter, b"+OK\r\n", "the password removed at last");
    drop(waiting);
    drop(setter);
    let mut fresh = connect_when_ready(port);
    fresh
        .write_all(&resp(&["PING"]))
        .expect("send with no password");
    expect_replies(
        &mut fresh,
        b"+PONG\r\n",
        "a new connection with no password",
    );
    let mut batch = resp(&["CONFIG", "SET", "requirepass", "last"]);
    batch.extend(resp(&["PING"]));
    fresh
        .write_all(&batch)
        .expect("set a password from a connection accepted with none");
    expect_replies(
        &mut fresh,
        b"+OK\r\n+PONG\r\n",
        "a connection accepted with no password, once one is set",
    );
    drop(fresh);
    let (status, _) = finished(child);
    assert_eq!(status, 0);
}

/// firn answers the connection commands as Redis does. CLIENT ID counts the
/// connections from 1 in the order firn accepted them, and HELLO reports the
/// same id; CLIENT SETNAME gives a name, refuses one with a space keeping the
/// old one, and removes it with the empty name; CLIENT SETINFO, which Redis
/// 7.0 does not have, is an unknown subcommand; HELLO answers RESP2's map with
/// no version or version 2 and RESP3's with 3, and refuses 1 as unsupported,
/// and its SETNAME names the connection, an option's name
/// read up to a zero byte as Redis's strcasecmp reads it; AUTH without a
/// configured password is answered with Redis's error for the password alone
/// and succeeds for the user default; SELECT takes 0 alone, as Redis does with
/// one database; COMMAND and COMMAND COUNT describe no command and COMMAND
/// DOCS is an unknown subcommand; TIME answers the calendar time; INFO
/// answers Redis's sections in Redis's form and order, the default ones, all
/// of them for all or everything, or those named, an unknown name adding none,
/// with the real port, two clients connected, connections accepted, calendar
/// time and uptime, and its cluster, keyspace and modules sections byte for
/// byte as Redis answers them, the keyspace section with no line while the
/// keyspace is empty; CONFIG RESETSTAT zeroes the connections INFO counts;
/// and QUIT answers OK and closes the connection, leaving the request after it
/// unanswered. The expected bytes are redis-server 7.0.15's, started with one
/// database, but for the ids and the keyspace line, which leaves out the
/// expires and avg_ttl firn does not count.
#[cfg(target_os = "linux")]
#[test]
fn firn_answers_connection_commands_as_redis_does() {
    let program = firn();
    let port = free_port();
    let text = port.to_string();
    let spawned = Instant::now();
    let child = program.spawn_on_route(true, &[text.as_bytes(), b"3"]);
    let mut client = connect_when_ready(port);
    let mut batch = Vec::new();
    for request in [
        vec!["CLIENT", "ID"],
        vec!["CLIENT", "GETNAME"],
        vec!["CLIENT", "SETNAME", "conn-1"],
        vec!["CLIENT", "GETNAME"],
        vec!["CLIENT", "SETNAME", "a b"],
        vec!["CLIENT", "GETNAME"],
        vec!["CLIENT", "SETNAME", ""],
        vec!["CLIENT", "GETNAME"],
        vec!["CLIENT", "SETINFO", "lib-name", "x"],
        vec!["CLIENT", "ID", "x"],
        vec!["CLIENT"],
        vec!["HELLO"],
        vec!["HELLO", "3"],
        vec!["HELLO", "1"],
        vec!["HELLO", "x"],
        vec!["HELLO", "2", "SETNAME", "via-hello"],
        vec!["CLIENT", "GETNAME"],
        vec!["HELLO", "2", "FOO"],
        vec!["HELLO", "2", "SETNAME\0x", "via-zero"],
        vec!["CLIENT", "GETNAME"],
        vec!["HELLO", "2", "AUTH\0x", "default", "x"],
        vec!["AUTH", "x"],
        vec!["AUTH", "default", "x"],
        vec!["SELECT", "0"],
        vec!["SELECT", "1"],
        vec!["SELECT", "abc"],
        vec!["SELECT", "3000000000"],
        vec!["COMMAND"],
        vec!["COMMAND", "COUNT"],
        vec!["COMMAND", "COUNT", "x"],
        vec!["COMMAND", "DOCS"],
        vec!["TIME", "x"],
        vec!["INFO", "keyspace"],
    ] {
        batch.extend(resp(&request));
    }
    client.write_all(&batch).expect("send the connection batch");
    let hello = "*14\r\n$6\r\nserver\r\n$5\r\nredis\r\n$7\r\nversion\r\n$6\r\n7.0.15\r\n$5\r\nproto\r\n:2\r\n$2\r\nid\r\n:1\r\n$4\r\nmode\r\n$10\r\nstandalone\r\n$4\r\nrole\r\n$6\r\nmaster\r\n$7\r\nmodules\r\n*0\r\n";
    let hello3 = "%7\r\n$6\r\nserver\r\n$5\r\nredis\r\n$7\r\nversion\r\n$6\r\n7.0.15\r\n$5\r\nproto\r\n:3\r\n$2\r\nid\r\n:1\r\n$4\r\nmode\r\n$10\r\nstandalone\r\n$4\r\nrole\r\n$6\r\nmaster\r\n$7\r\nmodules\r\n*0\r\n";
    let expected = format!(
        ":1\r\n$-1\r\n+OK\r\n$6\r\nconn-1\r\n-ERR Client names cannot contain spaces, newlines or special characters.\r\n$6\r\nconn-1\r\n+OK\r\n$-1\r\n-ERR unknown subcommand 'SETINFO'. Try CLIENT HELP.\r\n-ERR wrong number of arguments for 'client|id' command\r\n-ERR wrong number of arguments for 'client' command\r\n{hello}{hello3}-NOPROTO unsupported protocol version\r\n-ERR Protocol version is not an integer or out of range\r\n{hello}$9\r\nvia-hello\r\n-ERR Syntax error in HELLO option 'FOO'\r\n{hello}$8\r\nvia-zero\r\n{hello}-ERR AUTH <password> called without any password configured for the default user. Are you sure your configuration is correct?\r\n+OK\r\n+OK\r\n-ERR DB index is out of range\r\n-ERR value is not an integer or out of range\r\n-ERR value is out of range, value must between -2147483648 and 2147483647\r\n*0\r\n:0\r\n-ERR wrong number of arguments for 'command|count' command\r\n-ERR unknown subcommand 'DOCS'. Try COMMAND HELP.\r\n-ERR wrong number of arguments for 'time' command\r\n$12\r\n# Keyspace\r\n\r\n"
    );
    expect_replies(&mut client, expected.as_bytes(), "the connection batch");
    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("the host clock is past 1970")
        .as_secs();
    client.write_all(&resp(&["TIME"])).expect("ask the time");
    assert_eq!(reply_line(&mut client, "TIME"), "*2\r\n");
    assert_eq!(reply_line(&mut client, "TIME's seconds"), "$10\r\n");
    let seconds = reply_line(&mut client, "TIME's seconds");
    let seconds = seconds.trim_end().parse::<u64>().expect("TIME's seconds");
    assert!(
        seconds + 5 >= before && seconds <= before + 5,
        "{seconds} against {before}"
    );
    let length = reply_line(&mut client, "TIME's microseconds");
    let micro = reply_line(&mut client, "TIME's microseconds");
    let micro = micro.trim_end();
    assert_eq!(length, format!("${}\r\n", micro.len()));
    assert!(micro.parse::<u64>().expect("TIME's microseconds") < 1_000_000);
    let mut batch = resp(&["SET", "k", "v"]);
    for request in [
        vec!["INFO", "cluster"],
        vec!["INFO", "NoSuch"],
        vec!["INFO", "KEYSPACE\0x", "nosuch"],
        vec!["INFO", "module_list"],
    ] {
        batch.extend(resp(&request));
    }
    client
        .write_all(&batch)
        .expect("ask for INFO's fixed sections");
    expect_replies(
        &mut client,
        b"+OK\r\n$30\r\n# Cluster\r\ncluster_enabled:0\r\n\r\n$0\r\n\r\n$24\r\n# Keyspace\r\ndb0:keys=1\r\n\r\n$11\r\n# Modules\r\n\r\n",
        "INFO's fixed sections",
    );
    let defaults = [
        "Server",
        "Clients",
        "Memory",
        "Persistence",
        "Stats",
        "Replication",
        "CPU",
        "Modules",
        "Errorstats",
        "Cluster",
        "Keyspace",
    ];
    let everything = [
        "Server",
        "Clients",
        "Memory",
        "Persistence",
        "Stats",
        "Replication",
        "CPU",
        "Modules",
        "Commandstats",
        "Errorstats",
        "Latencystats",
        "Cluster",
        "Keyspace",
    ];
    for (request, sections) in [
        (vec!["INFO", "cpu", "DEFAULT", "cpu"], &defaults[..]),
        (vec!["INFO", "everything"], &everything[..]),
        (vec!["INFO", "All"], &everything[..]),
        (vec!["INFO", "stats", "server"], &["Server", "Stats"][..]),
    ] {
        client.write_all(&resp(&request)).expect("ask for INFO");
        let info = bulk_reply(&mut client, "INFO");
        assert_eq!(info_sections(&info, "INFO"), sections, "{request:?}");
    }
    let mut second = connect_when_ready(port);
    second
        .write_all(&resp(&["PING"]))
        .expect("send a ping on a second connection");
    expect_replies(&mut second, b"+PONG\r\n", "a second connection");
    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("the host clock is past 1970")
        .as_micros();
    client.write_all(&resp(&["INFO"])).expect("ask for INFO");
    let info = bulk_reply(&mut client, "INFO");
    assert_eq!(info_sections(&info, "INFO"), &defaults[..]);
    for (field, value) in [
        ("redis_version", "7.0.15"),
        ("redis_git_sha1", "00000000"),
        ("tcp_port", text.as_str()),
        ("uptime_in_days", "0"),
        ("connected_clients", "2"),
        ("loading", "0"),
        ("aof_enabled", "0"),
        ("aof_rewrite_in_progress", "0"),
        ("aof_rewrite_scheduled", "0"),
        ("total_connections_received", "2"),
        ("role", "master"),
    ] {
        assert_eq!(
            info_field(&info, field).as_deref(),
            Some(value),
            "INFO's {field}"
        );
    }
    let now = info_field(&info, "server_time_usec")
        .and_then(|value| value.parse::<u128>().ok())
        .expect("INFO's server_time_usec");
    assert!(
        now + 5_000_000 >= before && now <= before + 5_000_000,
        "{now} against {before}"
    );
    let uptime = info_field(&info, "uptime_in_seconds")
        .and_then(|value| value.parse::<u64>().ok())
        .expect("INFO's uptime_in_seconds");
    let elapsed = spawned.elapsed().as_secs();
    assert!(
        uptime <= elapsed + 1,
        "{uptime} seconds up, {elapsed} since the start"
    );
    drop(second);
    let mut batch = resp(&["CONFIG", "RESETSTAT"]);
    batch.extend(resp(&["INFO", "stats"]));
    client.write_all(&batch).expect("reset the counts");
    expect_replies(&mut client, b"+OK\r\n", "CONFIG RESETSTAT");
    let stats = bulk_reply(&mut client, "INFO stats");
    assert_eq!(
        info_field(&stats, "total_connections_received").as_deref(),
        Some("0")
    );
    let mut batch = resp(&["QUIT"]);
    batch.extend(resp(&["PING"]));
    client.write_all(&batch).expect("send QUIT and a ping");
    expect_replies(&mut client, b"+OK\r\n", "QUIT");
    expect_closed(&mut client, "after QUIT");
    drop(client);
    let mut client = connect_when_ready(port);
    client
        .write_all(&resp(&["CLIENT", "ID"]))
        .expect("ask the third connection's id");
    expect_replies(&mut client, b":3\r\n", "the third connection's id");
    drop(client);
    let (status, _) = finished(child);
    assert_eq!(status, 0);
}

/// firn keeps a transaction as Redis 7.0.15's MULTI, EXEC and DISCARD do:
/// EXEC and DISCARD outside one and a nested MULTI are refused; queued
/// commands answer QUEUED and EXEC answers an array of their replies, a
/// runtime error among them leaving the others done; a command refused while
/// queueing for its arity makes EXEC abort the transaction; WATCH inside one
/// is refused without aborting it, unless its arity is wrong; DISCARD drops
/// what was queued; an EXEC with an argument, inside a transaction or not,
/// discards it and answers EXECABORT with the reason, so that a command after
/// it runs at once. The expected
/// bytes are redis-server 7.0.15's, but for FOO and EXPIRETIMEX, which firn
/// queues and then refuses whole at EXEC as a command it does not run in
/// transactions, where Redis refuses it as unknown when it is sent and EXEC
/// then aborts. EXPIRETIMEX shares EXPIRETIME's eight-byte code, so its case
/// shows that a transaction, and a script through the same held_kind, tells
/// the two names apart.
#[cfg(target_os = "linux")]
#[test]
fn firn_keeps_transactions_as_redis_does() {
    let program = firn();
    let port = free_port();
    let text = port.to_string();
    let child = program.spawn_on_route(true, &[text.as_bytes(), b"1"]);
    let mut client = connect_when_ready(port);
    let mut batch = Vec::new();
    for request in [
        vec!["EXEC"],
        vec!["DISCARD"],
        vec!["MULTI"],
        vec!["MULTI"],
        vec!["SET", "a", "1"],
        vec!["INCRBY", "a", "5"],
        vec!["EXPIRE", "a", "100"],
        vec!["TTL", "a"],
        vec!["MSET", "b", "2", "c", "3"],
        vec!["GET", "b"],
        vec!["EXEC"],
        vec!["MULTI"],
        vec!["SET", "s", "abc"],
        vec!["INCR", "s"],
        vec!["GET", "s"],
        vec!["EXEC"],
        vec!["MULTI"],
        vec!["SET", "x", "1"],
        vec!["GET", "x", "y"],
        vec!["EXEC"],
        vec!["GET", "x"],
        vec!["MULTI"],
        vec!["SET", "x", "1"],
        vec!["FOO"],
        vec!["EXEC"],
        vec!["MULTI"],
        vec!["WATCH", "x"],
        vec!["EXEC"],
        vec!["MULTI"],
        vec!["SET", "x", "2"],
        vec!["DISCARD"],
        vec!["GET", "x"],
        vec!["EXEC", "now"],
        vec!["MULTI"],
        vec!["EXPIRETIME", "absent"],
        vec!["EXEC"],
        vec!["MULTI"],
        vec!["EXPIRETIMEX", "absent"],
        vec!["EXEC"],
        vec!["MULTI"],
        vec!["EXEC", "extra"],
        vec!["SET", "k", "v"],
        vec!["GET", "k"],
        vec!["MULTI"],
        vec!["WATCH"],
        vec!["EXEC"],
    ] {
        batch.extend(resp(&request));
    }
    client
        .write_all(&batch)
        .expect("send the transaction batch");
    let expected = "-ERR EXEC without MULTI\r\n-ERR DISCARD without MULTI\r\n+OK\r\n-ERR MULTI calls can not be nested\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n*6\r\n+OK\r\n:6\r\n:1\r\n:100\r\n+OK\r\n$1\r\n2\r\n+OK\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n*3\r\n+OK\r\n-ERR value is not an integer or out of range\r\n$3\r\nabc\r\n+OK\r\n+QUEUED\r\n-ERR wrong number of arguments for 'get' command\r\n-EXECABORT Transaction discarded because of previous errors.\r\n$-1\r\n+OK\r\n+QUEUED\r\n+QUEUED\r\n-EXECABORT Transaction discarded because it holds a command firn does not run in transactions\r\n+OK\r\n-ERR WATCH inside MULTI is not allowed\r\n*0\r\n+OK\r\n+QUEUED\r\n+OK\r\n$-1\r\n-EXECABORT Transaction discarded because of: wrong number of arguments for 'exec' command\r\n+OK\r\n+QUEUED\r\n*1\r\n:-2\r\n+OK\r\n+QUEUED\r\n-EXECABORT Transaction discarded because it holds a command firn does not run in transactions\r\n+OK\r\n-EXECABORT Transaction discarded because of: wrong number of arguments for 'exec' command\r\n+OK\r\n$1\r\nv\r\n+OK\r\n-ERR wrong number of arguments for 'watch' command\r\n-EXECABORT Transaction discarded because of previous errors.\r\n";
    expect_replies(&mut client, expected.as_bytes(), "the transaction batch");
    drop(client);
    let (status, _) = finished(child);
    assert_eq!(status, 0);
}

/// firn answers in RESP3 after HELLO 3 as Redis 7.0.15 does, and in RESP2
/// again after HELLO 2: HELLO's map with protocol 3; the null for an absent
/// string, an absent element of MGET and LPOP's count on an absent key; a map
/// for HGETALL and CONFIG GET, an empty one for an absent hash; a set for
/// SMEMBERS, set algebra and SPOP's count; doubles for scores; member and
/// score pairs nested in ZRANGE WITHSCORES, ZPOPMIN with a count and
/// HRANDFIELD WITHVALUES but not in ZPOPMIN without one; and INFO's text as a
/// verbatim string. The expected bytes follow redis-server 7.0.15's reply
/// writers (addReplyNull, addReplyNullArray, addReplyMapLen, addReplySetLen,
/// addReplyDouble, addReplyVerbatim) for the same requests.
#[cfg(target_os = "linux")]
#[test]
fn firn_answers_in_resp3_after_hello_3_as_redis_does() {
    let program = firn();
    let port = free_port();
    let text = port.to_string();
    let child = program.spawn_on_route(true, &[text.as_bytes(), b"1"]);
    let mut client = connect_when_ready(port);
    let mut batch = Vec::new();
    for request in [
        vec!["HELLO", "3"],
        vec!["GET", "absent"],
        vec!["SET", "k", "v"],
        vec!["MGET", "k", "absent"],
        vec!["LPOP", "absent", "2"],
        vec!["HSET", "h", "f", "v"],
        vec!["HGETALL", "h"],
        vec!["HGETALL", "absent"],
        vec!["HRANDFIELD", "h", "1", "WITHVALUES"],
        vec!["SADD", "s", "a"],
        vec!["SMEMBERS", "s"],
        vec!["SMEMBERS", "absent"],
        vec!["SUNION", "s", "absent"],
        vec!["ZADD", "z", "1.5", "a", "2", "b", "3", "c"],
        vec!["ZSCORE", "z", "a"],
        vec!["ZSCORE", "z", "absent"],
        vec!["ZRANGE", "z", "0", "0", "WITHSCORES"],
        vec!["ZPOPMIN", "z"],
        vec!["ZPOPMIN", "z", "1"],
        vec!["CONFIG", "GET", "appendonly"],
        vec!["INFO", "cluster"],
        vec!["SPOP", "s", "1"],
        vec!["HELLO", "2"],
        vec!["GET", "absent"],
        vec!["HGETALL", "absent"],
    ] {
        batch.extend(resp(&request));
    }
    client.write_all(&batch).expect("send the RESP3 batch");
    let hello3 = "%7\r\n$6\r\nserver\r\n$5\r\nredis\r\n$7\r\nversion\r\n$6\r\n7.0.15\r\n$5\r\nproto\r\n:3\r\n$2\r\nid\r\n:1\r\n$4\r\nmode\r\n$10\r\nstandalone\r\n$4\r\nrole\r\n$6\r\nmaster\r\n$7\r\nmodules\r\n*0\r\n";
    let hello2 = "*14\r\n$6\r\nserver\r\n$5\r\nredis\r\n$7\r\nversion\r\n$6\r\n7.0.15\r\n$5\r\nproto\r\n:2\r\n$2\r\nid\r\n:1\r\n$4\r\nmode\r\n$10\r\nstandalone\r\n$4\r\nrole\r\n$6\r\nmaster\r\n$7\r\nmodules\r\n*0\r\n";
    let expected = format!(
        "{hello3}_\r\n+OK\r\n*2\r\n$1\r\nv\r\n_\r\n_\r\n:1\r\n%1\r\n$1\r\nf\r\n$1\r\nv\r\n%0\r\n*1\r\n*2\r\n$1\r\nf\r\n$1\r\nv\r\n:1\r\n~1\r\n$1\r\na\r\n~0\r\n~1\r\n$1\r\na\r\n:3\r\n,1.5\r\n_\r\n*1\r\n*2\r\n$1\r\na\r\n,1.5\r\n*2\r\n$1\r\na\r\n,1.5\r\n*1\r\n*2\r\n$1\r\nb\r\n,2\r\n%1\r\n$10\r\nappendonly\r\n$2\r\nno\r\n=34\r\ntxt:# Cluster\r\ncluster_enabled:0\r\n\r\n~1\r\n$1\r\na\r\n{hello2}$-1\r\n*0\r\n"
    );
    expect_replies(&mut client, expected.as_bytes(), "the RESP3 batch");
    drop(client);
    let (status, _) = finished(child);
    assert_eq!(status, 0);
}

/// firn answers CLIENT INFO with the connection's line of facts in Redis
/// 7.0.15's form, a bulk string under RESP2 and a verbatim string under RESP3,
/// the name CLIENT SETNAME gave and the protocol HELLO selected in it, and
/// CLIENT INFO with an argument is Redis's arity error. The line is
/// redis-server 7.0.15's but for the fields firn leaves out, the addresses,
/// descriptor, events and buffer sizes, and for the age, 0 or 1 as the second
/// may turn between the connection and the command, and at least 2 after
/// more than two seconds, though no more than firn has run.
#[cfg(target_os = "linux")]
#[test]
fn firn_answers_client_info_as_redis_does() {
    let program = firn();
    let port = free_port();
    let text = port.to_string();
    let spawned = Instant::now();
    let child = program.spawn_on_route(true, &[text.as_bytes(), b"1"]);
    let mut client = connect_when_ready(port);
    let mut batch = Vec::new();
    for request in [
        vec!["CLIENT", "INFO"],
        vec!["CLIENT", "SETNAME", "conn-1"],
        vec!["CLIENT", "info", "x"],
        vec!["HELLO", "3"],
        vec!["CLIENT", "INFO"],
    ] {
        batch.extend(resp(&request));
    }
    client
        .write_all(&batch)
        .expect("send the CLIENT INFO batch");
    let line = |name: &str, protocol: u32| {
        format!(
            "id=1 name={name} age=0 idle=0 flags=N db=0 sub=0 psub=0 ssub=0 multi=-1 cmd=client|info user=default redir=-1 resp={protocol}\n"
        )
    };
    let first = line("", 2);
    let second = line("conn-1", 3);
    let hello3 = "%7\r\n$6\r\nserver\r\n$5\r\nredis\r\n$7\r\nversion\r\n$6\r\n7.0.15\r\n$5\r\nproto\r\n:3\r\n$2\r\nid\r\n:1\r\n$4\r\nmode\r\n$10\r\nstandalone\r\n$4\r\nrole\r\n$6\r\nmaster\r\n$7\r\nmodules\r\n*0\r\n";
    let expected = format!(
        "${}\r\n{first}\r\n+OK\r\n-ERR wrong number of arguments for 'client|info' command\r\n{hello3}={}\r\ntxt:{second}\r\n",
        first.len(),
        second.len() + 4
    );
    let mut returned = vec![0_u8; expected.len()];
    client
        .read_exact(&mut returned)
        .expect("read the CLIENT INFO batch's replies");
    let returned = String::from_utf8_lossy(&returned).replace(" age=1 ", " age=0 ");
    assert_eq!(returned, expected, "the CLIENT INFO batch");
    std::thread::sleep(Duration::from_millis(2200));
    client
        .write_all(&resp(&["CLIENT", "INFO"]))
        .expect("ask for the line after two seconds");
    let later = format!("={}\r\ntxt:{second}\r\n", second.len() + 4);
    let mut returned = vec![0_u8; later.len()];
    client
        .read_exact(&mut returned)
        .expect("read the line after two seconds");
    let returned = String::from_utf8_lossy(&returned).into_owned();
    let age = returned
        .split(" age=")
        .nth(1)
        .and_then(|rest| rest.split(' ').next())
        .and_then(|digits| digits.parse::<u64>().ok())
        .unwrap_or_else(|| panic!("no age in {returned:?}"));
    // At least 2.2 seconds passed between serving the connection and this
    // read, and fewer than firn has run, so the whole seconds between the
    // two clock readings lie in that range.
    let running = spawned.elapsed().as_secs() + 1;
    assert!(
        (2..=running).contains(&age),
        "the age after two seconds, firn having run under {running} s: {returned:?}"
    );
    assert_eq!(
        returned.replace(&format!(" age={age} "), " age=0 "),
        later,
        "the line after two seconds"
    );
    drop(client);
    let (status, _) = finished(child);
    assert_eq!(status, 0);
}

/// firn records a transaction's writes as Redis 7.0.15 propagates them: a
/// transaction of two writes bracketed in MULTI and EXEC, one of one write as
/// that write alone, and one that only reads not at all; a restart replays
/// the file to the same keys. The expected file is redis-server 7.0.15's for
/// the same requests but for the SELECT 0 it writes first.
#[cfg(target_os = "linux")]
#[test]
fn firn_records_transactions_as_redis_propagates_them() {
    let program = firn();
    let fixture = fixture_directory();
    let port = free_port();
    let text = port.to_string();
    let client = std::thread::spawn(move || {
        let mut client = connect_when_ready(port);
        let mut batch = Vec::new();
        for request in [
            vec!["MULTI"],
            vec!["SET", "a", "1"],
            vec!["SET", "b", "2"],
            vec!["EXEC"],
            vec!["MULTI"],
            vec!["SET", "c", "3"],
            vec!["EXEC"],
            vec!["MULTI"],
            vec!["GET", "a"],
            vec!["EXEC"],
            vec!["MULTI"],
            vec!["INCR", "a"],
            vec!["GET", "a"],
            vec!["EXEC"],
        ] {
            batch.extend(resp(&request));
        }
        client.write_all(&batch).expect("send the transactions");
        expect_replies(
            &mut client,
            b"+OK\r\n+QUEUED\r\n+QUEUED\r\n*2\r\n+OK\r\n+OK\r\n+OK\r\n+QUEUED\r\n*1\r\n+OK\r\n+OK\r\n+QUEUED\r\n*1\r\n$1\r\n1\r\n+OK\r\n+QUEUED\r\n+QUEUED\r\n*2\r\n:2\r\n$1\r\n2\r\n",
            "the transactions",
        );
    });
    let output = program.run(
        fixture.path(),
        &[text.as_bytes(), b"1", b"transactions.aof"],
    );
    client.join().expect("the client's exchange");
    assert!(output.status.success(), "firn: {:?}", output.status);
    let file = std::fs::read(fixture.path().join("transactions.aof")).expect("read firn's file");
    assert_eq!(
        String::from_utf8_lossy(&file),
        "*1\r\n$5\r\nMULTI\r\n*3\r\n$3\r\nSET\r\n$1\r\na\r\n$1\r\n1\r\n*3\r\n$3\r\nSET\r\n$1\r\nb\r\n$1\r\n2\r\n*1\r\n$4\r\nEXEC\r\n*3\r\n$3\r\nSET\r\n$1\r\nc\r\n$1\r\n3\r\n*2\r\n$4\r\nINCR\r\n$1\r\na\r\n",
    );
    let port = free_port();
    let text = port.to_string();
    let client = std::thread::spawn(move || {
        let mut client = connect_when_ready(port);
        client
            .write_all(&resp(&["MGET", "a", "b", "c"]))
            .expect("read the replayed keys");
        expect_replies(
            &mut client,
            b"*3\r\n$1\r\n2\r\n$1\r\n2\r\n$1\r\n3\r\n",
            "the replayed keys",
        );
    });
    let output = program.run(
        fixture.path(),
        &[text.as_bytes(), b"1", b"transactions.aof"],
    );
    client.join().expect("the replay's exchange");
    assert!(output.status.success(), "firn: {:?}", output.status);
}

/// firn runs scripts as Redis 7.0.15's EVAL, EVALSHA and SCRIPT do: a script
/// compiled by EVAL or SCRIPT LOAD is cached under its SHA1 until SCRIPT FLUSH,
/// EVALSHA finds it in either case and SCRIPT EXISTS in lower case only; KEYS
/// and ARGV reach the script; the number of keys is checked as Redis checks it;
/// a script that runs past its first budget is run again with a larger one; the
/// globals' metatable is read-only; SCRIPT's subcommands and the commands'
/// arities are answered as Redis answers them; redis.error_reply and
/// redis.pcall's argument check answer Redis's error tables; and a script's
/// result is written in the client's protocol, as luaReplyToRedisReply writes
/// it, booleans following redis.setresp, and a result that contains itself
/// ending in Redis's stack-limit error at the depth Redis reaches; redis.call
/// and redis.pcall run the commands written as parts, numbers formatted as
/// Redis formats them, an error reply raised at redis.call, at the line of the
/// call or of a rethrow, through a local alias of error included, as Redis's
/// error handler reports it, and returned by redis.pcall, Lua's pcall
/// returning an error table's err field as Redis's replacement does, and the
/// reply read in the script's protocol whatever the client's. The expected
/// bytes are redis-server 7.0.15's.
#[cfg(target_os = "linux")]
#[test]
fn firn_runs_scripts_as_redis_does() {
    let program = firn();
    let port = free_port();
    let text = port.to_string();
    let child = program.spawn_on_route(true, &[text.as_bytes(), b"1"]);
    let mut client = connect_when_ready(port);
    let sha = "1fa00e76656cc152ad327c13fe365858fd7be306";
    let upper = sha.to_ascii_uppercase();
    let mut batch = Vec::new();
    for request in [
        vec!["EVAL", "return 42", "0"],
        vec!["SCRIPT", "EXISTS", sha],
        vec!["EVALSHA", sha, "0"],
        vec!["SCRIPT", "LOAD", "return 42"],
        vec!["SCRIPT", "FLUSH", "ASYNC"],
        vec!["SCRIPT", "EXISTS", sha],
        vec!["EVALSHA", sha, "0"],
        vec![
            "EVAL",
            "return {KEYS[1],ARGV[1],false,true}",
            "1",
            "key",
            "arg",
        ],
        vec!["EVAL", "return 0", "-1"],
        vec!["EVAL", "return 0", "1"],
        vec!["EVAL", "return 0", "+0"],
        vec!["SCRIPT", "EXISTS"],
        vec!["SCRIPT", "FLUSH", "wrong"],
        vec!["SCRIPT", "FLUSH", "SYNC", "extra"],
        vec!["EVAL", "redis.setresp(3);return {false,true}", "0"],
        vec!["EVAL", "local n=0;for i=1,3000 do n=n+1 end;return n", "0"],
        vec![
            "EVAL",
            "local m=getmetatable(_G);return pcall(function()m.__index=nil end)",
            "0",
        ],
        vec!["SCRIPT", "LOAD"],
        vec!["EVALSHA", "short", "wrong"],
        vec!["EVAL", "return 42", "0"],
        vec!["EVALSHA", &upper, "0"],
        vec!["SCRIPT", "EXISTS", &upper],
        vec!["EVAL"],
        vec!["EVALSHA"],
        vec!["SCRIPT"],
        vec!["SCRIPT", "unknown"],
        vec!["EVAL", "return redis.pcall().err", "0"],
        vec![
            "EVAL",
            "return redis.error_reply('ERR \\r\\nprobe\\r\\n').err",
            "0",
        ],
        vec!["EVAL", "return redis.pcall('GET','key')", "0"],
        vec![
            "EVAL",
            "redis.call('SET',KEYS[1],ARGV[1]); return redis.call('GET',KEYS[1])",
            "1",
            "sk",
            "sv",
        ],
        vec!["EVAL", "return redis.call('INCR',KEYS[1])", "1", "counter"],
        vec![
            "EVAL",
            "return redis.call('INCRBY',KEYS[1],ARGV[1])",
            "1",
            "counter",
            "41",
        ],
        vec![
            "EVAL",
            "return redis.call('INCRBY',KEYS[1],5)",
            "1",
            "counter",
        ],
        vec!["EVAL", "return redis.pcall('INCR',KEYS[1])", "1", "sk"],
        vec!["EVAL", "return redis.call('INCR',KEYS[1])", "1", "sk"],
        vec![
            "EVAL",
            "local f = redis.call\nreturn f('INCR',KEYS[1])",
            "1",
            "sk",
        ],
        vec![
            "EVAL",
            "local _, e = xpcall(function()\n  redis.call('INCR', KEYS[1])\nend, function(e) return e end)\nerror(e, 0)",
            "1",
            "sk",
        ],
        vec!["EVAL", "local err = error\nerr('boom')", "0"],
        vec![
            "EVAL",
            "local err = error\nlocal _, e = xpcall(function()\n  redis.call('INCR', KEYS[1])\nend, function(e) return e end)\nerr(e, 0)",
            "1",
            "sk",
        ],
        vec![
            "EVAL",
            "local f = redis.call\nlocal t = {}\nreturn f('INCR', KEYS[1])",
            "1",
            "sk",
        ],
        vec![
            "EVAL",
            "local ok,e = pcall(function() error({err='ERR x'},0) end) return type(e)",
            "0",
        ],
        vec![
            "EVAL",
            "local ok,e = pcall(function() error({err='ERR x'},0) end) return e",
            "0",
        ],
        vec![
            "EVAL",
            "local ok,e = pcall(function() error({x=1},0) end) return type(e)",
            "0",
        ],
        vec![
            "EVAL",
            "return redis.call('SET',KEYS[1],'v','EX',100,'NX')",
            "1",
            "sk2",
        ],
        vec!["EVAL", "return redis.call('TTL',KEYS[1])", "1", "sk2"],
        vec!["EVAL", "return redis.call('EXPIRE',KEYS[1],50)", "1", "sk2"],
        vec!["EVAL", "return redis.call('MSET','m1','a','m2','b')", "0"],
        vec!["EVAL", "return redis.call('GET','m2')", "0"],
        vec!["EVAL", "local a={}; local b={a}; a[1]=b; return a", "0"],
        vec!["EVAL", "local a={}; a.map={k=a}; return a", "0"],
        vec!["EVAL", "local a={}; a.set={}; a.set[a]=true; return a", "0"],
        vec!["EVAL", "return {map={a=1}}", "0"],
        vec!["EVAL", "return {set={a=true}}", "0"],
        vec!["EVAL", "return {double=1.5}", "0"],
        vec!["EVAL", "return {big_number='123'}", "0"],
        vec![
            "EVAL",
            "return {verbatim_string={format='txt',string='hi'}}",
            "0",
        ],
        vec!["HELLO", "3"],
        vec!["EVAL", "return {false,true}", "0"],
        vec!["EVAL", "redis.setresp(3);return {false,true}", "0"],
        vec!["EVAL", "return {map={a=1}}", "0"],
        vec!["EVAL", "return {set={a=true}}", "0"],
        vec!["EVAL", "return {double=1.5}", "0"],
        vec!["EVAL", "return {big_number='123'}", "0"],
        vec![
            "EVAL",
            "return {verbatim_string={format='md',string='hi'}}",
            "0",
        ],
        vec!["EVAL", "return nil", "0"],
        vec!["EVAL", "return redis.call('GET',KEYS[1])", "1", "sk"],
        vec![
            "EVAL",
            "redis.setresp(3); return redis.call('GET','absent')",
            "0",
        ],
        vec!["EVAL", "return redis.call('GET','absent')", "0"],
    ] {
        batch.extend(resp(&request));
    }
    client.write_all(&batch).expect("send the scripting batch");
    let hello3 = "%7\r\n$6\r\nserver\r\n$5\r\nredis\r\n$7\r\nversion\r\n$6\r\n7.0.15\r\n$5\r\nproto\r\n:3\r\n$2\r\nid\r\n:1\r\n$4\r\nmode\r\n$10\r\nstandalone\r\n$4\r\nrole\r\n$6\r\nmaster\r\n$7\r\nmodules\r\n*0\r\n";
    // redis-server 7.0.15 writes a result that contains itself to these
    // depths before its stack-limit error; a map's last level refuses both
    // its key and its value.
    let arrays = "*1\r\n".repeat(7995);
    let maps = "*2\r\n$1\r\nk\r\n".repeat(2664) + "*2\r\n";
    let sets = "*1\r\n".repeat(2665);
    let limit = "-ERR reached lua stack limit\r\n";
    let expected = format!(
        ":42\r\n*1\r\n:1\r\n:42\r\n$40\r\n{sha}\r\n+OK\r\n*1\r\n:0\r\n-NOSCRIPT No matching script. Please use EVAL.\r\n*4\r\n$3\r\nkey\r\n$3\r\narg\r\n$-1\r\n:1\r\n-ERR Number of keys can't be negative\r\n-ERR Number of keys can't be greater than number of args\r\n-ERR value is not an integer or out of range\r\n-ERR wrong number of arguments for 'script|exists' command\r\n-ERR SCRIPT FLUSH only support SYNC|ASYNC option\r\n-ERR SCRIPT FLUSH only support SYNC|ASYNC option\r\n*2\r\n:0\r\n:1\r\n:3000\r\n$-1\r\n-ERR wrong number of arguments for 'script|load' command\r\n-NOSCRIPT No matching script. Please use EVAL.\r\n:42\r\n:42\r\n*1\r\n:0\r\n-ERR wrong number of arguments for 'eval' command\r\n-ERR wrong number of arguments for 'evalsha' command\r\n-ERR wrong number of arguments for 'script' command\r\n-ERR unknown subcommand 'unknown'. Try SCRIPT HELP.\r\n$64\r\nERR Please specify at least one argument for this redis lib call\r\n$9\r\nERR probe\r\n$-1\r\n$2\r\nsv\r\n:1\r\n:42\r\n:47\r\n-ERR value is not an integer or out of range\r\n-ERR value is not an integer or out of range script: da8455f0535fd532821b3713a4eccd80fc4b8457, on @user_script:1.\r\n-ERR value is not an integer or out of range script: 8225a61dd7e7b8c6f60bf49e74d2d38d8fbd695f, on @user_script:2.\r\n-ERR value is not an integer or out of range script: 6793b20f57c81afc3e867639a73e30ff2bd19609, on @user_script:4.\r\n-ERR user_script:2: boom script: 443852b874bae87a7a4b0129daea4709fa17a0f1, on @user_script:2.\r\n-ERR value is not an integer or out of range script: d3a069bf51964f7e9ac333589ead69a481f56199, on @user_script:5.\r\n-ERR value is not an integer or out of range script: 3676a1037fb941aa22fb13a86e521811d4392a31, on @user_script:3.\r\n$6\r\nstring\r\n$5\r\nERR x\r\n$5\r\ntable\r\n+OK\r\n:100\r\n:1\r\n+OK\r\n$1\r\nb\r\n{arrays}{limit}{maps}{limit}{limit}{sets}{limit}*2\r\n$1\r\na\r\n:1\r\n*1\r\n$1\r\na\r\n$3\r\n1.5\r\n$3\r\n123\r\n$2\r\nhi\r\n{hello3}*2\r\n_\r\n:1\r\n*2\r\n#f\r\n#t\r\n%1\r\n$1\r\na\r\n:1\r\n~1\r\n$1\r\na\r\n,1.5\r\n(123\r\n=6\r\nmd :hi\r\n_\r\n$2\r\nsv\r\n_\r\n_\r\n"
    );
    expect_replies(&mut client, expected.as_bytes(), "the scripting batch");
    drop(client);
    let (status, _) = finished(child);
    assert_eq!(status, 0);
}

/// firn's SCRIPT KILL stops a script that has written nothing at the end of
/// its current attempt and answers it Redis 7.0.15's error, naming the
/// script's SHA1 and the line it ran, and SCRIPT KILL answers NOTBUSY when no
/// script runs and Redis's arity error for an argument. SCRIPT KILL needs
/// only the scripts' pool, so it is answered while the script holds the
/// keyspace.
#[cfg(target_os = "linux")]
#[test]
fn firn_kills_a_looping_script_as_redis_does() {
    let program = firn();
    let port = free_port();
    let text = port.to_string();
    let child = program.spawn_on_route(true, &[text.as_bytes(), b"2"]);
    let mut looping = connect_when_ready(port);
    let mut other = connect_when_ready(port);
    let mut batch = resp(&["SCRIPT", "KILL"]);
    batch.extend(resp(&["SCRIPT", "KILL", "x"]));
    other
        .write_all(&batch)
        .expect("send SCRIPT KILL with no script");
    expect_replies(
        &mut other,
        b"-NOTBUSY No scripts in execution right now.\r\n-ERR wrong number of arguments for 'script|kill' command\r\n",
        "SCRIPT KILL with no script",
    );
    looping
        .write_all(&resp(&["EVAL", "while true do end", "0"]))
        .expect("start the looping script");
    std::thread::sleep(Duration::from_millis(300));
    other
        .write_all(&resp(&["SCRIPT", "KILL"]))
        .expect("kill the script");
    expect_replies(&mut other, b"+OK\r\n", "SCRIPT KILL");
    expect_replies(
        &mut looping,
        b"-ERR Script killed by user with SCRIPT KILL... script: 694a5fe1ddb97a4c6a1bf299d9537c7d3d0f84e7, on @user_script:1.\r\n",
        "the killed script",
    );
    other
        .write_all(&resp(&["SCRIPT", "KILL"]))
        .expect("kill again");
    expect_replies(
        &mut other,
        b"-NOTBUSY No scripts in execution right now.\r\n",
        "SCRIPT KILL once the script ended",
    );
    drop(looping);
    drop(other);
    let (status, _) = finished(child);
    assert_eq!(status, 0);
}

/// firn's scripts share one Lua state as Redis 7.0.15's do: the cjson
/// precision one connection's script sets reaches another connection's
/// script sent while a third connection's script runs. That script has
/// written, so it runs to its end holding the one engine and the later
/// script waits for both; with a pool of engines the later script, served
/// on another of the four drivers meanwhile, would take a second engine and
/// encode 3.14159 at the default precision of 14 digits. The expected bytes
/// are redis-server 7.0.15's (Firn-wf probe run 37487261232), the running
/// script's count its loop's.
#[cfg(target_os = "linux")]
#[test]
fn firn_scripts_share_one_lua_state_as_redis_does() {
    let program = firn();
    let port = free_port();
    let text = port.to_string();
    let child = program.spawn_on_route_with(true, &[("WF_DRIVERS", "4")], &[text.as_bytes(), b"3"]);
    let mut setter = connect_when_ready(port);
    let mut writer = connect_when_ready(port);
    let mut reader = connect_when_ready(port);
    for stream in [&writer, &reader] {
        stream
            .set_read_timeout(Some(Duration::from_secs(60)))
            .expect("bound the reads that wait for the writing script");
    }
    setter
        .write_all(&resp(&[
            "EVAL",
            "cjson.encode_number_precision(3) return 1",
            "0",
        ]))
        .expect("set the precision");
    expect_replies(&mut setter, b":1\r\n", "the precision set");
    writer
        .write_all(&resp(&[
            "EVAL",
            "redis.call('SET',KEYS[1],'1') local i = 0 while i < 10000000 do i = i + 1 end return i",
            "1",
            "written",
        ]))
        .expect("start the writing script");
    std::thread::sleep(Duration::from_millis(100));
    reader
        .write_all(&resp(&["EVAL", "return cjson.encode(3.14159)", "0"]))
        .expect("encode a number");
    expect_replies(
        &mut reader,
        b"$4\r\n3.14\r\n",
        "the number sent while a script runs",
    );
    expect_replies(&mut writer, b":10000000\r\n", "the writing script");
    drop(setter);
    drop(writer);
    drop(reader);
    let (status, _) = finished(child);
    assert_eq!(status, 0);
}

/// firn removes a key INCRBYFLOAT finds expired before it refuses an
/// increment that is not a number, as Redis 7.0.15's incrbyfloatCommand
/// looks the key up for writing first: DBSIZE, which counts entries not
/// yet removed, then counts none. A refusal that kept the entry would count
/// it while the file records its removal.
#[cfg(target_os = "linux")]
#[test]
fn firn_removes_an_expired_key_incrbyfloat_refuses_as_redis_does() {
    let program = firn();
    let port = free_port();
    let text = port.to_string();
    let child = program.spawn_on_route(true, &[text.as_bytes(), b"1"]);
    let mut client = connect_when_ready(port);
    client
        .write_all(&resp(&["SET", "lapsing", "1", "PX", "1"]))
        .expect("set a key that lapses");
    expect_replies(&mut client, b"+OK\r\n", "the key set");
    std::thread::sleep(Duration::from_millis(5));
    let mut batch = resp(&["INCRBYFLOAT", "lapsing", "bad"]);
    batch.extend(resp(&["DBSIZE"]));
    client.write_all(&batch).expect("refuse the increment");
    expect_replies(
        &mut client,
        b"-ERR value is not a valid float\r\n:0\r\n",
        "the refusal and the count",
    );
    drop(client);
    let (status, _) = finished(child);
    assert_eq!(status, 0);
}

/// firn runs the keys and strings commands written as parts, from DEL to
/// INCRBYFLOAT, inside a transaction's EXEC and through a script's
/// redis.call, and their errors through redis.pcall, as Redis 7.0.15 does:
/// the same requests, from the same keys, give the same replies whichever
/// path runs them, PING's and MSETNX's own argument checks among them,
/// which a transaction queues and EXEC answers. The expected bytes are
/// redis-server 7.0.15's (Firn-wf probe run 37490938223).
#[cfg(target_os = "linux")]
#[test]
fn firn_runs_keys_and_strings_parts_as_redis_does() {
    const VALID: &[&[&str]] = &[
        &["PING"],
        &["PING", "hello"],
        &["ECHO", "hello"],
        &["SETNX", "parts:a", "one"],
        &["SETNX", "parts:a", "two"],
        &["EXISTS", "parts:a", "parts:a", "parts:missing"],
        &["TYPE", "parts:a"],
        &["TYPE", "parts:list"],
        &["TYPE", "parts:missing"],
        &["SETEX", "parts:seconds", "60", "value"],
        &["PSETEX", "parts:millis", "60000", "value"],
        &["PERSIST", "parts:seconds"],
        &["PERSIST", "parts:seconds"],
        &["GETSET", "parts:a", "replacement"],
        &["GETDEL", "parts:a"],
        &["GETDEL", "parts:a"],
        &["SET", "parts:a", "abc"],
        &["GETEX", "parts:a", "PX", "60000"],
        &["GETEX", "parts:a", "PERSIST"],
        &["APPEND", "parts:a", "def"],
        &["SETRANGE", "parts:a", "8", "Z"],
        &["STRLEN", "parts:a"],
        &["GETRANGE", "parts:a", "-3", "-1"],
        &["MGET", "parts:a", "parts:missing", "parts:list", "parts:a"],
        &[
            "MSETNX", "parts:x", "first", "parts:x", "last", "parts:y", "other",
        ],
        &["MGET", "parts:x", "parts:y"],
        &["MSETNX", "parts:x", "rejected", "parts:z", "untouched"],
        &["EXISTS", "parts:z"],
        &["INCRBYFLOAT", "parts:number", "0.1"],
        &["INCRBYFLOAT", "parts:number", "0.2"],
        &["DEL", "parts:x", "parts:x", "parts:missing"],
        &["UNLINK", "parts:y", "parts:y"],
        &["DBSIZE"],
    ];
    const ERRORS: &[&[&str]] = &[
        &["PING", "one", "two"],
        &["MSETNX", "parts:odd", "value", "dangling"],
        &["GETEX", "parts:missing", "EX", "bad"],
        &["GETEX", "parts:list", "EX", "bad"],
        &["GETEX", "parts:a", "EX", "bad"],
        &["GETEX", "parts:a", "PXAT", "1"],
        &["SETEX", "parts:bad", "0", "value"],
        &["PSETEX", "parts:bad", "bad", "value"],
        &["GETSET", "parts:list", "value"],
        &["GETDEL", "parts:list"],
        &["APPEND", "parts:list", "value"],
        &["SETRANGE", "parts:a", "-1", "value"],
        &["SETRANGE", "parts:a", "536870912", "x"],
        &["SETRANGE", "parts:missing", "10", ""],
        &["GETRANGE", "parts:a", "bad", "1"],
        &["INCRBYFLOAT", "parts:list", "bad"],
        &["INCRBYFLOAT", "parts:number", "bad"],
        &["INCRBYFLOAT", "parts:number", "inf"],
        &["DBSIZE"],
    ];
    const TRANSACTIONS: &[u8] =
        b"+OK\r\n:1\r\n+OK\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n\
+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n\
+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n\
+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n\
+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n\
*33\r\n+PONG\r\n$5\r\nhello\r\n$5\r\nhello\r\n:1\r\n:0\r\n:2\r\n+string\r\n\
+list\r\n+none\r\n+OK\r\n+OK\r\n:1\r\n:0\r\n$3\r\none\r\n$11\r\nreplacement\r\n\
$-1\r\n+OK\r\n$3\r\nabc\r\n$3\r\nabc\r\n:6\r\n:9\r\n:9\r\n$3\r\n\x00\x00Z\r\n\
*4\r\n$9\r\nabcdef\x00\x00Z\r\n$-1\r\n$-1\r\n$9\r\nabcdef\x00\x00Z\r\n:1\r\n\
*2\r\n$4\r\nlast\r\n$5\r\nother\r\n:0\r\n:0\r\n$3\r\n0.1\r\n$3\r\n0.3\r\n:1\r\n\
:1\r\n:5\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-ERR wrong number of arguments for 'ping' command\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-ERR wrong number of arguments for 'msetnx' command\r\n+OK\r\n+QUEUED\r\n*1\r\n\
$-1\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n-ERR value is not an integer or out of range\r\n+OK\r\n\
+QUEUED\r\n*1\r\n$9\r\nabcdef\x00\x00Z\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-ERR invalid expire time in 'setex' command\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-ERR value is not an integer or out of range\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n-ERR offset is out of range\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-ERR string exceeds maximum allowed size (proto-max-bulk-len)\r\n+OK\r\n\
+QUEUED\r\n*1\r\n:0\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-ERR value is not an integer or out of range\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n-ERR value is not a valid float\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-ERR increment would produce NaN or Infinity\r\n+OK\r\n+QUEUED\r\n*1\r\n:4\r\n";
    const SCRIPTS: &[u8] =
        b"+OK\r\n:1\r\n+PONG\r\n$5\r\nhello\r\n$5\r\nhello\r\n:1\r\n:0\r\n:2\r\n\
+string\r\n+list\r\n+none\r\n+OK\r\n+OK\r\n:1\r\n:0\r\n$3\r\none\r\n$11\r\n\
replacement\r\n$-1\r\n+OK\r\n$3\r\nabc\r\n$3\r\nabc\r\n:6\r\n:9\r\n:9\r\n$3\r\n\
\x00\x00Z\r\n*4\r\n$9\r\nabcdef\x00\x00Z\r\n$-1\r\n$-1\r\n$9\r\n\
abcdef\x00\x00Z\r\n:1\r\n*2\r\n$4\r\nlast\r\n$5\r\nother\r\n:0\r\n:0\r\n$3\r\n\
0.1\r\n$3\r\n0.3\r\n:1\r\n:1\r\n:5\r\n\
-ERR wrong number of arguments for 'ping' command\r\n\
-ERR wrong number of arguments for 'msetnx' command\r\n$-1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-ERR value is not an integer or out of range\r\n$9\r\nabcdef\x00\x00Z\r\n\
-ERR invalid expire time in 'setex' command\r\n\
-ERR value is not an integer or out of range\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-ERR offset is out of range\r\n\
-ERR string exceeds maximum allowed size (proto-max-bulk-len)\r\n:0\r\n\
-ERR value is not an integer or out of range\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-ERR value is not a valid float\r\n\
-ERR increment would produce NaN or Infinity\r\n:4\r\n";
    let program = firn();
    let port = free_port();
    let text = port.to_string();
    let child = program.spawn_on_route(true, &[text.as_bytes(), b"1"]);
    let mut client = connect_when_ready(port);
    let setup: [&[&str]; 2] = [&["FLUSHALL"], &["LPUSH", "parts:list", "x"]];
    let mut transactions = Vec::new();
    for request in setup {
        transactions.extend(resp(request));
    }
    transactions.extend(resp(&["MULTI"]));
    for request in VALID {
        transactions.extend(resp(request));
    }
    transactions.extend(resp(&["EXEC"]));
    for request in ERRORS {
        transactions.extend(resp(&["MULTI"]));
        transactions.extend(resp(request));
        transactions.extend(resp(&["EXEC"]));
    }
    client
        .write_all(&transactions)
        .expect("send the transactions");
    expect_replies(&mut client, TRANSACTIONS, "the transactions");
    let mut scripts = Vec::new();
    for request in setup {
        scripts.extend(resp(request));
    }
    for (calls, script) in [
        (VALID, "return redis.call(unpack(ARGV))"),
        (ERRORS, "return redis.pcall(unpack(ARGV))"),
    ] {
        for request in calls {
            let mut call = vec!["EVAL", script, "0"];
            call.extend_from_slice(request);
            scripts.extend(resp(&call));
        }
    }
    client.write_all(&scripts).expect("send the scripts");
    expect_replies(&mut client, SCRIPTS, "the scripts");
    drop(client);
    let (status, _) = finished(child);
    assert_eq!(status, 0);
}

/// firn runs RENAME, RENAMENX and COPY, written as parts over the entries
/// of the keys they name, inside a transaction's EXEC and through a
/// script's redis.call, and their errors through redis.pcall, as Redis
/// 7.0.15 does: a key renamed onto another and onto itself, RENAMENX onto a
/// live key, COPY with and without REPLACE and DB 0, an expiry carried to
/// the new key, a list copied, and the refusals, the arity errors among
/// them aborting a transaction; a key that lapsed before the requests is
/// gone by the time RENAME names it, since EXISTS found it first, and the
/// commands meeting expired keys themselves are
/// firn_records_held_renames_and_copies_as_redis_propagates_them's. The
/// expected bytes are redis-server 7.0.15's (Firn-wf probe run 37494816783).
#[cfg(target_os = "linux")]
#[test]
fn firn_runs_rename_and_copy_parts_as_redis_does() {
    const SETUP: &[&[&str]] = &[
        &["FLUSHALL"],
        &["RPUSH", "l", "a", "b"],
        &["SET", "e", "1", "PX", "1"],
    ];
    const VALID: &[&[&str]] = &[
        &["SET", "s", "one"],
        &["RENAME", "s", "d"],
        &["GET", "d"],
        &["EXISTS", "s"],
        &["SET", "s", "two"],
        &["RENAMENX", "s", "d"],
        &["RENAMENX", "s", "n"],
        &["GET", "n"],
        &["COPY", "n", "d"],
        &["COPY", "n", "d", "REPLACE"],
        &["GET", "d"],
        &["SET", "s", "one"],
        &["RENAME", "s", "s"],
        &["RENAMENX", "s", "s"],
        &["COPY", "s", "d", "DB", "0", "REPLACE"],
        &["COPY", "s", "d", "rePlace", "db", "0", "REPLACE", "DB", "0"],
        &["SET", "t", "one", "PXAT", "4102444800000"],
        &["RENAME", "t", "t2"],
        &["PEXPIRETIME", "t2"],
        &["COPY", "t2", "t3"],
        &["PEXPIRETIME", "t3"],
        &["COPY", "l", "l2"],
        &["TYPE", "l2"],
        &["EXISTS", "e"],
    ];
    const ERRORS: &[&[&str]] = &[
        &["RENAME", "e", "f"],
        &["RENAME", "absent", "d"],
        &["RENAMENX", "absent", "d"],
        &["COPY", "absent", "d"],
        &["COPY", "s", "s"],
        &["COPY", "s", "d", "DB"],
        &["COPY", "s", "d", "UNKNOWN"],
        &["COPY", "s", "d", "DB", "bad"],
        &["COPY", "s", "d", "DB", "-1"],
        &["RENAME", "s"],
        &["RENAME", "s", "d", "extra"],
        &["RENAMENX", "s"],
        &["COPY", "s"],
    ];
    const SET_UP: &[u8] = b"+OK\r\n:2\r\n+OK\r\n";
    const TRANSACTIONS: &[u8] =
        b"+OK\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n\
+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n\
+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n\
+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n*24\r\n+OK\r\n+OK\r\n$3\r\none\r\n\
:0\r\n+OK\r\n:0\r\n:1\r\n$3\r\ntwo\r\n:0\r\n:1\r\n$3\r\ntwo\r\n+OK\r\n+OK\r\n\
:0\r\n:1\r\n:1\r\n+OK\r\n+OK\r\n:4102444800000\r\n:1\r\n:4102444800000\r\n:1\r\n\
+list\r\n:0\r\n+OK\r\n+QUEUED\r\n*1\r\n-ERR no such key\r\n+OK\r\n+QUEUED\r\n\
*1\r\n-ERR no such key\r\n+OK\r\n+QUEUED\r\n*1\r\n-ERR no such key\r\n+OK\r\n\
+QUEUED\r\n*1\r\n:0\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-ERR source and destination objects are the same\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-ERR syntax error\r\n+OK\r\n+QUEUED\r\n*1\r\n-ERR syntax error\r\n+OK\r\n\
+QUEUED\r\n*1\r\n-ERR value is not an integer or out of range\r\n+OK\r\n\
+QUEUED\r\n*1\r\n-ERR DB index is out of range\r\n+OK\r\n\
-ERR wrong number of arguments for 'rename' command\r\n\
-EXECABORT Transaction discarded because of previous errors.\r\n+OK\r\n\
-ERR wrong number of arguments for 'rename' command\r\n\
-EXECABORT Transaction discarded because of previous errors.\r\n+OK\r\n\
-ERR wrong number of arguments for 'renamenx' command\r\n\
-EXECABORT Transaction discarded because of previous errors.\r\n+OK\r\n\
-ERR wrong number of arguments for 'copy' command\r\n\
-EXECABORT Transaction discarded because of previous errors.\r\n";
    const SCRIPTS: &[u8] =
        b"+OK\r\n+OK\r\n$3\r\none\r\n:0\r\n+OK\r\n:0\r\n:1\r\n$3\r\ntwo\r\n:0\r\n:1\r\n\
$3\r\ntwo\r\n+OK\r\n+OK\r\n:0\r\n:1\r\n:1\r\n+OK\r\n+OK\r\n:4102444800000\r\n\
:1\r\n:4102444800000\r\n:1\r\n+list\r\n:0\r\n-ERR no such key\r\n\
-ERR no such key\r\n-ERR no such key\r\n:0\r\n\
-ERR source and destination objects are the same\r\n-ERR syntax error\r\n\
-ERR syntax error\r\n-ERR value is not an integer or out of range\r\n\
-ERR DB index is out of range\r\n\
-ERR Wrong number of args calling Redis command from script\r\n\
-ERR Wrong number of args calling Redis command from script\r\n\
-ERR Wrong number of args calling Redis command from script\r\n\
-ERR Wrong number of args calling Redis command from script\r\n";
    let program = firn();
    let port = free_port();
    let text = port.to_string();
    let child = program.spawn_on_route(true, &[text.as_bytes(), b"1"]);
    let mut client = connect_when_ready(port);
    let mut transactions = vec![resp(&["MULTI"])];
    for request in VALID {
        transactions.push(resp(request));
    }
    transactions.push(resp(&["EXEC"]));
    for request in ERRORS {
        transactions.push(resp(&["MULTI"]));
        transactions.push(resp(request));
        transactions.push(resp(&["EXEC"]));
    }
    let mut scripts = Vec::new();
    for (calls, script) in [
        (VALID, "return redis.call(unpack(ARGV))"),
        (ERRORS, "return redis.pcall(unpack(ARGV))"),
    ] {
        for request in calls {
            let mut call = vec!["EVAL", script, "0"];
            call.extend_from_slice(request);
            scripts.push(resp(&call));
        }
    }
    for (requests, expected, what) in [
        (transactions, TRANSACTIONS, "the transactions"),
        (scripts, SCRIPTS, "the scripts"),
    ] {
        let set_up: Vec<u8> = SETUP.iter().flat_map(|request| resp(request)).collect();
        client.write_all(&set_up).expect("set the keys up");
        expect_replies(&mut client, SET_UP, "the keys set up");
        std::thread::sleep(Duration::from_millis(50));
        client
            .write_all(&requests.concat())
            .expect("send the requests");
        expect_replies(&mut client, expected, what);
    }
    drop(client);
    let (status, _) = finished(child);
    assert_eq!(status, 0);
}

/// The commands of an append-only file, each its arguments.
#[cfg(target_os = "linux")]
fn file_records(mut bytes: &[u8]) -> Vec<Vec<Vec<u8>>> {
    fn line(bytes: &mut &[u8]) -> usize {
        let rest: &[u8] = *bytes;
        let end = rest
            .windows(2)
            .position(|pair| pair == b"\r\n")
            .expect("a record's line ends");
        let count = std::str::from_utf8(&rest[1..end])
            .expect("a count in ASCII")
            .parse()
            .expect("a count");
        *bytes = &rest[end + 2..];
        count
    }
    let mut records = Vec::new();
    while !bytes.is_empty() {
        let count = line(&mut bytes);
        let mut record = Vec::new();
        for _ in 0..count {
            let length = line(&mut bytes);
            record.push(bytes[..length].to_vec());
            bytes = &bytes[length + 2..];
        }
        records.push(record);
    }
    records
}

/// Runs commands that meet keys found expired inside a transaction or a
/// script and checks their replies and the records firn appends after a
/// file that loads those keys. `loaded` sets the keys, those to meet
/// expired with `PXAT 1`, an expiry the replay keeps as reached; `records`
/// is what redis-server 7.0.15 appends for the same keys expiring under it
/// with active expiry off, less the SELECT it begins with. firn's active
/// expiry first sweeps 100 ms after it starts; when that sweep removed the
/// keys before the commands met them, the sweep's DEL records stand outside
/// any MULTI and EXEC for keys Redis removes inside one, and the run starts
/// over, up to five times. The replies, which can name a key the sweep
/// removed, are compared on a run whose records match. A held command that
/// records such a removal outside its block shows the same form on every
/// run and fails.
#[cfg(target_os = "linux")]
fn check_held_records(
    loaded: &[&[&str]],
    requests: &[&[&str]],
    replies: &[u8],
    records: &[u8],
) {
    let program = firn();
    let loaded: Vec<u8> = loaded.iter().flat_map(|request| resp(request)).collect();
    let mut batch: Vec<u8> = requests.iter().flat_map(|request| resp(request)).collect();
    batch.extend(resp(&["QUIT"]));
    for _ in 0..5 {
        let fixture = fixture_directory();
        std::fs::write(fixture.path().join("held.aof"), &loaded).expect("write the loaded file");
        let port = free_port();
        let text = port.to_string();
        let sent = batch.clone();
        let client = std::thread::spawn(move || {
            let mut client = connect_when_ready(port);
            client.write_all(&sent).expect("send the commands");
            let mut returned = Vec::new();
            client.read_to_end(&mut returned).expect("read the replies");
            returned
        });
        let output = program.run(fixture.path(), &[text.as_bytes(), b"1", b"held.aof"]);
        let returned = client.join().expect("the client's exchange");
        assert!(output.status.success(), "firn: {:?}", output.status);
        let file = std::fs::read(fixture.path().join("held.aof")).expect("read firn's file");
        let recorded = file
            .strip_prefix(loaded.as_slice())
            .expect("the loaded records kept at the file's start");
        if recorded == records {
            assert_eq!(
                String::from_utf8_lossy(&returned),
                String::from_utf8_lossy(&[replies, b"+OK\r\n"].concat()),
                "the commands"
            );
            return;
        }
        let removals = |bytes: &[u8], within: bool| -> Vec<Vec<u8>> {
            let mut inside = false;
            let mut keys = Vec::new();
            for record in file_records(bytes) {
                match record[0].as_slice() {
                    b"MULTI" => inside = true,
                    b"EXEC" => inside = false,
                    b"DEL" if inside == within => keys.push(record[1].clone()),
                    _ => {}
                }
            }
            keys
        };
        let met = removals(records, true);
        let swept = removals(recorded, false)
            .iter()
            .any(|key| met.contains(key));
        assert!(
            swept,
            "the records:\n{}\nwhere Redis appends:\n{}",
            String::from_utf8_lossy(recorded),
            String::from_utf8_lossy(records)
        );
    }
    panic!("active expiry removed the loaded keys before the commands met them in five runs");
}

/// firn records RENAME and COPY run inside a transaction's EXEC and a
/// script as Redis 7.0.15 propagates them over keys found expired: each
/// expired key the commands find is removed and recorded as DEL where the
/// command finds it, inside the transaction's MULTI and EXEC or the
/// script's, with the commands that ran; a refused RENAME records only its
/// source's removal. The expected records are redis-server 7.0.15's
/// (Firn-wf probe run 37499574103).
#[cfg(target_os = "linux")]
#[test]
fn firn_records_held_renames_and_copies_as_redis_propagates_them() {
    check_held_records(
        &[
            &["SET", "a", "1"],
            &["SET", "b", "2", "PXAT", "1"],
            &["SET", "c", "3", "PXAT", "1"],
            &["SET", "e", "5"],
            &["SET", "f", "6", "PXAT", "1"],
            &["SET", "g", "7", "PXAT", "1"],
        ],
        &[
            &["MULTI"],
            &["RENAME", "a", "b"],
            &["RENAME", "c", "d"],
            &["COPY", "e", "f"],
            &["EXEC"],
            &[
                "EVAL",
                "redis.call('RENAME', KEYS[1], KEYS[2]) return redis.pcall('COPY', KEYS[3], KEYS[4])",
                "4",
                "b",
                "x",
                "g",
                "y",
            ],
            &["MGET", "a", "b", "c", "d", "e", "f", "g", "x", "y"],
        ],
        b"+OK\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n*3\r\n+OK\r\n-ERR no such key\r\n:1\r\n:0\r\n*9\r\n$-1\r\n$-1\r\n$-1\r\n$-1\r\n$1\r\n5\r\n$1\r\n5\r\n$-1\r\n$1\r\n1\r\n$-1\r\n",
        b"*1\r\n$5\r\nMULTI\r\n*2\r\n$3\r\nDEL\r\n$1\r\nb\r\n*3\r\n$6\r\nRENAME\r\n$1\r\na\r\n$1\r\nb\r\n*2\r\n$3\r\nDEL\r\n$1\r\nc\r\n*2\r\n$3\r\nDEL\r\n$1\r\nf\r\n*3\r\n$4\r\nCOPY\r\n$1\r\ne\r\n$1\r\nf\r\n*1\r\n$4\r\nEXEC\r\n*1\r\n$5\r\nMULTI\r\n*3\r\n$6\r\nRENAME\r\n$1\r\nb\r\n$1\r\nx\r\n*2\r\n$3\r\nDEL\r\n$1\r\ng\r\n*1\r\n$4\r\nEXEC\r\n",
    );
}

/// firn runs the hashes commands written as parts, HSET to HRANDFIELD, inside a transaction's EXEC and through a script as Redis 7.0.15 does, singleton hashes keeping HRANDFIELD's and HGETALL's replies in one order. The same requests, from the same keys, run in one transaction,
/// each error or boundary request in a transaction of its own, then through
/// a script's redis.call and the errors through redis.pcall, and give the
/// replies redis-server 7.0.15 gives (Firn-wf probe run 37499945138).
#[cfg(target_os = "linux")]
#[test]
fn firn_runs_hashes_parts_as_redis_does() {
    const SETUP: &[&[&str]] = &[
        &["FLUSHALL"],
        &["SET", "wrong", "text"],
        &["HSET", "h", "f", "7"],
        &["HSET", "one", "f", "value"],
        &["HSET", "erase", "f", "value"],
        &[
            "HSET",
            "bad",
            "integer",
            "1.5",
            "float",
            "nope",
            "max",
            "9223372036854775807",
            "min",
            "-9223372036854775808",
            "huge",
            "1e4932",
        ],
        &["HSET", "timed", "f", "1"],
        &["PEXPIREAT", "timed", "4102444800000"],
    ];
    const VALID: &[&[&str]] = &[
        &["HSET", "h", "f", "8", "f", "9"],
        &["HMSET", "h", "f", "10"],
        &["HSETNX", "h", "f", "ignored"],
        &["HSETNX", "nx", "f", "created"],
        &["HGET", "h", "f"],
        &["HMGET", "h", "f", "absent", "f"],
        &["HEXISTS", "h", "f"],
        &["HSTRLEN", "h", "f"],
        &["HLEN", "h"],
        &["HGETALL", "one"],
        &["HKEYS", "one"],
        &["HVALS", "one"],
        &["HINCRBY", "h", "f", "-2"],
        &["HINCRBY", "counter", "f", "5"],
        &["HINCRBYFLOAT", "floating", "f", "0.1"],
        &["HINCRBYFLOAT", "floating", "f", "0.2"],
        &["HGET", "floating", "f"],
        &["HRANDFIELD", "one"],
        &["HRANDFIELD", "one", "9", "WITHVALUES"],
        &["HRANDFIELD", "one", "-2", "WITHVALUES"],
        &["HRANDFIELD", "one", "0"],
        &["HDEL", "erase", "f", "f", "absent"],
        &["EXISTS", "erase"],
        &["HGET", "missing", "f"],
        &["HMGET", "missing", "f", "g"],
        &["HEXISTS", "missing", "f"],
        &["HSTRLEN", "missing", "f"],
        &["HLEN", "missing"],
        &["HGETALL", "missing"],
        &["HRANDFIELD", "missing"],
        &["HRANDFIELD", "missing", "2", "WITHVALUES"],
        &["HSET", "binary", "", "", "f\0x", "v\0x"],
        &["HMGET", "binary", "", "f\0x"],
        &["HSET", "timed", "f", "2"],
        &["HINCRBY", "timed", "f", "1"],
        &["HINCRBYFLOAT", "timed", "f", "0.5"],
        &["PEXPIRETIME", "timed"],
    ];
    const ERRORS: &[&[&str]] = &[
        &["HSET", "wrong", "f", "v"],
        &["HMSET", "wrong", "f", "v"],
        &["HSETNX", "wrong", "f", "v"],
        &["HGET", "wrong", "f"],
        &["HMGET", "wrong", "f", "g"],
        &["HDEL", "wrong", "f"],
        &["HEXISTS", "wrong", "f"],
        &["HSTRLEN", "wrong", "f"],
        &["HLEN", "wrong"],
        &["HGETALL", "wrong"],
        &["HKEYS", "wrong"],
        &["HVALS", "wrong"],
        &["HINCRBY", "wrong", "f", "1"],
        &["HINCRBYFLOAT", "wrong", "f", "1"],
        &["HRANDFIELD", "wrong", "0"],
        &["HSET", "h", "f", "v", "unpaired"],
        &["HMSET", "h", "f", "v", "unpaired"],
        &["HINCRBY", "wrong", "f", "nope"],
        &["HINCRBY", "bad", "integer", "1"],
        &["HINCRBY", "bad", "max", "1"],
        &["HINCRBY", "bad", "min", "-1"],
        &["HINCRBYFLOAT", "wrong", "f", "nope"],
        &["HINCRBYFLOAT", "wrong", "f", "inf"],
        &["HINCRBYFLOAT", "bad", "float", "1"],
        &["HINCRBYFLOAT", "bad", "huge", "1e4932"],
        &["HRANDFIELD", "wrong", "nope", "bad"],
        &["HRANDFIELD", "one", "-9223372036854775808"],
        &["HRANDFIELD", "one", "9223372036854775808"],
        &["HRANDFIELD", "one", "1", "bad"],
        &["HRANDFIELD", "one", "1", "WITHVALUES", "extra"],
        &["HRANDFIELD", "one", "4611686018427387904", "WITHVALUES"],
        &["HRANDFIELD", "one", "-4611686018427387904", "WITHVALUES"],
    ];
    const TRANSACTIONS: &[u8] =
        b"+OK\r\n+OK\r\n:1\r\n:1\r\n:1\r\n:5\r\n:1\r\n:1\r\n+OK\r\n+QUEUED\r\n+QUEUED\r\n\
+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n\
+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n\
+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n\
+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n\
+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n\
*37\r\n:0\r\n+OK\r\n:0\r\n:1\r\n$2\r\n10\r\n*3\r\n$2\r\n10\r\n$-1\r\n$2\r\n\
10\r\n:1\r\n:2\r\n:1\r\n*2\r\n$1\r\nf\r\n$5\r\nvalue\r\n*1\r\n$1\r\nf\r\n*1\r\n\
$5\r\nvalue\r\n:8\r\n:5\r\n$3\r\n0.1\r\n$3\r\n0.3\r\n$3\r\n0.3\r\n$1\r\nf\r\n\
*2\r\n$1\r\nf\r\n$5\r\nvalue\r\n*4\r\n$1\r\nf\r\n$5\r\nvalue\r\n$1\r\nf\r\n\
$5\r\nvalue\r\n*0\r\n:1\r\n:0\r\n$-1\r\n*2\r\n$-1\r\n$-1\r\n:0\r\n:0\r\n:0\r\n\
*0\r\n$-1\r\n*0\r\n:2\r\n*2\r\n$0\r\n\r\n$3\r\nv\x00x\r\n:0\r\n:3\r\n$3\r\n\
3.5\r\n:4102444800000\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n-ERR wrong number of arguments for 'hset' command\r\n+OK\r\n\
+QUEUED\r\n*1\r\n-ERR wrong number of arguments for 'hmset' command\r\n+OK\r\n\
+QUEUED\r\n*1\r\n-ERR value is not an integer or out of range\r\n+OK\r\n\
+QUEUED\r\n*1\r\n-ERR hash value is not an integer\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-ERR increment or decrement would overflow\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-ERR increment or decrement would overflow\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-ERR value is not a valid float\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-ERR value is NaN or Infinity\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-ERR hash value is not a float\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-ERR increment would produce NaN or Infinity\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-ERR value is not an integer or out of range\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-ERR value is out of range, value must between -9223372036854775807 and 9223372036854775807\r\n\
+OK\r\n+QUEUED\r\n*1\r\n-ERR value is not an integer or out of range\r\n+OK\r\n\
+QUEUED\r\n*1\r\n-ERR syntax error\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-ERR syntax error\r\n+OK\r\n+QUEUED\r\n*1\r\n-ERR value is out of range\r\n\
+OK\r\n+QUEUED\r\n*1\r\n-ERR value is out of range\r\n";
    const SCRIPTS: &[u8] =
        b"+OK\r\n+OK\r\n:1\r\n:1\r\n:1\r\n:5\r\n:1\r\n:1\r\n:0\r\n+OK\r\n:0\r\n:1\r\n\
$2\r\n10\r\n*3\r\n$2\r\n10\r\n$-1\r\n$2\r\n10\r\n:1\r\n:2\r\n:1\r\n*2\r\n$1\r\n\
f\r\n$5\r\nvalue\r\n*1\r\n$1\r\nf\r\n*1\r\n$5\r\nvalue\r\n:8\r\n:5\r\n$3\r\n\
0.1\r\n$3\r\n0.3\r\n$3\r\n0.3\r\n$1\r\nf\r\n*2\r\n$1\r\nf\r\n$5\r\nvalue\r\n\
*4\r\n$1\r\nf\r\n$5\r\nvalue\r\n$1\r\nf\r\n$5\r\nvalue\r\n*0\r\n:1\r\n:0\r\n\
$-1\r\n*2\r\n$-1\r\n$-1\r\n:0\r\n:0\r\n:0\r\n*0\r\n$-1\r\n*0\r\n:2\r\n*2\r\n\
$0\r\n\r\n$3\r\nv\x00x\r\n:0\r\n:3\r\n$3\r\n3.5\r\n:4102444800000\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-ERR wrong number of arguments for 'hset' command\r\n\
-ERR wrong number of arguments for 'hmset' command\r\n\
-ERR value is not an integer or out of range\r\n\
-ERR hash value is not an integer\r\n\
-ERR increment or decrement would overflow\r\n\
-ERR increment or decrement would overflow\r\n\
-ERR value is not a valid float\r\n-ERR value is NaN or Infinity\r\n\
-ERR hash value is not a float\r\n\
-ERR increment would produce NaN or Infinity\r\n\
-ERR value is not an integer or out of range\r\n\
-ERR value is out of range, value must between -9223372036854775807 and 9223372036854775807\r\n\
-ERR value is not an integer or out of range\r\n-ERR syntax error\r\n\
-ERR syntax error\r\n-ERR value is out of range\r\n\
-ERR value is out of range\r\n";
    let program = firn();
    let port = free_port();
    let text = port.to_string();
    let child = program.spawn_on_route(true, &[text.as_bytes(), b"1"]);
    let mut client = connect_when_ready(port);
    let mut transactions: Vec<Vec<u8>> = SETUP.iter().map(|request| resp(request)).collect();
    transactions.push(resp(&["MULTI"]));
    for request in VALID {
        transactions.push(resp(request));
    }
    transactions.push(resp(&["EXEC"]));
    for request in ERRORS {
        transactions.push(resp(&["MULTI"]));
        transactions.push(resp(request));
        transactions.push(resp(&["EXEC"]));
    }
    let mut scripts: Vec<Vec<u8>> = SETUP.iter().map(|request| resp(request)).collect();
    for (calls, script) in [
        (VALID, "return redis.call(unpack(ARGV))"),
        (ERRORS, "return redis.pcall(unpack(ARGV))"),
    ] {
        for request in calls {
            let mut call = vec!["EVAL", script, "0"];
            call.extend_from_slice(request);
            scripts.push(resp(&call));
        }
    }
    for (requests, expected, what) in [
        (transactions, TRANSACTIONS, "the transactions"),
        (scripts, SCRIPTS, "the scripts"),
    ] {
        client
            .write_all(&requests.concat())
            .expect("send the requests");
        expect_replies(&mut client, expected, what);
    }
    drop(client);
    let (status, _) = finished(child);
    assert_eq!(status, 0);
}

/// firn records the hashes commands run inside a transaction's EXEC and a script as Redis 7.0.15 propagates them over hashes found expired: a read records the removal alone, a write the removal and then itself, HINCRBYFLOAT as HSET with the sum, inside the transaction's MULTI and EXEC, and a script whose one effect is a removal records it bare. The expected
/// records are redis-server 7.0.15's (Firn-wf probe run 37504720562).
#[cfg(target_os = "linux")]
#[test]
fn firn_records_held_hashes_commands_as_redis_propagates_them() {
    check_held_records(
        &[
            &["HSET", "h1", "f", "1"],
            &["PEXPIREAT", "h1", "1"],
            &["HSET", "h2", "f", "2"],
            &["PEXPIREAT", "h2", "1"],
            &["HSET", "h3", "f", "3"],
            &["PEXPIREAT", "h3", "1"],
            &["HSET", "h4", "f", "4"],
            &["PEXPIREAT", "h4", "1"],
        ],
        &[
            &["MULTI"],
            &["HGET", "h1", "f"],
            &["HSET", "h2", "g", "v"],
            &["HINCRBYFLOAT", "h3", "f", "1.5"],
            &["EXEC"],
            &["EVAL", "return redis.call('HDEL', KEYS[1], 'f')", "1", "h4"],
            &["HGETALL", "h2"],
            &["HGETALL", "h3"],
            &["DBSIZE"],
        ],
        b"+OK\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n*3\r\n$-1\r\n:1\r\n$3\r\n1.5\r\n:0\r\n*2\r\n$1\r\ng\r\n$1\r\nv\r\n*2\r\n$1\r\nf\r\n$3\r\n1.5\r\n:2\r\n",
        b"*1\r\n$5\r\nMULTI\r\n*2\r\n$3\r\nDEL\r\n$2\r\nh1\r\n*2\r\n$3\r\nDEL\r\n$2\r\nh2\r\n*4\r\n$4\r\nHSET\r\n$2\r\nh2\r\n$1\r\ng\r\n$1\r\nv\r\n*2\r\n$3\r\nDEL\r\n$2\r\nh3\r\n*4\r\n$4\r\nHSET\r\n$2\r\nh3\r\n$1\r\nf\r\n$3\r\n1.5\r\n*1\r\n$4\r\nEXEC\r\n*2\r\n$3\r\nDEL\r\n$2\r\nh4\r\n",
    );
}

/// firn runs the lists commands written as parts, LPUSH to RPOPLPUSH, inside a transaction's EXEC and through a script as Redis 7.0.15 does, LMOVE and RPOPLPUSH over one key and two among them. The same requests, from the same keys, run in one transaction,
/// each error or boundary request in a transaction of its own, then through
/// a script's redis.call and the errors through redis.pcall, and give the
/// replies redis-server 7.0.15 gives (Firn-wf probe run 37499945138).
#[cfg(target_os = "linux")]
#[test]
fn firn_runs_lists_parts_as_redis_does() {
    const SETUP: &[&[&str]] = &[
        &["FLUSHALL"],
        &["SET", "wrong", "text"],
        &["RPUSH", "a", "a", "b", "a", "c"],
        &["RPUSH", "b", "x", "y"],
        &["RPUSH", "err", "a", "b", "a"],
        &["RPUSH", "one", "z"],
        &["RPUSH", "trim", "a", "b", "c"],
        &["RPUSH", "rem", "a", "a", "b", "a"],
    ];
    const VALID: &[&[&str]] = &[
        &["LPUSH", "a", "head", "head2"],
        &["RPUSH", "a", "tail"],
        &["LPUSHX", "a", "hx"],
        &["RPUSHX", "a", "tx"],
        &["LPUSHX", "missing", "x"],
        &["RPUSHX", "missing", "x"],
        &["LRANGE", "a", "0", "-1"],
        &["LLEN", "a"],
        &["LINDEX", "a", "-1"],
        &["LSET", "a", "-1", "changed"],
        &["LINSERT", "a", "BEFORE", "b", "before"],
        &["LINSERT", "a", "AFTER", "b", "after"],
        &["LPOS", "a", "a"],
        &["LPOS", "a", "a", "RANK", "-1", "COUNT", "0", "MAXLEN", "0"],
        &["LREM", "rem", "-1", "a"],
        &["LTRIM", "trim", "1", "-1"],
        &["LPOP", "a"],
        &["RPOP", "a"],
        &["LPOP", "a", "0"],
        &["RPOP", "a", "2"],
        &["LMOVE", "a", "b", "LEFT", "RIGHT"],
        &["LMOVE", "b", "b", "RIGHT", "LEFT"],
        &["LMOVE", "b", "b", "LEFT", "LEFT"],
        &["LMOVE", "b", "b", "RIGHT", "RIGHT"],
        &["RPOPLPUSH", "b", "a"],
        &["RPOPLPUSH", "one", "one"],
        &["LRANGE", "b", "0", "-1"],
        &["LLEN", "one"],
        &["LPOP", "one", "10"],
        &["EXISTS", "one"],
        &["LPOP", "missing", "0"],
        &["RPOP", "missing"],
        &["LMOVE", "missing", "wrong", "LEFT", "LEFT"],
        &["LINSERT", "missing", "BEFORE", "p", "v"],
        &["LTRIM", "missing", "0", "-1"],
        &["LPOS", "missing", "a", "COUNT", "0"],
    ];
    const ERRORS: &[&[&str]] = &[
        &["LPUSH", "wrong", "x"],
        &["RPUSHX", "wrong", "x"],
        &["LPOP", "wrong"],
        &["RPOP", "wrong", "0"],
        &["LPOP", "err", "-1"],
        &["RPOP", "err", "bad"],
        &["LPOP", "err", "9223372036854775808"],
        &["LPOP", "err", "1", "extra"],
        &["LRANGE", "wrong", "0", "-1"],
        &["LRANGE", "missing", "bad", "-1"],
        &["LLEN", "wrong"],
        &["LINDEX", "missing", "bad"],
        &["LINDEX", "wrong", "bad"],
        &["LINDEX", "err", "bad"],
        &["LINDEX", "err", "-9223372036854775808"],
        &["LSET", "missing", "bad", "x"],
        &["LSET", "wrong", "bad", "x"],
        &["LSET", "err", "bad", "x"],
        &["LSET", "err", "99", "x"],
        &["LREM", "wrong", "0", "a"],
        &["LREM", "missing", "bad", "a"],
        &["LTRIM", "wrong", "0", "-1"],
        &["LTRIM", "missing", "0", "bad"],
        &["LINSERT", "wrong", "BEFORE", "a", "x"],
        &["LINSERT", "missing", "SIDEWAYS", "a", "x"],
        &["LINSERT", "err", "AFTER", "absent-pivot", "x"],
        &["LPOS", "wrong", "a"],
        &["LPOS", "err", "a", "RANK", "0"],
        &["LPOS", "err", "a", "RANK", "bad"],
        &["LPOS", "err", "a", "COUNT", "-1"],
        &["LPOS", "err", "a", "COUNT", "bad"],
        &["LPOS", "err", "a", "MAXLEN", "-1"],
        &["LPOS", "err", "a", "MAXLEN", "bad"],
        &["LPOS", "err", "a", "COUNT"],
        &["LPOS", "err", "a", "UNKNOWN", "1"],
        &[
            "LPOS",
            "err",
            "a",
            "RANK",
            "-9223372036854775808",
            "COUNT",
            "1",
        ],
        &["LMOVE", "err", "wrong", "LEFT", "RIGHT"],
        &["LMOVE", "wrong", "err", "LEFT", "RIGHT"],
        &["LMOVE", "missing", "err", "BAD", "RIGHT"],
        &["LMOVE", "err", "err", "LEFT", "BAD"],
        &["RPOPLPUSH", "err", "wrong"],
        &["RPOPLPUSH", "wrong", "err"],
    ];
    const TRANSACTIONS: &[u8] = b"+OK\r\n+OK\r\n:4\r\n:2\r\n:3\r\n:1\r\n:3\r\n:4\r\n+OK\r\n+QUEUED\r\n+QUEUED\r\n\
+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n\
+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n\
+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n\
+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n\
+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n*36\r\n:6\r\n\
:7\r\n:8\r\n:9\r\n:0\r\n:0\r\n*9\r\n$2\r\nhx\r\n$5\r\nhead2\r\n$4\r\nhead\r\n\
$1\r\na\r\n$1\r\nb\r\n$1\r\na\r\n$1\r\nc\r\n$4\r\ntail\r\n$2\r\ntx\r\n:9\r\n\
$2\r\ntx\r\n+OK\r\n:10\r\n:11\r\n:3\r\n*2\r\n:7\r\n:3\r\n:1\r\n+OK\r\n$2\r\n\
hx\r\n$7\r\nchanged\r\n*0\r\n*2\r\n$4\r\ntail\r\n$1\r\nc\r\n$5\r\nhead2\r\n\
$5\r\nhead2\r\n$5\r\nhead2\r\n$1\r\ny\r\n$1\r\ny\r\n$1\r\nz\r\n*2\r\n$5\r\n\
head2\r\n$1\r\nx\r\n:1\r\n*1\r\n$1\r\nz\r\n:0\r\n*-1\r\n$-1\r\n$-1\r\n:0\r\n\
+OK\r\n*0\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n-ERR value is out of range, must be positive\r\n+OK\r\n\
+QUEUED\r\n*1\r\n-ERR value is out of range, must be positive\r\n+OK\r\n\
+QUEUED\r\n*1\r\n-ERR value is out of range, must be positive\r\n+OK\r\n\
+QUEUED\r\n*1\r\n-ERR wrong number of arguments for 'lpop' command\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n-ERR value is not an integer or out of range\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n$-1\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n-ERR value is not an integer or out of range\r\n+OK\r\n\
+QUEUED\r\n*1\r\n$-1\r\n+OK\r\n+QUEUED\r\n*1\r\n-ERR no such key\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n-ERR value is not an integer or out of range\r\n+OK\r\n\
+QUEUED\r\n*1\r\n-ERR index out of range\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n-ERR value is not an integer or out of range\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n-ERR value is not an integer or out of range\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n-ERR syntax error\r\n+OK\r\n+QUEUED\r\n*1\r\n:-1\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-ERR RANK can't be zero: use 1 to start from the first match, 2 from the second ... or use negative to start from the end of the list\r\n\
+OK\r\n+QUEUED\r\n*1\r\n-ERR value is not an integer or out of range\r\n+OK\r\n\
+QUEUED\r\n*1\r\n-ERR COUNT can't be negative\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-ERR COUNT can't be negative\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-ERR MAXLEN can't be negative\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-ERR MAXLEN can't be negative\r\n+OK\r\n+QUEUED\r\n*1\r\n-ERR syntax error\r\n\
+OK\r\n+QUEUED\r\n*1\r\n-ERR syntax error\r\n+OK\r\n+QUEUED\r\n*1\r\n*2\r\n\
:2\r\n:0\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n-ERR syntax error\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-ERR syntax error\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n";
    const SCRIPTS: &[u8] = b"+OK\r\n+OK\r\n:4\r\n:2\r\n:3\r\n:1\r\n:3\r\n:4\r\n:6\r\n:7\r\n:8\r\n:9\r\n:0\r\n\
:0\r\n*9\r\n$2\r\nhx\r\n$5\r\nhead2\r\n$4\r\nhead\r\n$1\r\na\r\n$1\r\nb\r\n\
$1\r\na\r\n$1\r\nc\r\n$4\r\ntail\r\n$2\r\ntx\r\n:9\r\n$2\r\ntx\r\n+OK\r\n:10\r\n\
:11\r\n:3\r\n*2\r\n:7\r\n:3\r\n:1\r\n+OK\r\n$2\r\nhx\r\n$7\r\nchanged\r\n*0\r\n\
*2\r\n$4\r\ntail\r\n$1\r\nc\r\n$5\r\nhead2\r\n$5\r\nhead2\r\n$5\r\nhead2\r\n\
$1\r\ny\r\n$1\r\ny\r\n$1\r\nz\r\n*2\r\n$5\r\nhead2\r\n$1\r\nx\r\n:1\r\n*1\r\n\
$1\r\nz\r\n:0\r\n$-1\r\n$-1\r\n$-1\r\n:0\r\n+OK\r\n*0\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-ERR value is out of range, must be positive\r\n\
-ERR value is out of range, must be positive\r\n\
-ERR value is out of range, must be positive\r\n\
-ERR wrong number of arguments for 'lpop' command\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-ERR value is not an integer or out of range\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n$-1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-ERR value is not an integer or out of range\r\n$-1\r\n-ERR no such key\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-ERR value is not an integer or out of range\r\n-ERR index out of range\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-ERR value is not an integer or out of range\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-ERR value is not an integer or out of range\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-ERR syntax error\r\n:-1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-ERR RANK can't be zero: use 1 to start from the first match, 2 from the second ... or use negative to start from the end of the list\r\n\
-ERR value is not an integer or out of range\r\n-ERR COUNT can't be negative\r\n\
-ERR COUNT can't be negative\r\n-ERR MAXLEN can't be negative\r\n\
-ERR MAXLEN can't be negative\r\n-ERR syntax error\r\n-ERR syntax error\r\n\
*2\r\n:2\r\n:0\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-ERR syntax error\r\n-ERR syntax error\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n";
    let program = firn();
    let port = free_port();
    let text = port.to_string();
    let child = program.spawn_on_route(true, &[text.as_bytes(), b"1"]);
    let mut client = connect_when_ready(port);
    let mut transactions: Vec<Vec<u8>> = SETUP.iter().map(|request| resp(request)).collect();
    transactions.push(resp(&["MULTI"]));
    for request in VALID {
        transactions.push(resp(request));
    }
    transactions.push(resp(&["EXEC"]));
    for request in ERRORS {
        transactions.push(resp(&["MULTI"]));
        transactions.push(resp(request));
        transactions.push(resp(&["EXEC"]));
    }
    let mut scripts: Vec<Vec<u8>> = SETUP.iter().map(|request| resp(request)).collect();
    for (calls, script) in [
        (VALID, "return redis.call(unpack(ARGV))"),
        (ERRORS, "return redis.pcall(unpack(ARGV))"),
    ] {
        for request in calls {
            let mut call = vec!["EVAL", script, "0"];
            call.extend_from_slice(request);
            scripts.push(resp(&call));
        }
    }
    for (requests, expected, what) in [
        (transactions, TRANSACTIONS, "the transactions"),
        (scripts, SCRIPTS, "the scripts"),
    ] {
        client
            .write_all(&requests.concat())
            .expect("send the requests");
        expect_replies(&mut client, expected, what);
    }
    drop(client);
    let (status, _) = finished(child);
    assert_eq!(status, 0);
}

/// firn records the lists commands run inside a transaction's EXEC and a script as Redis 7.0.15 propagates them over lists found expired: a read records the removal alone, a push the removal and then itself, LMOVE onto an expired list the destination's removal and then itself, and a script whose one effect is a removal records it bare. The expected
/// records are redis-server 7.0.15's (Firn-wf probe run 37504720562).
#[cfg(target_os = "linux")]
#[test]
fn firn_records_held_lists_commands_as_redis_propagates_them() {
    check_held_records(
        &[
            &["RPUSH", "l1", "a"],
            &["PEXPIREAT", "l1", "1"],
            &["RPUSH", "l2", "a", "b"],
            &["PEXPIREAT", "l2", "1"],
            &["RPUSH", "l3", "a"],
            &["PEXPIREAT", "l3", "1"],
            &["RPUSH", "l4", "a"],
            &["PEXPIREAT", "l4", "1"],
            &["RPUSH", "src", "x"],
        ],
        &[
            &["MULTI"],
            &["LLEN", "l1"],
            &["LPUSH", "l2", "n"],
            &["LMOVE", "src", "l3", "LEFT", "RIGHT"],
            &["EXEC"],
            &["EVAL", "return redis.call('RPOP', KEYS[1])", "1", "l4"],
            &["LRANGE", "l2", "0", "-1"],
            &["LRANGE", "l3", "0", "-1"],
            &["DBSIZE"],
        ],
        b"+OK\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n*3\r\n:0\r\n:1\r\n$1\r\nx\r\n$-1\r\n*1\r\n$1\r\nn\r\n*1\r\n$1\r\nx\r\n:2\r\n",
        b"*1\r\n$5\r\nMULTI\r\n*2\r\n$3\r\nDEL\r\n$2\r\nl1\r\n*2\r\n$3\r\nDEL\r\n$2\r\nl2\r\n*3\r\n$5\r\nLPUSH\r\n$2\r\nl2\r\n$1\r\nn\r\n*2\r\n$3\r\nDEL\r\n$2\r\nl3\r\n*5\r\n$5\r\nLMOVE\r\n$3\r\nsrc\r\n$2\r\nl3\r\n$4\r\nLEFT\r\n$5\r\nRIGHT\r\n*1\r\n$4\r\nEXEC\r\n*2\r\n$3\r\nDEL\r\n$2\r\nl4\r\n",
    );
}

/// firn runs the sets commands written as parts, SADD to SDIFFSTORE, inside a transaction's EXEC and through a script as Redis 7.0.15 does, SMOVE and the commands over several sets among them, sets of one member keeping SPOP's, SRANDMEMBER's and SMEMBERS's replies in one order. The same requests, from the same keys, run in one transaction,
/// each error or boundary request in a transaction of its own, then through
/// a script's redis.call and the errors through redis.pcall, and give the
/// replies redis-server 7.0.15 gives (Firn-wf probe run 37503178190).
#[cfg(target_os = "linux")]
#[test]
fn firn_runs_sets_parts_as_redis_does() {
    const SETUP: &[&[&str]] = &[
        &["FLUSHALL"],
        &["SET", "wrong", "text"],
        &["SET", "out", "old"],
        &["SADD", "a", "x"],
        &["SADD", "b", "x"],
        &["SADD", "c", "y"],
        &["SADD", "pop_one", "p"],
        &["SADD", "pop_all", "q"],
        &["SADD", "move", "x"],
        &["SADD", "card", "x", "y"],
    ];
    const VALID: &[&[&str]] = &[
        &["SADD", "a", "x", "x"],
        &["SADD", "new", "n", "n"],
        &["SCARD", "new"],
        &["SISMEMBER", "a", "x"],
        &["SMISMEMBER", "a", "x", "y", "x"],
        &["SMEMBERS", "a"],
        &["SRANDMEMBER", "a"],
        &["SRANDMEMBER", "a", "0"],
        &["SRANDMEMBER", "a", "8"],
        &["SRANDMEMBER", "a", "-3"],
        &["SPOP", "pop_one"],
        &["SPOP", "pop_all", "8"],
        &["SPOP", "missing"],
        &["SPOP", "missing", "0"],
        &["SPOP", "a", "0"],
        &["SREM", "new", "missing", "n", "n"],
        &["EXISTS", "new"],
        &["SMOVE", "move", "b", "x"],
        &["SMOVE", "b", "b", "x"],
        &["SMOVE", "b", "b", "absent"],
        &["SMOVE", "b", "moved", "x"],
        &["SMOVE", "missing", "wrong", "x"],
        &["SINTER", "a", "moved"],
        &["SUNION", "a", "moved", "a"],
        &["SDIFF", "a", "c"],
        &["SDIFF", "a", "a"],
        &["SINTER", "a", "card"],
        &["SDIFF", "card", "a"],
        &["SUNIONSTORE", "union_card", "a", "c"],
        &["SCARD", "union_card"],
        &["SINTERCARD", "1", "card", "LIMIT", "1"],
        &["SINTERCARD", "2", "card", "card", "LIMIT", "0"],
        &["SINTERCARD", "2", "a", "moved", "LIMIT", "1"],
        &["SINTERCARD", "2", "a", "moved", "LIMIT", "0", "LIMIT", "3"],
        &["SINTERSTORE", "out", "a", "moved"],
        &["SMEMBERS", "out"],
        &["PEXPIREAT", "out", "4102444800000"],
        &["SADD", "out", "x"],
        &["PEXPIRETIME", "out"],
        &["SREM", "out", "absent"],
        &["PEXPIRETIME", "out"],
        &["SUNIONSTORE", "out", "out", "a"],
        &["PEXPIRETIME", "out"],
        &["SDIFFSTORE", "out", "out", "a"],
        &["EXISTS", "out"],
        &["SINTERSTORE", "a", "a", "moved"],
        &["SMEMBERS", "a"],
        &["SUNIONSTORE", "missing_out", "missing"],
        &["SINTER", "a", "missing"],
        &["SDIFF", "missing", "a"],
        &["SINTERCARD", "2", "a", "missing"],
        &["SCARD", "missing"],
        &["SMEMBERS", "missing"],
        &["SISMEMBER", "missing", "x"],
        &["SMISMEMBER", "missing", "x", "y"],
        &["SRANDMEMBER", "missing"],
        &["SRANDMEMBER", "missing", "-3"],
        &["PEXPIREAT", "moved", "1"],
        &["SCARD", "moved"],
    ];
    const ERRORS: &[&[&str]] = &[
        &["SADD", "wrong", "x"],
        &["SREM", "wrong", "x"],
        &["SPOP", "wrong"],
        &["SPOP", "wrong", "0"],
        &["SCARD", "wrong"],
        &["SMEMBERS", "wrong"],
        &["SISMEMBER", "wrong", "x"],
        &["SMISMEMBER", "wrong", "x", "y"],
        &["SRANDMEMBER", "wrong"],
        &["SRANDMEMBER", "wrong", "0"],
        &["SMOVE", "a", "wrong", "x"],
        &["SMOVE", "wrong", "missing", "x"],
        &["SINTER", "a", "wrong"],
        &["SUNION", "missing", "wrong"],
        &["SDIFF", "missing", "wrong"],
        &["SINTERCARD", "2", "missing", "wrong"],
        &["SINTERSTORE", "out", "a", "wrong"],
        &["SUNIONSTORE", "out", "wrong"],
        &["SDIFFSTORE", "out", "a", "wrong"],
        &["SPOP", "a", "-1"],
        &["SPOP", "a", "nope"],
        &["SPOP", "a", "9223372036854775808"],
        &["SPOP", "a", "1", "extra"],
        &["SRANDMEMBER", "a", "nope"],
        &["SRANDMEMBER", "a", "-9223372036854775808"],
        &["SRANDMEMBER", "a", "9223372036854775808"],
        &["SRANDMEMBER", "a", "1", "extra"],
        &["SINTERCARD", "0", "a"],
        &["SINTERCARD", "nope", "a"],
        &["SINTERCARD", "2", "a"],
        &["SINTERCARD", "1", "a", "LIMIT", "-1"],
        &["SINTERCARD", "1", "a", "LIMIT", "nope"],
        &["SINTERCARD", "1", "a", "LIMIT"],
        &["SINTERCARD", "1", "a", "unknown"],
        &["SINTERCARD", "1", "wrong", "LIMIT", "nope"],
    ];
    const TRANSACTIONS: &[u8] =
        b"+OK\r\n+OK\r\n+OK\r\n:1\r\n:1\r\n:1\r\n:1\r\n:1\r\n:1\r\n:2\r\n+OK\r\n\
+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n\
+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n\
+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n\
+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n\
+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n\
+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n\
+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n\
+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n\
+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n*59\r\n:0\r\n:1\r\n:1\r\n:1\r\n*3\r\n:1\r\n\
:0\r\n:1\r\n*1\r\n$1\r\nx\r\n$1\r\nx\r\n*0\r\n*1\r\n$1\r\nx\r\n*3\r\n$1\r\nx\r\n\
$1\r\nx\r\n$1\r\nx\r\n$1\r\np\r\n*1\r\n$1\r\nq\r\n$-1\r\n*0\r\n*0\r\n:1\r\n\
:0\r\n:1\r\n:1\r\n:0\r\n:1\r\n:0\r\n*1\r\n$1\r\nx\r\n*1\r\n$1\r\nx\r\n*1\r\n\
$1\r\nx\r\n*0\r\n*1\r\n$1\r\nx\r\n*1\r\n$1\r\ny\r\n:2\r\n:2\r\n:1\r\n:2\r\n\
:1\r\n:1\r\n:1\r\n*1\r\n$1\r\nx\r\n:1\r\n:0\r\n:4102444800000\r\n:0\r\n\
:4102444800000\r\n:1\r\n:-1\r\n:0\r\n:0\r\n:1\r\n*1\r\n$1\r\nx\r\n:0\r\n*0\r\n\
*0\r\n:0\r\n:0\r\n*0\r\n:0\r\n*2\r\n:0\r\n:0\r\n$-1\r\n*0\r\n:1\r\n:0\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n-ERR value is out of range, must be positive\r\n+OK\r\n\
+QUEUED\r\n*1\r\n-ERR value is out of range, must be positive\r\n+OK\r\n\
+QUEUED\r\n*1\r\n-ERR value is out of range, must be positive\r\n+OK\r\n\
+QUEUED\r\n*1\r\n-ERR syntax error\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-ERR value is not an integer or out of range\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-ERR value is out of range, value must between -9223372036854775807 and 9223372036854775807\r\n\
+OK\r\n+QUEUED\r\n*1\r\n-ERR value is not an integer or out of range\r\n+OK\r\n\
+QUEUED\r\n*1\r\n-ERR syntax error\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-ERR numkeys should be greater than 0\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-ERR numkeys should be greater than 0\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-ERR Number of keys can't be greater than number of args\r\n+OK\r\n+QUEUED\r\n\
*1\r\n-ERR LIMIT can't be negative\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-ERR LIMIT can't be negative\r\n+OK\r\n+QUEUED\r\n*1\r\n-ERR syntax error\r\n\
+OK\r\n+QUEUED\r\n*1\r\n-ERR syntax error\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-ERR LIMIT can't be negative\r\n";
    const SCRIPTS: &[u8] =
        b"+OK\r\n+OK\r\n+OK\r\n:1\r\n:1\r\n:1\r\n:1\r\n:1\r\n:1\r\n:2\r\n:0\r\n:1\r\n\
:1\r\n:1\r\n*3\r\n:1\r\n:0\r\n:1\r\n*1\r\n$1\r\nx\r\n$1\r\nx\r\n*0\r\n*1\r\n\
$1\r\nx\r\n*3\r\n$1\r\nx\r\n$1\r\nx\r\n$1\r\nx\r\n$1\r\np\r\n*1\r\n$1\r\nq\r\n\
$-1\r\n*0\r\n*0\r\n:1\r\n:0\r\n:1\r\n:1\r\n:0\r\n:1\r\n:0\r\n*1\r\n$1\r\nx\r\n\
*1\r\n$1\r\nx\r\n*1\r\n$1\r\nx\r\n*0\r\n*1\r\n$1\r\nx\r\n*1\r\n$1\r\ny\r\n:2\r\n\
:2\r\n:1\r\n:2\r\n:1\r\n:1\r\n:1\r\n*1\r\n$1\r\nx\r\n:1\r\n:0\r\n\
:4102444800000\r\n:0\r\n:4102444800000\r\n:1\r\n:-1\r\n:0\r\n:0\r\n:1\r\n*1\r\n\
$1\r\nx\r\n:0\r\n*0\r\n*0\r\n:0\r\n:0\r\n*0\r\n:0\r\n*2\r\n:0\r\n:0\r\n$-1\r\n\
*0\r\n:1\r\n:0\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-ERR value is out of range, must be positive\r\n\
-ERR value is out of range, must be positive\r\n\
-ERR value is out of range, must be positive\r\n-ERR syntax error\r\n\
-ERR value is not an integer or out of range\r\n\
-ERR value is out of range, value must between -9223372036854775807 and 9223372036854775807\r\n\
-ERR value is not an integer or out of range\r\n-ERR syntax error\r\n\
-ERR numkeys should be greater than 0\r\n\
-ERR numkeys should be greater than 0\r\n\
-ERR Number of keys can't be greater than number of args\r\n\
-ERR LIMIT can't be negative\r\n-ERR LIMIT can't be negative\r\n\
-ERR syntax error\r\n-ERR syntax error\r\n-ERR LIMIT can't be negative\r\n";
    let program = firn();
    let port = free_port();
    let text = port.to_string();
    let child = program.spawn_on_route(true, &[text.as_bytes(), b"1"]);
    let mut client = connect_when_ready(port);
    let mut transactions: Vec<Vec<u8>> = SETUP.iter().map(|request| resp(request)).collect();
    transactions.push(resp(&["MULTI"]));
    for request in VALID {
        transactions.push(resp(request));
    }
    transactions.push(resp(&["EXEC"]));
    for request in ERRORS {
        transactions.push(resp(&["MULTI"]));
        transactions.push(resp(request));
        transactions.push(resp(&["EXEC"]));
    }
    let mut scripts: Vec<Vec<u8>> = SETUP.iter().map(|request| resp(request)).collect();
    for (calls, script) in [
        (VALID, "return redis.call(unpack(ARGV))"),
        (ERRORS, "return redis.pcall(unpack(ARGV))"),
    ] {
        for request in calls {
            let mut call = vec!["EVAL", script, "0"];
            call.extend_from_slice(request);
            scripts.push(resp(&call));
        }
    }
    for (requests, expected, what) in [
        (transactions, TRANSACTIONS, "the transactions"),
        (scripts, SCRIPTS, "the scripts"),
    ] {
        client
            .write_all(&requests.concat())
            .expect("send the requests");
        expect_replies(&mut client, expected, what);
    }
    drop(client);
    let (status, _) = finished(child);
    assert_eq!(status, 0);
}

/// firn treats a set found expired as absent when it combines sets, as
/// Redis 7.0.15's sunionDiffGenericCommand and sinterGenericCommand look
/// each key up first: SUNION, SDIFF and SINTER of a lapsed set answer no
/// members, and a combination refused for a key of another kind has still
/// removed the lapsed set named before it, so DBSIZE, which counts entries
/// not yet removed, counts only the other key.
#[cfg(target_os = "linux")]
#[test]
fn firn_combines_lapsed_sets_as_redis_does() {
    let program = firn();
    let port = free_port();
    let text = port.to_string();
    let child = program.spawn_on_route(true, &[text.as_bytes(), b"1"]);
    let mut client = connect_when_ready(port);
    let mut batch = Vec::new();
    for request in [
        vec!["SADD", "lapsing", "x"],
        vec!["PEXPIRE", "lapsing", "1"],
        vec!["SADD", "lapsing2", "x"],
        vec!["PEXPIRE", "lapsing2", "1"],
        vec!["SET", "text", "v"],
    ] {
        batch.extend(resp(&request));
    }
    client.write_all(&batch).expect("set the keys up");
    expect_replies(
        &mut client,
        b":1\r\n:1\r\n:1\r\n:1\r\n+OK\r\n",
        "the keys set up",
    );
    std::thread::sleep(Duration::from_millis(5));
    let mut batch = Vec::new();
    for request in [
        vec!["SUNION", "lapsing"],
        vec!["SDIFF", "lapsing"],
        vec!["SINTER", "lapsing"],
        vec!["SUNION", "lapsing2", "text"],
        vec!["DBSIZE"],
    ] {
        batch.extend(resp(&request));
    }
    client.write_all(&batch).expect("combine the sets");
    expect_replies(
        &mut client,
        b"*0\r\n*0\r\n*0\r\n-WRONGTYPE Operation against a key holding the wrong kind of value\r\n:1\r\n",
        "the combinations",
    );
    drop(client);
    let (status, _) = finished(child);
    assert_eq!(status, 0);
}

/// firn records the sets commands run inside a transaction's EXEC and a script as Redis 7.0.15 propagates them over sets found expired: a read records the removal alone, SADD the removal and then itself, SMOVE onto an expired set the destination's removal and then itself, and a script whose one effect is a removal records it bare. The expected
/// records are redis-server 7.0.15's (Firn-wf probe run 37504720562).
#[cfg(target_os = "linux")]
#[test]
fn firn_records_held_sets_commands_as_redis_propagates_them() {
    check_held_records(
        &[
            &["SADD", "s1", "a"],
            &["PEXPIREAT", "s1", "1"],
            &["SADD", "s2", "a", "b"],
            &["PEXPIREAT", "s2", "1"],
            &["SADD", "s3", "a"],
            &["PEXPIREAT", "s3", "1"],
            &["SADD", "s4", "a"],
            &["PEXPIREAT", "s4", "1"],
            &["SADD", "src", "x"],
        ],
        &[
            &["MULTI"],
            &["SCARD", "s1"],
            &["SADD", "s2", "n"],
            &["SMOVE", "src", "s3", "x"],
            &["EXEC"],
            &["EVAL", "return redis.call('SPOP', KEYS[1])", "1", "s4"],
            &["SMEMBERS", "s2"],
            &["SMEMBERS", "s3"],
            &["DBSIZE"],
        ],
        b"+OK\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n*3\r\n:0\r\n:1\r\n:1\r\n$-1\r\n*1\r\n$1\r\nn\r\n*1\r\n$1\r\nx\r\n:2\r\n",
        b"*1\r\n$5\r\nMULTI\r\n*2\r\n$3\r\nDEL\r\n$2\r\ns1\r\n*2\r\n$3\r\nDEL\r\n$2\r\ns2\r\n*3\r\n$4\r\nSADD\r\n$2\r\ns2\r\n$1\r\nn\r\n*2\r\n$3\r\nDEL\r\n$2\r\ns3\r\n*4\r\n$5\r\nSMOVE\r\n$3\r\nsrc\r\n$2\r\ns3\r\n$1\r\nx\r\n*1\r\n$4\r\nEXEC\r\n*2\r\n$3\r\nDEL\r\n$2\r\ns4\r\n",
    );
}

/// firn records SINTERSTORE, SUNIONSTORE and SDIFFSTORE run inside a transaction's EXEC and a script as Redis 7.0.15 propagates them over destinations found expired: a nonempty result removes the expired destination and records its DEL before the command, as setKey's lookup does, while an empty result records the command alone, as dbDelete does; a script of two effects is bracketed and one of one is not. The expected
/// records are redis-server 7.0.15's (Firn-wf probe run 37512787568).
#[cfg(target_os = "linux")]
#[test]
fn firn_records_held_setstore_commands_as_redis_propagates_them() {
    check_held_records(
        &[
            &["SADD", "src", "x"],
            &["SADD", "d1", "old"],
            &["PEXPIREAT", "d1", "1"],
            &["SADD", "d2", "old"],
            &["PEXPIREAT", "d2", "1"],
            &["SADD", "d3", "old"],
            &["PEXPIREAT", "d3", "1"],
            &["SADD", "d4", "old"],
            &["PEXPIREAT", "d4", "1"],
        ],
        &[
            &["MULTI"],
            &["SUNIONSTORE", "d1", "src"],
            &["SINTERSTORE", "d2", "src", "none"],
            &["EXEC"],
            &["EVAL", "return redis.call('SDIFFSTORE', KEYS[1], KEYS[2])", "2", "d3", "src"],
            &["EVAL", "return redis.call('SINTERSTORE', KEYS[1], KEYS[2], 'none')", "2", "d4", "src"],
            &["SMEMBERS", "d1"],
            &["SMEMBERS", "d3"],
            &["DBSIZE"],
        ],
        b"+OK\r\n+QUEUED\r\n+QUEUED\r\n*2\r\n:1\r\n:0\r\n:1\r\n:0\r\n*1\r\n$1\r\nx\r\n*1\r\n$1\r\nx\r\n:3\r\n",
        b"*1\r\n$5\r\nMULTI\r\n*2\r\n$3\r\nDEL\r\n$2\r\nd1\r\n*3\r\n$11\r\nSUNIONSTORE\r\n$2\r\nd1\r\n$3\r\nsrc\r\n*4\r\n$11\r\nSINTERSTORE\r\n$2\r\nd2\r\n$3\r\nsrc\r\n$4\r\nnone\r\n*1\r\n$4\r\nEXEC\r\n*1\r\n$5\r\nMULTI\r\n*2\r\n$3\r\nDEL\r\n$2\r\nd3\r\n*3\r\n$10\r\nSDIFFSTORE\r\n$2\r\nd3\r\n$3\r\nsrc\r\n*1\r\n$4\r\nEXEC\r\n*4\r\n$11\r\nSINTERSTORE\r\n$2\r\nd4\r\n$3\r\nsrc\r\n$4\r\nnone\r\n",
    );
}

/// firn runs the sorted sets commands written as parts, ZADD to ZREMRANGEBYLEX, inside a transaction's EXEC and through a script as Redis 7.0.15 does. The same requests, from the same keys, run in one transaction,
/// each error or boundary request in a transaction of its own, then through
/// a script's redis.call and the errors through redis.pcall, and give the
/// replies redis-server 7.0.15 gives (Firn-wf probe run 37503178190).
#[cfg(target_os = "linux")]
#[test]
fn firn_runs_sorted_parts_as_redis_does() {
    const SETUP: &[&[&str]] = &[
        &["FLUSHALL"],
        &["SET", "wrong", "text"],
        &["ZADD", "z", "1", "a", "2", "b", "2", "c", "4", "d"],
        &["ZADD", "lex", "0", "a", "0", "b", "0", "c", "0", "d"],
        &["ZADD", "pop", "1", "a", "2", "b", "3", "c", "4", "d"],
        &["ZADD", "remove", "1", "a", "2", "b", "3", "c", "4", "d"],
        &["ZADD", "infinite", "inf", "a"],
    ];
    const VALID: &[&[&str]] = &[
        &["ZADD", "z", "NX", "9", "a"],
        &["ZADD", "missing", "XX", "1", "a"],
        &["ZADD", "z", "CH", "3", "b", "5", "e"],
        &["ZADD", "z", "XX", "GT", "2", "b"],
        &["ZADD", "z", "XX", "LT", "2", "b"],
        &["ZADD", "z", "NX", "INCR", "1", "a"],
        &["ZADD", "z", "INCR", "0.5", "a"],
        &["ZINCRBY", "z", "0.5", "a"],
        &["ZCARD", "z"],
        &["ZSCORE", "z", "a"],
        &["ZMSCORE", "z", "a", "absent", "b", "a"],
        &["ZRANK", "z", "a"],
        &["ZREVRANK", "z", "a"],
        &["ZRANGE", "z", "0", "-1", "WITHSCORES"],
        &[
            "ZRANGE",
            "z",
            "5",
            "(2",
            "BYSCORE",
            "REV",
            "LIMIT",
            "0",
            "2",
            "WITHSCORES",
        ],
        &["ZRANGE", "lex", "[b", "[d", "BYLEX"],
        &["ZREVRANGE", "z", "-3", "-1", "WITHSCORES"],
        &["ZRANGEBYSCORE", "z", "(2", "+inf", "WITHSCORES"],
        &["ZREVRANGEBYSCORE", "z", "+inf", "-inf", "LIMIT", "1", "2"],
        &["ZRANGEBYLEX", "lex", "(a", "[d", "LIMIT", "1", "2"],
        &["ZREVRANGEBYLEX", "lex", "+", "-", "LIMIT", "0", "2"],
        &["ZCOUNT", "z", "(2", "+inf"],
        &["ZLEXCOUNT", "lex", "[b", "(d"],
        &["ZREM", "z", "e", "e", "absent"],
        &["ZPOPMIN", "pop"],
        &["ZPOPMAX", "pop", "2"],
        &["ZPOPMIN", "pop", "99"],
        &["EXISTS", "pop"],
        &["ZREMRANGEBYRANK", "remove", "0", "0"],
        &["ZREMRANGEBYSCORE", "remove", "(1", "2"],
        &["ZREMRANGEBYLEX", "lex", "[b", "[c"],
        &["ZRANGE", "remove", "0", "-1", "WITHSCORES"],
        &["ZRANGE", "lex", "0", "-1"],
        &["ZREMRANGEBYRANK", "remove", "0", "-1"],
        &["TYPE", "remove"],
        &["ZCARD", "missing"],
        &["ZSCORE", "missing", "a"],
        &["ZMSCORE", "missing", "a", "b"],
        &["ZRANK", "missing", "a"],
        &["ZRANGE", "missing", "0", "-1"],
        &["ZCOUNT", "missing", "-inf", "+inf"],
        &["ZPOPMAX", "missing"],
        &["ZREM", "missing", "a"],
    ];
    const ERRORS: &[&[&str]] = &[
        &["ZADD", "z", "1", "a", "2"],
        &["ZADD", "z", "NX", "XX", "1", "a"],
        &["ZADD", "z", "NX", "GT", "1", "a"],
        &["ZADD", "z", "GT", "LT", "1", "a"],
        &["ZADD", "z", "INCR", "1", "a", "2", "b"],
        &["ZADD", "wrong", "nan", "a"],
        &["ZADD", "wrong", "1", "a"],
        &["ZADD", "infinite", "INCR", "-inf", "a"],
        &["ZINCRBY", "infinite", "-inf", "a"],
        &["ZINCRBY", "z", "bad", "a"],
        &["ZPOPMIN", "wrong", "-1"],
        &["ZPOPMAX", "z", "bad"],
        &["ZPOPMIN", "z", "1", "extra"],
        &["ZPOPMAX", "wrong", "0"],
        &["ZPOPMIN", "z", "0"],
        &["ZCARD", "wrong"],
        &["ZSCORE", "wrong", "a"],
        &["ZMSCORE", "wrong", "a", "b"],
        &["ZRANK", "wrong", "a"],
        &["ZREM", "wrong", "a"],
        &["ZRANGE", "wrong", "bad", "-1"],
        &["ZRANGE", "wrong", "0", "-1"],
        &["ZRANGE", "z", "0", "-1", "LIMIT", "0", "1"],
        &["ZRANGE", "z", "0", "-1", "LIMIT", "1", "-1"],
        &["ZRANGE", "lex", "-", "+", "BYLEX", "WITHSCORES"],
        &["ZRANGE", "z", "0", "-1", "REV", "REV"],
        &["ZRANGEBYSCORE", "wrong", "bad", "+inf"],
        &["ZRANGEBYSCORE", "z", "-inf", "+inf", "LIMIT", "-1", "2"],
        &["ZRANGEBYSCORE", "z", "-inf", "+inf", "LIMIT", "0", "0"],
        &["ZRANGEBYLEX", "lex", "a", "+"],
        &["ZCOUNT", "wrong", "bad", "+inf"],
        &["ZLEXCOUNT", "wrong", "a", "+"],
        &["ZREMRANGEBYRANK", "wrong", "bad", "-1"],
        &["ZREMRANGEBYSCORE", "wrong", "nan", "+inf"],
        &["ZREMRANGEBYLEX", "wrong", "a", "+"],
        &["ZREMRANGEBYRANK", "z", "10", "0"],
    ];
    const TRANSACTIONS: &[u8] =
        b"+OK\r\n+OK\r\n:4\r\n:4\r\n:4\r\n:4\r\n:1\r\n+OK\r\n+QUEUED\r\n+QUEUED\r\n\
+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n\
+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n\
+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n\
+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n\
+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n\
+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n*43\r\n:0\r\n\
:0\r\n:2\r\n:0\r\n:0\r\n$-1\r\n$3\r\n1.5\r\n$1\r\n2\r\n:5\r\n$1\r\n2\r\n*4\r\n\
$1\r\n2\r\n$-1\r\n$1\r\n2\r\n$1\r\n2\r\n:0\r\n:4\r\n*10\r\n$1\r\na\r\n$1\r\n\
2\r\n$1\r\nb\r\n$1\r\n2\r\n$1\r\nc\r\n$1\r\n2\r\n$1\r\nd\r\n$1\r\n4\r\n$1\r\n\
e\r\n$1\r\n5\r\n*4\r\n$1\r\ne\r\n$1\r\n5\r\n$1\r\nd\r\n$1\r\n4\r\n*3\r\n$1\r\n\
b\r\n$1\r\nc\r\n$1\r\nd\r\n*6\r\n$1\r\nc\r\n$1\r\n2\r\n$1\r\nb\r\n$1\r\n2\r\n\
$1\r\na\r\n$1\r\n2\r\n*4\r\n$1\r\nd\r\n$1\r\n4\r\n$1\r\ne\r\n$1\r\n5\r\n*2\r\n\
$1\r\nd\r\n$1\r\nc\r\n*2\r\n$1\r\nc\r\n$1\r\nd\r\n*2\r\n$1\r\nd\r\n$1\r\nc\r\n\
:2\r\n:2\r\n:1\r\n*2\r\n$1\r\na\r\n$1\r\n1\r\n*4\r\n$1\r\nd\r\n$1\r\n4\r\n$1\r\n\
c\r\n$1\r\n3\r\n*2\r\n$1\r\nb\r\n$1\r\n2\r\n:0\r\n:1\r\n:1\r\n:2\r\n*4\r\n$1\r\n\
c\r\n$1\r\n3\r\n$1\r\nd\r\n$1\r\n4\r\n*2\r\n$1\r\na\r\n$1\r\nd\r\n:2\r\n\
+none\r\n:0\r\n$-1\r\n*2\r\n$-1\r\n$-1\r\n$-1\r\n*0\r\n:0\r\n*0\r\n:0\r\n+OK\r\n\
+QUEUED\r\n*1\r\n-ERR syntax error\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-ERR XX and NX options at the same time are not compatible\r\n+OK\r\n+QUEUED\r\n\
*1\r\n-ERR GT, LT, and/or NX options at the same time are not compatible\r\n\
+OK\r\n+QUEUED\r\n*1\r\n\
-ERR GT, LT, and/or NX options at the same time are not compatible\r\n+OK\r\n\
+QUEUED\r\n*1\r\n-ERR INCR option supports a single increment-element pair\r\n\
+OK\r\n+QUEUED\r\n*1\r\n-ERR value is not a valid float\r\n+OK\r\n+QUEUED\r\n\
*1\r\n-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
+OK\r\n+QUEUED\r\n*1\r\n-ERR resulting score is not a number (NaN)\r\n+OK\r\n\
+QUEUED\r\n*1\r\n-ERR resulting score is not a number (NaN)\r\n+OK\r\n\
+QUEUED\r\n*1\r\n-ERR value is not a valid float\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-ERR value is out of range, must be positive\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-ERR value is out of range, must be positive\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-ERR syntax error\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n*0\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n-ERR value is not an integer or out of range\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-ERR syntax error, LIMIT is only supported in combination with either BYSCORE or BYLEX\r\n\
+OK\r\n+QUEUED\r\n*1\r\n*4\r\n$1\r\na\r\n$1\r\nb\r\n$1\r\nc\r\n$1\r\nd\r\n\
+OK\r\n+QUEUED\r\n*1\r\n\
-ERR syntax error, WITHSCORES not supported in combination with BYLEX\r\n+OK\r\n\
+QUEUED\r\n*1\r\n-ERR syntax error\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-ERR min or max is not a float\r\n+OK\r\n+QUEUED\r\n*1\r\n*0\r\n+OK\r\n\
+QUEUED\r\n*1\r\n*0\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-ERR min or max not valid string range item\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-ERR min or max is not a float\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-ERR min or max not valid string range item\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-ERR value is not an integer or out of range\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-ERR min or max is not a float\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-ERR min or max not valid string range item\r\n+OK\r\n+QUEUED\r\n*1\r\n:0\r\n";
    const SCRIPTS: &[u8] =
        b"+OK\r\n+OK\r\n:4\r\n:4\r\n:4\r\n:4\r\n:1\r\n:0\r\n:0\r\n:2\r\n:0\r\n:0\r\n\
$-1\r\n$3\r\n1.5\r\n$1\r\n2\r\n:5\r\n$1\r\n2\r\n*4\r\n$1\r\n2\r\n$-1\r\n$1\r\n\
2\r\n$1\r\n2\r\n:0\r\n:4\r\n*10\r\n$1\r\na\r\n$1\r\n2\r\n$1\r\nb\r\n$1\r\n2\r\n\
$1\r\nc\r\n$1\r\n2\r\n$1\r\nd\r\n$1\r\n4\r\n$1\r\ne\r\n$1\r\n5\r\n*4\r\n$1\r\n\
e\r\n$1\r\n5\r\n$1\r\nd\r\n$1\r\n4\r\n*3\r\n$1\r\nb\r\n$1\r\nc\r\n$1\r\nd\r\n\
*6\r\n$1\r\nc\r\n$1\r\n2\r\n$1\r\nb\r\n$1\r\n2\r\n$1\r\na\r\n$1\r\n2\r\n*4\r\n\
$1\r\nd\r\n$1\r\n4\r\n$1\r\ne\r\n$1\r\n5\r\n*2\r\n$1\r\nd\r\n$1\r\nc\r\n*2\r\n\
$1\r\nc\r\n$1\r\nd\r\n*2\r\n$1\r\nd\r\n$1\r\nc\r\n:2\r\n:2\r\n:1\r\n*2\r\n$1\r\n\
a\r\n$1\r\n1\r\n*4\r\n$1\r\nd\r\n$1\r\n4\r\n$1\r\nc\r\n$1\r\n3\r\n*2\r\n$1\r\n\
b\r\n$1\r\n2\r\n:0\r\n:1\r\n:1\r\n:2\r\n*4\r\n$1\r\nc\r\n$1\r\n3\r\n$1\r\nd\r\n\
$1\r\n4\r\n*2\r\n$1\r\na\r\n$1\r\nd\r\n:2\r\n+none\r\n:0\r\n$-1\r\n*2\r\n$-1\r\n\
$-1\r\n$-1\r\n*0\r\n:0\r\n*0\r\n:0\r\n-ERR syntax error\r\n\
-ERR XX and NX options at the same time are not compatible\r\n\
-ERR GT, LT, and/or NX options at the same time are not compatible\r\n\
-ERR GT, LT, and/or NX options at the same time are not compatible\r\n\
-ERR INCR option supports a single increment-element pair\r\n\
-ERR value is not a valid float\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-ERR resulting score is not a number (NaN)\r\n\
-ERR resulting score is not a number (NaN)\r\n\
-ERR value is not a valid float\r\n\
-ERR value is out of range, must be positive\r\n\
-ERR value is out of range, must be positive\r\n-ERR syntax error\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n*0\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-ERR value is not an integer or out of range\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-ERR syntax error, LIMIT is only supported in combination with either BYSCORE or BYLEX\r\n\
*4\r\n$1\r\na\r\n$1\r\nb\r\n$1\r\nc\r\n$1\r\nd\r\n\
-ERR syntax error, WITHSCORES not supported in combination with BYLEX\r\n\
-ERR syntax error\r\n-ERR min or max is not a float\r\n*0\r\n*0\r\n\
-ERR min or max not valid string range item\r\n\
-ERR min or max is not a float\r\n\
-ERR min or max not valid string range item\r\n\
-ERR value is not an integer or out of range\r\n\
-ERR min or max is not a float\r\n\
-ERR min or max not valid string range item\r\n:0\r\n";
    let program = firn();
    let port = free_port();
    let text = port.to_string();
    let child = program.spawn_on_route(true, &[text.as_bytes(), b"1"]);
    let mut client = connect_when_ready(port);
    let mut transactions: Vec<Vec<u8>> = SETUP.iter().map(|request| resp(request)).collect();
    transactions.push(resp(&["MULTI"]));
    for request in VALID {
        transactions.push(resp(request));
    }
    transactions.push(resp(&["EXEC"]));
    for request in ERRORS {
        transactions.push(resp(&["MULTI"]));
        transactions.push(resp(request));
        transactions.push(resp(&["EXEC"]));
    }
    let mut scripts: Vec<Vec<u8>> = SETUP.iter().map(|request| resp(request)).collect();
    for (calls, script) in [
        (VALID, "return redis.call(unpack(ARGV))"),
        (ERRORS, "return redis.pcall(unpack(ARGV))"),
    ] {
        for request in calls {
            let mut call = vec!["EVAL", script, "0"];
            call.extend_from_slice(request);
            scripts.push(resp(&call));
        }
    }
    for (requests, expected, what) in [
        (transactions, TRANSACTIONS, "the transactions"),
        (scripts, SCRIPTS, "the scripts"),
    ] {
        client
            .write_all(&requests.concat())
            .expect("send the requests");
        expect_replies(&mut client, expected, what);
    }
    drop(client);
    let (status, _) = finished(child);
    assert_eq!(status, 0);
}

/// firn records the sorted sets commands run inside a transaction's EXEC and a script as Redis 7.0.15 propagates them over sorted sets found expired: a read records the removal alone, ZADD and ZINCRBY the removal and then themselves, and a script whose one effect is a removal records it bare. The expected
/// records are redis-server 7.0.15's (Firn-wf probe run 37504720562).
#[cfg(target_os = "linux")]
#[test]
fn firn_records_held_sorted_commands_as_redis_propagates_them() {
    check_held_records(
        &[
            &["ZADD", "z1", "1", "a"],
            &["PEXPIREAT", "z1", "1"],
            &["ZADD", "z2", "1", "a", "2", "b"],
            &["PEXPIREAT", "z2", "1"],
            &["ZADD", "z3", "1", "a"],
            &["PEXPIREAT", "z3", "1"],
            &["ZADD", "z4", "1", "a"],
            &["PEXPIREAT", "z4", "1"],
        ],
        &[
            &["MULTI"],
            &["ZCARD", "z1"],
            &["ZADD", "z2", "5", "n"],
            &["ZINCRBY", "z3", "2", "m"],
            &["EXEC"],
            &["EVAL", "return redis.call('ZPOPMIN', KEYS[1])", "1", "z4"],
            &["ZRANGE", "z2", "0", "-1", "WITHSCORES"],
            &["ZRANGE", "z3", "0", "-1", "WITHSCORES"],
            &["DBSIZE"],
        ],
        b"+OK\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n*3\r\n:0\r\n:1\r\n$1\r\n2\r\n*0\r\n*2\r\n$1\r\nn\r\n$1\r\n5\r\n*2\r\n$1\r\nm\r\n$1\r\n2\r\n:2\r\n",
        b"*1\r\n$5\r\nMULTI\r\n*2\r\n$3\r\nDEL\r\n$2\r\nz1\r\n*2\r\n$3\r\nDEL\r\n$2\r\nz2\r\n*4\r\n$4\r\nZADD\r\n$2\r\nz2\r\n$1\r\n5\r\n$1\r\nn\r\n*2\r\n$3\r\nDEL\r\n$2\r\nz3\r\n*4\r\n$7\r\nZINCRBY\r\n$2\r\nz3\r\n$1\r\n2\r\n$1\r\nm\r\n*1\r\n$4\r\nEXEC\r\n*2\r\n$3\r\nDEL\r\n$2\r\nz4\r\n",
    );
}

/// firn answers TIME inside a transaction's EXEC and through a script as
/// Redis 7.0.15 shapes it: two bulk strings, the seconds since the epoch
/// near the host's clock and the microseconds below one million. The values
/// move with the clock, so the case checks their form rather than bytes.
#[cfg(target_os = "linux")]
#[test]
fn firn_answers_time_inside_transactions_and_scripts() {
    let program = firn();
    let port = free_port();
    let text = port.to_string();
    let child = program.spawn_on_route(true, &[text.as_bytes(), b"1"]);
    let mut client = connect_when_ready(port);
    let mut batch = Vec::new();
    for request in [
        vec!["MULTI"],
        vec!["TIME"],
        vec!["EXEC"],
        vec!["EVAL", "return redis.call('TIME')", "0"],
    ] {
        batch.extend(resp(&request));
    }
    client.write_all(&batch).expect("send TIME");
    expect_replies(&mut client, b"+OK\r\n+QUEUED\r\n*1\r\n", "the transaction");
    for what in ["the transaction's TIME", "the script's TIME"] {
        assert_eq!(
            reply_line(&mut client, what).trim_end().to_string(),
            "*2",
            "{what}"
        );
        let mut values = Vec::new();
        for _ in 0..2 {
            let length: usize = reply_line(&mut client, what).trim_end().to_string()[1..]
                .parse()
                .expect("a bulk length");
            let value = reply_line(&mut client, what).trim_end().to_string();
            assert_eq!(value.len(), length, "{what}");
            values.push(value.parse::<u64>().expect("a decimal number"));
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("a clock after the epoch")
            .as_secs();
        assert!(
            values[0].abs_diff(now) <= 5,
            "{what}: {} seconds against {now}",
            values[0]
        );
        assert!(values[1] < 1_000_000, "{what}: {} microseconds", values[1]);
    }
    drop(client);
    let (status, _) = finished(child);
    assert_eq!(status, 0);
}

/// firn answers COMMAND inside a transaction's EXEC and through a script as
/// on the network path: firn describes no commands, so COMMAND answers an
/// empty array and COMMAND COUNT zero wherever they run, where Redis lists
/// its table. COMMAND DOCS, a subcommand Redis 7.0.15 has and firn does not
/// run, is queued and refuses its transaction whole at EXEC, as any command
/// firn does not run in transactions, and a script calling it gets the
/// extended unknown-command text of the command-parts decision.
#[cfg(target_os = "linux")]
#[test]
fn firn_answers_command_inside_transactions_and_scripts_as_on_the_network() {
    let program = firn();
    let port = free_port();
    let text = port.to_string();
    let child = program.spawn_on_route(true, &[text.as_bytes(), b"1"]);
    let mut client = connect_when_ready(port);
    let mut batch = Vec::new();
    for request in [
        vec!["COMMAND"],
        vec!["COMMAND", "COUNT"],
        vec!["MULTI"],
        vec!["COMMAND"],
        vec!["COMMAND", "COUNT"],
        vec!["EXEC"],
        vec!["EVAL", "return redis.call('COMMAND')", "0"],
        vec!["EVAL", "return redis.call('COMMAND', 'COUNT')", "0"],
        vec!["MULTI"],
        vec!["COMMAND", "DOCS"],
        vec!["EXEC"],
        vec!["EVAL", "return redis.pcall('COMMAND', 'DOCS')", "0"],
    ] {
        batch.extend(resp(&request));
    }
    client.write_all(&batch).expect("send COMMAND");
    expect_replies(
        &mut client,
        b"*0\r\n:0\r\n+OK\r\n+QUEUED\r\n+QUEUED\r\n*2\r\n*0\r\n:0\r\n*0\r\n:0\r\n+OK\r\n+QUEUED\r\n-EXECABORT Transaction discarded because it holds a command firn does not run in transactions\r\n-ERR Unknown Redis command called from script, or one firn does not yet run from scripts\r\n",
        "COMMAND",
    );
    drop(client);
    let (status, _) = finished(child);
    assert_eq!(status, 0);
}

/// firn runs TOUCH, SUBSTR, SELECT and COMMAND's refusals, written as parts, inside a transaction's EXEC and through a script as Redis 7.0.15 does, a COMMAND subcommand Redis does not have refused before queueing and answered from a script with Redis's own text. The same requests, from the same keys, run in one transaction,
/// each error or boundary request in a transaction of its own, then through
/// a script's redis.call and the errors through redis.pcall, and give the
/// replies redis-server 7.0.15 gives (Firn-wf probe run 37515932245).
#[cfg(target_os = "linux")]
#[test]
fn firn_runs_rest_parts_as_redis_does() {
    const SETUP: &[&[&str]] = &[
        &["FLUSHALL"],
        &["SET", "s", "abcdef"],
        &["SET", "empty", ""],
        &["LPUSH", "list", "x"],
    ];
    const VALID: &[&[&str]] = &[
        &["TOUCH", "s", "s", "missing", "list"],
        &["SUBSTR", "s", "1", "3"],
        &["SUBSTR", "s", "-3", "-1"],
        &["SUBSTR", "s", "9223372036854775807", "-1"],
        &["SUBSTR", "missing", "0", "-1"],
        &["SUBSTR", "empty", "0", "-1"],
        &["SELECT", "0"],
        &["SET", "after", "v"],
        &["DBSIZE"],
    ];
    const ERRORS: &[&[&str]] = &[
        &["TOUCH"],
        &["SUBSTR", "s", "0"],
        &["SUBSTR", "s", "bad", "2"],
        &["SUBSTR", "s", "0", "bad"],
        &["SUBSTR", "list", "0", "-1"],
        &["SELECT"],
        &["SELECT", "bad"],
        &["SELECT", "-1"],
        &["SELECT", "2147483647"],
        &["SELECT", "2147483648"],
        &["SELECT", "0", "extra"],
        &["TIME", "extra"],
        &["COMMAND", "COUNT", "extra"],
        &["COMMAND", "__unknown__"],
    ];
    const TRANSACTIONS: &[u8] =
        b"+OK\r\n+OK\r\n+OK\r\n:1\r\n+OK\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n\
+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n*9\r\n:3\r\n$3\r\nbcd\r\n\
$3\r\ndef\r\n$0\r\n\r\n$0\r\n\r\n$0\r\n\r\n+OK\r\n+OK\r\n:4\r\n+OK\r\n\
-ERR wrong number of arguments for 'touch' command\r\n\
-EXECABORT Transaction discarded because of previous errors.\r\n+OK\r\n\
-ERR wrong number of arguments for 'substr' command\r\n\
-EXECABORT Transaction discarded because of previous errors.\r\n+OK\r\n\
+QUEUED\r\n*1\r\n-ERR value is not an integer or out of range\r\n+OK\r\n\
+QUEUED\r\n*1\r\n-ERR value is not an integer or out of range\r\n+OK\r\n\
+QUEUED\r\n*1\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n+OK\r\n\
-ERR wrong number of arguments for 'select' command\r\n\
-EXECABORT Transaction discarded because of previous errors.\r\n+OK\r\n\
+QUEUED\r\n*1\r\n-ERR value is not an integer or out of range\r\n+OK\r\n\
+QUEUED\r\n*1\r\n-ERR DB index is out of range\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-ERR DB index is out of range\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-ERR value is out of range, value must between -2147483648 and 2147483647\r\n\
+OK\r\n-ERR wrong number of arguments for 'select' command\r\n\
-EXECABORT Transaction discarded because of previous errors.\r\n+OK\r\n\
-ERR wrong number of arguments for 'time' command\r\n\
-EXECABORT Transaction discarded because of previous errors.\r\n+OK\r\n\
-ERR wrong number of arguments for 'command|count' command\r\n\
-EXECABORT Transaction discarded because of previous errors.\r\n+OK\r\n\
-ERR unknown subcommand '__unknown__'. Try COMMAND HELP.\r\n\
-EXECABORT Transaction discarded because of previous errors.\r\n";
    const SCRIPTS: &[u8] =
        b"+OK\r\n+OK\r\n+OK\r\n:1\r\n:3\r\n$3\r\nbcd\r\n$3\r\ndef\r\n$0\r\n\r\n$0\r\n\r\n\
$0\r\n\r\n+OK\r\n+OK\r\n:4\r\n\
-ERR Wrong number of args calling Redis command from script\r\n\
-ERR Wrong number of args calling Redis command from script\r\n\
-ERR value is not an integer or out of range\r\n\
-ERR value is not an integer or out of range\r\n\
-WRONGTYPE Operation against a key holding the wrong kind of value\r\n\
-ERR Wrong number of args calling Redis command from script\r\n\
-ERR value is not an integer or out of range\r\n\
-ERR DB index is out of range\r\n-ERR DB index is out of range\r\n\
-ERR value is out of range, value must between -2147483648 and 2147483647\r\n\
-ERR Wrong number of args calling Redis command from script\r\n\
-ERR Wrong number of args calling Redis command from script\r\n\
-ERR Wrong number of args calling Redis command from script\r\n\
-ERR Unknown Redis command called from script\r\n";
    let program = firn();
    let port = free_port();
    let text = port.to_string();
    let child = program.spawn_on_route(true, &[text.as_bytes(), b"1"]);
    let mut client = connect_when_ready(port);
    let mut transactions: Vec<Vec<u8>> = SETUP.iter().map(|request| resp(request)).collect();
    transactions.push(resp(&["MULTI"]));
    for request in VALID {
        transactions.push(resp(request));
    }
    transactions.push(resp(&["EXEC"]));
    for request in ERRORS {
        transactions.push(resp(&["MULTI"]));
        transactions.push(resp(request));
        transactions.push(resp(&["EXEC"]));
    }
    let mut scripts: Vec<Vec<u8>> = SETUP.iter().map(|request| resp(request)).collect();
    for (calls, script) in [
        (VALID, "return redis.call(unpack(ARGV))"),
        (ERRORS, "return redis.pcall(unpack(ARGV))"),
    ] {
        for request in calls {
            let mut call = vec!["EVAL", script, "0"];
            call.extend_from_slice(request);
            scripts.push(resp(&call));
        }
    }
    for (requests, expected, what) in [
        (transactions, TRANSACTIONS, "the transactions"),
        (scripts, SCRIPTS, "the scripts"),
    ] {
        client
            .write_all(&requests.concat())
            .expect("send the requests");
        expect_replies(&mut client, expected, what);
    }
    drop(client);
    let (status, _) = finished(child);
    assert_eq!(status, 0);
}

/// firn parses SCAN, KEYS, RANDOMKEY, FLUSHALL and FLUSHDB as Redis does:
/// wrong argument counts, invalid cursors (out of range, signed, spaced or
/// empty), COUNT, MATCH and TYPE options with their syntax and range errors,
/// and the arguments Redis reads only up to their first zero byte. The same
/// requests, from the same keys, run in one transaction, each error or
/// boundary request in a transaction of its own, then through a script's
/// redis.call and the errors through redis.pcall, and give the replies
/// redis-server 7.0.15 gives (Firn-wf probe run 37551292895).
#[cfg(target_os = "linux")]
#[test]
fn firn_runs_scan_parts_as_redis_does() {
    const SETUP: &[&[&str]] = &[&["FLUSHALL"]];
    const VALID: &[&[&str]] = &[];
    const ERRORS: &[&[&str]] = &[
        &["SCAN"],
        &["SCAN", "bad"],
        &["SCAN", " "],
        &["SCAN", "0 "],
        &["SCAN", "+"],
        &["SCAN", "-"],
        &["SCAN", "18446744073709551616"],
        &["SCAN", "-18446744073709551616"],
        &["SCAN", "0", "COUNT"],
        &["SCAN", "0", "MATCH"],
        &["SCAN", "0", "TYPE"],
        &["SCAN", "0", "BOGUS", "x"],
        &["SCAN", "0", "COUNT", ""],
        &["SCAN", "0", "COUNT", "0"],
        &["SCAN", "0", "COUNT", "-1"],
        &["SCAN", "0", "COUNT", "+1"],
        &["SCAN", "0", "COUNT", "01"],
        &["SCAN", "0", "COUNT", "1.0"],
        &["SCAN", "0", "COUNT", "9223372036854775808"],
        &["SCAN", "0", "COUNT", "0", "COUNT", "1"],
        &["SCAN", ""],
        &["SCAN", "+0"],
        &["SCAN", "-0"],
        &["SCAN", "00"],
        &["SCAN", "-1"],
        &["SCAN", "18446744073709551615"],
        &["SCAN", "0\0ignored"],
        &["SCAN", "0", "COUNT", "9223372036854775807"],
        &["SCAN", "0", "COUNT\0ignored", "1"],
        &["SCAN", "0", "MATCH", ""],
        &["SCAN", "0", "TYPE", "StRiNg"],
        &["SCAN", "0", "TYPE", "none"],
        &["SCAN", "0", "TYPE", "unknown"],
        &["SCAN", "0", "TYPE", "string\0ignored"],
        &["SCAN", "0", "MATCH", "x", "MATCH", "*"],
        &["KEYS"],
        &["KEYS", "*", "extra"],
        &["KEYS", "*"],
        &["KEYS", ""],
        &["RANDOMKEY", "extra"],
        &["RANDOMKEY"],
        &["FLUSHALL", "bad"],
        &["FLUSHDB", "SYNC", "ASYNC"],
        &["FLUSHALL"],
        &["FLUSHDB", "ASYNC"],
        &["FLUSHALL", "sync"],
        &["FLUSHDB", "SYNC\0ignored"],
    ];
    const TRANSACTIONS: &[u8] =
        b"+OK\r\n+OK\r\n*0\r\n+OK\r\n-ERR wrong number of arguments for 'scan' command\r\n\
-EXECABORT Transaction discarded because of previous errors.\r\n+OK\r\n\
+QUEUED\r\n*1\r\n-ERR invalid cursor\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-ERR invalid cursor\r\n+OK\r\n+QUEUED\r\n*1\r\n-ERR invalid cursor\r\n+OK\r\n\
+QUEUED\r\n*1\r\n-ERR invalid cursor\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-ERR invalid cursor\r\n+OK\r\n+QUEUED\r\n*1\r\n-ERR invalid cursor\r\n+OK\r\n\
+QUEUED\r\n*1\r\n-ERR invalid cursor\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-ERR syntax error\r\n+OK\r\n+QUEUED\r\n*1\r\n-ERR syntax error\r\n+OK\r\n\
+QUEUED\r\n*1\r\n-ERR syntax error\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-ERR syntax error\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-ERR value is not an integer or out of range\r\n+OK\r\n+QUEUED\r\n*1\r\n\
-ERR syntax error\r\n+OK\r\n+QUEUED\r\n*1\r\n-ERR syntax error\r\n+OK\r\n\
+QUEUED\r\n*1\r\n-ERR value is not an integer or out of range\r\n+OK\r\n\
+QUEUED\r\n*1\r\n-ERR value is not an integer or out of range\r\n+OK\r\n\
+QUEUED\r\n*1\r\n-ERR value is not an integer or out of range\r\n+OK\r\n\
+QUEUED\r\n*1\r\n-ERR value is not an integer or out of range\r\n+OK\r\n\
+QUEUED\r\n*1\r\n-ERR syntax error\r\n+OK\r\n+QUEUED\r\n*1\r\n*2\r\n$1\r\n0\r\n\
*0\r\n+OK\r\n+QUEUED\r\n*1\r\n*2\r\n$1\r\n0\r\n*0\r\n+OK\r\n+QUEUED\r\n*1\r\n\
*2\r\n$1\r\n0\r\n*0\r\n+OK\r\n+QUEUED\r\n*1\r\n*2\r\n$1\r\n0\r\n*0\r\n+OK\r\n\
+QUEUED\r\n*1\r\n*2\r\n$1\r\n0\r\n*0\r\n+OK\r\n+QUEUED\r\n*1\r\n*2\r\n$1\r\n\
0\r\n*0\r\n+OK\r\n+QUEUED\r\n*1\r\n*2\r\n$1\r\n0\r\n*0\r\n+OK\r\n+QUEUED\r\n\
*1\r\n*2\r\n$1\r\n0\r\n*0\r\n+OK\r\n+QUEUED\r\n*1\r\n*2\r\n$1\r\n0\r\n*0\r\n\
+OK\r\n+QUEUED\r\n*1\r\n*2\r\n$1\r\n0\r\n*0\r\n+OK\r\n+QUEUED\r\n*1\r\n*2\r\n\
$1\r\n0\r\n*0\r\n+OK\r\n+QUEUED\r\n*1\r\n*2\r\n$1\r\n0\r\n*0\r\n+OK\r\n\
+QUEUED\r\n*1\r\n*2\r\n$1\r\n0\r\n*0\r\n+OK\r\n+QUEUED\r\n*1\r\n*2\r\n$1\r\n\
0\r\n*0\r\n+OK\r\n+QUEUED\r\n*1\r\n*2\r\n$1\r\n0\r\n*0\r\n+OK\r\n\
-ERR wrong number of arguments for 'keys' command\r\n\
-EXECABORT Transaction discarded because of previous errors.\r\n+OK\r\n\
-ERR wrong number of arguments for 'keys' command\r\n\
-EXECABORT Transaction discarded because of previous errors.\r\n+OK\r\n\
+QUEUED\r\n*1\r\n*0\r\n+OK\r\n+QUEUED\r\n*1\r\n*0\r\n+OK\r\n\
-ERR wrong number of arguments for 'randomkey' command\r\n\
-EXECABORT Transaction discarded because of previous errors.\r\n+OK\r\n\
+QUEUED\r\n*1\r\n$-1\r\n+OK\r\n+QUEUED\r\n*1\r\n-ERR syntax error\r\n+OK\r\n\
+QUEUED\r\n*1\r\n-ERR syntax error\r\n+OK\r\n+QUEUED\r\n*1\r\n+OK\r\n+OK\r\n\
+QUEUED\r\n*1\r\n+OK\r\n+OK\r\n+QUEUED\r\n*1\r\n+OK\r\n+OK\r\n+QUEUED\r\n*1\r\n\
+OK\r\n";
    const SCRIPTS: &[u8] =
        b"+OK\r\n-ERR Wrong number of args calling Redis command from script\r\n\
-ERR invalid cursor\r\n-ERR invalid cursor\r\n-ERR invalid cursor\r\n\
-ERR invalid cursor\r\n-ERR invalid cursor\r\n-ERR invalid cursor\r\n\
-ERR invalid cursor\r\n-ERR syntax error\r\n-ERR syntax error\r\n\
-ERR syntax error\r\n-ERR syntax error\r\n\
-ERR value is not an integer or out of range\r\n-ERR syntax error\r\n\
-ERR syntax error\r\n-ERR value is not an integer or out of range\r\n\
-ERR value is not an integer or out of range\r\n\
-ERR value is not an integer or out of range\r\n\
-ERR value is not an integer or out of range\r\n-ERR syntax error\r\n*2\r\n\
$1\r\n0\r\n*0\r\n*2\r\n$1\r\n0\r\n*0\r\n*2\r\n$1\r\n0\r\n*0\r\n*2\r\n$1\r\n0\r\n\
*0\r\n*2\r\n$1\r\n0\r\n*0\r\n*2\r\n$1\r\n0\r\n*0\r\n*2\r\n$1\r\n0\r\n*0\r\n\
*2\r\n$1\r\n0\r\n*0\r\n*2\r\n$1\r\n0\r\n*0\r\n*2\r\n$1\r\n0\r\n*0\r\n*2\r\n\
$1\r\n0\r\n*0\r\n*2\r\n$1\r\n0\r\n*0\r\n*2\r\n$1\r\n0\r\n*0\r\n*2\r\n$1\r\n0\r\n\
*0\r\n*2\r\n$1\r\n0\r\n*0\r\n\
-ERR Wrong number of args calling Redis command from script\r\n\
-ERR Wrong number of args calling Redis command from script\r\n*0\r\n*0\r\n\
-ERR Wrong number of args calling Redis command from script\r\n$-1\r\n\
-ERR syntax error\r\n-ERR syntax error\r\n+OK\r\n+OK\r\n+OK\r\n+OK\r\n";
    let program = firn();
    let port = free_port();
    let text = port.to_string();
    let child = program.spawn_on_route(true, &[text.as_bytes(), b"1"]);
    let mut client = connect_when_ready(port);
    let mut transactions: Vec<Vec<u8>> = SETUP.iter().map(|request| resp(request)).collect();
    transactions.push(resp(&["MULTI"]));
    for request in VALID {
        transactions.push(resp(request));
    }
    transactions.push(resp(&["EXEC"]));
    for request in ERRORS {
        transactions.push(resp(&["MULTI"]));
        transactions.push(resp(request));
        transactions.push(resp(&["EXEC"]));
    }
    let mut scripts: Vec<Vec<u8>> = SETUP.iter().map(|request| resp(request)).collect();
    for (calls, script) in [
        (VALID, "return redis.call(unpack(ARGV))"),
        (ERRORS, "return redis.pcall(unpack(ARGV))"),
    ] {
        for request in calls {
            let mut call = vec!["EVAL", script, "0"];
            call.extend_from_slice(request);
            scripts.push(resp(&call));
        }
    }
    for (requests, expected, what) in [
        (transactions, TRANSACTIONS, "the transactions"),
        (scripts, SCRIPTS, "the scripts"),
    ] {
        client
            .write_all(&requests.concat())
            .expect("send the requests");
        expect_replies(&mut client, expected, what);
    }
    drop(client);
    let (status, _) = finished(child);
    assert_eq!(status, 0);
}

/// firn records SCAN and KEYS run inside a transaction's EXEC and a script
/// as Redis 7.0.15 propagates them over keys found expired: SCAN removes
/// each expired key its MATCH pattern selects and records it as DEL where it
/// meets it, among the block's writes, including a key its TYPE option then
/// leaves out of the reply, and leaves an expired key the pattern does not
/// select; KEYS removes and records nothing. The expected records are
/// redis-server 7.0.15's (Firn-wf probe run 37551806041).
#[cfg(target_os = "linux")]
#[test]
fn firn_records_held_scan_commands_as_redis_propagates_them() {
    check_held_records(
        &[
            &["SET", "k1", "1"],
            &["PEXPIREAT", "k1", "1"],
            &["SET", "k2", "2"],
            &["HSET", "h1", "f", "v"],
            &["PEXPIREAT", "h1", "1"],
            &["SET", "x1", "3"],
            &["PEXPIREAT", "x1", "1"],
            &["SET", "s1", "4"],
            &["PEXPIREAT", "s1", "1"],
        ],
        &[
            &["MULTI"],
            &["SCAN", "0", "MATCH", "k*", "COUNT", "1000"],
            &["SCAN", "0", "MATCH", "h*", "TYPE", "string", "COUNT", "1000"],
            &["KEYS", "x*"],
            &["SET", "w", "1"],
            &["SCAN", "0", "MATCH", "x*", "COUNT", "1000"],
            &["EXEC"],
            &["EVAL", "local r = redis.call('SCAN', '0', 'MATCH', 's*', 'COUNT', '1000'); redis.call('SET', 't', '1'); return r", "0"],
            &["DBSIZE"],
        ],
        b"+OK\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n*5\r\n*2\r\n$1\r\n0\r\n*1\r\n$2\r\nk2\r\n*2\r\n$1\r\n0\r\n*0\r\n*0\r\n+OK\r\n*2\r\n$1\r\n0\r\n*0\r\n*2\r\n$1\r\n0\r\n*0\r\n:3\r\n",
        b"*1\r\n$5\r\nMULTI\r\n*2\r\n$3\r\nDEL\r\n$2\r\nk1\r\n*2\r\n$3\r\nDEL\r\n$2\r\nh1\r\n*3\r\n$3\r\nSET\r\n$1\r\nw\r\n$1\r\n1\r\n*2\r\n$3\r\nDEL\r\n$2\r\nx1\r\n*1\r\n$4\r\nEXEC\r\n*1\r\n$5\r\nMULTI\r\n*2\r\n$3\r\nDEL\r\n$2\r\ns1\r\n*3\r\n$3\r\nSET\r\n$1\r\nt\r\n$1\r\n1\r\n*1\r\n$4\r\nEXEC\r\n",
    );
}

/// firn records RANDOMKEY run inside a transaction's EXEC as Redis 7.0.15
/// propagates it: an expired key it draws is removed and recorded as DEL
/// inside the transaction's MULTI and EXEC, before the writes that follow
/// it, and with no live key left it answers nil. The expected records are
/// redis-server 7.0.15's (Firn-wf probe run 37551806041).
#[cfg(target_os = "linux")]
#[test]
fn firn_records_held_randomkey_as_redis_propagates_it() {
    check_held_records(
        &[
            &["SET", "r1", "1"],
            &["PEXPIREAT", "r1", "1"],
        ],
        &[
            &["MULTI"],
            &["RANDOMKEY"],
            &["SET", "w", "1"],
            &["EXEC"],
            &["RANDOMKEY"],
            &["DBSIZE"],
        ],
        b"+OK\r\n+QUEUED\r\n+QUEUED\r\n*2\r\n$-1\r\n+OK\r\n$1\r\nw\r\n:1\r\n",
        b"*1\r\n$5\r\nMULTI\r\n*2\r\n$3\r\nDEL\r\n$2\r\nr1\r\n*3\r\n$3\r\nSET\r\n$1\r\nw\r\n$1\r\n1\r\n*1\r\n$4\r\nEXEC\r\n",
    );
}

/// firn records RANDOMKEY run inside a script as Redis 7.0.15 propagates it:
/// an expired key it draws is removed and recorded as DEL inside the
/// script's MULTI and EXEC, before the script's writes, and with no live key
/// left it answers nil. The expected records are redis-server 7.0.15's
/// (Firn-wf probe run 37551806041).
#[cfg(target_os = "linux")]
#[test]
fn firn_records_held_randomkey_in_scripts_as_redis_propagates_it() {
    check_held_records(
        &[
            &["SET", "q1", "1"],
            &["PEXPIREAT", "q1", "1"],
        ],
        &[
            &["EVAL", "local k = redis.call('RANDOMKEY'); redis.call('SET', 'z', '1'); return k", "0"],
            &["DBSIZE"],
        ],
        b"$-1\r\n:1\r\n",
        b"*1\r\n$5\r\nMULTI\r\n*2\r\n$3\r\nDEL\r\n$2\r\nq1\r\n*3\r\n$3\r\nSET\r\n$1\r\nz\r\n$1\r\n1\r\n*1\r\n$4\r\nEXEC\r\n",
    );
}

/// firn records FLUSHALL and FLUSHDB run inside a transaction's EXEC and a
/// script as Redis 7.0.15 propagates them: each is recorded with its
/// arguments as sent, in place among the block's writes, and empties the
/// keyspace with its expiries, so a key written again after it has none. The
/// expected records are redis-server 7.0.15's (Firn-wf probe run
/// 37551806041).
#[cfg(target_os = "linux")]
#[test]
fn firn_records_held_flush_commands_as_redis_propagates_them() {
    check_held_records(
        &[
            &["SET", "a", "1"],
            &["SET", "e", "1"],
            &["PEXPIREAT", "e", "9999999999999"],
        ],
        &[
            &["MULTI"],
            &["SET", "c", "3"],
            &["FLUSHALL"],
            &["SET", "d", "4"],
            &["SET", "e", "1"],
            &["EXEC"],
            &["TTL", "e"],
            &["EVAL", "redis.call('SET', 'e', '5'); redis.call('FLUSHDB', 'async'); redis.call('SET', 'f', '6'); return redis.call('KEYS', '*')", "0"],
            &["DBSIZE"],
            &["TTL", "e"],
        ],
        b"+OK\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n*4\r\n+OK\r\n+OK\r\n+OK\r\n+OK\r\n:-1\r\n*1\r\n$1\r\nf\r\n:1\r\n:-2\r\n",
        b"*1\r\n$5\r\nMULTI\r\n*3\r\n$3\r\nSET\r\n$1\r\nc\r\n$1\r\n3\r\n*1\r\n$8\r\nFLUSHALL\r\n*3\r\n$3\r\nSET\r\n$1\r\nd\r\n$1\r\n4\r\n*3\r\n$3\r\nSET\r\n$1\r\ne\r\n$1\r\n1\r\n*1\r\n$4\r\nEXEC\r\n*1\r\n$5\r\nMULTI\r\n*3\r\n$3\r\nSET\r\n$1\r\ne\r\n$1\r\n5\r\n*2\r\n$7\r\nFLUSHDB\r\n$5\r\nasync\r\n*3\r\n$3\r\nSET\r\n$1\r\nf\r\n$1\r\n6\r\n*1\r\n$4\r\nEXEC\r\n",
    );
}

/// firn enumerates its keyspace as Redis 7.0.15's SCAN, KEYS and RANDOMKEY
/// promise: a SCAN iteration from cursor 0 until it returns 0 again yields
/// every key present throughout, at any COUNT, with MATCH selecting keys by
/// a case-sensitive glob pattern, empty and binary keys included, and TYPE
/// selecting them by kind in either case; an expired key is never returned;
/// KEYS returns the same keys in one reply; RANDOMKEY returns a present key,
/// not always the same one, and nil once FLUSHDB has emptied the keyspace,
/// after which SCAN and KEYS return nothing. Redis fixes no order, so the
/// keys the case writes are the oracle. Redis may return a key twice while
/// its table grows; firn's cursor never does, which the case also checks.
#[cfg(target_os = "linux")]
#[test]
fn firn_enumerates_keys_as_redis_does() {
    fn scan_all(client: &mut TcpStream, options: &[&str]) -> Vec<String> {
        let mut cursor = "0".to_string();
        let mut found = Vec::new();
        for _ in 0..100_000 {
            let request = {
                let mut request = vec!["SCAN", cursor.as_str()];
                request.extend_from_slice(options);
                resp(&request)
            };
            client.write_all(&request).expect("send SCAN");
            assert_eq!(reply_line(client, "SCAN"), "*2\r\n", "SCAN {options:?}");
            cursor = bulk_reply(client, "the SCAN cursor");
            found.extend(bulk_strings(client, "the SCAN keys"));
            if cursor == "0" {
                return found;
            }
        }
        panic!("SCAN {options:?} did not return to cursor 0");
    }
    fn same_keys(mut found: Vec<String>, mut expected: Vec<String>, what: &str) {
        found.sort();
        let count = found.len();
        found.dedup();
        assert_eq!(found.len(), count, "{what}: a key returned twice");
        expected.sort();
        assert_eq!(found, expected, "{what}");
    }
    let program = firn();
    let port = free_port();
    let text = port.to_string();
    let child = program.spawn_on_route(true, &[text.as_bytes(), b"1"]);
    let mut client = connect_when_ready(port);
    let mut kinds: Vec<(String, &str)> = Vec::new();
    let mut writes: Vec<Vec<u8>> = Vec::new();
    for index in 0..300 {
        let key = format!("user:{index}");
        writes.push(resp(&["SET", &key, "v"]));
        kinds.push((key, "string"));
    }
    for key in ["User:1", "", "bin\0key"] {
        writes.push(resp(&["SET", key, "v"]));
        kinds.push((key.to_string(), "string"));
    }
    for index in 0..20 {
        let hash = format!("h:{index}");
        writes.push(resp(&["HSET", &hash, "f", "v"]));
        kinds.push((hash, "hash"));
        let list = format!("l:{index}");
        writes.push(resp(&["RPUSH", &list, "v"]));
        kinds.push((list, "list"));
        let set = format!("s:{index}");
        writes.push(resp(&["SADD", &set, "v"]));
        kinds.push((set, "set"));
        let sorted = format!("z:{index}");
        writes.push(resp(&["ZADD", &sorted, "1", "v"]));
        kinds.push((sorted, "zset"));
    }
    writes.push(resp(&["SET", "gone", "v", "PX", "1"]));
    client.write_all(&writes.concat()).expect("send the writes");
    for _ in 0..writes.len() {
        let line = reply_line(&mut client, "a write");
        assert!(line == "+OK\r\n" || line == ":1\r\n", "a write: {line:?}");
    }
    std::thread::sleep(Duration::from_millis(50));
    let keys = |wanted: &dyn Fn(&str, &str) -> bool| -> Vec<String> {
        kinds
            .iter()
            .filter(|(key, kind)| wanted(key, kind))
            .map(|(key, _)| key.clone())
            .collect()
    };
    let all = keys(&|_, _| true);
    let counts: [&[&str]; 3] = [&[], &["COUNT", "1"], &["COUNT", "1000"]];
    for options in counts {
        same_keys(scan_all(&mut client, options), all.clone(), "SCAN");
    }
    let filtered: [(&[&str], Vec<String>); 5] = [
        (
            &["MATCH", "user:*", "COUNT", "7"],
            keys(&|key, _| key.starts_with("user:")),
        ),
        (
            &["MATCH", "user:1?"],
            (10..20).map(|index| format!("user:{index}")).collect(),
        ),
        (&["TYPE", "hash"], keys(&|_, kind| kind == "hash")),
        (
            &["TYPE", "ZSET", "COUNT", "3"],
            keys(&|_, kind| kind == "zset"),
        ),
        (
            &["MATCH", "*", "TYPE", "string"],
            keys(&|_, kind| kind == "string"),
        ),
    ];
    for (options, expected) in filtered {
        same_keys(
            scan_all(&mut client, options),
            expected,
            "SCAN with options",
        );
    }
    for (pattern, expected) in [
        ("*", all.clone()),
        ("h:*", keys(&|_, kind| kind == "hash")),
        ("[hl]:1", vec!["h:1".to_string(), "l:1".to_string()]),
    ] {
        client
            .write_all(&resp(&["KEYS", pattern]))
            .expect("send KEYS");
        same_keys(bulk_strings(&mut client, "KEYS"), expected, pattern);
    }
    client
        .write_all(&resp(&["RANDOMKEY"]).repeat(50))
        .expect("send RANDOMKEY");
    let mut drawn: Vec<String> = (0..50)
        .map(|_| bulk_reply(&mut client, "RANDOMKEY"))
        .collect();
    assert!(drawn.iter().all(|key| all.contains(key)), "{drawn:?}");
    drawn.sort();
    drawn.dedup();
    assert!(drawn.len() > 1, "RANDOMKEY always drew {drawn:?}");
    client.write_all(&resp(&["DBSIZE"])).expect("send DBSIZE");
    assert_eq!(integer_reply(&mut client, "DBSIZE"), all.len() as i64);
    client
        .write_all(&[resp(&["FLUSHDB"]), resp(&["RANDOMKEY"])].concat())
        .expect("send FLUSHDB");
    assert_eq!(reply_line(&mut client, "FLUSHDB"), "+OK\r\n");
    assert_eq!(reply_line(&mut client, "RANDOMKEY"), "$-1\r\n");
    same_keys(scan_all(&mut client, &[]), Vec::new(), "SCAN after FLUSHDB");
    client.write_all(&resp(&["KEYS", "*"])).expect("send KEYS");
    same_keys(
        bulk_strings(&mut client, "KEYS"),
        Vec::new(),
        "KEYS after FLUSHDB",
    );
    drop(client);
    let (status, _) = finished(child);
    assert_eq!(status, 0);
}

/// firn records SCAN with TYPE none run inside a transaction's EXEC and a script as Redis 7.0.15 propagates it: an expired key its MATCH pattern selects is removed, recorded as DEL where SCAN meets it, and named in the reply, since Redis reads a key's type before its expiry check; a live key is not named. The expected
/// records are redis-server 7.0.15's (Firn-wf probe run 37552883025).
#[cfg(target_os = "linux")]
#[test]
fn firn_records_held_scan_type_none_as_redis_propagates_it() {
    check_held_records(
        &[
            &["SET", "n1", "1"],
            &["PEXPIREAT", "n1", "1"],
            &["SET", "n2", "2"],
            &["SET", "n3", "3"],
            &["PEXPIREAT", "n3", "1"],
        ],
        &[
            &["MULTI"],
            &["SCAN", "0", "MATCH", "n1", "TYPE", "none", "COUNT", "1000"],
            &["SCAN", "0", "MATCH", "n2", "TYPE", "none", "COUNT", "1000"],
            &["SET", "w", "1"],
            &["EXEC"],
            &["EVAL", "local r = redis.call('SCAN', '0', 'MATCH', 'n3', 'TYPE', 'NONE', 'COUNT', '1000'); redis.call('SET', 't', '1'); return r", "0"],
            &["DBSIZE"],
        ],
        b"+OK\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n*3\r\n*2\r\n$1\r\n0\r\n*1\r\n$2\r\nn1\r\n*2\r\n$1\r\n0\r\n*0\r\n+OK\r\n*2\r\n$1\r\n0\r\n*1\r\n$2\r\nn3\r\n:3\r\n",
        b"*1\r\n$5\r\nMULTI\r\n*2\r\n$3\r\nDEL\r\n$2\r\nn1\r\n*3\r\n$3\r\nSET\r\n$1\r\nw\r\n$1\r\n1\r\n*1\r\n$4\r\nEXEC\r\n*1\r\n$5\r\nMULTI\r\n*2\r\n$3\r\nDEL\r\n$2\r\nn3\r\n*3\r\n$3\r\nSET\r\n$1\r\nt\r\n$1\r\n1\r\n*1\r\n$4\r\nEXEC\r\n",
    );
}

/// firn records a script's writes as Redis 7.0.15 propagates them: a script
/// of two writes bracketed in MULTI and EXEC, one of one write as that write
/// alone, and one that only reads not at all; a restart replays the file to
/// the same keys. The expected file is redis-server 7.0.15's for the same
/// scripts with one database, without the SELECT it begins with.
#[cfg(target_os = "linux")]
#[test]
fn firn_records_scripts_as_redis_propagates_them() {
    let program = firn();
    let fixture = fixture_directory();
    let port = free_port();
    let text = port.to_string();
    let client = std::thread::spawn(move || {
        let mut client = connect_when_ready(port);
        let mut batch = Vec::new();
        for request in [
            vec![
                "EVAL",
                "redis.call('SET','a','1'); redis.call('SET','b','2')",
                "0",
            ],
            vec!["EVAL", "return redis.call('GET','a')", "0"],
            vec!["EVAL", "return redis.call('INCR','a')", "0"],
        ] {
            batch.extend(resp(&request));
        }
        client.write_all(&batch).expect("send the scripts");
        expect_replies(&mut client, b"$-1\r\n$1\r\n1\r\n:2\r\n", "the scripts");
    });
    let output = program.run(fixture.path(), &[text.as_bytes(), b"1", b"scripts.aof"]);
    client.join().expect("the client's exchange");
    assert!(output.status.success(), "firn: {:?}", output.status);
    let file = std::fs::read(fixture.path().join("scripts.aof")).expect("read firn's file");
    assert_eq!(
        String::from_utf8_lossy(&file),
        "*1\r\n$5\r\nMULTI\r\n*3\r\n$3\r\nSET\r\n$1\r\na\r\n$1\r\n1\r\n*3\r\n$3\r\nSET\r\n$1\r\nb\r\n$1\r\n2\r\n*1\r\n$4\r\nEXEC\r\n*2\r\n$4\r\nINCR\r\n$1\r\na\r\n",
    );
    let port = free_port();
    let text = port.to_string();
    let client = std::thread::spawn(move || {
        let mut client = connect_when_ready(port);
        client
            .write_all(&resp(&["MGET", "a", "b"]))
            .expect("read the replayed keys");
        expect_replies(
            &mut client,
            b"*2\r\n$1\r\n2\r\n$1\r\n2\r\n",
            "the replayed keys",
        );
    });
    let output = program.run(fixture.path(), &[text.as_bytes(), b"1", b"scripts.aof"]);
    client.join().expect("the replay's exchange");
    assert!(output.status.success(), "firn: {:?}", output.status);
}

/// Reads one RESP2 reply of bulk strings: a bulk string, none for the null
/// bulk string, or each element of an array of bulk strings.
#[cfg(target_os = "linux")]
fn bulk_strings(stream: &mut TcpStream, what: &str) -> Vec<String> {
    let number = |line: &str| -> i64 {
        line[1..line.len() - 2]
            .parse()
            .unwrap_or_else(|_| panic!("{what}: not a count: {line:?}"))
    };
    let body = |stream: &mut TcpStream, line: &str| -> Option<String> {
        let length = usize::try_from(number(line)).ok()?;
        let mut bytes = vec![0_u8; length + 2];
        stream
            .read_exact(&mut bytes)
            .unwrap_or_else(|error| panic!("{what}: {error}"));
        bytes.truncate(length);
        Some(String::from_utf8_lossy(&bytes).into_owned())
    };
    let header = reply_line(stream, what);
    if header.starts_with('$') {
        return body(stream, &header).into_iter().collect();
    }
    assert!(
        header.starts_with('*'),
        "{what}: not bulk strings: {header:?}"
    );
    (0..number(&header))
        .map(|_| {
            let line = reply_line(stream, what);
            assert!(line.starts_with('$'), "{what}: not a bulk string: {line:?}");
            body(stream, &line).unwrap_or_else(|| panic!("{what}: a null element"))
        })
        .collect()
}

/// The commands an append-only file holds, each a RESP array of bulk
/// strings, up to the first one not yet written whole.
#[cfg(target_os = "linux")]
fn aof_records(bytes: &[u8]) -> Vec<Vec<String>> {
    fn number(bytes: &[u8], at: &mut usize, marker: u8) -> Option<usize> {
        if bytes.get(*at) != Some(&marker) {
            return None;
        }
        let end = *at + bytes[*at..].windows(2).position(|pair| pair == b"\r\n")?;
        let value = std::str::from_utf8(&bytes[*at + 1..end])
            .ok()?
            .parse()
            .ok()?;
        *at = end + 2;
        Some(value)
    }
    let mut records = Vec::new();
    let mut at = 0;
    while let Some(count) = number(bytes, &mut at, b'*') {
        let mut record = Vec::new();
        for _ in 0..count {
            let Some(length) = number(bytes, &mut at, b'$') else {
                return records;
            };
            let Some(body) = bytes.get(at..at + length) else {
                return records;
            };
            record.push(String::from_utf8_lossy(body).into_owned());
            at += length + 2;
        }
        if at > bytes.len() {
            break;
        }
        records.push(record);
    }
    records
}

/// firn answers hash updates, numeric increments, random field draws and
/// sorted-set updates, ranges, ranks and removals as Redis does, including
/// ranges in sets of 200 members. The expected bytes are those redis-server
/// 7.0.15 returns for the same requests.
#[cfg(target_os = "linux")]
#[test]
fn firn_answers_the_value_types_as_redis_does_with_hashes_and_sorted_sets() {
    let program = firn();
    let port = free_port();
    let text = port.to_string();
    let child = program.spawn_on_route(true, &[text.as_bytes(), b"1"]);
    let mut client = connect_when_ready(port);
    client
        .set_read_timeout(Some(Duration::from_secs(20)))
        .expect("bound the client's waits");
    let mut batch = Vec::new();
    for request in [
        vec!["HSET", "hash", "f", "v", "g", "2"],
        vec!["HMSET", "hash", "x", "1", "y", "2"],
        vec!["HMSET", "hash", "z"],
        vec!["HSET", "hash", "x", "1", "z"],
        vec!["HMGET", "hash", "f", "nope", "x"],
        vec!["HMGET", "nohash", "a", "b"],
        vec!["HDEL", "hash", "x", "nope", "y", "x"],
        vec!["HEXISTS", "hash", "f"],
        vec!["HEXISTS", "hash", "x"],
        vec!["HSTRLEN", "hash", "f"],
        vec!["HSTRLEN", "hash", "nope"],
        vec!["HLEN", "hash"],
        vec!["HLEN", "nohash"],
        vec!["HINCRBY", "hash", "g", "40"],
        vec!["HINCRBY", "hash", "f", "1"],
        vec!["HINCRBY", "hash", "g", "9223372036854775807"],
        vec!["HINCRBY", "hash", "g", "x"],
        vec!["HINCRBY", "hash", "new", "-5"],
        vec!["HINCRBYFLOAT", "hash", "r", "0.1"],
        vec!["HINCRBYFLOAT", "hash", "r", "0.2"],
        vec!["HINCRBYFLOAT", "hash", "t", "3.814697265625e-06"],
        vec!["HINCRBYFLOAT", "hash", "u", "0x1p-16445"],
        vec!["HINCRBYFLOAT", "hash", "u", "1.2e4932"],
        vec!["HINCRBYFLOAT", "hash", "w", "18446744073709551615"],
        vec!["HINCRBYFLOAT", "hash", "w", "0.5"],
        vec!["HINCRBYFLOAT", "hash", "f", "1"],
        vec!["HINCRBYFLOAT", "hash", "r", "inf"],
        vec!["HINCRBYFLOAT", "hash", "r", "1\0"],
        vec!["HINCRBYFLOAT", "hash", "r", "nan"],
        vec!["HINCRBYFLOAT", "hash", "r", " 1"],
        vec!["HINCRBYFLOAT", "hash", "big", "1e400"],
        vec!["HINCRBYFLOAT", "hash", "zero", "-0"],
        vec!["HINCRBYFLOAT", "hash", "zero", "-1e-30"],
        vec!["HSET", "hash", "i", "inf"],
        vec!["HINCRBYFLOAT", "hash", "i", "1"],
        vec!["HSETNX", "hash", "f", "w"],
        vec!["HSETNX", "hash", "n", "w"],
        vec!["HSETNX", "fresh", "a", "1"],
        vec!["HSET", "single", "a", "b"],
        vec!["HGETALL", "single"],
        vec!["HKEYS", "single"],
        vec!["HVALS", "single"],
        vec!["HGETALL", "nohash"],
        vec!["HDEL", "single", "a"],
        vec!["EXISTS", "single"],
        vec!["SET", "text", "v"],
        vec!["HGET", "text", "f"],
        vec!["HMGET", "text", "f"],
        vec!["HINCRBYFLOAT", "text", "f", "x"],
        vec!["HINCRBYFLOAT", "text", "f", "1"],
        vec!["HRANDFIELD", "nohash"],
        vec!["HRANDFIELD", "nohash", "3"],
        vec!["HRANDFIELD", "hash", "1", "WITHVALUE"],
        vec!["HRANDFIELD", "hash", "-4611686018427387904", "WITHVALUES"],
        vec!["HRANDFIELD", "hash", "-9223372036854775808"],
    ] {
        batch.extend(resp(&request));
    }
    client.write_all(&batch).expect("send the hash batch");
    expect_replies(
        &mut client,
        b":2\r\n+OK\r\n-ERR wrong number of arguments for 'hmset' command\r\n-ERR wrong number of arguments for 'hset' command\r\n*3\r\n$1\r\nv\r\n$-1\r\n$1\r\n1\r\n*2\r\n$-1\r\n$-1\r\n:2\r\n:1\r\n:0\r\n:1\r\n:0\r\n:2\r\n:0\r\n:42\r\n-ERR hash value is not an integer\r\n-ERR increment or decrement would overflow\r\n-ERR value is not an integer or out of range\r\n:-5\r\n$3\r\n0.1\r\n$3\r\n0.3\r\n$19\r\n0.00000381469726562\r\n$1\r\n0\r\n-ERR value is not a valid float\r\n$20\r\n18446744073709551615\r\n$20\r\n18446744073709551616\r\n-ERR hash value is not a float\r\n-ERR value is NaN or Infinity\r\n-ERR value is not a valid float\r\n-ERR value is not a valid float\r\n-ERR value is not a valid float\r\n$401\r\n10000000000000000000281880683947586514586453433629052038625910693539685534008629862039363994848324160522094053927317616200295822777259255734023828976593340661017797447434546173917862448116674971723778943824391593338047470675026246684401359237513603830343735485505244955964979021825038280091068414947402456898653040951017512658092615827588920183472511643316591362664138176309734806343732497430221946880\r\n$1\r\n0\r\n$1\r\n0\r\n:1\r\n-ERR increment would produce NaN or Infinity\r\n:0\r\n:1\r\n:1\r\n:1\r\n*2\r\n$1\r\na\r\n$1\r\nb\r\n*1\r\n$1\r\na\r\n*1\r\n$1\r\nb\r\n*0\r\n:1\r\n:0\r\n+OK\r\n-WRONGTYPE Operation against a key holding the wrong kind of value\r\n-WRONGTYPE Operation against a key holding the wrong kind of value\r\n-ERR value is not a valid float\r\n-WRONGTYPE Operation against a key holding the wrong kind of value\r\n$-1\r\n*0\r\n-ERR syntax error\r\n-ERR value is out of range\r\n-ERR value is out of range, value must between -9223372036854775807 and 9223372036854775807\r\n",
        "the hash batch",
    );
    let mut batch = Vec::new();
    for request in [
        vec!["ZADD", "zset", "1", "a", "2", "b", "3", "c", "4", "d"],
        vec!["ZADD", "zset", "NX", "CH", "9", "a", "5", "e"],
        vec!["ZADD", "zset", "XX", "CH", "1.5", "a", "9", "nobody"],
        vec!["ZADD", "zset", "GT", "CH", "0", "b", "10", "c"],
        vec!["ZADD", "zset", "LT", "100", "d"],
        vec!["ZADD", "zset", "INCR", "1", "a"],
        vec!["ZADD", "zset", "INCR", "NX", "1", "a"],
        vec!["ZADD", "zset", "nx", "xx", "1", "a"],
        vec!["ZADD", "zset", "gt", "lt", "1", "a"],
        vec!["ZADD", "zset", "incr", "1", "a", "2", "b"],
        vec!["ZADD", "zset", "ch"],
        vec!["ZINCRBY", "zset", "-0", "fresh"],
        vec!["ZSCORE", "zset", "fresh"],
        vec!["ZINCRBY", "zset", "inf", "inc"],
        vec!["ZINCRBY", "zset", "-inf", "inc"],
        vec!["ZRANGE", "zset", "0", "-1", "WITHSCORES"],
        vec!["ZRANGE", "zset", "1", "2", "REV"],
        vec!["ZRANGE", "zset", "(2", "+inf", "BYSCORE", "LIMIT", "1", "2"],
        vec![
            "ZRANGE",
            "zset",
            "+inf",
            "(2",
            "BYSCORE",
            "REV",
            "WITHSCORES",
        ],
        vec!["ZRANGE", "zset", "0", "-1", "LIMIT", "1", "2"],
        vec!["ZRANGE", "zset", "0", "-1", "REV", "REV"],
        vec!["ZREVRANGE", "zset", "0", "1"],
        vec!["ZRANGEBYSCORE", "zset", "-inf", "2.5"],
        vec!["ZREVRANGEBYSCORE", "zset", "10", "(2"],
        vec!["ZRANGEBYSCORE", "zset", " 2", "1e400"],
        vec!["ZRANGEBYSCORE", "zset", "", "("],
        vec!["ZRANGEBYSCORE", "zset", "x", "1"],
        vec!["ZRANGEBYSCORE", "zset", "1", "2", "REV"],
        vec!["ZADD", "lex", "0", "a", "0", "b", "0", "c", "0", "d"],
        vec!["ZRANGEBYLEX", "lex", "[b", "(d"],
        vec!["ZREVRANGEBYLEX", "lex", "+", "-", "LIMIT", "1", "2"],
        vec!["ZRANGE", "lex", "-", "+", "BYLEX", "WITHSCORES"],
        vec!["ZRANGEBYLEX", "lex", "b", "+"],
        vec!["ZLEXCOUNT", "lex", "[b", "+"],
        vec!["ZADD", "mixed", "1", "d", "2", "a", "3", "c", "4", "b"],
        vec!["ZRANGEBYLEX", "mixed", "[b", "[c"],
        vec!["ZRANGEBYLEX", "mixed", "[a", "[c"],
        vec!["ZRANGEBYLEX", "mixed", "[c", "+"],
        vec!["ZREVRANGEBYLEX", "mixed", "[b", "[a"],
        vec!["ZCOUNT", "zset", "(2", "10"],
        vec!["ZRANK", "zset", "c"],
        vec!["ZREVRANK", "zset", "c"],
        vec!["ZRANK", "zset", "nope"],
        vec!["ZMSCORE", "zset", "a", "nope", "e"],
        vec!["ZMSCORE", "nozset", "a"],
        vec!["ZREM", "zset", "e", "nope"],
        vec!["ZPOPMAX", "zset", "2"],
        vec!["ZPOPMIN", "zset", "x"],
        vec!["ZPOPMIN", "zset", "1", "2"],
        vec!["ZPOPMIN", "text", "0"],
        vec![
            "ZADD", "ranks", "1", "a", "2", "b", "3", "c", "4", "d", "5", "e",
        ],
        vec!["ZREMRANGEBYRANK", "ranks", "-2", "-1"],
        vec!["ZREMRANGEBYSCORE", "ranks", "(1", "2"],
        vec!["ZREMRANGEBYLEX", "lex", "[b", "[c"],
        vec!["ZRANGE", "ranks", "0", "-1"],
        vec!["ZRANGE", "lex", "0", "-1"],
        vec!["ZREMRANGEBYRANK", "ranks", "0", "-1"],
        vec!["EXISTS", "ranks"],
        vec!["ZRANGE", "text", "0", "-1"],
        vec!["ZCOUNT", "text", "0", "1"],
    ] {
        batch.extend(resp(&request));
    }
    client.write_all(&batch).expect("send the sorted-set batch");
    expect_replies(
        &mut client,
        b":4\r\n:1\r\n:1\r\n:1\r\n:0\r\n$3\r\n2.5\r\n$-1\r\n-ERR XX and NX options at the same time are not compatible\r\n-ERR GT, LT, and/or NX options at the same time are not compatible\r\n-ERR INCR option supports a single increment-element pair\r\n-ERR wrong number of arguments for 'zadd' command\r\n$2\r\n-0\r\n$1\r\n0\r\n$3\r\ninf\r\n-ERR resulting score is not a number (NaN)\r\n*14\r\n$5\r\nfresh\r\n$1\r\n0\r\n$1\r\nb\r\n$1\r\n2\r\n$1\r\na\r\n$3\r\n2.5\r\n$1\r\nd\r\n$1\r\n4\r\n$1\r\ne\r\n$1\r\n5\r\n$1\r\nc\r\n$2\r\n10\r\n$3\r\ninc\r\n$3\r\ninf\r\n*2\r\n$1\r\nc\r\n$1\r\ne\r\n*2\r\n$1\r\nd\r\n$1\r\ne\r\n*10\r\n$3\r\ninc\r\n$3\r\ninf\r\n$1\r\nc\r\n$2\r\n10\r\n$1\r\ne\r\n$1\r\n5\r\n$1\r\nd\r\n$1\r\n4\r\n$1\r\na\r\n$3\r\n2.5\r\n-ERR syntax error, LIMIT is only supported in combination with either BYSCORE or BYLEX\r\n-ERR syntax error\r\n*2\r\n$3\r\ninc\r\n$1\r\nc\r\n*3\r\n$5\r\nfresh\r\n$1\r\nb\r\n$1\r\na\r\n*4\r\n$1\r\nc\r\n$1\r\ne\r\n$1\r\nd\r\n$1\r\na\r\n*6\r\n$1\r\nb\r\n$1\r\na\r\n$1\r\nd\r\n$1\r\ne\r\n$1\r\nc\r\n$3\r\ninc\r\n*0\r\n-ERR min or max is not a float\r\n-ERR syntax error\r\n:4\r\n*2\r\n$1\r\nb\r\n$1\r\nc\r\n*2\r\n$1\r\nc\r\n$1\r\nb\r\n-ERR syntax error, WITHSCORES not supported in combination with BYLEX\r\n-ERR min or max not valid string range item\r\n:3\r\n:4\r\n*0\r\n*0\r\n*0\r\n*0\r\n:4\r\n:5\r\n:1\r\n$-1\r\n*3\r\n$3\r\n2.5\r\n$-1\r\n$1\r\n5\r\n*1\r\n$-1\r\n:1\r\n*4\r\n$3\r\ninc\r\n$3\r\ninf\r\n$1\r\nc\r\n$2\r\n10\r\n-ERR value is out of range, must be positive\r\n-ERR syntax error\r\n-WRONGTYPE Operation against a key holding the wrong kind of value\r\n:5\r\n:2\r\n:1\r\n:2\r\n*2\r\n$1\r\na\r\n$1\r\nc\r\n*2\r\n$1\r\na\r\n$1\r\nd\r\n:2\r\n:0\r\n-WRONGTYPE Operation against a key holding the wrong kind of value\r\n-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
        "the sorted-set batch",
    );
    let fields = [
        ("f1", "v1"),
        ("f2", "v2"),
        ("f3", "v3"),
        ("f4", "v4"),
        ("f5", "v5"),
    ];
    let mut fill = vec!["HSET", "drawn"];
    for (field, value) in fields {
        fill.push(field);
        fill.push(value);
    }
    client
        .write_all(&resp(&fill))
        .expect("fill the hash to draw from");
    expect_replies(&mut client, b":5\r\n", "the hash to draw from");
    for (count, values) in [
        (None, false),
        (Some(3), false),
        (Some(9), false),
        (Some(-9), false),
        (Some(2), true),
        (Some(-4), true),
    ] {
        let number = count.map(|count: i64| count.to_string());
        let mut request = vec!["HRANDFIELD", "drawn"];
        request.extend(number.as_deref());
        if values {
            request.push("WITHVALUES");
        }
        client.write_all(&resp(&request)).expect("draw fields");
        let what = format!("{request:?}");
        let drawn = bulk_strings(&mut client, &what);
        let names = if values {
            assert_eq!(drawn.len() % 2, 0, "{what}: {drawn:?}");
            for pair in drawn.chunks(2) {
                let value = fields
                    .iter()
                    .find(|(field, _)| *field == pair[0])
                    .map(|(_, value)| *value);
                assert_eq!(value, Some(pair[1].as_str()), "{what}: {drawn:?}");
            }
            drawn.iter().step_by(2).cloned().collect::<Vec<_>>()
        } else {
            drawn
        };
        let wanted = match count {
            None => 1,
            Some(count) if count >= 0 => count.min(fields.len() as i64) as usize,
            Some(count) => count.unsigned_abs() as usize,
        };
        assert_eq!(names.len(), wanted, "{what}: {names:?}");
        assert!(
            names
                .iter()
                .all(|name| fields.iter().any(|(field, _)| field == name)),
            "{what}: {names:?}"
        );
        if !matches!(count, Some(count) if count < 0) {
            let distinct = names.iter().collect::<std::collections::BTreeSet<_>>();
            assert_eq!(distinct.len(), names.len(), "{what}: {names:?}");
        }
    }
    let mut draws = Vec::new();
    for _ in 0..50 {
        draws.extend(resp(&["HRANDFIELD", "drawn"]));
    }
    client.write_all(&draws).expect("draw single fields");
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..50 {
        seen.extend(bulk_strings(&mut client, "a single draw"));
    }
    assert!(seen.len() > 1, "fifty draws all chose {seen:?}");
    let words = |list: &[&str]| list.iter().map(|word| word.to_string()).collect::<Vec<_>>();
    let array = |items: Vec<String>| {
        let mut text = format!("*{}\r\n", items.len());
        for item in items {
            text.push_str(&format!("${}\r\n{item}\r\n", item.len()));
        }
        text
    };
    let scored = |index: usize| format!("m{index:03}");
    let equal = |index: usize| format!("w{index:03}");
    let mut scored_set = words(&["ZADD", "big"]);
    let mut equal_set = words(&["ZADD", "lexbig"]);
    for index in 0..200 {
        scored_set.push(index.to_string());
        scored_set.push(scored(index));
        equal_set.push("0".to_owned());
        equal_set.push(equal(index));
    }
    let checks = [
        (scored_set, ":200\r\n".to_owned()),
        (
            words(&["ZRANGE", "big", "37", "41"]),
            array((37..42).map(scored).collect()),
        ),
        (
            words(&["ZRANGE", "big", "0", "2", "REV"]),
            array([199, 198, 197].map(scored).to_vec()),
        ),
        (
            words(&["ZRANGEBYSCORE", "big", "(99.5", "102"]),
            array((100..103).map(scored).collect()),
        ),
        (
            words(&["ZREVRANGEBYSCORE", "big", "150", "-inf", "LIMIT", "10", "3"]),
            array([140, 139, 138].map(scored).to_vec()),
        ),
        (words(&["ZRANK", "big", "m123"]), ":123\r\n".to_owned()),
        (words(&["ZREVRANK", "big", "m123"]), ":76\r\n".to_owned()),
        (words(&["ZCOUNT", "big", "10", "(20"]), ":10\r\n".to_owned()),
        (
            words(&["ZREMRANGEBYSCORE", "big", "50", "(60"]),
            ":10\r\n".to_owned(),
        ),
        (
            words(&["ZRANGE", "big", "49", "51", "WITHSCORES"]),
            array(words(&["m049", "49", "m060", "60", "m061", "61"])),
        ),
        (
            words(&["ZREMRANGEBYRANK", "big", "0", "9"]),
            ":10\r\n".to_owned(),
        ),
        (words(&["ZCARD", "big"]), ":180\r\n".to_owned()),
        (
            words(&["ZPOPMAX", "big", "2"]),
            array(words(&["m199", "199", "m198", "198"])),
        ),
        (equal_set, ":200\r\n".to_owned()),
        (
            words(&["ZRANGEBYLEX", "lexbig", "[w123", "(w127"]),
            array((123..127).map(equal).collect()),
        ),
        (
            words(&["ZREVRANGEBYLEX", "lexbig", "(w050", "-", "LIMIT", "0", "2"]),
            array([49, 48].map(equal).to_vec()),
        ),
        (
            words(&["ZLEXCOUNT", "lexbig", "[w100", "+"]),
            ":100\r\n".to_owned(),
        ),
        (
            words(&["ZREMRANGEBYLEX", "lexbig", "-", "(w100"]),
            ":100\r\n".to_owned(),
        ),
        (
            words(&["ZRANGE", "lexbig", "0", "0"]),
            array(vec![equal(100)]),
        ),
    ];
    let mut batch = Vec::new();
    let mut expected = String::new();
    for (request, reply) in &checks {
        let request = request.iter().map(String::as_str).collect::<Vec<_>>();
        batch.extend(resp(&request));
        expected.push_str(reply);
    }
    client.write_all(&batch).expect("send the large sets");
    expect_replies(&mut client, expected.as_bytes(), "the sets of 200 members");
    drop(client);
    let (status, _) = finished(child);
    assert_eq!(status, 0);
}

/// firn replays hash and sorted-set writes from its append-only file.
/// The file records HINCRBYFLOAT's sum as the HSET of its text, as Redis
/// propagates it, so after the restart 0.1 and 0.2 read back as 0.3; HDEL's
/// removal holds, and HSETNX refuses to replace an existing field before the
/// restart. Each hash write path that finds its key expired, a key set already
/// expired with
/// PXAT 1, records the removal before its own record, so that after the
/// restart HSET's, HSETNX's, HINCRBY's and HINCRBYFLOAT's new hashes hold
/// and a SET with NX after HDEL holds its value, where a replay that kept the
/// old string would refuse each. ZADD
/// with INCR and ZINCRBY are recorded as sent, as Redis propagates them, so
/// their increments sum to 3.5 again, and the removals ZREMRANGEBYSCORE,
/// ZPOPMAX and ZREM made hold; the sorted-set write paths record a removal
/// as the hash ones do, so ZADD's new set holds and a SET with NX after ZADD
/// with XX, ZREM, ZPOPMIN or ZREMRANGEBYSCORE holds its value.
#[cfg(target_os = "linux")]
#[test]
fn firn_replays_the_value_types_from_its_append_only_file_with_hashes_and_sorted_sets() {
    let program = firn();
    let name = "replay-hash-zset-types.aof";
    let port = free_port();
    let text = port.to_string();
    let child = program.spawn_on_route(true, &[text.as_bytes(), b"1", name.as_bytes()]);
    let mut client = connect_when_ready(port);
    client
        .set_read_timeout(Some(Duration::from_secs(20)))
        .expect("bound the first client's waits");
    // HSETNX must find h.f already present and refuse to replace it.
    client
        .write_all(&resp(&["HSET", "h", "f", "v"]))
        .expect("seed the hash field");
    expect_replies(&mut client, b":1\r\n", "the existing hash field");
    let mut batch = Vec::new();
    for request in [
        vec!["HINCRBYFLOAT", "h", "r", "0.1"],
        vec!["HINCRBYFLOAT", "h", "r", "0.2"],
        vec!["HSET", "h", "gone", "1"],
        vec!["HDEL", "h", "gone"],
        vec!["HSETNX", "h", "f", "w"],
        vec!["SET", "gone:hset", "old", "PXAT", "1"],
        vec!["HSET", "gone:hset", "f", "v"],
        vec!["SET", "gone:hsetnx", "old", "PXAT", "1"],
        vec!["HSETNX", "gone:hsetnx", "f", "v"],
        vec!["SET", "gone:hincrby", "old", "PXAT", "1"],
        vec!["HINCRBY", "gone:hincrby", "f", "5"],
        vec!["SET", "gone:hincrbyfloat", "old", "PXAT", "1"],
        vec!["HINCRBYFLOAT", "gone:hincrbyfloat", "f", "1.5"],
        vec!["SET", "gone:hdel", "old", "PXAT", "1"],
        vec!["HDEL", "gone:hdel", "f"],
        vec!["SET", "gone:hdel", "new", "NX"],
        vec!["ZADD", "zi", "INCR", "1.5", "m"],
        vec!["ZINCRBY", "zi", "2", "m"],
        vec!["ZADD", "zr", "1", "a", "2", "b", "3", "c", "4", "d"],
        vec!["ZREMRANGEBYSCORE", "zr", "(1", "2"],
        vec!["ZPOPMAX", "zr"],
        vec!["ZREM", "zr", "a"],
        vec!["SET", "gone:zadd", "old", "PXAT", "1"],
        vec!["ZADD", "gone:zadd", "1", "m"],
        vec!["SET", "gone:zaddxx", "old", "PXAT", "1"],
        vec!["ZADD", "gone:zaddxx", "XX", "1", "m"],
        vec!["SET", "gone:zaddxx", "new", "NX"],
        vec!["SET", "gone:zrem", "old", "PXAT", "1"],
        vec!["ZREM", "gone:zrem", "m"],
        vec!["SET", "gone:zrem", "new", "NX"],
        vec!["SET", "gone:zpopmin", "old", "PXAT", "1"],
        vec!["ZPOPMIN", "gone:zpopmin"],
        vec!["SET", "gone:zpopmin", "new", "NX"],
        vec!["SET", "gone:zremrange", "old", "PXAT", "1"],
        vec!["ZREMRANGEBYSCORE", "gone:zremrange", "-inf", "+inf"],
        vec!["SET", "gone:zremrange", "new", "NX"],
    ] {
        batch.extend(resp(&request));
    }
    client.write_all(&batch).expect("send the changes");
    expect_replies(
        &mut client,
        b"$3\r\n0.1\r\n$3\r\n0.3\r\n:1\r\n:1\r\n:0\r\n+OK\r\n:1\r\n+OK\r\n:1\r\n+OK\r\n:5\r\n+OK\r\n$3\r\n1.5\r\n+OK\r\n:0\r\n+OK\r\n$3\r\n1.5\r\n$3\r\n3.5\r\n:4\r\n:1\r\n*2\r\n$1\r\nd\r\n$1\r\n4\r\n:1\r\n+OK\r\n:1\r\n+OK\r\n:0\r\n+OK\r\n+OK\r\n:0\r\n+OK\r\n+OK\r\n*0\r\n+OK\r\n+OK\r\n:0\r\n+OK\r\n",
        "the first run's changes",
    );
    let file = format!("/proc/{}/cwd/{name}", child.id());
    let started = Instant::now();
    let records = loop {
        let records = aof_records(&std::fs::read(&file).unwrap_or_default());
        if records
            .iter()
            .any(|record| record == &["SET", "gone:zremrange", "new", "NX"])
        {
            break records;
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the file holds {records:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    for record in [
        &["HSET", "h", "r", "0.1"][..],
        &["HSET", "h", "r", "0.3"],
        &["ZADD", "zi", "INCR", "1.5", "m"],
        &["ZINCRBY", "zi", "2", "m"],
    ] {
        assert!(
            records.iter().any(|held| held == record),
            "{record:?} not in {records:?}"
        );
    }
    assert!(
        records.iter().all(|record| record[0] != "HINCRBYFLOAT"),
        "{records:?}"
    );
    drop(client);
    let (status, _) = finished(child);
    assert_eq!(status, 0, "the first run");
    let port = free_port();
    let text = port.to_string();
    let child = program.spawn_on_route(true, &[text.as_bytes(), b"1", name.as_bytes()]);
    let mut client = connect_when_ready(port);
    client
        .set_read_timeout(Some(Duration::from_secs(20)))
        .expect("bound the second client's waits");
    let mut batch = Vec::new();
    for request in [
        vec!["HGET", "h", "r"],
        vec!["HEXISTS", "h", "gone"],
        vec!["HGETALL", "gone:hset"],
        vec!["HGETALL", "gone:hsetnx"],
        vec!["HGETALL", "gone:hincrby"],
        vec!["HGETALL", "gone:hincrbyfloat"],
        vec!["GET", "gone:hdel"],
        vec!["ZSCORE", "zi", "m"],
        vec!["ZRANGE", "zr", "0", "-1", "WITHSCORES"],
        vec!["ZRANGE", "gone:zadd", "0", "-1", "WITHSCORES"],
        vec!["GET", "gone:zaddxx"],
        vec!["GET", "gone:zrem"],
        vec!["GET", "gone:zpopmin"],
        vec!["GET", "gone:zremrange"],
    ] {
        batch.extend(resp(&request));
    }
    client.write_all(&batch).expect("read the replayed values");
    expect_replies(
        &mut client,
        b"$3\r\n0.3\r\n:0\r\n*2\r\n$1\r\nf\r\n$1\r\nv\r\n*2\r\n$1\r\nf\r\n$1\r\nv\r\n*2\r\n$1\r\nf\r\n$1\r\n5\r\n*2\r\n$1\r\nf\r\n$3\r\n1.5\r\n$3\r\nnew\r\n$3\r\n3.5\r\n*2\r\n$1\r\nc\r\n$1\r\n3\r\n*2\r\n$1\r\nm\r\n$1\r\n1\r\n$3\r\nnew\r\n$3\r\nnew\r\n$3\r\nnew\r\n$3\r\nnew\r\n",
        "the replayed values",
    );
    drop(client);
    let (status, _) = finished(child);
    assert_eq!(status, 0, "the second run");
}
