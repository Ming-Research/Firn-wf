//! Acceptance witnesses for firn-scope-pubsub. Expectations come from Redis
//! 7.2.16 src/pubsub.c, src/networking.c, src/multi.c and tests/unit/pubsub.tcl,
//! not from Firn output. The ordinary network gate runs these cases. All
//! builds and executions are owner-driven CI; no local pass is claimed.

use super::*;

#[path = "pubsub_oracle.rs"]
mod oracle;

fn start(connections: usize) -> (ProgramChild, TcpStream, u16) {
    start_with_drivers(connections, "1")
}

fn start_with_drivers(connections: usize, drivers: &str) -> (ProgramChild, TcpStream, u16) {
    start_for(connections, drivers, support::PROGRAM_DEADLINE)
}

/// Starts firn for a case that outlasts the shared program deadline.
fn start_for(connections: usize, drivers: &str, limit: Duration) -> (ProgramChild, TcpStream, u16) {
    let lease = support::lease_port();
    let port = lease.port();
    let port_text = port.to_string();
    let count = connections.to_string();
    let child = firn().spawn_on_route_with_limit(
        true, &[("WF_DRIVERS", drivers)], &[port_text.as_bytes(), count.as_bytes()], limit,
    ).with_port(&lease);
    let stream = connect_when_ready(port, &child);
    (child, stream, port)
}

fn send(stream: &mut TcpStream, commands: &[&[&str]]) {
    let bytes: Vec<u8> = commands.iter().flat_map(|command| resp(command)).collect();
    stream.write_all(&bytes).expect("send pub/sub command batch");
}

fn binary_request(arguments: &[&[u8]]) -> Vec<u8> {
    let mut bytes = format!("*{}\r\n", arguments.len()).into_bytes();
    for argument in arguments {
        bytes.extend(format!("${}\r\n", argument.len()).as_bytes());
        bytes.extend_from_slice(argument);
        bytes.extend_from_slice(b"\r\n");
    }
    bytes
}

fn binary_frame(version: u8, arguments: &[&[u8]], count: Option<usize>) -> Vec<u8> {
    let mut bytes = binary_request(arguments);
    if let Some(count) = count {
        // A subscription frame has a final integer, not a bulk-string count.
        let header_end = bytes.iter().position(|&byte| byte == b'\r').unwrap() + 2;
        let mut changed = format!("*{}\r\n", arguments.len() + 1).into_bytes();
        changed.extend_from_slice(&bytes[header_end..]);
        changed.extend(format!(":{count}\r\n").as_bytes());
        bytes = changed;
    }
    if version == 3 { bytes[0] = b'>'; }
    bytes
}

/// Preserve the wire type: a RESP3 push must not be accepted as an array.
fn frame(stream: &mut TcpStream) -> Vec<u8> {
    let line = reply_line(stream, "pub/sub frame");
    let mut bytes = line.as_bytes().to_vec();
    let count = line[1..line.len() - 2].parse::<i64>().unwrap_or(-1);
    match bytes[0] {
        b'$' | b'=' | b'!' if count >= 0 => {
            let mut body = vec![0; count as usize + 2];
            stream.read_exact(&mut body).expect("pub/sub bulk body");
            assert!(body.ends_with(b"\r\n"), "unterminated bulk body");
            bytes.extend(body);
        }
        b'*' | b'>' | b'~' | b'%' | b'|' if count >= 0 => {
            let elements = if matches!(bytes[0], b'%' | b'|') { count * 2 } else { count };
            for _ in 0..elements { bytes.extend(frame(stream)); }
        }
        _ => {}
    }
    bytes
}

fn request(stream: &mut TcpStream, arguments: &[&str]) -> Vec<u8> {
    send(stream, &[arguments]);
    frame(stream)
}

fn hello(stream: &mut TcpStream, version: u8) {
    if version == 3 {
        let reply = request(stream, &["HELLO", "3"]);
        assert!(reply.starts_with(b"%"), "HELLO 3 must answer a map: {reply:?}");
    }
}

fn bulk(text: &str) -> Vec<u8> {
    format!("${}\r\n{text}\r\n", text.len()).into_bytes()
}

fn acknowledgment(version: u8, kind: &str, channel: Option<&str>, count: usize) -> Vec<u8> {
    let mut bytes = format!("{}3\r\n", if version == 3 { '>' } else { '*' }).into_bytes();
    bytes.extend(bulk(kind));
    bytes.extend(match channel {
        Some(channel) => bulk(channel),
        None if version == 3 => b"_\r\n".to_vec(),
        None => b"$-1\r\n".to_vec(),
    });
    bytes.extend(format!(":{count}\r\n").as_bytes());
    bytes
}

fn message(version: u8, kind: &str, channel: &str, value: &str) -> Vec<u8> {
    let mut bytes = format!("{}3\r\n", if version == 3 { '>' } else { '*' }).into_bytes();
    for part in [kind, channel, value] { bytes.extend(bulk(part)); }
    bytes
}

fn pattern_message(version: u8, pattern: &str, channel: &str, value: &str) -> Vec<u8> {
    let mut bytes = format!("{}4\r\n", if version == 3 { '>' } else { '*' }).into_bytes();
    for part in ["pmessage", pattern, channel, value] { bytes.extend(bulk(part)); }
    bytes
}

/// Redis iterates a dictionary for UNSUBSCRIBE without arguments. Only the
/// two channel names may exchange positions; frame types, counts, the EXEC
/// header and any trailing PING reply must still match byte for byte.
fn assert_unsubscribe_all_recording(actual: &[u8], recorded: &[u8], version: u8, in_exec: bool) {
    let transcript = |first, second| {
        let mut bytes = if in_exec { b"*2\r\n".to_vec() } else { Vec::new() };
        bytes.extend(acknowledgment(version, "unsubscribe", Some(first), 1));
        bytes.extend(acknowledgment(version, "unsubscribe", Some(second), 0));
        if in_exec { bytes.extend_from_slice(b"$4\r\ntail\r\n"); }
        bytes
    };
    let first_second = transcript("first", "second");
    let second_first = transcript("second", "first");
    assert!(recorded == first_second || recorded == second_first,
        "recorded unsubscribe-all bytes changed: {recorded:?}");
    assert!(actual == first_second || actual == second_first,
        "unsubscribe-all bytes differ: expected {recorded:?}, got {actual:?}");
}

/// All twelve batches were recorded on Redis 7.2.16 at
/// https://github.com/Ming-Research/Firn-wf/actions/runs/38399914253.
/// Read exact byte lengths instead of parsing frames: multi-channel EXEC
/// produces more acknowledgment frames than its aggregate header counts.
#[test]
fn all_twelve_recorded_pubsub_batches_match_redis_bytes() {
    for batch in oracle::BATCHES {
        let (child, mut client, _) = start(1);
        hello(&mut client, batch.resp);
        for &(arguments, expected) in batch.replies {
            send(&mut client, &[arguments]);
            let mut actual = vec![0; expected.len()];
            client.read_exact(&mut actual).unwrap_or_else(|error|
                panic!("{} {arguments:?}: reading {} recorded bytes: {error}", batch.name, expected.len()));
            match (batch.name, arguments) {
                ("ORACLE-step4-multichannel-subscription-exec-resp2" |
                 "ORACLE-step4-multichannel-subscription-exec-resp3", ["UNSUBSCRIBE"]) =>
                    assert_unsubscribe_all_recording(&actual, expected, batch.resp, false),
                ("ORACLE-step4-unsubscribe-all-exec-resp3", ["EXEC"]) =>
                    assert_unsubscribe_all_recording(&actual, expected, batch.resp, true),
                _ => assert_eq!(actual, expected, "{} {arguments:?}", batch.name),
            }
        }
        // Every batch ends with no memberships (or RESET). This ordinary
        // reply barrier detects output left beyond the last recorded bytes.
        send(&mut client, &[&["PING"]]);
        let mut barrier = [0; 7];
        client.read_exact(&mut barrier).expect("recorded pub/sub final PING barrier");
        assert_eq!(&barrier, b"+PONG\r\n", "{} left extra output", batch.name);
        drop(client);
        assert_eq!(finished(child).0, 0, "{} server exit", batch.name);
    }
}

// Step 4: direct channels and the common connection lifecycle.

/// Subscription identities and publication bodies are binary lengths, not
/// UTF-8/C strings; matching must retain NUL, non-UTF-8 bytes and embedded CRLF.
#[test]
fn binary_channels_and_messages_preserve_every_byte() {
    for version in [2, 3] {
        let (child, mut publisher, port) = start(2);
        let mut subscriber = connect_when_ready(port, &child);
        hello(&mut subscriber, version);
        let channel = b"channel\0\xff\r\n";
        let value = b"message\0\xfe\r\n";
        subscriber.write_all(&binary_request(&[b"SUBSCRIBE", channel])).unwrap();
        assert_eq!(frame(&mut subscriber), binary_frame(version, &[b"subscribe", channel], Some(1)));
        publisher.write_all(&binary_request(&[b"PUBLISH", channel, value])).unwrap();
        assert_eq!(frame(&mut publisher), b":1\r\n");
        assert_eq!(frame(&mut subscriber), binary_frame(version, &[b"message", channel, value], None));
        subscriber.write_all(&binary_request(&[b"UNSUBSCRIBE", channel])).unwrap();
        assert_eq!(frame(&mut subscriber), binary_frame(version, &[b"unsubscribe", channel], Some(0)));
        drop((publisher, subscriber));
        assert_eq!(finished(child).0, 0);
    }
}

/// The first receive contains a complete subscription and an incomplete next
/// bulk argument. Promotion must preserve that parser/input suffix exactly.
/// Recorded basic SUBSCRIBE/UNSUBSCRIBE and post-unsubscribe PING bytes at
/// https://github.com/Ming-Research/Firn-wf/actions/runs/38399914253.
/// Fragmented input and the PING fragment payload remain source-derived.
#[test]
fn promotion_keeps_fragmented_pipeline_suffix() {
    for version in [2, 3] {
        let (child, mut client, _) = start(1);
        hello(&mut client, version);
        let following = resp(&["PING", "fragment"]);
        let split = following.len() - 5;
        let mut prefix = resp(&["SUBSCRIBE", "news"]);
        prefix.extend_from_slice(&following[..split]);
        client.write_all(&prefix).unwrap();
        assert_eq!(frame(&mut client), acknowledgment(version, "subscribe", Some("news"), 1));
        client.write_all(&following[split..]).unwrap();
        assert_eq!(frame(&mut client), if version == 2 { b"*2\r\n$4\r\npong\r\n$8\r\nfragment\r\n".as_slice() } else { b"$8\r\nfragment\r\n".as_slice() });
        assert_eq!(request(&mut client, &["UNSUBSCRIBE"]), acknowledgment(version, "unsubscribe", Some("news"), 0));
        assert_eq!(request(&mut client, &["PING"]), b"+PONG\r\n");
        drop(client);
        assert_eq!(finished(child).0, 0);
    }
}

/// A queued classic subscription can perform first promotion during EXEC.
/// Unlike SSUBSCRIBE, Redis exempts SUBSCRIBE from EXEC's DENY_BLOCKING flag.
/// MULTI/QUEUED literals and subscription frame fields are independently
/// recorded at https://github.com/Ming-Research/Firn-wf/actions/runs/38399914253.
/// Its two-channel EXEC differs from this single-channel aggregate.
#[test]
fn queued_classic_subscription_promotes_during_exec() {
    for version in [2, 3] {
        let (child, mut client, _) = start(1);
        hello(&mut client, version);
        send(&mut client, &[&["MULTI"], &["SUBSCRIBE", "news"], &["EXEC"]]);
        assert_eq!(frame(&mut client), b"+OK\r\n");
        assert_eq!(frame(&mut client), b"+QUEUED\r\n");
        let mut expected = b"*1\r\n".to_vec();
        expected.extend(acknowledgment(version, "subscribe", Some("news"), 1));
        assert_eq!(frame(&mut client), expected);
        assert_eq!(request(&mut client, &["UNSUBSCRIBE"]), acknowledgment(version, "unsubscribe", Some("news"), 0));
        drop(client);
        assert_eq!(finished(child).0, 0);
    }
}

/// Duplicate memberships acknowledge every argument but receive one copy;
/// unsolicited output arrives while the receiver has no command to execute.
/// Basic news subscription and zero-recipient PUBLISH literals recorded at
/// https://github.com/Ming-Research/Firn-wf/actions/runs/38399914253.
/// Two-client counts and duplicate direct subscriptions remain source-derived.
#[test]
fn direct_channels_multiple_subscribers_and_introspection() {
    for version in [2, 3] {
        let (child, mut control, port) = start(3);
        let mut first = connect_when_ready(port, &child);
        let mut second = connect_when_ready(port, &child);
        hello(&mut first, version);
        hello(&mut second, version);
        send(&mut first, &[&["SUBSCRIBE", "news", "news"]]);
        for _ in 0..2 {
            assert_eq!(frame(&mut first), acknowledgment(version, "subscribe", Some("news"), 1));
        }
        assert_eq!(request(&mut second, &["SUBSCRIBE", "news"]), acknowledgment(version, "subscribe", Some("news"), 1));
        assert_eq!(request(&mut control, &["PUBSUB", "CHANNELS"]), b"*1\r\n$4\r\nnews\r\n");
        assert_eq!(request(&mut control, &["PUBSUB", "NUMSUB", "news", "absent", "news"]), b"*6\r\n$4\r\nnews\r\n:2\r\n$6\r\nabsent\r\n:0\r\n$4\r\nnews\r\n:2\r\n");
        assert_eq!(request(&mut control, &["PUBSUB", "NUMSUB"]), b"*0\r\n");
        assert_eq!(request(&mut control, &["PUBLISH", "news", "hello"]), b":2\r\n");
        for stream in [&mut first, &mut second] {
            assert_eq!(frame(stream), message(version, "message", "news", "hello"));
            assert_eq!(request(stream, &["UNSUBSCRIBE", "news", "news"]), acknowledgment(version, "unsubscribe", Some("news"), 0));
            assert_eq!(frame(stream), acknowledgment(version, "unsubscribe", Some("news"), 0));
            assert_eq!(request(stream, &["PING"]), b"+PONG\r\n");
        }
        assert_eq!(request(&mut control, &["PUBLISH", "news", "gone"]), b":0\r\n");
        assert_eq!(request(&mut control, &["PUBSUB", "CHANNELS"]), b"*0\r\n");
        drop((control, first, second));
        assert_eq!(finished(child).0, 0);
    }
}

/// Arity/name checks precede RESP2 subscriber admission; its PING is an
/// ordinary array of two bulk strings, never a push, and RESET clears mode.
/// All commands/expected bytes in this case were recorded at
/// https://github.com/Ming-Research/Firn-wf/actions/runs/38399914253
/// (step4-resp2-mode-and-precedence).
#[test]
fn resp2_subscriber_restrictions_error_precedence_ping_and_reset() {
    let (child, mut subscriber, _) = start(1);
    assert_eq!(request(&mut subscriber, &["SUBSCRIBE", "news"]), acknowledgment(2, "subscribe", Some("news"), 1));
    for (arguments, canonical) in [
        (vec!["gEt", "key"], "get"),
        (vec!["PUBLISH", "news", "value"], "publish"),
        (vec!["MULTI"], "multi"),
        (vec!["HELLO", "3"], "hello"),
        (vec!["PUBSUB", "NUMSUB", "news"], "pubsub|numsub"),
    ] {
        let expected = format!("-ERR Can't execute '{canonical}': only (P|S)SUBSCRIBE / (P|S)UNSUBSCRIBE / PING / QUIT / RESET are allowed in this context\r\n");
        assert_eq!(request(&mut subscriber, &arguments), expected.as_bytes());
    }
    assert_eq!(request(&mut subscriber, &["GET"]), b"-ERR wrong number of arguments for 'get' command\r\n");
    assert_eq!(request(&mut subscriber, &["PUBSUB", "unknown"]), b"-ERR unknown subcommand 'unknown'. Try PUBSUB HELP.\r\n");
    assert_eq!(request(&mut subscriber, &["PING"]), b"*2\r\n$4\r\npong\r\n$0\r\n\r\n");
    assert_eq!(request(&mut subscriber, &["PING", "hi"]), b"*2\r\n$4\r\npong\r\n$2\r\nhi\r\n");
    assert_eq!(request(&mut subscriber, &["RESET"]), b"+RESET\r\n");
    assert_eq!(request(&mut subscriber, &["PUBLISH", "news", "gone"]), b":0\r\n");
    assert_eq!(request(&mut subscriber, &["PING"]), b"+PONG\r\n");
    drop(subscriber);
    assert_eq!(finished(child).0, 0);
}

/// Promotion retains earlier reply bytes and the complete pipeline suffix;
/// unsubscribe-all always replies, including its protocol-specific null.
/// Every expected frame was recorded in step4-promotion-resp2/resp3 at
/// https://github.com/Ming-Research/Firn-wf/actions/runs/38399914253.
/// The recorder sent commands separately; pipeline ownership is tested here.
#[test]
fn promotion_preserves_pipeline_prefix_suffix_and_unsubscribe_all() {
    for version in [2, 3] {
        let (child, mut client, _) = start(1);
        hello(&mut client, version);
        send(&mut client, &[&["SET", "key", "value"], &["SUBSCRIBE", "news"], &["PING", "tail"], &["UNSUBSCRIBE"], &["GET", "key"], &["UNSUBSCRIBE"]]);
        assert_eq!(frame(&mut client), b"+OK\r\n");
        assert_eq!(frame(&mut client), acknowledgment(version, "subscribe", Some("news"), 1));
        let ping = if version == 2 { b"*2\r\n$4\r\npong\r\n$4\r\ntail\r\n".as_slice() } else { b"$4\r\ntail\r\n".as_slice() };
        assert_eq!(frame(&mut client), ping);
        assert_eq!(frame(&mut client), acknowledgment(version, "unsubscribe", Some("news"), 0));
        assert_eq!(frame(&mut client), b"$5\r\nvalue\r\n");
        assert_eq!(frame(&mut client), acknowledgment(version, "unsubscribe", None, 0));
        drop(client);
        assert_eq!(finished(child).0, 0);
    }
}

/// Commands that follow a subscription in the same write run in the mode it
/// started: Redis 7.2.16 processCommand reads the client's pub/sub flag at each
/// command, so in RESP2 the restricted commands are refused from the next one
/// on, while RESP3 has no such mode. The refusal literals for GET and PUBLISH
/// were recorded in step4-resp2-mode-and-precedence (pubsub_oracle.rs); the
/// EXEC variant is source-derived from the same function. The ordinary owner
/// stops at the first successful subscription, direct or inside EXEC, and the
/// promoted owner, which alone checks the mode, runs the rest of the write.
#[test]
fn commands_after_a_subscription_in_one_write_see_subscriber_mode() {
    let refusal = |name: &str| {
        format!("-ERR Can't execute '{name}': only (P|S)SUBSCRIBE / (P|S)UNSUBSCRIBE / PING / QUIT / RESET are allowed in this context\r\n").into_bytes()
    };
    {
        let (child, mut client, _) = start(1);
        send(&mut client, &[&["SUBSCRIBE", "news"], &["GET", "key"], &["PUBLISH", "news", "value"], &["PING", "tail"]]);
        assert_eq!(frame(&mut client), acknowledgment(2, "subscribe", Some("news"), 1));
        assert_eq!(frame(&mut client), refusal("get"));
        assert_eq!(frame(&mut client), refusal("publish"));
        assert_eq!(frame(&mut client), b"*2\r\n$4\r\npong\r\n$4\r\ntail\r\n");
        drop(client);
        assert_eq!(finished(child).0, 0);
    }
    {
        let (child, mut client, _) = start(1);
        send(&mut client, &[&["MULTI"], &["SUBSCRIBE", "news"], &["EXEC"], &["GET", "key"], &["PING", "tail"]]);
        assert_eq!(frame(&mut client), b"+OK\r\n");
        assert_eq!(frame(&mut client), b"+QUEUED\r\n");
        let mut expected = b"*1\r\n".to_vec();
        expected.extend(acknowledgment(2, "subscribe", Some("news"), 1));
        assert_eq!(frame(&mut client), expected);
        assert_eq!(frame(&mut client), refusal("get"));
        assert_eq!(frame(&mut client), b"*2\r\n$4\r\npong\r\n$4\r\ntail\r\n");
        drop(client);
        assert_eq!(finished(child).0, 0);
    }
    {
        let (child, mut client, _) = start(1);
        hello(&mut client, 3);
        send(&mut client, &[&["SUBSCRIBE", "news"], &["GET", "key"], &["PING", "tail"]]);
        assert_eq!(frame(&mut client), acknowledgment(3, "subscribe", Some("news"), 1));
        assert_eq!(frame(&mut client), b"_\r\n");
        assert_eq!(frame(&mut client), b"$4\r\ntail\r\n");
        drop(client);
        assert_eq!(finished(child).0, 0);
    }
}

/// Dictionary iteration order is unspecified, but every membership must
/// appear exactly once, with the count decreasing at each removal.
/// First/second subscription frame fields and empty UNSUBSCRIBE bytes were
/// recorded at https://github.com/Ming-Research/Firn-wf/actions/runs/38399914253.
/// Three-member cleanup/count progression remains source-derived.
#[test]
fn unsubscribe_all_returns_every_membership_and_empty_acknowledgment() {
    for version in [2, 3] {
        let (child, mut client, _) = start(1);
        hello(&mut client, version);
        send(&mut client, &[&["SUBSCRIBE", "first", "second", "third"]]);
        for (index, channel) in ["first", "second", "third"].iter().enumerate() {
            assert_eq!(frame(&mut client), acknowledgment(version, "subscribe", Some(channel), index + 1));
        }
        send(&mut client, &[&["UNSUBSCRIBE"]]);
        let mut remaining = vec!["first", "second", "third"];
        for count in [2, 1, 0] {
            let reply = frame(&mut client);
            let position = remaining.iter().position(|channel| reply == acknowledgment(version, "unsubscribe", Some(channel), count))
                .unwrap_or_else(|| panic!("UNSUBSCRIBE missing, repeated, or wrong count: {reply:?}"));
            remaining.remove(position);
        }
        assert!(remaining.is_empty());
        assert_eq!(request(&mut client, &["UNSUBSCRIBE"]), acknowledgment(version, "unsubscribe", None, 0));
        assert_eq!(request(&mut client, &["PING"]), b"+PONG\r\n");
        drop(client);
        assert_eq!(finished(child).0, 0);
    }
}

/// Redis 7.2.16 pubsub.c updates both client and server indexes before each
/// acknowledgment. Its unit/pubsub test uses chan1.*, chan2.* and chan3.*,
/// but the upstream no-argument helper drains no PUNSUBSCRIBE replies. This
/// witness explicitly reads all three replies before publishing chanN.hi.
/// Four drivers repeat that boundary for every family, protocol and removal
/// form: the acknowledgment itself is the barrier, without a sleep or poll.
#[test]
fn membership_acknowledgments_are_cross_connection_barriers_with_multiple_drivers() {
    for version in [2, 3] {
        let (child, mut publisher, port) = start_with_drivers(2, "4");
        let mut subscriber = connect_when_ready(port, &child);
        for stream in [&mut publisher, &mut subscriber] {
            stream.set_nodelay(true).expect("send membership barrier commands promptly");
        }
        hello(&mut subscriber, version);
        for (subscribe, unsubscribe, publish, message_kind, names) in [
            ("SUBSCRIBE", "UNSUBSCRIBE", "PUBLISH", "message", ["chan1.hi", "chan2.hi", "chan3.hi"]),
            ("PSUBSCRIBE", "PUNSUBSCRIBE", "PUBLISH", "pmessage", ["chan1.*", "chan2.*", "chan3.*"]),
            ("SSUBSCRIBE", "SUNSUBSCRIBE", "SPUBLISH", "smessage", ["chan1.hi", "chan2.hi", "chan3.hi"]),
        ] {
            let subscribe_kind = subscribe.to_ascii_lowercase();
            let unsubscribe_kind = unsubscribe.to_ascii_lowercase();
            for with_arguments in [false, true] {
                for iteration in 0..100 {
                    let context = format!("RESP{version} {subscribe}/{unsubscribe} arguments={with_arguments} round={iteration}");
                    send(&mut subscriber, &[&[subscribe, names[0], names[1], names[2]]]);
                    for (index, name) in names.iter().enumerate() {
                        assert_eq!(frame(&mut subscriber), acknowledgment(version, &subscribe_kind, Some(name), index + 1), "{context}: subscribe acknowledgment");
                    }
                    // The first publisher command follows only the final
                    // acknowledgment; no query can mask a late index update.
                    let value = format!("live-{iteration}");
                    for (index, channel) in ["chan1.hi", "chan2.hi", "chan3.hi"].iter().enumerate() {
                        assert_eq!(request(&mut publisher, &[publish, channel, &value]), b":1\r\n", "{context}: {channel} must count one recipient after subscribe acknowledgment");
                        let expected = if message_kind == "pmessage" {
                            pattern_message(version, names[index], channel, &value)
                        } else {
                            message(version, message_kind, channel, &value)
                        };
                        assert_eq!(frame(&mut subscriber), expected, "{context}: {channel} must deliver after subscribe acknowledgment");
                    }
                    if with_arguments {
                        send(&mut subscriber, &[&[unsubscribe, names[0], names[1], names[2]]]);
                        for (index, name) in names.iter().enumerate() {
                            assert_eq!(frame(&mut subscriber), acknowledgment(version, &unsubscribe_kind, Some(name), 2 - index), "{context}: explicit unsubscribe acknowledgment");
                        }
                    } else {
                        send(&mut subscriber, &[&[unsubscribe]]);
                        let mut remaining = names.to_vec();
                        for count in [2, 1, 0] {
                            let reply = frame(&mut subscriber);
                            let position = remaining.iter().position(|name| reply == acknowledgment(version, &unsubscribe_kind, Some(name), count))
                                .unwrap_or_else(|| panic!("{context}: unsubscribe-all missing, repeated, or wrong count: {reply:?}"));
                            remaining.remove(position);
                        }
                        assert!(remaining.is_empty(), "{context}: unsubscribe-all omitted a membership");
                    }
                    for channel in ["chan1.hi", "chan2.hi", "chan3.hi"] {
                        assert_eq!(request(&mut publisher, &[publish, channel, "hello"]), b":0\r\n", "{context}: {channel} must count zero recipients after the last unsubscribe acknowledgment");
                    }
                    assert_eq!(request(&mut subscriber, &["PING"]), b"+PONG\r\n", "{context}: an unsubscribed delivery leaked before the PING barrier");
                }
            }
        }
        drop((publisher, subscriber));
        assert_eq!(finished(child).0, 0);
    }
}

/// Redis 7.2.16 networking.c:resetCommand calls clearClientConnectionState
/// before replying +RESET; that clears all three indexes without unsubscribe
/// notifications and restores RESP2. pubsub.c supplies the query counts and
/// the RESP2 empty-unsubscribe frame used to verify the protocol reset.
#[test]
fn reset_reply_is_cross_connection_membership_barrier_with_multiple_drivers() {
    for version in [2, 3] {
        let (child, mut publisher, port) = start_with_drivers(2, "4");
        let mut subscriber = connect_when_ready(port, &child);
        for stream in [&mut publisher, &mut subscriber] {
            stream.set_nodelay(true).expect("send RESET barrier commands promptly");
        }
        for iteration in 0..100 {
            let context = format!("RESP{version} RESET round={iteration}");
            // RESET also resets the protocol, so negotiate anew each round.
            hello(&mut subscriber, version);
            for (command, kind, names, prior_count) in [
                ("SUBSCRIBE", "subscribe", ["chan1.hi", "chan2.hi", "chan3.hi"], 0),
                ("PSUBSCRIBE", "psubscribe", ["chan1.*", "chan2.*", "chan3.*"], 3),
                ("SSUBSCRIBE", "ssubscribe", ["chan1.hi", "chan2.hi", "chan3.hi"], 0),
            ] {
                send(&mut subscriber, &[&[command, names[0], names[1], names[2]]]);
                for (index, name) in names.iter().enumerate() {
                    assert_eq!(frame(&mut subscriber), acknowledgment(version, kind, Some(name), prior_count + index + 1), "{context}: {command} acknowledgment");
                }
            }
            assert_eq!(request(&mut publisher, &["PUBSUB", "NUMPAT"]), b":3\r\n", "{context}: establish three live patterns before RESET");
            let mut live_counts = b"*6\r\n".to_vec();
            for channel in ["chan1.hi", "chan2.hi", "chan3.hi"] {
                live_counts.extend(bulk(channel));
                live_counts.extend_from_slice(b":1\r\n");
            }
            for query in ["NUMSUB", "SHARDNUMSUB"] {
                assert_eq!(request(&mut publisher, &["PUBSUB", query, "chan1.hi", "chan2.hi", "chan3.hi"]), live_counts, "{context}: establish three live {query} memberships before RESET");
            }
            assert_eq!(request(&mut subscriber, &["RESET"]), b"+RESET\r\n", "{context}: RESET reply must contain no unsubscribe notifications");
            // No subscriber command or observer poll separates the RESET
            // reply from these publications on the other connection.
            for channel in ["chan1.hi", "chan2.hi", "chan3.hi"] {
                for publish in ["PUBLISH", "SPUBLISH"] {
                    assert_eq!(request(&mut publisher, &[publish, channel, "after-reset"]), b":0\r\n", "{context}: {publish} {channel} must count zero recipients after RESET reply");
                }
            }
            assert_eq!(request(&mut publisher, &["PUBSUB", "NUMPAT"]), b":0\r\n", "{context}: RESET left a pattern in the server index");
            let mut empty_counts = b"*6\r\n".to_vec();
            for channel in ["chan1.hi", "chan2.hi", "chan3.hi"] {
                empty_counts.extend(bulk(channel));
                empty_counts.extend_from_slice(b":0\r\n");
            }
            for query in ["NUMSUB", "SHARDNUMSUB"] {
                assert_eq!(request(&mut publisher, &["PUBSUB", query, "chan1.hi", "chan2.hi", "chan3.hi"]), empty_counts, "{context}: RESET left a {query} membership in the server index");
            }
            assert_eq!(request(&mut subscriber, &["PING"]), b"+PONG\r\n", "{context}: a post-RESET delivery leaked before the PING barrier");
            assert_eq!(request(&mut subscriber, &["UNSUBSCRIBE"]), acknowledgment(2, "unsubscribe", None, 0), "{context}: RESET must restore RESP2, including its array and bulk null");
        }
        drop((publisher, subscriber));
        assert_eq!(finished(child).0, 0);
    }
}

/// PUBSUB subcommand table arity is checked before execution; excess CHANNELS
/// arguments are a handler syntax error. Wrong PUBLISH arity dirties MULTI.
/// A valid queued PUBLISH is observable to a real subscriber if it leaks on
/// DISCARD or EXECABORT. The next barrier publication must be the next frame.
/// Expected bytes follow Redis 7.2.16 src/multi.c and src/pubsub.c; the arity
/// errors follow src/server.c and src/pubsub.c, rather than a new recording.
#[test]
fn pubsub_admission_errors_abort_transaction_without_publication() {
    let (child, mut client, port) = start(2);
    let mut subscriber = connect_when_ready(port, &child);
    assert_eq!(request(&mut subscriber, &["SUBSCRIBE", "news"]), acknowledgment(2, "subscribe", Some("news"), 1));
    assert_eq!(request(&mut client, &["PUBSUB"]), b"-ERR wrong number of arguments for 'pubsub' command\r\n");
    assert_eq!(request(&mut client, &["PUBSUB", "NUMPAT", "extra"]), b"-ERR wrong number of arguments for 'pubsub|numpat' command\r\n");
    assert_eq!(request(&mut client, &["PUBSUB", "HELP", "extra"]), b"-ERR wrong number of arguments for 'pubsub|help' command\r\n");
    assert_eq!(request(&mut client, &["PUBSUB", "CHANNELS", "*", "extra"]), b"-ERR unknown subcommand or wrong number of arguments for 'CHANNELS'. Try PUBSUB HELP.\r\n");
    assert_eq!(request(&mut client, &["MULTI"]), b"+OK\r\n");
    assert_eq!(request(&mut client, &["PUBLISH", "news", "discarded"]), b"+QUEUED\r\n");
    assert_eq!(request(&mut client, &["DISCARD"]), b"+OK\r\n");
    assert_eq!(request(&mut client, &["PUBLISH", "news", "after-discard"]), b":1\r\n");
    assert_eq!(frame(&mut subscriber), message(2, "message", "news", "after-discard"), "DISCARD must not leak its valid queued PUBLISH");
    assert_eq!(request(&mut client, &["MULTI"]), b"+OK\r\n");
    assert_eq!(request(&mut client, &["PUBLISH", "news", "aborted"]), b"+QUEUED\r\n");
    assert_eq!(request(&mut client, &["PUBLISH", "news"]), b"-ERR wrong number of arguments for 'publish' command\r\n");
    assert_eq!(request(&mut client, &["EXEC"]), b"-EXECABORT Transaction discarded because of previous errors.\r\n");
    assert_eq!(request(&mut client, &["PUBLISH", "news", "after-abort"]), b":1\r\n");
    assert_eq!(frame(&mut subscriber), message(2, "message", "news", "after-abort"), "EXECABORT must not leak the earlier valid queued PUBLISH");
    drop((client, subscriber));
    assert_eq!(finished(child).0, 0);
}

/// Redis executes each command, queues its complete reply, and only then runs
/// another client's command. Seeing the new value with GET therefore forces
/// the subscribed writer's reply ahead of B's later publication. Four drivers
/// exercise the gap between releasing data locks and committing that reply.
/// SET's status, EXEC's aggregate and Lua's status-table conversion follow
/// Redis 7.2.16 src/t_string.c, src/multi.c and src/script_lua.c.
#[test]
fn resp3_visible_write_reply_precedes_causally_later_publication_with_multiple_drivers() {
    let (child, mut observer, port) = start_with_drivers(2, "4");
    let mut writer = connect_when_ready(port, &child);
    for stream in [&mut observer, &mut writer] {
        stream.set_nodelay(true).expect("send causal barrier commands promptly");
    }
    hello(&mut writer, 3);
    assert_eq!(request(&mut writer, &["SUBSCRIBE", "news"]), acknowledgment(3, "subscribe", Some("news"), 1));
    for kind in ["direct", "exec", "script"] {
        for iteration in 0..200 {
            let value = format!("{kind}-{iteration}");
            let expected_reply: &[u8] = match kind {
                "direct" => {
                    send(&mut writer, &[&["SET", "causal-key", &value]]);
                    b"+OK\r\n"
                }
                "exec" => {
                    assert_eq!(request(&mut writer, &["MULTI"]), b"+OK\r\n");
                    assert_eq!(request(&mut writer, &["SET", "causal-key", &value]), b"+QUEUED\r\n");
                    send(&mut writer, &[&["EXEC"]]);
                    b"*1\r\n+OK\r\n"
                }
                "script" => {
                    send(&mut writer, &[&["EVAL", "return redis.call('SET',KEYS[1],ARGV[1])", "1", "causal-key", &value]]);
                    b"+OK\r\n"
                }
                _ => unreachable!(),
            };
            // Do not read A's reply until B has observed this iteration's
            // distinct value and completed the resulting PUBLISH.
            let deadline = Instant::now() + Duration::from_secs(10);
            let visible = bulk(&value);
            loop {
                if request(&mut observer, &["GET", "causal-key"]) == visible { break; }
                assert!(Instant::now() < deadline, "{value}: write never became visible to the other connection");
            }
            assert_eq!(request(&mut observer, &["PUBLISH", "news", &value]), b":1\r\n");
            assert_eq!(frame(&mut writer), expected_reply, "{value}: a causally later publication overtook the write reply");
            assert_eq!(frame(&mut writer), message(3, "message", "news", &value), "{value}: the publication follows the complete reply");
        }
    }
    drop((observer, writer));
    assert_eq!(finished(child).0, 0);
}

/// Redis 7.2.16 src/server.c genRedisInfoString (keyspace section) emits
/// "# Keyspace\r\n" and a db0 line only when the database is nonempty. These
/// exact text frames preserve the fields Firn reports: keys, with expires and
/// avg_ttl omitted as documented. RESP3's txt verbatim envelope follows
/// Redis src/networking.c addReplyVerbatim. Redis executes INFO or the whole
/// EXEC first, so a message followed by an empty snapshot is impossible.
/// Concurrent writes from two barrier-released threads exercise the old gap
/// between INFO's key-count snapshot and its output commit. With a promoted
/// connection's INFO routed through the ordinary run_info, as before the fix,
/// the case failed in 9 of 20 CI runs, each with the message ahead of an empty
/// INFO; the fixed path failed in none of 20.
#[test]
fn resp3_info_keyspace_and_exec_publication_have_one_serial_order_with_multiple_drivers() {
    let (child, mut publisher, port) = start_with_drivers(2, "4");
    let mut subscriber = connect_when_ready(port, &child);
    for stream in [&mut publisher, &mut subscriber] {
        stream.set_nodelay(true).expect("send INFO/EXEC race commands promptly");
    }
    hello(&mut subscriber, 3);
    assert_eq!(request(&mut subscriber, &["SUBSCRIBE", "news"]), acknowledgment(3, "subscribe", Some("news"), 1));
    let info_frame = |text: &str| format!("={}\r\ntxt:{text}\r\n", text.len() + 4).into_bytes();
    let empty_info = info_frame("# Keyspace\r\n");
    let one_key_info = info_frame("# Keyspace\r\ndb0:keys=1\r\n");
    let info_command = resp(&["INFO", "KEYSPACE"]);
    let exec_command = resp(&["EXEC"]);
    for iteration in 0..200 {
        let key = format!("fresh_{iteration}");
        let payload = iteration.to_string();
        assert_eq!(request(&mut publisher, &["MULTI"]), b"+OK\r\n");
        assert_eq!(request(&mut publisher, &["SET", &key, "v"]), b"+QUEUED\r\n");
        assert_eq!(request(&mut publisher, &["PUBLISH", "news", &payload]), b"+QUEUED\r\n");
        let publication = message(3, "message", "news", &payload);
        let ready = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            let executing = scope.spawn(|| {
                ready.wait();
                publisher.write_all(&exec_command).expect("send racing EXEC");
                frame(&mut publisher)
            });
            ready.wait();
            subscriber.write_all(&info_command).expect("send racing INFO KEYSPACE");
            // Read both complete frames before cleanup. Do not poll the data,
            // sleep, or wait for B's EXEC reply before A sends INFO.
            let first = frame(&mut subscriber);
            let second = frame(&mut subscriber);
            assert!(
                (first == empty_info && second == publication) ||
                (first == publication && second == one_key_info),
                "round {iteration}: INFO/publication is not a Redis serial outcome: first={first:?}, second={second:?}"
            );
            assert_eq!(executing.join().expect("INFO/EXEC sender thread"), b"*2\r\n+OK\r\n:1\r\n", "round {iteration}: EXEC replies");
        });
        // This reply is a barrier only between rounds: every new race starts
        // with no dataset keys and uses a distinct key and message payload.
        assert_eq!(request(&mut publisher, &["DEL", &key]), b":1\r\n");
    }
    assert_eq!(request(&mut subscriber, &["PING"]), b"+PONG\r\n", "no extra INFO or publication frame remains");
    drop((publisher, subscriber));
    assert_eq!(finished(child).0, 0);
}

/// self-publication must follow the command's full reply. Redis's Tcl tests
/// "publish to self inside multi" and "publish to self inside script" supply
/// the transaction/script expectations, including ordered pushes afterward.
/// Command/EXEC/script self-delivery and single unsubscribe inside EXEC
/// recorded at https://github.com/Ming-Research/Firn-wf/actions/runs/38399914253.
/// Additional Lua PING/ECHO/PUBSUB calls remain source-derived.
#[test]
fn resp3_self_publication_follows_complete_command_transaction_and_script_reply() {
    let (child, mut client, _) = start(1);
    hello(&mut client, 3);
    assert_eq!(request(&mut client, &["SUBSCRIBE", "news"]), acknowledgment(3, "subscribe", Some("news"), 1));
    assert_eq!(request(&mut client, &["SET", "key", "value"]), b"+OK\r\n");
    assert_eq!(request(&mut client, &["GET", "key"]), b"$5\r\nvalue\r\n");
    assert_eq!(request(&mut client, &["EVAL", "return redis.call('PING')", "0"]), b"+PONG\r\n", "the script's fake client has no subscriber mode");
    assert_eq!(request(&mut client, &["EVAL", "return redis.call('ECHO','script-echo')", "0"]), b"$11\r\nscript-echo\r\n", "the script's fake client uses ordinary ECHO framing");
    assert_eq!(request(&mut client, &["EVAL", "return redis.call('PUBSUB','NUMSUB','news')", "0"]), b"*2\r\n$4\r\nnews\r\n:1\r\n");
    // The read-only variants answer as EVAL does on a subscribed connection.
    // Redis 7.2.16's scriptVerifyWriteCommandAllow refuses a command flagged
    // WRITE or MAY_REPLICATE in a read-only script, and publish.json and
    // spublish.json flag both publications MAY_REPLICATE: source-derived, not
    // recorded. A refused publication must send no push to the subscriber.
    const READ_ONLY_REFUSED: &[u8] = b"-ERR Write commands are not allowed from read-only scripts.\r\n";
    assert_eq!(request(&mut client, &["EVAL_RO", "return redis.call('PING')", "0"]), b"+PONG\r\n");
    // The SHA1 of the script above, cached by the EVAL that ran it.
    assert_eq!(request(&mut client, &["EVALSHA_RO", "9e15db9d82a8a2f2029845f70d37494af8690c35", "0"]), b"+PONG\r\n");
    assert_eq!(request(&mut client, &["EVAL_RO", "return redis.pcall('PUBLISH','news','refused')", "0"]), READ_ONLY_REFUSED);
    assert_eq!(request(&mut client, &["EVAL_RO", "return redis.pcall('SPUBLISH','news','refused')", "0"]), READ_ONLY_REFUSED);
    assert_eq!(request(&mut client, &["PUBLISH", "news", "direct"]), b":1\r\n");
    assert_eq!(frame(&mut client), message(3, "message", "news", "direct"));
    send(&mut client, &[&["MULTI"], &["PING", "abc"], &["PUBLISH", "news", "first"], &["PUBLISH", "news", "second"], &["PING", "def"], &["EXEC"]]);
    assert_eq!(frame(&mut client), b"+OK\r\n");
    for _ in 0..4 { assert_eq!(frame(&mut client), b"+QUEUED\r\n"); }
    assert_eq!(frame(&mut client), b"*4\r\n$3\r\nabc\r\n:1\r\n:1\r\n$3\r\ndef\r\n");
    for value in ["first", "second"] { assert_eq!(frame(&mut client), message(3, "message", "news", value)); }
    assert_eq!(request(&mut client, &["EVAL", "redis.call('PUBLISH','news','script-first'); redis.call('PUBLISH','news','script-second'); return 'done'", "0"]), b"$4\r\ndone\r\n");
    for value in ["script-first", "script-second"] { assert_eq!(frame(&mut client), message(3, "message", "news", value)); }
    // An unsubscribe push occupies the queued command's element in EXEC.
    send(&mut client, &[&["MULTI"], &["PING", "before"], &["UNSUBSCRIBE", "news"], &["PING", "after"], &["EXEC"]]);
    assert_eq!(frame(&mut client), b"+OK\r\n");
    for _ in 0..3 { assert_eq!(frame(&mut client), b"+QUEUED\r\n"); }
    let mut expected = b"*3\r\n$6\r\nbefore\r\n".to_vec();
    expected.extend(acknowledgment(3, "unsubscribe", Some("news"), 0));
    expected.extend(b"$5\r\nafter\r\n");
    assert_eq!(frame(&mut client), expected);
    drop(client);
    assert_eq!(finished(child).0, 0);
}

/// A publish before undeclared-key fallback is not delivered twice; the
/// accepted final Lua error retains preceding publications just like writes.
/// Publisher bulk/error/count and NOSCRIPT bytes recorded at
/// https://github.com/Ming-Research/Firn-wf/actions/runs/38399914253.
/// That batch used RESP3 self-delivery; this separate RESP2 subscriber
/// framing and retry behavior remain source-derived.
#[test]
fn script_retry_stages_publications_and_final_error_keeps_them() {
    let (child, mut publisher, port) = start(2);
    let mut subscriber = connect_when_ready(port, &child);
    assert_eq!(request(&mut subscriber, &["SUBSCRIBE", "news"]), acknowledgment(2, "subscribe", Some("news"), 1));
    assert_eq!(request(&mut publisher, &["EVAL", "redis.call('PUBLISH','news','once'); redis.call('SET','undeclared','value'); return redis.call('GET','undeclared')", "0"]), b"$5\r\nvalue\r\n");
    assert_eq!(frame(&mut subscriber), message(2, "message", "news", "once"));
    assert_eq!(request(&mut publisher, &["EVAL", "redis.call('PUBLISH','news','accepted'); redis.call('GET'); return 'unreachable'", "0"]), b"-ERR Wrong number of args calling Redis command from script script: 805f867bced03fa755ce0d4cb7cae8e4c37142fe, on @user_script:1.\r\n");
    assert_eq!(frame(&mut subscriber), message(2, "message", "news", "accepted"));
    assert_eq!(request(&mut publisher, &["PUBLISH", "news", "barrier"]), b":1\r\n");
    assert_eq!(frame(&mut subscriber), message(2, "message", "news", "barrier"));
    assert_eq!(request(&mut publisher, &["EVAL", "return redis.pcall('SUBSCRIBE','news').err", "0"]), b"$49\r\nERR This Redis command is not allowed from script\r\n");
    drop((publisher, subscriber));
    assert_eq!(finished(child).0, 0);
}

/// Restarting for the first hub use after INCR must restore the declared
/// mutation; whole attempts retain their original dataset and hub execution.
/// A publication with no dataset writes must survive a later budget boundary.
/// Only the initial news subscription frame is covered by
/// https://github.com/Ming-Research/Firn-wf/actions/runs/38399914253.
/// INCR rollback, whole-map and budget boundaries are not recorded there.
#[test]
fn script_hub_restart_rollback_whole_attempt_and_budget_preserve_once() {
    let (child, mut publisher, port) = start(2);
    let mut subscriber = connect_when_ready(port, &child);
    assert_eq!(request(&mut subscriber, &["SUBSCRIBE", "news"]), acknowledgment(2, "subscribe", Some("news"), 1));
    let declared = "local n=redis.call('INCR',KEYS[1]); local c=redis.call('PUBLISH','news','declared'); return {n,c}";
    assert_eq!(request(&mut publisher, &["EVAL", declared, "1", "declared-counter"]), b"*2\r\n:1\r\n:1\r\n");
    assert_eq!(frame(&mut subscriber), message(2, "message", "news", "declared"));
    assert_eq!(request(&mut publisher, &["GET", "declared-counter"]), b"$1\r\n1\r\n");
    let whole = "redis.call('DBSIZE'); local n=redis.call('INCR','whole-counter'); local c=redis.call('PUBLISH','news','whole'); return {n,c}";
    assert_eq!(request(&mut publisher, &["EVAL", whole, "0"]), b"*2\r\n:1\r\n:1\r\n");
    assert_eq!(frame(&mut subscriber), message(2, "message", "news", "whole"));
    assert_eq!(request(&mut publisher, &["GET", "whole-counter"]), b"$1\r\n1\r\n");
    let budget = "local c=redis.call('PUBLISH','news','budget'); local sum=0; for i=1,2000 do sum=sum+i end; return {c,sum}";
    assert_eq!(request(&mut publisher, &["EVAL", budget, "0"]), b"*2\r\n:1\r\n:2001000\r\n");
    assert_eq!(frame(&mut subscriber), message(2, "message", "news", "budget"));
    let zero = "local c=redis.call('PUBLISH','absent','zero'); local sum=0; for i=1,2000 do sum=sum+i end; return {c,sum}";
    assert_eq!(request(&mut publisher, &["EVAL", zero, "0"]), b"*2\r\n:0\r\n:2001000\r\n");
    // The barrier detects extra frames from either replay or budget restart.
    assert_eq!(request(&mut publisher, &["PUBLISH", "news", "barrier"]), b":1\r\n");
    assert_eq!(frame(&mut subscriber), message(2, "message", "news", "barrier"));
    drop((publisher, subscriber));
    assert_eq!(finished(child).0, 0);
}

/// All six membership commands are NOSCRIPT; their table arity is checked
/// before NOSCRIPT. PUBLISH and PUBSUB introspection use separate allowed paths.
/// The SUBSCRIBE NOSCRIPT pcall result is recorded at
/// https://github.com/Ming-Research/Firn-wf/actions/runs/38399914253.
/// Other families, wrong arity and introspection calls remain source-derived.
#[test]
fn every_subscription_family_is_noscript_after_arity_validation() {
    let (child, mut client, _) = start(1);
    for command in ["SUBSCRIBE", "UNSUBSCRIBE", "PSUBSCRIBE", "PUNSUBSCRIBE", "SSUBSCRIBE", "SUNSUBSCRIBE"] {
        let script = format!("return redis.pcall('{command}','news').err");
        assert_eq!(request(&mut client, &["EVAL", &script, "0"]), b"$49\r\nERR This Redis command is not allowed from script\r\n", "{command}");
    }
    for command in ["SUBSCRIBE", "PSUBSCRIBE", "SSUBSCRIBE"] {
        let script = format!("return redis.pcall('{command}').err");
        assert_eq!(request(&mut client, &["EVAL", &script, "0"]), b"$58\r\nERR Wrong number of args calling Redis command from script\r\n", "{command}");
    }
    for command in ["UNSUBSCRIBE", "PUNSUBSCRIBE", "SUNSUBSCRIBE"] {
        let script = format!("return redis.pcall('{command}').err");
        assert_eq!(request(&mut client, &["EVAL", &script, "0"]), b"$49\r\nERR This Redis command is not allowed from script\r\n", "{command}");
    }
    assert_eq!(request(&mut client, &["PUBSUB", "NUMSUB", "news"]), b"*2\r\n$4\r\nnews\r\n:0\r\n");
    assert_eq!(request(&mut client, &["PUBSUB", "SHARDNUMSUB", "news"]), b"*2\r\n$4\r\nnews\r\n:0\r\n");
    assert_eq!(request(&mut client, &["PUBSUB", "NUMPAT"]), b":0\r\n");
    drop(client);
    assert_eq!(finished(child).0, 0);
}

/// Black-box TCP EOF cleanup: a new connection subscribing to the same name
/// receives no earlier delivery. This does not force an in-flight generation
/// reuse race, which requires a controllable dispatcher pause in a CI witness.
/// The initial news subscription and live NUMSUB count-1 bytes agree with
/// https://github.com/Ming-Research/Firn-wf/actions/runs/38399914253.
/// EOF removal, replacement identity and QUIT are not recorded there.
#[test]
fn disconnect_cleanup_and_new_connection_do_not_keep_old_membership() {
    let (child, mut control, port) = start(3);
    let mut old = connect_when_ready(port, &child);
    assert_eq!(request(&mut old, &["SUBSCRIBE", "news"]), acknowledgment(2, "subscribe", Some("news"), 1));
    old.shutdown(Shutdown::Both).expect("close subscriber");
    drop(old);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let count = request(&mut control, &["PUBSUB", "NUMSUB", "news"]);
        if count == b"*2\r\n$4\r\nnews\r\n:0\r\n" { break; }
        assert_eq!(count, b"*2\r\n$4\r\nnews\r\n:1\r\n");
        assert!(Instant::now() < deadline, "EOF left a pub/sub membership");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(request(&mut control, &["PUBLISH", "news", "stale"]), b":0\r\n");
    let mut replacement = connect_when_ready(port, &child);
    assert_eq!(request(&mut replacement, &["SUBSCRIBE", "news"]), acknowledgment(2, "subscribe", Some("news"), 1));
    assert_eq!(request(&mut control, &["PUBLISH", "news", "fresh"]), b":1\r\n");
    assert_eq!(frame(&mut replacement), message(2, "message", "news", "fresh"));
    assert_eq!(request(&mut replacement, &["QUIT"]), b"+OK\r\n");
    let mut byte = [0];
    assert_eq!(replacement.read(&mut byte).expect("QUIT closes output"), 0);
    drop((control, replacement));
    assert_eq!(finished(child).0, 0);
}

/// Redis 7.2.16 timeout.c:clientsCronHandleTimeout excludes CLIENT_PUBSUB
/// under both protocols. pubsub.c clears that flag when the last membership
/// disappears. The controller also subscribes so it survives the fixed wait.
/// Basic news acknowledgment bytes are recorded at
/// https://github.com/Ming-Research/Firn-wf/actions/runs/38399914253.
/// The timeout, all-family lifetime and still-alive payload are not recorded.
#[test]
fn active_subscription_families_exempt_idle_timeout_then_last_unsubscribe_closes() {
    for version in [2, 3] {
        for (subscribe, unsubscribe, publish, acknowledgment_kind, message_kind) in [
            ("SUBSCRIBE", "UNSUBSCRIBE", "PUBLISH", "subscribe", "message"),
            ("PSUBSCRIBE", "PUNSUBSCRIBE", "PUBLISH", "psubscribe", "pmessage"),
            ("SSUBSCRIBE", "SUNSUBSCRIBE", "SPUBLISH", "ssubscribe", "smessage"),
        ] {
            let (child, mut control, port) = start(2);
            let mut subscriber = connect_when_ready(port, &child);
            hello(&mut control, 3);
            hello(&mut subscriber, version);
            assert_eq!(request(&mut control, &["SUBSCRIBE", "__timeout_control__"]), acknowledgment(3, "subscribe", Some("__timeout_control__"), 1));
            assert_eq!(request(&mut subscriber, &[subscribe, "news"]), acknowledgment(version, acknowledgment_kind, Some("news"), 1));
            assert_eq!(request(&mut control, &["CONFIG", "SET", "timeout", "1"]), b"+OK\r\n");
            // Two whole Unix seconds must pass for Redis's idle > 1 rule.
            // No requests are sent on either connection during this wait.
            std::thread::sleep(Duration::from_millis(2200));
            assert_eq!(request(&mut control, &[publish, "news", "still-alive"]), b":1\r\n", "{subscribe} RESP{version} lost its idle exemption");
            let expected = if message_kind == "pmessage" {
                pattern_message(version, "news", "news", "still-alive")
            } else {
                message(version, message_kind, "news", "still-alive")
            };
            assert_eq!(frame(&mut subscriber), expected);
            let kind = unsubscribe.to_ascii_lowercase();
            assert_eq!(request(&mut subscriber, &[unsubscribe, "news"]), acknowledgment(version, &kind, Some("news"), 0));
            subscriber.set_read_timeout(Some(Duration::from_secs(5))).expect("bound the normal idle close");
            let mut byte = [0];
            assert_eq!(subscriber.read(&mut byte).expect("last unsubscribe restores normal idle timeout"), 0, "{unsubscribe} RESP{version} left unexpected output");
            assert_eq!(request(&mut control, &["PING"]), b"+PONG\r\n", "the subscribed controller remains exempt");
            drop((control, subscriber));
            assert_eq!(finished(child).0, 0);
        }
    }
}

/// Redis networking.c closes a QUIT/protocol-error connection after buffered
/// output drains. One connection's graceful stop cannot cancel another owner.
/// Only the initial news subscribe frame is covered by
/// https://github.com/Ming-Research/Firn-wf/actions/runs/38399914253.
/// Graceful close, protocol errors and independent-connection lifetime remain
/// source-derived; no such drain was captured by those twelve batches.
#[test]
fn promoted_quit_and_protocol_error_drain_pipeline_and_leave_other_subscriber_live() {
    for quit in [true, false] {
        let (child, mut publisher, port) = start(3);
        let mut ending = connect_when_ready(port, &child);
        let mut independent = connect_when_ready(port, &child);
        hello(&mut ending, 3);
        assert_eq!(request(&mut ending, &["SUBSCRIBE", "news"]), acknowledgment(3, "subscribe", Some("news"), 1));
        assert_eq!(request(&mut independent, &["SUBSCRIBE", "news"]), acknowledgment(2, "subscribe", Some("news"), 1));
        assert_eq!(request(&mut publisher, &["PUBLISH", "news", "before-close"]), b":2\r\n");
        assert_eq!(frame(&mut independent), message(2, "message", "news", "before-close"));
        let mut pipeline = resp(&["ECHO", "earlier-reply"]);
        if quit {
            pipeline.extend(resp(&["QUIT"]));
        } else {
            pipeline.extend_from_slice(b"*1\r\n!0\r\n");
        }
        pipeline.extend(resp(&["ECHO", "must-not-execute"]));
        ending.write_all(&pipeline).unwrap();
        assert_eq!(frame(&mut ending), message(3, "message", "news", "before-close"));
        assert_eq!(frame(&mut ending), b"$13\r\nearlier-reply\r\n");
        assert_eq!(frame(&mut ending), if quit { b"+OK\r\n".as_slice() } else { b"-ERR Protocol error: expected '$', got '!'\r\n".as_slice() });
        let mut byte = [0];
        assert_eq!(ending.read(&mut byte).expect("graceful output closes after its final marker"), 0);
        assert_eq!(request(&mut publisher, &["PUBLISH", "news", "after-close"]), b":1\r\n");
        assert_eq!(frame(&mut independent), message(2, "message", "news", "after-close"));
        assert_eq!(request(&mut independent, &["PING"]), b"*2\r\n$4\r\npong\r\n$0\r\n\r\n");
        drop((publisher, ending, independent));
        assert_eq!(finished(child).0, 0);
    }
}

/// An unread subscriber cannot park delivery to another subscriber. This
/// progress case allows the hard limit to close the slow peer; the separate
/// oversized-publication case below requires that closure to happen.
#[test]
fn slow_subscriber_does_not_delay_other_subscribers() {
    let (child, mut publisher, port) = start(3);
    let mut slow = connect_when_ready(port, &child);
    let mut fast = connect_when_ready(port, &child);
    for stream in [&mut slow, &mut fast] {
        assert_eq!(request(stream, &["SUBSCRIBE", "large"]), acknowledgment(2, "subscribe", Some("large"), 1));
    }
    let payload = "x".repeat(1024 * 1024);
    let expected = message(2, "message", "large", &payload);
    for _ in 0..40 {
        let count = request(&mut publisher, &["PUBLISH", "large", &payload]);
        assert!(count == b":2\r\n" || count == b":1\r\n", "unexpected recipient count: {count:?}");
        assert_eq!(frame(&mut fast), expected, "fast subscriber receives while slow subscriber never reads");
    }
    drop((publisher, slow, fast));
    assert_eq!(finished(child).0, 0);
}

/// One 33 MiB body exceeds Redis's 32 MiB pub/sub hard output limit when its
/// complete reply is queued, before socket flushing, regardless of kernel
/// buffer sizes. Redis 7.2.16 src/networking.c:_addReplyProtoToList checks that
/// limit synchronously; src/config.c:clientBufferLimitsDefaults defines it.
/// A separate small channel keeps the fast subscriber below that same limit.
/// This witnesses the hard limit, not the 8 MiB / 60-second soft limit.
#[test]
fn hard_output_limit_closes_slow_subscriber_and_other_subscriber_keeps_receiving() {
    let (child, mut publisher, port) = start(3);
    let mut slow = connect_when_ready(port, &child);
    let mut fast = connect_when_ready(port, &child);
    assert_eq!(request(&mut slow, &["SUBSCRIBE", "large"]), acknowledgment(2, "subscribe", Some("large"), 1));
    assert_eq!(request(&mut fast, &["SUBSCRIBE", "heartbeat"]), acknowledgment(2, "subscribe", Some("heartbeat"), 1));
    let payload = "x".repeat(33 * 1024 * 1024);
    assert_eq!(request(&mut publisher, &["PUBLISH", "large", &payload]), b":1\r\n", "the publication counts the subscriber before its output limit closes it");
    // Closing is asynchronous in Redis. Require actual registry removal;
    // disabled limits leave :1 here and fail this bounded observation.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let count = request(&mut publisher, &["PUBSUB", "NUMSUB", "large"]);
        if count == b"*2\r\n$5\r\nlarge\r\n:0\r\n" { break; }
        assert_eq!(count, b"*2\r\n$5\r\nlarge\r\n:1\r\n");
        assert!(Instant::now() < deadline, "32 MiB hard limit did not remove the unread subscriber after a 33 MiB publication");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(request(&mut publisher, &["PUBLISH", "heartbeat", "still-live"]), b":1\r\n");
    assert_eq!(frame(&mut fast), message(2, "message", "heartbeat", "still-live"), "closing the slow subscriber does not close or stall the fast subscriber");
    // Allow bytes already handed to the socket before closing. Drain with
    // fixed storage and require closure, rather than treating registry removal
    // alone as evidence that the connection was closed.
    let mut buffer = [0; 64 * 1024];
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match slow.read(&mut buffer) {
            Ok(0) => break,
            Ok(_) => assert!(Instant::now() < deadline, "output-limited subscriber never reached EOF"),
            Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => break,
            Err(error) => panic!("output-limited subscriber did not close: {error}"),
        }
    }
    drop((publisher, slow, fast));
    assert_eq!(finished(child).0, 0);
}

/// The soft cases keep firn running past the shared 60-second program
/// deadline: one waits out the 60-second interval, the other 70 seconds.
const SOFT_CASE_LIMIT: Duration = Duration::from_secs(150);

/// Reads what the kernel still holds for a closed subscriber and requires the
/// end of the stream, so a registry removal is not taken for a closed socket.
fn assert_closed_after_draining(stream: &mut TcpStream, what: &str) {
    let mut buffer = [0; 64 * 1024];
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(_) => assert!(Instant::now() < deadline, "{what}: never reached EOF"),
            Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => break,
            Err(error) => panic!("{what}: did not close: {error}"),
        }
    }
}

/// Redis 7.2.16 gives a pub/sub client the default limit `32mb 8mb 60`
/// (src/config.c clientBufferLimitsDefaults): output that stays at or over
/// 8 MiB for more than 60 seconds closes it (src/networking.c
/// checkClientOutputBufferLimits compares whole-second times). A 14 MiB
/// publication is over the soft and under the hard limit, and larger than the
/// kernel's loopback socket buffers, so an unread subscriber holds its whole
/// frame; firn charges a frame until it has been sent completely. The
/// subscriber must stay registered well past half the interval and be gone
/// after it, closed and not merely unregistered.
#[test]
fn soft_output_limit_closes_a_subscriber_whose_backlog_stays_over_8_mib_for_60_seconds() {
    let (child, mut publisher, port) = start_for(2, "1", SOFT_CASE_LIMIT);
    let mut slow = connect_when_ready(port, &child);
    assert_eq!(request(&mut slow, &["SUBSCRIBE", "soft"]), acknowledgment(2, "subscribe", Some("soft"), 1));
    let payload = "x".repeat(14 * 1024 * 1024);
    assert_eq!(request(&mut publisher, &["PUBLISH", "soft", &payload]), b":1\r\n");
    let published = Instant::now();
    let mut registered_past_half = false;
    loop {
        let count = request(&mut publisher, &["PUBSUB", "NUMSUB", "soft"]);
        let elapsed = published.elapsed();
        if count == b"*2\r\n$4\r\nsoft\r\n:0\r\n" {
            assert!(
                elapsed >= Duration::from_secs(59),
                "the soft limit closed the subscriber after {elapsed:?}, before its 60 seconds"
            );
            break;
        }
        assert_eq!(count, b"*2\r\n$4\r\nsoft\r\n:1\r\n");
        registered_past_half |= elapsed >= Duration::from_secs(30);
        assert!(elapsed < Duration::from_secs(90), "the soft limit did not close the unread subscriber within 90 seconds");
        std::thread::sleep(Duration::from_millis(250));
    }
    assert!(registered_past_half, "the subscriber was observed registered after 30 seconds");
    assert_closed_after_draining(&mut slow, "soft-limited subscriber");
    drop((publisher, slow));
    assert_eq!(finished(child).0, 0);
}

/// The soft interval is the time the backlog stays continuously at or over
/// 8 MiB: Redis 7.2.16 clears obuf_soft_limit_reached_time when the output
/// falls below the limit (checkClientOutputBufferLimits). The subscriber
/// reads its 14 MiB frame after 35 seconds, a second frame is published
/// after the first one's charge is released, and the subscriber must still be
/// registered at 70 seconds, past the 61 seconds after the first publication
/// that the first interval would have ended at, and must receive the second
/// frame whole.
#[test]
fn soft_output_limit_interval_restarts_after_the_backlog_drains() {
    let (child, mut publisher, port) = start_for(2, "1", SOFT_CASE_LIMIT);
    let mut subscriber = connect_when_ready(port, &child);
    assert_eq!(request(&mut subscriber, &["SUBSCRIBE", "soft"]), acknowledgment(2, "subscribe", Some("soft"), 1));
    let payload = "x".repeat(14 * 1024 * 1024);
    let expected = message(2, "message", "soft", &payload);
    assert_eq!(request(&mut publisher, &["PUBLISH", "soft", &payload]), b":1\r\n");
    let first = Instant::now();
    std::thread::sleep(Duration::from_secs(35));
    assert_eq!(frame(&mut subscriber), expected, "the first frame arrives whole after the wait");
    // The sender releases the frame's charge after its last byte is accepted;
    // wait that out so the second publication starts a new interval.
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(request(&mut publisher, &["PUBLISH", "soft", &payload]), b":1\r\n");
    std::thread::sleep((first + Duration::from_secs(70)).saturating_duration_since(Instant::now()));
    assert_eq!(
        request(&mut publisher, &["PUBSUB", "NUMSUB", "soft"]),
        b"*2\r\n$4\r\nsoft\r\n:1\r\n",
        "the first interval ended with the drained backlog and must not close the subscriber"
    );
    assert_eq!(frame(&mut subscriber), expected, "the second frame arrives whole");
    assert_eq!(request(&mut publisher, &["PUBLISH", "soft", "tail"]), b":1\r\n");
    assert_eq!(frame(&mut subscriber), message(2, "message", "soft", "tail"));
    drop((publisher, subscriber));
    assert_eq!(finished(child).0, 0);
}

/// A subscribed RESP3 connection writes the append-only file as any other
/// connection does: the lazy-expiry DELs a command makes before its own
/// effect, and a command with more than one effect, are wrapped in MULTI and
/// EXEC. The expected bytes are the fragments of the file Redis 7.2.16
/// recorded for the same requests in
/// research/experiments/aof-oracle/legacy-forms-recording.json
/// (legacy-direct-propagation-forms): an MSET over an expired key, a DEL and
/// an EXISTS that each find two expired keys.
#[test]
fn subscribed_connection_wraps_multi_record_commands_in_the_append_only_file() {
    let program = firn();
    let fixture = fixture_directory();
    let lease = support::lease_port();
    let port = lease.port();
    let text = port.to_string();
    let name = "subscribed-forms.aof";
    let child = program
        .spawn_in_directory(fixture.path(), &[text.as_bytes(), b"1", name.as_bytes()])
        .with_port(&lease);
    let mut client = connect_when_ready(port, &child);
    hello(&mut client, 3);
    assert_eq!(request(&mut client, &["SUBSCRIBE", "watch"]), acknowledgment(3, "subscribe", Some("watch"), 1));
    let mut batch = Vec::new();
    let mut expected = Vec::new();
    for (command, effects) in [
        (vec!["SET", "mset:k", "old", "PXAT", "1"], vec![vec!["SET", "mset:k", "old", "PXAT", "1"]]),
        (vec!["MSET", "mset:k", "new"], vec![vec!["MULTI"], vec!["DEL", "mset:k"], vec!["MSET", "mset:k", "new"], vec!["EXEC"]]),
        (vec!["SET", "y2", "1", "PXAT", "1"], vec![vec!["SET", "y2", "1", "PXAT", "1"]]),
        (vec!["SET", "y1", "1", "PXAT", "1"], vec![vec!["SET", "y1", "1", "PXAT", "1"]]),
        (vec!["DEL", "y2", "y1", "y2"], vec![vec!["MULTI"], vec!["DEL", "y2"], vec!["DEL", "y1"], vec!["EXEC"]]),
        (vec!["SET", "x2", "1", "PXAT", "1"], vec![vec!["SET", "x2", "1", "PXAT", "1"]]),
        (vec!["SET", "x1", "1", "PXAT", "1"], vec![vec!["SET", "x1", "1", "PXAT", "1"]]),
        (vec!["EXISTS", "x2", "x1"], vec![vec!["MULTI"], vec!["DEL", "x2"], vec!["DEL", "x1"], vec!["EXEC"]]),
    ] {
        batch.extend(resp(&command));
        for effect in effects {
            expected.extend(resp(&effect));
        }
    }
    client.write_all(&batch).expect("send the writes");
    expect_replies(
        &mut client,
        b"+OK\r\n+OK\r\n+OK\r\n+OK\r\n:0\r\n+OK\r\n+OK\r\n:0\r\n",
        "the writes on a subscribed connection",
    );
    drop(client);
    assert_eq!(finished(child).0, 0);
    let file = std::fs::read(aof_incremental_path(fixture.path(), name)).expect("read firn's file");
    assert_eq!(
        aof_record_bytes(&aof_records_without_select_zero(&file)),
        expected,
        "a subscribed connection's append-only file"
    );
}

// Step 5: pattern and shard channels. These are separate acceptance cases.

/// One direct and two pattern matches deliver three frames and count three
/// recipients; NUMPAT counts distinct registered patterns across clients.
/// RESP3 news/n* acknowledgments, unique NUMPAT and the first client
/// delivery frames recorded at https://github.com/Ming-Research/Firn-wf/actions/runs/38399914253.
/// That batch has one client (:2); this two-client fanout correctly uses :3.
/// Its empty PUNSUBSCRIBE retains a channel (:1); this second client uses :0.
#[test]
fn patterns_count_each_delivery_and_unique_registry_patterns() {
    for version in [2, 3] {
        let (child, mut control, port) = start(3);
        let mut first = connect_when_ready(port, &child);
        let mut second = connect_when_ready(port, &child);
        hello(&mut first, version);
        hello(&mut second, version);
        assert_eq!(request(&mut first, &["SUBSCRIBE", "news"]), acknowledgment(version, "subscribe", Some("news"), 1));
        assert_eq!(request(&mut first, &["PSUBSCRIBE", "n*", "n*"]), acknowledgment(version, "psubscribe", Some("n*"), 2));
        assert_eq!(frame(&mut first), acknowledgment(version, "psubscribe", Some("n*"), 2));
        assert_eq!(request(&mut second, &["PSUBSCRIBE", "n*"]), acknowledgment(version, "psubscribe", Some("n*"), 1));
        assert_eq!(request(&mut control, &["PUBSUB", "NUMPAT"]), b":1\r\n");
        assert_eq!(request(&mut control, &["PUBLISH", "news", "value"]), b":3\r\n");
        assert_eq!(frame(&mut first), message(version, "message", "news", "value"));
        assert_eq!(frame(&mut first), pattern_message(version, "n*", "news", "value"));
        assert_eq!(frame(&mut second), pattern_message(version, "n*", "news", "value"));
        assert_eq!(request(&mut first, &["PUNSUBSCRIBE"]), acknowledgment(version, "punsubscribe", Some("n*"), 1));
        assert_eq!(request(&mut second, &["PUNSUBSCRIBE"]), acknowledgment(version, "punsubscribe", Some("n*"), 0));
        assert_eq!(request(&mut second, &["PUNSUBSCRIBE"]), acknowledgment(version, "punsubscribe", None, 0));
        assert_eq!(request(&mut control, &["PUBSUB", "NUMPAT"]), b":0\r\n");
        drop((control, first, second));
        assert_eq!(finished(child).0, 0);
    }
}

/// Redis 7.2.16 server.c:genRedisInfoString derives these fields from the
/// three server registries, so shared names and repeated memberships each
/// contribute one registry entry until its last subscriber leaves.
/// Duplicate SSUBSCRIBE acknowledgment bytes are recorded at
/// https://github.com/Ming-Research/Firn-wf/actions/runs/38399914253.
/// INFO framing/counts, shared memberships and last-member cleanup remain
/// source-derived; INFO was not sent in those batches.
#[test]
fn info_stats_counts_unique_pubsub_registries_and_last_membership_cleanup() {
    fn counts(control: &mut TcpStream, version: u8, expected: [usize; 3]) {
        let wire = request(control, &["INFO", "stats"]);
        assert_eq!(wire[0], if version == 3 { b'=' } else { b'$' }, "INFO uses bulk/verbatim framing");
        let header = wire.windows(2).position(|bytes| bytes == b"\r\n").expect("INFO bulk header") + 2;
        assert!(wire.ends_with(b"\r\n"));
        let mut body = &wire[header..wire.len() - 2];
        if version == 3 { body = body.strip_prefix(b"txt:").expect("INFO verbatim text format"); }
        let text = std::str::from_utf8(body).expect("INFO stats text");
        for (field, expected) in ["pubsub_channels", "pubsub_patterns", "pubsubshard_channels"].into_iter().zip(expected) {
            assert_eq!(info_field(text, field), Some(expected.to_string()), "{field}: {text}");
        }
    }
    for version in [2, 3] {
        let (child, mut control, port) = start(3);
        let mut first = connect_when_ready(port, &child);
        let mut second = connect_when_ready(port, &child);
        hello(&mut control, version);
        hello(&mut first, version);
        hello(&mut second, version);
        counts(&mut control, version, [0, 0, 0]);
        for (index, stream) in [&mut first, &mut second].into_iter().enumerate() {
            assert_eq!(request(stream, &["SUBSCRIBE", "news", "news"]), acknowledgment(version, "subscribe", Some("news"), 1));
            assert_eq!(frame(stream), acknowledgment(version, "subscribe", Some("news"), 1));
            counts(&mut control, version, [1, index, index]);
            assert_eq!(request(stream, &["PSUBSCRIBE", "n*", "n*"]), acknowledgment(version, "psubscribe", Some("n*"), 2));
            assert_eq!(frame(stream), acknowledgment(version, "psubscribe", Some("n*"), 2));
            counts(&mut control, version, [1, 1, index]);
            assert_eq!(request(stream, &["SSUBSCRIBE", "news", "news"]), acknowledgment(version, "ssubscribe", Some("news"), 1));
            assert_eq!(frame(stream), acknowledgment(version, "ssubscribe", Some("news"), 1));
        }
        counts(&mut control, version, [1, 1, 1]);
        for (index, stream) in [&mut first, &mut second].into_iter().enumerate() {
            assert_eq!(request(stream, &["UNSUBSCRIBE", "news"]), acknowledgment(version, "unsubscribe", Some("news"), 1));
            let remaining = if index == 0 { 1 } else { 0 };
            counts(&mut control, version, [remaining, 1, 1]);
            assert_eq!(request(stream, &["PUNSUBSCRIBE", "n*"]), acknowledgment(version, "punsubscribe", Some("n*"), 0));
            counts(&mut control, version, [remaining, remaining, 1]);
            assert_eq!(request(stream, &["SUNSUBSCRIBE", "news"]), acknowledgment(version, "sunsubscribe", Some("news"), 0));
            // The other client keeps each shared entry alive after the first
            // cleanup; after the second cleanup every registry is empty.
            assert_eq!(request(stream, &["PING"]), b"+PONG\r\n", "cleanup must end subscriber mode");
            counts(&mut control, version, [remaining, remaining, remaining]);
        }
        drop((control, first, second));
        assert_eq!(finished(child).0, 0);
    }
}

/// Every expected match is derived from Redis util.c:stringmatchlen, including
/// reversed ranges, literal escapes, negation, malformed classes and empty names.
#[test]
fn patterns_and_channel_filter_use_redis_stringmatchlen() {
    let (child, mut control, port) = start(2);
    let mut subscriber = connect_when_ready(port, &child);
    let cases = [
        ("n?ws", "news", true), ("n?ws", "News", false),
        ("[z-a]", "m", true), ("[^a]", "a", false),
        ("[^a]", "b", true), ("\\*", "*", true),
        ("[abc", "a", true), ("*", "", false),
        ("", "", true), ("a**?", "ab", true),
    ];
    for (pattern, channel, matched) in cases {
        assert_eq!(request(&mut subscriber, &["PSUBSCRIBE", pattern]), acknowledgment(2, "psubscribe", Some(pattern), 1));
        let expected = if matched { b":1\r\n".as_slice() } else { b":0\r\n".as_slice() };
        assert_eq!(request(&mut control, &["PUBLISH", channel, "value"]), expected, "pattern {pattern:?} channel {channel:?}");
        if matched { assert_eq!(frame(&mut subscriber), pattern_message(2, pattern, channel, "value")); }
        assert_eq!(request(&mut subscriber, &["PUNSUBSCRIBE", pattern]), acknowledgment(2, "punsubscribe", Some(pattern), 0));
    }
    assert_eq!(request(&mut subscriber, &["SUBSCRIBE", "news"]), acknowledgment(2, "subscribe", Some("news"), 1));
    assert_eq!(request(&mut control, &["PUBSUB", "CHANNELS", "n[ea]ws"]), b"*1\r\n$4\r\nnews\r\n");
    assert_eq!(request(&mut control, &["PUBSUB", "CHANNELS", "N*"]), b"*0\r\n");
    drop((control, subscriber));
    assert_eq!(finished(child).0, 0);
}

/// Redis util.c refuses recursive nesting greater than 1000. A final trailing
/// star after the last byte is skipped without another recursive match call.
#[test]
fn pattern_recursion_depth_matches_redis_boundary() {
    let (child, mut publisher, port) = start(2);
    let mut subscriber = connect_when_ready(port, &child);
    for stars in [1000, 1001] {
        let pattern = format!("{}a", "*a".repeat(stars));
        let channel = "a".repeat(stars + 1);
        assert_eq!(request(&mut subscriber, &["PSUBSCRIBE", &pattern]), acknowledgment(2, "psubscribe", Some(&pattern), 1));
        assert_eq!(request(&mut publisher, &["PUBLISH", &channel, "depth"]), if stars == 1000 { b":1\r\n".as_slice() } else { b":0\r\n".as_slice() });
        if stars == 1000 { assert_eq!(frame(&mut subscriber), pattern_message(2, &pattern, &channel, "depth")); }
        assert_eq!(request(&mut subscriber, &["PUNSUBSCRIBE", &pattern]), acknowledgment(2, "punsubscribe", Some(&pattern), 0));
    }
    let pattern = format!("{}*", "*a".repeat(1000));
    let channel = "a".repeat(1000);
    assert_eq!(request(&mut subscriber, &["PSUBSCRIBE", &pattern]), acknowledgment(2, "psubscribe", Some(&pattern), 1));
    assert_eq!(request(&mut publisher, &["PUBLISH", &channel, "trailing"]), b":1\r\n");
    assert_eq!(frame(&mut subscriber), pattern_message(2, &pattern, &channel, "trailing"));
    drop((publisher, subscriber));
    assert_eq!(finished(child).0, 0);
}

/// Shard channels occupy a separate registry and acknowledge their own count;
/// removing global channels alone leaves RESP2 subscriber mode active.
/// RESP3 shard acknowledgments, query bytes and publication frames recorded
/// at https://github.com/Ming-Research/Firn-wf/actions/runs/38399914253.
/// Recorder deliveries are self-pushes; these use separate publisher/receiver
/// sockets. RESP2 mode lifetime and empty query variants remain source-derived.
#[test]
fn shard_channels_are_separate_with_independent_counts_and_mode_lifetime() {
    for version in [2, 3] {
        let (child, mut control, port) = start(2);
        let mut subscriber = connect_when_ready(port, &child);
        hello(&mut subscriber, version);
        assert_eq!(request(&mut subscriber, &["SUBSCRIBE", "news"]), acknowledgment(version, "subscribe", Some("news"), 1));
        assert_eq!(request(&mut subscriber, &["SSUBSCRIBE", "news", "news"]), acknowledgment(version, "ssubscribe", Some("news"), 1));
        assert_eq!(frame(&mut subscriber), acknowledgment(version, "ssubscribe", Some("news"), 1));
        assert_eq!(request(&mut control, &["PUBSUB", "SHARDNUMSUB", "news", "absent"]), b"*4\r\n$4\r\nnews\r\n:1\r\n$6\r\nabsent\r\n:0\r\n");
        assert_eq!(request(&mut control, &["PUBSUB", "SHARDNUMSUB"]), b"*0\r\n");
        assert_eq!(request(&mut control, &["PUBSUB", "SHARDCHANNELS", "n*"]), b"*1\r\n$4\r\nnews\r\n");
        assert_eq!(request(&mut control, &["PUBLISH", "news", "global"]), b":1\r\n");
        assert_eq!(frame(&mut subscriber), message(version, "message", "news", "global"));
        assert_eq!(request(&mut control, &["SPUBLISH", "news", "shard"]), b":1\r\n");
        assert_eq!(frame(&mut subscriber), message(version, "smessage", "news", "shard"));
        assert_eq!(request(&mut subscriber, &["UNSUBSCRIBE"]), acknowledgment(version, "unsubscribe", Some("news"), 0));
        let ping = request(&mut subscriber, &["PING"]);
        assert_eq!(ping, if version == 2 { b"*2\r\n$4\r\npong\r\n$0\r\n\r\n".as_slice() } else { b"+PONG\r\n".as_slice() });
        assert_eq!(request(&mut subscriber, &["SUNSUBSCRIBE"]), acknowledgment(version, "sunsubscribe", Some("news"), 0));
        assert_eq!(request(&mut subscriber, &["SUNSUBSCRIBE"]), acknowledgment(version, "sunsubscribe", None, 0));
        assert_eq!(request(&mut subscriber, &["PING"]), b"+PONG\r\n");
        assert_eq!(request(&mut control, &["SPUBLISH", "news", "gone"]), b":0\r\n");
        assert_eq!(request(&mut control, &["PUBSUB", "SHARDCHANNELS"]), b"*0\r\n");
        drop((control, subscriber));
        assert_eq!(finished(child).0, 0);
    }
}

/// EXEC sets DENY_BLOCKING: Redis permits regular SUBSCRIBE there but refuses
/// SSUBSCRIBE unconditionally. SPUBLISH remains legal in EXEC and scripts.
/// DENY BLOCKING and EXEC/Lua self-SPUBLISH bytes recorded at
/// https://github.com/Ming-Research/Firn-wf/actions/runs/38399914253
/// (step5-shard-exec-and-script).
#[test]
fn shard_subscribe_inside_exec_refuses_and_shard_publication_composes() {
    let (child, mut client, _) = start(1);
    hello(&mut client, 3);
    send(&mut client, &[&["MULTI"], &["SSUBSCRIBE", "news"], &["EXEC"]]);
    assert_eq!(frame(&mut client), b"+OK\r\n");
    assert_eq!(frame(&mut client), b"+QUEUED\r\n");
    assert_eq!(frame(&mut client), b"*1\r\n-ERR SSUBSCRIBE isn't allowed for a DENY BLOCKING client\r\n");
    assert_eq!(request(&mut client, &["SSUBSCRIBE", "news"]), acknowledgment(3, "ssubscribe", Some("news"), 1));
    send(&mut client, &[&["MULTI"], &["SPUBLISH", "news", "transaction"], &["EXEC"]]);
    assert_eq!(frame(&mut client), b"+OK\r\n");
    assert_eq!(frame(&mut client), b"+QUEUED\r\n");
    assert_eq!(frame(&mut client), b"*1\r\n:1\r\n");
    assert_eq!(frame(&mut client), message(3, "smessage", "news", "transaction"));
    assert_eq!(request(&mut client, &["EVAL", "return redis.call('SPUBLISH','news','script')", "0"]), b":1\r\n");
    assert_eq!(frame(&mut client), message(3, "smessage", "news", "script"));
    drop(client);
    assert_eq!(finished(child).0, 0);
}
