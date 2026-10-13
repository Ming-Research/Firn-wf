//! Checked connection setup and depth-one client enumeration.
use super::*;

pub(super) fn request(out: &mut Vec<u8>) -> usize {
    command(out, &[b"HELLO", b"3"]);
    command(out, &[b"CLIENT", b"SETINFO", b"LIB-NAME", b"firn-workload"]);
    command(out, &[b"CLIENT", b"SETINFO", b"LIB-VER", b"1.0"]);
    command(out, &[b"PING"]);
    4
}

// The shared framing parser checked each complete value, including map entries
// and nested errors, before these command-specific expectations are applied.
pub(super) fn check_reply(wire: &[u8], index: usize) -> Result<()> {
    match index {
        0 if wire.first() == Some(&b'%') => Ok(()),
        1 | 2 if wire == b"+OK\r\n" => Ok(()),
        3 if wire == b"+PONG\r\n" => Ok(()),
        _ => Err(format!("handshake reply {} has wrong type or value: {:?}",
            index + 1, String::from_utf8_lossy(wire)).into()),
    }
}

pub(super) fn setup(socket: &mut TcpStream) -> Result<()> {
    let mut out = Vec::new();
    command(&mut out, &[b"CLIENT", b"SETINFO", b"LIB-NAME", b"firn-workload"]);
    socket.write_all(&out)?;
    let mut input = Vec::new();
    loop {
        if let Some(size) = reply(&input, 0)? {
            if size != input.len() || input != b"+OK\r\n" {
                return Err("CLIENT SETINFO setup did not return exactly +OK".into());
            }
            return Ok(());
        }
        read_more(socket, &mut input)?;
    }
}

pub(super) fn check_list(wire: &[u8], connections: usize) -> Result<()> {
    if wire.first() != Some(&b'$') || wire == b"$-1\r\n" {
        return Err("CLIENT LIST did not return a non-null bulk string".into());
    }
    let start = wire.windows(2).position(|w| w == b"\r\n").unwrap() + 2;
    let body = &wire[start..wire.len() - 2];
    let records = body.strip_suffix(b"\n").ok_or("CLIENT LIST has no final newline")?;
    let mut count = 0;
    for record in records.split(|&b| b == b'\n') {
        if record.is_empty() || record == b"\r" {
            return Err("CLIENT LIST contains an empty client record".into());
        }
        count += 1;
    }
    if count < connections {
        return Err(format!("CLIENT LIST returned {count} clients; expected at least {connections}").into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn expect_handshake(socket: &mut TcpStream) {
        super::super::tests::expect_command(socket, &[b"HELLO", b"3"]);
        super::super::tests::expect_command(socket, &[b"CLIENT", b"SETINFO", b"LIB-NAME", b"firn-workload"]);
        super::super::tests::expect_command(socket, &[b"CLIENT", b"SETINFO", b"LIB-VER", b"1.0"]);
        super::super::tests::expect_command(socket, &[b"PING"]);
    }

    #[test]
    fn fragmented_hello_map_and_nested_errors() {
        // HELLO's map contains strings, integers and a nested modules array.
        let wire = b"%3\r\n$6\r\nserver\r\n$5\r\nredis\r\n$5\r\nproto\r\n:3\r\n$7\r\nmodules\r\n*1\r\n%1\r\n+name\r\n+test\r\n";
        for n in 0..wire.len() { assert_eq!(reply(&wire[..n], 0).unwrap(), None); }
        assert_eq!(reply(wire, 0).unwrap(), Some(wire.len()));
        check_reply(wire, 0).unwrap();
        for bad in [b"%-1\r\n".as_slice(), b"%1\r\n+k\r\n*1\r\n-ERR nested\r\n"] {
            assert!(reply(bad, 0).is_err());
        }
        for (index, wire) in [(0, b"*0\r\n".as_slice()), (1, b":1\r\n"),
                              (2, b"+QUEUED\r\n"), (3, b"$4\r\nPONG\r\n")] {
            assert!(check_reply(wire, index).is_err());
        }
    }

    #[test]
    fn handshake_uses_one_fresh_socket_per_completed_operation() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let mut opts = super::super::tests::options(listener.local_addr().unwrap().port(), "handshake");
        opts.threads = 1; // Two slots share a worker, rather than one thread per socket.
        let server = thread::spawn(move || {
            let mut clients = Vec::new();
            for _ in 0..3 {
                let (mut socket, _) = listener.accept().unwrap();
                clients.push(thread::spawn(move || {
                    socket.set_read_timeout(Some(TIMEOUT)).unwrap();
                    expect_handshake(&mut socket);
                    for part in [b"%1\r\n+proto\r".as_slice(), b"\n:3\r\n+OK\r\n+OK\r\n+PONG\r\n"] {
                        socket.write_all(part).unwrap();
                    }
                    let mut byte = [0];
                    assert_eq!(socket.read(&mut byte).unwrap(), 0, "socket was reused after PONG");
                }));
            }
            for client in clients { client.join().unwrap(); }
        });
        let measured = measure(&opts, None, None).unwrap();
        server.join().unwrap();
        assert_eq!(measured.count, 3); // Four replies are still only one operation.
        assert_eq!(measured.csv(&opts).split(',').count(), 7);
        let fraction = measured.client_work_fraction();
        assert!(fraction > 0.0 && fraction <= 1.0, "work fraction {fraction}");
    }

    #[test]
    fn handshake_errors_and_reply_mismatches_fail_the_run() {
        for (response, message) in [
            (b"*0\r\n+OK\r\n+OK\r\n+PONG\r\n".as_slice(), "handshake reply 1"),
            (b"%0\r\n:1\r\n+OK\r\n+PONG\r\n", "handshake reply 2"),
            (b"%0\r\n+OK\r\n+QUEUED\r\n+PONG\r\n", "handshake reply 3"),
            (b"%0\r\n+OK\r\n+OK\r\n+OK\r\n", "handshake reply 4"),
            (b"%0\r\n-ERR unsupported\r\n", "server error: ERR unsupported"),
            (b"%1\r\n+modules\r\n*1\r\n-ERR nested\r\n", "server error: ERR nested"),
        ] {
            let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let mut opts = super::super::tests::options(listener.local_addr().unwrap().port(), "handshake");
            opts.connections = 1;
            opts.threads = 1;
            opts.requests = Some(1);
            let server = thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                socket.set_read_timeout(Some(TIMEOUT)).unwrap();
                expect_handshake(&mut socket);
                socket.write_all(response).unwrap();
                // Keep the server end alive until the client reads and fails.
                let mut byte = [0];
                assert_eq!(socket.read(&mut byte).unwrap(), 0);
            });
            let result = measure(&opts, None, None);
            server.join().unwrap();
            assert!(result.err().unwrap().to_string().contains(message));
        }
    }

    #[test]
    fn client_list_requires_nonnull_bulk_and_enough_complete_records() {
        for (body, good) in [("id=1\nid=2\n", true), ("id=1\nid=2\nid=3\n", true),
                             ("id=1\n", false), ("id=1\nid=2", false), ("id=1\n\n", false), ("", false)] {
            let wire = format!("${}\r\n{body}\r\n", body.len());
            assert_eq!(reply(wire.as_bytes(), 0).unwrap(), Some(wire.len()));
            assert_eq!(check_list(wire.as_bytes(), 2).is_ok(), good, "{body:?}");
        }
        for wire in [b"$-1\r\n".as_slice(), b"+OK\r\n", b"*0\r\n"] {
            assert!(check_list(wire, 2).is_err());
        }
    }

    #[test]
    fn client_list_sets_up_every_socket_and_retains_short_and_zero_shares() {
        for requests in [1, 3] {
            let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let mut opts = super::super::tests::options(listener.local_addr().unwrap().port(), "client-list");
            opts.requests = Some(requests);
            let server = thread::spawn(move || {
                let (mut first, _) = listener.accept().unwrap();
                first.set_read_timeout(Some(TIMEOUT)).unwrap();
                super::super::tests::expect_command(&mut first, &[b"CLIENT", b"SETINFO", b"LIB-NAME", b"firn-workload"]);
                first.write_all(b"+OK\r\n").unwrap();
                let (mut second, _) = listener.accept().unwrap();
                second.set_read_timeout(Some(TIMEOUT)).unwrap();
                super::super::tests::expect_command(&mut second, &[b"CLIENT", b"SETINFO", b"LIB-NAME", b"firn-workload"]);
                // Before the last setup completes, even the first socket has
                // not queued CLIENT LIST. peek observes without consuming bytes.
                first.set_nonblocking(true).unwrap();
                let mut byte = [0];
                assert_eq!(first.peek(&mut byte).unwrap_err().kind(), io::ErrorKind::WouldBlock);
                first.set_nonblocking(false).unwrap();
                second.write_all(b"+OK\r\n").unwrap();
                let (ready_tx, ready_rx) = std::sync::mpsc::channel();
                let shorter = thread::spawn(move || {
                    if requests == 3 {
                        super::super::tests::expect_command(&mut second, &[b"CLIENT", b"LIST"]);
                        second.write_all(b"$10\r\nid=1\nid=2\n\r\n").unwrap();
                    }
                    // While the first worker is awaiting its last reply, the
                    // second has finished. Its connection must still be open.
                    second.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
                    let mut byte = [0];
                    let error = second.read(&mut byte).unwrap_err();
                    assert!(matches!(error.kind(), io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock));
                    ready_tx.send(()).unwrap();
                });
                for index in 0..share(requests, 2, 0) {
                    super::super::tests::expect_command(&mut first, &[b"CLIENT", b"LIST"]);
                    if index + 1 == share(requests, 2, 0) { ready_rx.recv_timeout(TIMEOUT).unwrap(); }
                    first.write_all(b"$10\r\nid=1\nid=2\n\r\n").unwrap();
                }
                shorter.join().unwrap();
            });
            let measured = measure(&opts, None, None).unwrap();
            server.join().unwrap();
            assert_eq!(measured.count, requests);
        }
    }

    #[test]
    fn client_list_rejects_setup_errors_before_workers_start() {
        for response in [b":1\r\n".as_slice(), b"-ERR unsupported\r\n"] {
            let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let opts = super::super::tests::options(listener.local_addr().unwrap().port(), "client-list");
            let server = thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                socket.set_read_timeout(Some(TIMEOUT)).unwrap();
                super::super::tests::expect_command(&mut socket, &[b"CLIENT", b"SETINFO", b"LIB-NAME", b"firn-workload"]);
                socket.write_all(response).unwrap();
                let mut byte = [0];
                assert_eq!(socket.read(&mut byte).unwrap(), 0);
            });
            assert!(measure(&opts, None, None).is_err());
            server.join().unwrap();
        }
    }

    #[test]
    fn client_list_bad_replies_fail_without_a_measurement() {
        for response in [b"$-1\r\n".as_slice(), b"+OK\r\n", b"$0\r\n\r\n",
                         b"$5\r\nid=1\n\r\n", b"-ERR refused\r\n"] {
            let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let mut opts = super::super::tests::options(listener.local_addr().unwrap().port(), "client-list");
            opts.connections = 2;
            opts.threads = 1;
            opts.requests = Some(1);
            let server = thread::spawn(move || {
                let mut clients = Vec::new();
                for _ in 0..2 {
                    let (mut socket, _) = listener.accept().unwrap();
                    socket.set_read_timeout(Some(TIMEOUT)).unwrap();
                    super::super::tests::expect_command(&mut socket, &[b"CLIENT", b"SETINFO", b"LIB-NAME", b"firn-workload"]);
                    socket.write_all(b"+OK\r\n").unwrap();
                    clients.push(socket);
                }
                super::super::tests::expect_command(&mut clients[0], &[b"CLIENT", b"LIST"]);
                clients[0].write_all(response).unwrap();
                let mut byte = [0];
                assert_eq!(clients[0].read(&mut byte).unwrap(), 0);
            });
            assert!(measure(&opts, None, None).is_err());
            server.join().unwrap();
        }
    }
}
