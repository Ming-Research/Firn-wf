//! Collection scans: SSCAN, HSCAN or ZSCAN of one collection of MEMBERS members,
//! followed from cursor 0 to cursor 0 with COUNT 100, over loopback RESP2. Each
//! connection keeps `--pipeline` cursor chains in flight: it sends one scan
//! command for every chain, waits for all their replies, as redis-benchmark -P
//! does, and sends the next batch. The rate is the members the replies carry per
//! second; the latency is one cursor call's, from the write of its batch to the
//! read of its reply.
//!
//! The collection does not change while the clients run, so every chain, from
//! cursor 0 to cursor 0, must receive each of the MEMBERS members exactly once;
//! a chain that does not fails the run instead of being counted.
use super::*;

pub(super) const MEMBERS: usize = 10_000;
const COUNT: &[u8] = b"100";
// Members per SADD/HSET/ZADD while the collection is filled.
const BATCH: usize = 500;

/// The server answered a command of the workload with an unknown-command error:
/// the workload does not exist for it, which is not a failure of the measurement.
#[derive(Debug)]
pub(super) struct Unsupported(String);

impl std::fmt::Display for Unsupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "server does not support the workload: {}", self.0)
    }
}

impl std::error::Error for Unsupported {}

pub(super) fn is_scan(workload: &str) -> bool {
    matches!(workload, "scan-set" | "scan-hash" | "scan-zset")
}

// The scan command, the key it scans and how many reply elements a member takes.
fn kind(workload: &str) -> (&'static [u8], &'static [u8], usize) {
    match workload {
        "scan-set" => (b"SSCAN", b"bench:scan:set", 1),
        "scan-hash" => (b"HSCAN", b"bench:scan:hash", 2),
        _ => (b"ZSCAN", b"bench:scan:zset", 2),
    }
}

fn name(index: usize) -> Vec<u8> { format!("m{index:05}").into_bytes() }

// The reply to a command during setup, read whole; an error stops the setup, and
// an unknown-command error says the server lacks the workload.
fn setup_reply(socket: &mut TcpStream, input: &mut Vec<u8>) -> Result<()> {
    loop {
        if input.first() == Some(&b'-') {
            if let Some(end) = input.windows(2).position(|w| w == b"\r\n") {
                let text = String::from_utf8_lossy(&input[1..end]).into_owned();
                if text.starts_with("ERR unknown command") { return Err(Unsupported(text).into()); }
                return Err(format!("server error: {text}").into());
            }
        } else if let Some(size) = reply(input, 0)? {
            if input[0] != b':' { return Err("setup command did not return an integer".into()); }
            input.drain(..size);
            return Ok(());
        }
        read_more(socket, input)?;
    }
}

// DEL, then MEMBERS members in commands of BATCH: a set of m00000..., a hash of
// them with the value v, a sorted set of them with their numbers as scores.
fn fill(opts: &Options) -> Result<()> {
    let (_, key, _) = kind(&opts.workload);
    let mut socket = connect(opts.port)?;
    let mut input = Vec::new();
    let mut out = Vec::new();
    command(&mut out, &[b"DEL", key]);
    socket.write_all(&out)?;
    setup_reply(&mut socket, &mut input)?;
    for first in (0..MEMBERS).step_by(BATCH) {
        out.clear();
        let mut args: Vec<Vec<u8>> = vec![
            match opts.workload.as_str() { "scan-set" => b"SADD".to_vec(), "scan-hash" => b"HSET".to_vec(), _ => b"ZADD".to_vec() },
            key.to_vec(),
        ];
        for index in first..(first + BATCH).min(MEMBERS) {
            match opts.workload.as_str() {
                "scan-set" => args.push(name(index)),
                "scan-hash" => { args.push(name(index)); args.push(b"v".to_vec()); }
                _ => { args.push(index.to_string().into_bytes()); args.push(name(index)); }
            }
        }
        let refs: Vec<&[u8]> = args.iter().map(Vec::as_slice).collect();
        command(&mut out, &refs);
        socket.write_all(&out)?;
        setup_reply(&mut socket, &mut input)?;
    }
    Ok(())
}

// One cursor chain: the cursor to send next and which members it has received.
struct Chain {
    cursor: Vec<u8>,
    seen: Vec<u64>,
    received: usize,
}

impl Chain {
    fn new() -> Self { Self { cursor: b"0".to_vec(), seen: vec![0; MEMBERS.div_ceil(64)], received: 0 } }

    fn restart(&mut self) {
        self.cursor.clear();
        self.cursor.push(b'0');
        self.seen.iter_mut().for_each(|word| *word = 0);
        self.received = 0;
    }

    // Whether every member has come exactly once.
    fn complete(&self) -> bool {
        self.received == MEMBERS && self.seen.iter().map(|word| word.count_ones() as usize).sum::<usize>() == MEMBERS
    }
}

// One bulk string at the front of `input`: its bytes and the length of the whole.
fn bulk_at(input: &[u8]) -> Result<(&[u8], usize)> {
    if input.first() != Some(&b'$') { return Err("a scan reply element is not a bulk string".into()); }
    let end = input.windows(2).position(|w| w == b"\r\n").ok_or("truncated bulk header")?;
    let length: usize = std::str::from_utf8(&input[1..end])?.parse()?;
    let body = end + 2;
    let total = body + length + 2;
    if input.len() < total { return Err("truncated bulk string".into()); }
    Ok((&input[body..body + length], total))
}

// Reads one complete scan reply, `*2`, the cursor, the array of elements, into
// the chain: the members it carries are marked, a repeat or a name outside the
// collection is an error, and the cursor is kept. Returns the members it carried
// and whether the chain is at its end.
fn take(reply: &[u8], width: usize, chain: &mut Chain) -> Result<(usize, bool)> {
    let header = b"*2\r\n";
    if !reply.starts_with(header) { return Err("a scan reply is not a two-element array".into()); }
    let (cursor, used) = bulk_at(&reply[header.len()..])?;
    let cursor = cursor.to_vec();
    let mut at = header.len() + used;
    let end = at + reply[at..].windows(2).position(|w| w == b"\r\n").ok_or("truncated element count")?;
    if reply[at] != b'*' { return Err("a scan reply's elements are not an array".into()); }
    let elements: usize = std::str::from_utf8(&reply[at + 1..end])?.parse()?;
    if elements % width != 0 { return Err("a scan reply splits a member from its value".into()); }
    at = end + 2;
    for _ in 0..elements / width {
        let (member, used) = bulk_at(&reply[at..])?;
        at += used;
        for _ in 1..width { let (_, skipped) = bulk_at(&reply[at..])?; at += skipped; }
        let index = member.strip_prefix(b"m").and_then(|digits| std::str::from_utf8(digits).ok())
            .and_then(|digits| digits.parse::<usize>().ok()).filter(|&index| index < MEMBERS && member.len() == 6)
            .ok_or_else(|| format!("a scan returned {:?}, which was never filled", String::from_utf8_lossy(member)))?;
        let (word, bit) = (index / 64, 1u64 << (index % 64));
        if chain.seen[word] & bit != 0 {
            return Err(format!("a scan of an unchanged collection returned {} twice", String::from_utf8_lossy(member)).into());
        }
        chain.seen[word] |= bit;
        chain.received += 1;
    }
    if at != reply.len() { return Err("a scan reply has bytes after its elements".into()); }
    let done = cursor == b"0";
    chain.cursor = cursor;
    Ok((elements / width, done))
}

struct Connection {
    socket: TcpStream,
    chains: Vec<Chain>,
    out: Vec<u8>,
    sent: usize,
    input: Vec<u8>,
    // Replies of the batch still to come: the chain of the next is chains.len() - pending.
    pending: usize,
    started: Instant,
    moved: bool,
}

impl Connection {
    fn new(opts: &Options) -> Result<Self> {
        let socket = connect(opts.port)?;
        socket.set_nonblocking(true)?;
        Ok(Self { socket, chains: (0..opts.pipeline).map(|_| Chain::new()).collect(), out: Vec::new(),
            sent: 0, input: Vec::new(), pending: 0, started: Instant::now(), moved: false })
    }

    // The next batch: one scan command for every chain, at its cursor.
    fn batch(&mut self, opts: &Options) {
        let (scan, key, _) = kind(&opts.workload);
        self.out.clear();
        for chain in &self.chains { command(&mut self.out, &[scan, key, &chain.cursor, b"COUNT", COUNT]); }
        self.sent = 0;
        self.pending = self.chains.len();
        self.started = Instant::now();
    }

    // Writes and reads what the socket takes and hands each completed reply to
    // its chain. Returns the members those replies carried, and appends to
    // `calls` the latency in microseconds of each of them.
    fn advance(&mut self, width: usize, calls: &mut Vec<u64>) -> Result<usize> {
        if self.started.elapsed() > TIMEOUT { return Err("scan batch exceeded 30 seconds".into()); }
        self.moved = false;
        if self.sent < self.out.len() {
            match self.socket.write(&self.out[self.sent..]) {
                Ok(0) => return Err("zero-byte socket write".into()),
                Ok(n) => { self.sent += n; self.moved = true; }
                Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted) => (),
                Err(e) => return Err(e.into()),
            }
        }
        match read_more(&mut self.socket, &mut self.input) {
            Ok(_) => self.moved = true,
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted) => (),
            Err(e) => return Err(e.into()),
        }
        let mut members = 0;
        while self.pending > 0 {
            let Some(size) = reply(&self.input, 0)? else { break; };
            let slot = self.chains.len() - self.pending;
            let (carried, done) = take(&self.input[..size], width, &mut self.chains[slot])?;
            members += carried;
            calls.push(self.started.elapsed().as_nanos().div_ceil(1000) as u64);
            if done {
                if !self.chains[slot].complete() {
                    return Err(format!("a scan chain ended with {} of the {MEMBERS} members", self.chains[slot].received).into());
                }
                self.chains[slot].restart();
            }
            self.input.drain(..size);
            self.pending -= 1;
        }
        Ok(members)
    }
}

struct Worked {
    histogram: Histogram,
    calls: u64,
    members: u64,
    end: Instant,
    work: Duration,
    total: Duration,
}

fn worker(mut connections: Vec<Connection>, opts: &Options, start: Instant, cancelled: &AtomicBool) -> Result<Worked> {
    let (_, _, width) = kind(&opts.workload);
    let seconds = opts.seconds.ok_or("scan workloads take --seconds")?;
    let mut done = Worked { histogram: Histogram::new(), calls: 0, members: 0, end: start,
        work: Duration::ZERO, total: Duration::ZERO };
    let loop_start = Instant::now();
    let mut latencies = Vec::new();
    loop {
        if cancelled.load(Ordering::Relaxed) { return Err("another worker failed".into()); }
        let scan_start = Instant::now();
        let (mut active, mut moved) = (false, false);
        for c in &mut connections {
            if c.pending == 0 {
                if start.elapsed() >= seconds { continue; }
                c.batch(opts);
            }
            active = true;
            latencies.clear();
            let members = c.advance(width, &mut latencies)?;
            moved |= c.moved;
            if !latencies.is_empty() {
                moved = true;
                done.calls += latencies.len() as u64;
                done.members += members as u64;
                done.end = Instant::now();
                for &us in &latencies { *done.histogram.entry(us).or_insert(0) += 1; }
            }
        }
        if moved { done.work += scan_start.elapsed(); }
        if !active { break; }
        thread::yield_now();
    }
    done.total = loop_start.elapsed();
    Ok(done)
}

struct Measured {
    calls: u64,
    members: u64,
    seconds: f64,
    p50: f64,
    p99: f64,
    work_fraction: f64,
}

impl Measured {
    // workload,connections,pipeline,calls,members,seconds,rate,p50_ms,p99_ms
    fn csv(&self, opts: &Options) -> String {
        format!("{},{},{},{},{},{:.6},{:.3},{:.3},{:.3}", opts.workload, opts.connections, opts.pipeline,
            self.calls, self.members, self.seconds, self.members as f64 / self.seconds, self.p50, self.p99)
    }
}

fn measure(opts: &Options, run_for: Duration) -> Result<Measured> {
    let mut timed = opts.clone();
    timed.seconds = Some(run_for);
    let threads = opts.threads.min(opts.connections);
    let mut groups: Vec<Vec<Connection>> = (0..threads).map(|_| Vec::new()).collect();
    for id in 0..opts.connections { groups[id % threads].push(Connection::new(opts)?); }
    let timed = Arc::new(timed);
    let barrier = Arc::new(Barrier::new(threads + 1));
    let cancelled = Arc::new(AtomicBool::new(false));
    let start = Arc::new(std::sync::OnceLock::new());
    let mut handles = Vec::new();
    for group in groups {
        let (timed, barrier, cancelled, start) = (timed.clone(), barrier.clone(), cancelled.clone(), start.clone());
        handles.push(thread::spawn(move || {
            barrier.wait();
            let result = worker(group, &timed, *start.get().unwrap(), &cancelled);
            if result.is_err() { cancelled.store(true, Ordering::Relaxed); }
            result
        }));
    }
    let begin = Instant::now();
    start.set(begin).unwrap();
    barrier.wait();
    let mut failure: Option<Box<dyn std::error::Error + Send + Sync>> = None;
    let mut histogram = BTreeMap::new();
    let (mut calls, mut members, mut end) = (0, 0, begin);
    let (mut work, mut total) = (Duration::ZERO, Duration::ZERO);
    for handle in handles {
        match handle.join().unwrap_or_else(|_| Err("worker panicked".into())) {
            Ok(done) => {
                calls += done.calls;
                members += done.members;
                end = end.max(done.end);
                work += done.work;
                total += done.total;
                for (us, n) in done.histogram { *histogram.entry(us).or_insert(0) += n; }
            }
            // A worker that fails sets `cancelled`, which the others report as
            // their own failure; the first error that is not that echo is the cause.
            Err(e) => {
                if failure.is_none() || failure.as_ref().is_some_and(|f| f.to_string() == "another worker failed") {
                    failure = Some(e);
                }
            }
        }
    }
    if let Some(e) = failure { return Err(e); }
    if calls == 0 { return Err("no scan call completed".into()); }
    Ok(Measured { calls, members, seconds: end.duration_since(begin).as_secs_f64(),
        p50: percentile(&histogram, calls, 50), p99: percentile(&histogram, calls, 99),
        work_fraction: work_fraction(work, total) })
}

pub(super) fn run(opts: &Options) -> Result<()> {
    let seconds = opts.seconds.ok_or("scan workloads take --seconds")?;
    fill(opts)?;
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

    #[derive(Clone, Copy, PartialEq)]
    enum Fault { None, Unsupported, Missing, Repeat }

    // One command of the client, read whole from the socket.
    fn read_command(socket: &mut TcpStream, input: &mut Vec<u8>) -> Option<Vec<Vec<u8>>> {
        loop {
            if let Some(size) = reply(input, 0).ok().flatten().filter(|_| input.first() == Some(&b'*')) {
                let mut args = Vec::new();
                let mut rest = &input[..size];
                let end = rest.windows(2).position(|w| w == b"\r\n").unwrap();
                let count: usize = std::str::from_utf8(&rest[1..end]).unwrap().parse().unwrap();
                rest = &rest[end + 2..];
                for _ in 0..count {
                    let (body, used) = bulk_at(rest).unwrap();
                    args.push(body.to_vec());
                    rest = &rest[used..];
                }
                input.drain(..size);
                return Some(args);
            }
            if read_more(socket, input).is_err() { return None; }
        }
    }

    // A server holding one collection of MEMBERS members that answers the scan
    // in pages of `page` members, the cursor being the index of the next one.
    fn serve(listener: TcpListener, workload: &'static str, page: usize, fault: Fault) -> thread::JoinHandle<()> {
        thread::spawn(move || {
            let mut clients = Vec::new();
            let width = kind(workload).2;
            // Accept clients until the test is over.
            listener.set_nonblocking(true).unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            while Instant::now() < deadline {
                match listener.accept() {
                    Ok((socket, _)) => {
                        socket.set_nonblocking(false).unwrap();
                        clients.push(thread::spawn(move || {
                            let mut socket = socket;
                            socket.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                            let mut input = Vec::new();
                            while let Some(args) = read_command(&mut socket, &mut input) {
                                let mut out = Vec::new();
                                let verb = args[0].to_ascii_uppercase();
                                if fault == Fault::Unsupported {
                                    out.extend_from_slice(b"-ERR unknown command 'SADD', with args beginning with:\r\n");
                                } else if verb == b"DEL" || verb == b"SADD" || verb == b"HSET" || verb == b"ZADD" {
                                    out.extend_from_slice(b":1\r\n");
                                } else {
                                    let from: usize = std::str::from_utf8(&args[2]).unwrap().parse().unwrap();
                                    let mut to = (from + page).min(MEMBERS);
                                    let mut indexes: Vec<usize> = (from..to).collect();
                                    if fault == Fault::Missing { indexes.retain(|&index| index != 5); }
                                    if fault == Fault::Repeat && from > 0 { indexes.push(from - 1); }
                                    if to == MEMBERS { to = 0; }
                                    let cursor = to.to_string();
                                    out.extend_from_slice(format!("*2\r\n${}\r\n{cursor}\r\n*{}\r\n", cursor.len(), indexes.len() * width).as_bytes());
                                    for index in indexes {
                                        let member = name(index);
                                        out.extend_from_slice(format!("${}\r\n", member.len()).as_bytes());
                                        out.extend_from_slice(&member);
                                        out.extend_from_slice(b"\r\n");
                                        if width == 2 { out.extend_from_slice(b"$1\r\nv\r\n"); }
                                    }
                                }
                                if socket.write_all(&out).is_err() { break; }
                            }
                        }));
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(5)),
                    Err(_) => break,
                }
            }
            for client in clients { let _ = client.join(); }
        })
    }

    fn options(port: u16, workload: &str, pipeline: usize) -> Options {
        let mut opts = super::super::tests::options(port, workload);
        opts.connections = 3;
        opts.threads = 2;
        opts.requests = None;
        opts.seconds = Some(Duration::from_millis(300));
        opts.pipeline = pipeline;
        opts
    }

    fn run_against(workload: &'static str, fault: Fault, pipeline: usize) -> Result<Measured> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let opts = options(listener.local_addr().unwrap().port(), workload, pipeline);
        let server = serve(listener, workload, 100, fault);
        let result = fill(&opts).and_then(|_| measure(&opts, opts.seconds.unwrap()));
        drop(server);
        result
    }

    #[test]
    fn every_chain_receives_every_member_once() {
        for workload in ["scan-set", "scan-hash", "scan-zset"] {
            for pipeline in [1, 16] {
                let measured = run_against(workload, Fault::None, pipeline).unwrap();
                // 100 members a call: a call carries 100 members, so the rate
                // counts members, not calls.
                assert_eq!(measured.members, measured.calls * 100, "{workload} at {pipeline}");
                assert!(measured.calls >= 100, "{workload} at {pipeline}: at least one whole chain");
            }
        }
    }

    #[test]
    fn a_missing_member_fails_the_run() {
        let error = run_against("scan-set", Fault::Missing, 1).err().unwrap();
        assert!(error.to_string().contains("ended with 9999 of the 10000 members"), "{error}");
    }

    #[test]
    fn a_repeated_member_fails_the_run() {
        let error = run_against("scan-hash", Fault::Repeat, 2).err().unwrap();
        assert!(error.to_string().contains("twice"), "{error}");
    }

    #[test]
    fn unknown_commands_are_reported_as_unsupported() {
        let error = run_against("scan-zset", Fault::Unsupported, 1).err().unwrap();
        assert!(error.downcast_ref::<Unsupported>().is_some(), "{error}");
    }

    #[test]
    fn replies_parse_when_fragmented_and_nothing_else_does() {
        let mut chain = Chain::new();
        let reply = b"*2\r\n$3\r\n100\r\n*4\r\n$6\r\nm00001\r\n$1\r\nv\r\n$6\r\nm00002\r\n$1\r\nv\r\n";
        assert_eq!(take(reply, 2, &mut chain).unwrap(), (2, false));
        assert_eq!(chain.cursor, b"100");
        for cut in 1..reply.len() {
            assert!(super::super::reply(&reply[..cut], 0).unwrap().is_none(), "a cut at {cut} is not a reply");
        }
        assert!(take(b"*2\r\n$1\r\n0\r\n*1\r\n$6\r\nm00001\r\n", 2, &mut Chain::new()).is_err(), "half a pair");
        assert!(take(b"*2\r\n$1\r\n0\r\n*1\r\n$6\r\nx00001\r\n", 1, &mut Chain::new()).is_err(), "a name never filled");
        assert!(take(b"*2\r\n$1\r\n0\r\n*1\r\n$6\r\nm10000\r\n", 1, &mut Chain::new()).is_err(), "a member past the end");
    }
}
