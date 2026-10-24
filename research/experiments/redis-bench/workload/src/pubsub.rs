//! Publish/subscribe delivery: one publisher sends PUBLISH to one channel and
//! `--connections` subscribers hold it (SUBSCRIBE) or a pattern that matches it
//! (PSUBSCRIBE), RESP2 over loopback. The rate is the messages delivered to
//! subscribers per second; a message's latency runs from just before the
//! publisher writes it to its arrival at the last subscriber.
//!
//! The publisher is closed-loop in two ways: it sends `--pipeline` PUBLISH
//! commands and waits for all their replies before the next batch, as
//! redis-benchmark -P does, and it never runs more than `--window` messages
//! ahead of the slowest subscriber, so that no server's output limit for slow
//! subscribers disconnects one and the rate is a sustained one.
use super::*;
use std::sync::atomic::AtomicU64;
use std::sync::OnceLock;

const CHANNEL: &[u8] = b"bench:ch";
const PATTERN: &[u8] = b"bench:*";
// A payload starts with the message's sequence number and the time the batch
// holding it was built, both little-endian u64, then padding.
const HEADER: usize = 16;
const READ_CHUNK: usize = 65536;

/// The server answered a subscription with an unknown-command error: the
/// workload does not exist for it, which is not a failure of the measurement.
#[derive(Debug)]
pub(super) struct Unsupported(String);

impl std::fmt::Display for Unsupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "server does not support the workload: {}", self.0)
    }
}

impl std::error::Error for Unsupported {}

pub(super) fn is_pubsub(workload: &str) -> bool {
    matches!(workload, "pubsub-channel" | "pubsub-pattern")
}

fn bulk(out: &mut Vec<u8>, bytes: &[u8]) {
    write!(out, "${}\r\n", bytes.len()).unwrap();
    out.extend_from_slice(bytes);
    out.extend_from_slice(b"\r\n");
}

// The reply that confirms one subscription, the first of the connection.
fn confirmation(pattern: bool) -> Vec<u8> {
    let mut out = b"*3\r\n".to_vec();
    bulk(&mut out, if pattern { b"psubscribe" } else { b"subscribe" });
    bulk(&mut out, if pattern { PATTERN } else { CHANNEL });
    out.extend_from_slice(b":1\r\n");
    out
}

// What every delivered message looks like: all of them have one length.
struct Wire {
    // Every byte of a message before its payload: the array header, the kind,
    // the pattern when there is one, the channel and the payload's length.
    head: Vec<u8>,
    payload_len: usize,
    padding: Vec<u8>,
}

impl Wire {
    fn new(pattern: bool, payload_len: usize) -> Self {
        let mut head = Vec::new();
        if pattern {
            head.extend_from_slice(b"*4\r\n");
            bulk(&mut head, b"pmessage");
            bulk(&mut head, PATTERN);
        } else {
            head.extend_from_slice(b"*3\r\n");
            bulk(&mut head, b"message");
        }
        bulk(&mut head, CHANNEL);
        write!(head, "${payload_len}\r\n").unwrap();
        Wire { head, payload_len, padding: vec![b'x'; payload_len - HEADER] }
    }

    fn frame_len(&self) -> usize { self.head.len() + self.payload_len + 2 }

    // The length of the message at the start of `input`, or None while it is
    // incomplete. Anything but the message the publisher sends is an error,
    // also while the message is still incomplete.
    fn parse(&self, input: &[u8]) -> Result<Option<usize>> {
        let seen = input.len().min(self.head.len());
        if input[..seen] != self.head[..seen] {
            return Err(format!("unexpected pub/sub message: {:?}",
                String::from_utf8_lossy(&input[..input.len().min(80)])).into());
        }
        let frame = self.frame_len();
        if input.len() < frame { return Ok(None); }
        if &input[frame - 2..frame] != b"\r\n" {
            return Err("pub/sub message lacks its closing CRLF".into());
        }
        Ok(Some(frame))
    }

    // The sequence number and send time in a complete message.
    fn decode(&self, frame: &[u8]) -> Result<(u64, u64)> {
        let payload = &frame[self.head.len()..self.head.len() + self.payload_len];
        if payload[HEADER..] != self.padding[..] { return Err("message payload was altered".into()); }
        let seq = u64::from_le_bytes(payload[..8].try_into().unwrap());
        let sent = u64::from_le_bytes(payload[8..HEADER].try_into().unwrap());
        Ok((seq, sent))
    }
}

// Reads what the socket has into `input` through `scratch`; whether it read.
fn read_available(socket: &mut TcpStream, input: &mut Vec<u8>, scratch: &mut [u8]) -> Result<bool> {
    match socket.read(scratch) {
        Ok(0) => Err("server closed connection".into()),
        Ok(n) => { input.extend_from_slice(&scratch[..n]); Ok(true) }
        Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted) => Ok(false),
        Err(e) => Err(e.into()),
    }
}

struct Subscriber {
    socket: TcpStream,
    input: Vec<u8>,
    // Messages received so far, which is also the sequence number due next.
    count: u64,
    slot: usize,
}

// Connects, subscribes and waits for the confirmation, before the shared clock.
fn subscribe(opts: &Options, pattern: bool, slot: usize) -> Result<Subscriber> {
    let mut socket = connect(opts.port)?;
    let mut out = Vec::new();
    if pattern {
        command(&mut out, &[b"PSUBSCRIBE", PATTERN]);
    } else {
        command(&mut out, &[b"SUBSCRIBE", CHANNEL]);
    }
    socket.write_all(&out)?;
    let expected = confirmation(pattern);
    let mut input = Vec::new();
    loop {
        if input.first() == Some(&b'-') {
            if let Some(end) = input.windows(2).position(|w| w == b"\r\n") {
                let text = String::from_utf8_lossy(&input[1..end]).into_owned();
                if text.starts_with("ERR unknown command") { return Err(Unsupported(text).into()); }
                return Err(format!("subscription refused: {text}").into());
            }
        } else {
            let seen = input.len().min(expected.len());
            if input[..seen] != expected[..seen] || input.len() > expected.len() {
                return Err(format!("unexpected subscription reply: {:?}",
                    String::from_utf8_lossy(&input)).into());
            }
            if input.len() == expected.len() { break; }
        }
        read_more(&mut socket, &mut input)?;
    }
    socket.set_nonblocking(true)?;
    Ok(Subscriber { socket, input: Vec::new(), count: 0, slot })
}

struct Publisher {
    socket: TcpStream,
    input: Vec<u8>,
    out: Vec<u8>,
    sent: usize,
    // PUBLISH replies still due for the batch in `out`.
    awaiting: usize,
    // Messages built so far, which is also the next sequence number.
    next: u64,
}

// Whether a batch of `depth` more messages keeps the publisher within `window`
// messages of the slowest subscriber, which has received `slowest`.
fn has_room(next: u64, slowest: u64, depth: usize, window: usize) -> bool {
    next + depth as u64 - slowest <= window as u64
}

// Padded to a cache line, so that subscribers' counters do not share one.
#[repr(align(64))]
struct Counter(AtomicU64);

struct Shared {
    // Per subscriber, the messages it has received.
    received: Vec<Counter>,
    // What the publisher sent, final once publishing_done is set.
    published: AtomicU64,
    publishing_done: AtomicBool,
    cancelled: AtomicBool,
}

impl Shared {
    fn slowest(&self) -> u64 {
        self.received.iter().map(|c| c.0.load(Ordering::Relaxed)).min().unwrap_or(0)
    }
}

struct Context {
    wire: Wire,
    depth: usize,
    window: usize,
}

struct Accounting {
    // Busy time and all loop time. The publisher's busy time is its scans that
    // moved bytes, as for the other workloads. A subscriber thread scans many
    // sockets, most of them empty, so a scan in which any socket moved would
    // make a thread that polls 50 sockets look busy as soon as the server
    // trickles data: its busy time is the reads that returned data and the
    // parsing of what they returned.
    work: Duration,
    total: Duration,
}

fn publisher_worker(mut p: Publisher, ctx: &Context, begin: Instant, run_for: Duration,
                    shared: &Shared) -> Result<Accounting> {
    let mut scratch = vec![0; READ_CHUNK];
    let mut payload = vec![b'x'; ctx.wire.payload_len];
    let (loop_start, mut work) = (Instant::now(), Duration::ZERO);
    let mut slowest = 0;
    loop {
        if shared.cancelled.load(Ordering::Relaxed) { return Err("another worker failed".into()); }
        let scan_start = Instant::now();
        let mut moved = read_available(&mut p.socket, &mut p.input, &mut scratch)?;
        while p.awaiting > 0 {
            let Some(size) = reply(&p.input, 0)? else { break; };
            if p.input[0] != b':' {
                return Err(format!("PUBLISH replied {:?}", String::from_utf8_lossy(&p.input[..size])).into());
            }
            p.input.drain(..size);
            p.awaiting -= 1;
        }
        if p.awaiting == 0 && p.sent == p.out.len() {
            if !p.input.is_empty() { return Err("extra bytes on the publisher's connection".into()); }
            if begin.elapsed() >= run_for {
                shared.published.store(p.next, Ordering::Relaxed);
                shared.publishing_done.store(true, Ordering::Release);
                break;
            }
            if !has_room(p.next, slowest, ctx.depth, ctx.window) { slowest = shared.slowest(); }
            if has_room(p.next, slowest, ctx.depth, ctx.window) {
                let sent = begin.elapsed().as_nanos() as u64;
                p.out.clear();
                p.sent = 0;
                for i in 0..ctx.depth as u64 {
                    payload[..8].copy_from_slice(&(p.next + i).to_le_bytes());
                    payload[8..HEADER].copy_from_slice(&sent.to_le_bytes());
                    command(&mut p.out, &[b"PUBLISH", CHANNEL, &payload]);
                }
                p.next += ctx.depth as u64;
                p.awaiting = ctx.depth;
            }
        }
        if p.sent < p.out.len() {
            match p.socket.write(&p.out[p.sent..]) {
                Ok(0) => return Err("zero-byte socket write".into()),
                Ok(n) => { p.sent += n; moved = true; }
                Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted) => (),
                Err(e) => return Err(e.into()),
            }
        }
        if moved { work += scan_start.elapsed(); }
        thread::yield_now();
    }
    Ok(Accounting { work, total: loop_start.elapsed() })
}

struct SubscriberResult {
    // Per sequence number, the longest delay from send to receipt among this
    // thread's subscribers, in microseconds, zero past the last message.
    latencies: Vec<u32>,
    // The time, since the run began, of this thread's last message.
    last_message: u64,
    accounting: Accounting,
}

fn subscriber_worker(mut subscribers: Vec<Subscriber>, ctx: &Context, begin: Instant,
                     shared: &Shared) -> Result<SubscriberResult> {
    let mut scratch = vec![0; READ_CHUNK];
    let mut latencies: Vec<u32> = Vec::new();
    let (loop_start, mut work) = (Instant::now(), Duration::ZERO);
    let (mut last_message, mut last_progress) = (0, Instant::now());
    loop {
        if shared.cancelled.load(Ordering::Relaxed) { return Err("another worker failed".into()); }
        let mut moved = false;
        for s in &mut subscribers {
            let read_start = Instant::now();
            if !read_available(&mut s.socket, &mut s.input, &mut scratch)? { continue; }
            moved = true;
            // Messages that arrive in one read share the time it returned.
            let now = begin.elapsed().as_nanos() as u64;
            let before = s.count;
            let mut pos = 0;
            while let Some(frame) = ctx.wire.parse(&s.input[pos..])? {
                let (seq, sent) = ctx.wire.decode(&s.input[pos..pos + frame])?;
                if seq != s.count {
                    return Err(format!("subscriber {} received message {seq} where {} was due",
                        s.slot, s.count).into());
                }
                let us = u32::try_from(now.saturating_sub(sent).div_ceil(1000)).unwrap_or(u32::MAX).max(1);
                let at = seq as usize;
                if at >= latencies.len() { latencies.resize((at + 1).next_power_of_two().max(1024), 0); }
                latencies[at] = latencies[at].max(us);
                s.count += 1;
                pos += frame;
            }
            s.input.drain(..pos);
            if s.count != before {
                shared.received[s.slot].0.store(s.count, Ordering::Relaxed);
                last_message = now;
            }
            work += read_start.elapsed();
        }
        if moved { last_progress = Instant::now(); }
        if shared.publishing_done.load(Ordering::Acquire) {
            let published = shared.published.load(Ordering::Relaxed);
            if subscribers.iter().any(|s| s.count > published) {
                return Err("a subscriber received more messages than were published".into());
            }
            if subscribers.iter().all(|s| s.count == published) { break; }
        }
        if last_progress.elapsed() > TIMEOUT { return Err("no message for 30 seconds".into()); }
        thread::yield_now();
    }
    Ok(SubscriberResult { latencies, last_message,
        accounting: Accounting { work, total: loop_start.elapsed() } })
}

struct Measured {
    published: u64,
    delivered: u64,
    seconds: f64,
    p50: f64,
    p99: f64,
    // The larger of the publisher's and the subscribers' work fractions: either
    // side at its limit makes the client the limit.
    work_fraction: f64,
}

impl Measured {
    // workload,connections,pipeline,published,delivered,seconds,rate,p50_ms,p99_ms
    fn csv(&self, opts: &Options) -> String {
        format!("{},{},{},{},{},{:.6},{:.3},{:.3},{:.3}", opts.workload, opts.connections,
            opts.pipeline, self.published, self.delivered, self.seconds,
            self.delivered as f64 / self.seconds, self.p50, self.p99)
    }
}

// One run of `run_for`: subscribe, publish until the time is up, then wait
// until every subscriber has every message. The span counts from the first
// message to the last arrival; setup and the merging of results are outside it.
fn measure(opts: &Options, run_for: Duration) -> Result<Measured> {
    let pattern = opts.workload == "pubsub-pattern";
    let mut subscribers = Vec::new();
    for slot in 0..opts.connections { subscribers.push(subscribe(opts, pattern, slot)?); }
    let publisher = Publisher { socket: connect(opts.port)?, input: Vec::new(), out: Vec::new(),
        sent: 0, awaiting: 0, next: 0 };
    publisher.socket.set_nonblocking(true)?;
    // The publisher has a thread of its own; the other client threads share
    // the subscribers.
    let threads = opts.threads.saturating_sub(1).max(1).min(opts.connections);
    let mut groups: Vec<Vec<Subscriber>> = (0..threads).map(|_| Vec::new()).collect();
    for s in subscribers { groups[s.slot % threads].push(s); }
    let shared = Arc::new(Shared {
        received: (0..opts.connections).map(|_| Counter(AtomicU64::new(0))).collect(),
        published: AtomicU64::new(0),
        publishing_done: AtomicBool::new(false),
        cancelled: AtomicBool::new(false),
    });
    let ctx = Arc::new(Context { wire: Wire::new(pattern, opts.value.len()),
        depth: opts.pipeline, window: opts.window });
    let barrier = Arc::new(Barrier::new(threads + 2));
    let start = Arc::new(OnceLock::new());
    let mut handles = Vec::new();
    for group in groups {
        let (ctx, shared, barrier, start) = (ctx.clone(), shared.clone(), barrier.clone(), start.clone());
        handles.push(thread::spawn(move || {
            barrier.wait();
            let result = subscriber_worker(group, &ctx, *start.get().unwrap(), &shared);
            if result.is_err() { shared.cancelled.store(true, Ordering::Relaxed); }
            result
        }));
    }
    let publishing = {
        let (ctx, shared, barrier, start) = (ctx.clone(), shared.clone(), barrier.clone(), start.clone());
        thread::spawn(move || {
            barrier.wait();
            let result = publisher_worker(publisher, &ctx, *start.get().unwrap(), run_for, &shared);
            if result.is_err() { shared.cancelled.store(true, Ordering::Relaxed); }
            result
        })
    };
    start.set(Instant::now()).unwrap();
    barrier.wait();
    // A worker that fails sets `cancelled`, which the others report as their
    // own failure; the first error that is not that echo is the cause.
    let mut failure: Option<Box<dyn std::error::Error + Send + Sync>> = None;
    let mut keep = |e: Box<dyn std::error::Error + Send + Sync>| {
        if failure.is_none() || failure.as_ref().is_some_and(|f| f.to_string() == "another worker failed") {
            failure = Some(e);
        }
    };
    let publisher_accounting = match publishing.join().unwrap_or_else(|_| Err("worker panicked".into())) {
        Ok(a) => Some(a),
        Err(e) => { keep(e); None }
    };
    let mut results = Vec::new();
    for handle in handles {
        match handle.join().unwrap_or_else(|_| Err("worker panicked".into())) {
            Ok(r) => results.push(r),
            Err(e) => keep(e),
        }
    }
    if let Some(e) = failure { return Err(e); }
    let publisher_accounting = publisher_accounting.unwrap();
    let published = shared.published.load(Ordering::Relaxed);
    if published == 0 { return Err("no message was published".into()); }
    let mut histogram = BTreeMap::new();
    for seq in 0..published as usize {
        let mut slowest = 0;
        for r in &results { slowest = slowest.max(r.latencies.get(seq).copied().unwrap_or(0)); }
        if slowest == 0 { return Err(format!("message {seq} has no recorded arrival").into()); }
        *histogram.entry(u64::from(slowest)).or_insert(0) += 1;
    }
    let end = results.iter().map(|r| r.last_message).max().unwrap();
    let subscribers_work = results.iter().map(|r| r.accounting.work).sum::<Duration>();
    let subscribers_total = results.iter().map(|r| r.accounting.total).sum::<Duration>();
    Ok(Measured {
        published,
        delivered: published * opts.connections as u64,
        seconds: end as f64 / 1e9,
        p50: percentile(&histogram, published, 50),
        p99: percentile(&histogram, published, 99),
        work_fraction: work_fraction(subscribers_work, subscribers_total)
            .max(work_fraction(publisher_accounting.work, publisher_accounting.total)),
    })
}

pub(super) fn run(opts: &Options) -> Result<()> {
    let seconds = opts.seconds.ok_or("pub/sub workloads take --seconds")?;
    if !opts.warmup.is_zero() { measure(opts, opts.warmup)?; }
    let measured = measure(opts, seconds)?;
    if let Some(path) = &opts.work_file {
        std::fs::write(path, format!("{:.4}\n", measured.work_fraction))?;
    }
    println!("{}", measured.csv(opts));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    // The payload length every test publishes.
    const PAYLOAD: usize = 24;

    #[derive(Clone, Copy, PartialEq)]
    enum Fault { None, Unsupported, SkipFifth, Slow }

    fn message(wire: &Wire, payload: &[u8]) -> Vec<u8> {
        let mut frame = wire.head.clone();
        frame.extend_from_slice(payload);
        frame.extend_from_slice(b"\r\n");
        frame
    }

    // A server with the subscriber behaviour the client relies on: it
    // confirms each subscription, then answers every PUBLISH with the
    // subscriber count after sending the message to each subscriber.
    fn serve(listener: TcpListener, subscribers: usize, pattern: bool, fault: Fault) -> thread::JoinHandle<()> {
        thread::spawn(move || {
            let wire = Wire::new(pattern, PAYLOAD);
            let mut held = Vec::new();
            for _ in 0..subscribers {
                let (mut socket, _) = listener.accept().unwrap();
                let mut expected = Vec::new();
                if pattern {
                    command(&mut expected, &[b"PSUBSCRIBE", PATTERN]);
                } else {
                    command(&mut expected, &[b"SUBSCRIBE", CHANNEL]);
                }
                let mut got = vec![0; expected.len()];
                socket.read_exact(&mut got).unwrap();
                assert_eq!(got, expected);
                if fault == Fault::Unsupported {
                    socket.write_all(b"-ERR unknown command 'SUBSCRIBE', with args beginning with: 'bench:ch' \r\n").unwrap();
                    return;
                }
                socket.write_all(&confirmation(pattern)).unwrap();
                held.push(socket);
            }
            let (mut publisher, _) = listener.accept().unwrap();
            let mut prefix = b"*3\r\n".to_vec();
            bulk(&mut prefix, b"PUBLISH");
            bulk(&mut prefix, CHANNEL);
            write!(prefix, "${PAYLOAD}\r\n").unwrap();
            let length = prefix.len() + PAYLOAD + 2;
            let (mut buffer, mut chunk, mut number) = (Vec::new(), [0; 4096], 0);
            loop {
                match publisher.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => buffer.extend_from_slice(&chunk[..n]),
                }
                while buffer.len() >= length {
                    assert!(buffer.starts_with(&prefix));
                    let frame = message(&wire, &buffer[prefix.len()..prefix.len() + PAYLOAD]);
                    buffer.drain(..length);
                    for (i, socket) in held.iter_mut().enumerate() {
                        if fault == Fault::SkipFifth && i == 0 && number == 5 { continue; }
                        // A client that has failed has closed its sockets.
                        let _ = socket.write_all(&frame);
                    }
                    number += 1;
                    if fault == Fault::Slow { thread::sleep(Duration::from_millis(5)); }
                    let _ = publisher.write_all(format!(":{}\r\n", held.len()).as_bytes());
                }
            }
            // A server does not close its subscribers when the publisher
            // goes: wait for the client to close them, or a subscriber that
            // has yet to read the last messages could meet the close first.
            for socket in &mut held {
                socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut sink = [0; 64];
                while matches!(socket.read(&mut sink), Ok(n) if n > 0) {}
            }
        })
    }

    fn options(port: u16, pattern: bool, subscribers: usize, pipeline: usize) -> Options {
        let mut opts = super::super::tests::options(port, if pattern { "pubsub-pattern" } else { "pubsub-channel" });
        opts.connections = subscribers;
        opts.threads = 3;
        opts.requests = None;
        opts.seconds = Some(Duration::from_millis(300));
        opts.value = vec![b'x'; PAYLOAD];
        opts.pipeline = pipeline;
        opts.window = 8;
        opts
    }

    fn run_against(fault: Fault, pattern: bool, pipeline: usize) -> Result<Measured> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let opts = options(listener.local_addr().unwrap().port(), pattern, 3, pipeline);
        let server = serve(listener, 3, pattern, fault);
        let result = measure(&opts, opts.seconds.unwrap());
        server.join().unwrap();
        result
    }

    #[test]
    fn every_subscriber_receives_every_message() {
        for pattern in [false, true] {
            for pipeline in [1, 4] {
                let measured = run_against(Fault::None, pattern, pipeline).unwrap();
                assert!(measured.published > 0);
                assert_eq!(measured.delivered, measured.published * 3);
                assert!(measured.seconds > 0.0 && measured.p50 > 0.0 && measured.p99 >= measured.p50);
                assert!((0.0..=1.0).contains(&measured.work_fraction));
            }
        }
    }

    #[test]
    fn an_idle_client_reports_a_low_work_fraction() {
        // A server that answers one PUBLISH in 5 ms leaves the client polling
        // almost all the time.
        let measured = run_against(Fault::Slow, false, 1).unwrap();
        assert!(measured.published > 10, "{}", measured.published);
        assert!(measured.work_fraction < 0.5, "{}", measured.work_fraction);
    }

    #[test]
    fn a_missing_message_fails_the_run() {
        let error = run_against(Fault::SkipFifth, false, 1).err().unwrap();
        assert!(error.to_string().contains("received message 6 where 5 was due"), "{error}");
    }

    #[test]
    fn unknown_subscribe_is_reported_as_unsupported() {
        let error = run_against(Fault::Unsupported, false, 1).err().unwrap();
        assert!(error.downcast_ref::<Unsupported>().is_some(), "{error}");
    }

    #[test]
    fn other_subscription_errors_fail() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let opts = options(listener.local_addr().unwrap().port(), false, 1, 1);
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut got = [0; 64];
            let _ = socket.read(&mut got).unwrap();
            socket.write_all(b"-NOPERM no permission\r\n").unwrap();
        });
        let error = subscribe(&opts, false, 0).err().unwrap();
        server.join().unwrap();
        assert!(error.downcast_ref::<Unsupported>().is_none());
        assert!(error.to_string().contains("NOPERM"), "{error}");
    }

    #[test]
    fn messages_parse_when_fragmented_and_nothing_else_does() {
        for pattern in [false, true] {
            let wire = Wire::new(pattern, PAYLOAD);
            let mut payload = vec![b'x'; PAYLOAD];
            payload[..8].copy_from_slice(&7u64.to_le_bytes());
            payload[8..HEADER].copy_from_slice(&9u64.to_le_bytes());
            let frame = message(&wire, &payload);
            for n in 0..frame.len() { assert_eq!(wire.parse(&frame[..n]).unwrap(), None, "{n}"); }
            assert_eq!(wire.parse(&frame).unwrap(), Some(frame.len()));
            // A second message after the first is not part of it.
            let two = [frame.clone(), frame.clone()].concat();
            assert_eq!(wire.parse(&two).unwrap(), Some(frame.len()));
            assert_eq!(wire.decode(&frame).unwrap(), (7, 9));
            // Another channel, another kind, another length, no closing CRLF
            // and an altered payload are all errors.
            let mut other_channel = frame.clone();
            let at = other_channel.windows(8).position(|w| w == CHANNEL).unwrap();
            other_channel[at + 7] = b'x';
            let other_kind = if pattern { message(&Wire::new(false, PAYLOAD), &payload) }
                else { message(&Wire::new(true, PAYLOAD), &payload) };
            let other_length = message(&Wire::new(pattern, PAYLOAD + 1), &[payload.clone(), vec![b'x']].concat());
            let mut no_crlf = frame.clone();
            *no_crlf.last_mut().unwrap() = b'!';
            for bad in [other_channel, other_kind, other_length, no_crlf] {
                assert!(wire.parse(&bad).is_err(), "{:?}", String::from_utf8_lossy(&bad));
            }
            let mut altered = payload.clone();
            altered[HEADER] = b'y';
            assert!(wire.decode(&message(&wire, &altered)).is_err());
        }
    }

    #[test]
    fn the_publisher_stays_within_the_window() {
        // 6 sent, the slowest subscriber has 4: a batch of 2 reaches 4 ahead.
        assert!(has_room(6, 4, 2, 4));
        assert!(!has_room(6, 4, 3, 4));
        assert!(has_room(0, 0, 16, 16));
        assert!(!has_room(1, 0, 16, 16));
    }
}
