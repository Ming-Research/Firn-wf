//! Depth-one consumer command forms for the deployment-performance investigation.
//! Loopback RESP2/HELLO RESP3; worker threads multiplex nonblocking connections using std only.
mod limiter;
mod eviction;
mod handshake;
mod pubsub;

use std::collections::{BTreeMap, HashMap};
use std::env;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::{atomic::{AtomicBool, Ordering}, Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
const TIMEOUT: Duration = Duration::from_secs(30);
const WORKLOADS: &[&str] = &[
    "limiter-script", "limiter-tx", "setmany-tx", "session-set", "session-get",
    "evict-zipf", "handshake", "client-list", "pubsub-channel", "pubsub-pattern",
];

#[derive(Clone)]
struct Options {
    port: u16,
    connections: usize,
    threads: usize,
    seconds: Option<Duration>,
    requests: Option<u64>,
    keys: u64,
    value: Vec<u8>,
    workload: String,
    fill: Option<u64>,
    seed: u64,
    zipf_s: f64,
    warmup: Duration,
    sample_interval: Duration,
    maxmemory: Option<u64>,
    // File that receives client_work_fraction after the measured run.
    work_file: Option<String>,
    // Pub/sub only: PUBLISH commands the publisher keeps unanswered, and how
    // many messages it may send ahead of the slowest subscriber.
    pipeline: usize,
    window: usize,
}

impl Options {
    fn parse() -> Result<Self> {
        let mut opts = Self {
            port: 0, connections: 50,
            threads: thread::available_parallelism()?.get(),
            seconds: None, requests: None, keys: 100_000,
            value: vec![b'x'; 200], workload: String::new(), fill: None,
            seed: 1, zipf_s: 0.99, warmup: Duration::from_secs(5),
            sample_interval: Duration::from_millis(10), maxmemory: None, work_file: None,
            pipeline: 1, window: 1024,
        };
        let (mut keys_given, mut value_given, mut pubsub_given) = (false, false, false);
        let mut args = env::args().skip(1);
        while let Some(flag) = args.next() {
            let value = args.next().ok_or("each option needs a value")?;
            match flag.as_str() {
                "--port" => opts.port = value.parse()?,
                "--connections" => opts.connections = value.parse()?,
                "--threads" => opts.threads = value.parse()?,
                "--seconds" => {
                    let seconds: f64 = value.parse()?;
                    if seconds <= 0.0 {
                        return Err("--seconds must be positive".into());
                    }
                    let duration = Duration::try_from_secs_f64(seconds)?;
                    if duration.is_zero() { return Err("--seconds is too small".into()); }
                    opts.seconds = Some(duration);
                }
                "--requests" => opts.requests = Some(value.parse()?),
                "--keys" => { opts.keys = value.parse()?; keys_given = true; }
                "--value-size" => { opts.value = vec![b'x'; value.parse()?]; value_given = true; }
                "--workload" => opts.workload = value,
                "--fill" => opts.fill = Some(value.parse()?),
                "--seed" => opts.seed = value.parse()?,
                "--zipf-s" => opts.zipf_s = value.parse()?,
                "--warmup-seconds" => opts.warmup = Duration::try_from_secs_f64(value.parse()?)?,
                "--sample-ms" => opts.sample_interval = Duration::from_millis(value.parse()?),
                "--maxmemory" => opts.maxmemory = Some(value.parse()?),
                "--client-work-file" => opts.work_file = Some(value),
                "--pipeline" => { opts.pipeline = value.parse()?; pubsub_given = true; }
                "--window" => { opts.window = value.parse()?; pubsub_given = true; }
                _ => return Err(format!("unknown option: {flag}").into()),
            }
        }
        if opts.workload == "evict-zipf" {
            if !keys_given { opts.keys = 1_000_000; }
            if !value_given { opts.value = vec![b'x'; 64]; }
        }
        let pubsub = pubsub::is_pubsub(&opts.workload);
        if pubsub && !value_given { opts.value = vec![b'x'; 64]; }
        if pubsub_given && !pubsub {
            return Err("--pipeline and --window only apply to the pub/sub workloads".into());
        }
        if pubsub {
            if opts.requests.is_some() || opts.seconds.is_none() {
                return Err("the pub/sub workloads take --seconds, not --requests".into());
            }
            if opts.value.len() < 16 {
                return Err("a pub/sub payload holds a 16-byte header: --value-size is at least 16".into());
            }
            if opts.pipeline == 0 || opts.window < opts.pipeline {
                return Err("--pipeline must be positive and --window at least --pipeline".into());
            }
        }
        if !opts.zipf_s.is_finite() || opts.zipf_s < 0.0 || opts.sample_interval.is_zero() {
            return Err("--zipf-s must be finite and nonnegative; --sample-ms must be positive".into());
        }
        if opts.maxmemory.is_some() && (!matches!(opts.workload.as_str(), "session-set" | "session-get")
            || opts.fill.is_some()) {
            return Err("--maxmemory only applies to measured session-set/session-get".into());
        }
        if opts.port == 0 || opts.connections == 0 || opts.threads == 0 || opts.keys == 0 {
            return Err("--port, --connections, --threads and --keys must be positive".into());
        }
        if opts.fill.is_some() {
            if opts.seconds.is_some() || opts.requests.is_some()
                || (!opts.workload.is_empty() && opts.workload != "session-get") {
                return Err("--fill takes no run limit and only fills session-get data".into());
            }
        } else if !WORKLOADS.contains(&opts.workload.as_str())
            || opts.seconds.is_some() == opts.requests.is_some() || opts.requests == Some(0) {
            return Err("give --workload and exactly one of --seconds or --requests (> 0)".into());
        }
        // Threads without connections do no work; all connections remain open.
        opts.threads = opts.threads.min(opts.connections);
        Ok(opts)
    }
}

fn command(out: &mut Vec<u8>, args: &[&[u8]]) {
    write!(out, "*{}\r\n", args.len()).unwrap();
    for arg in args {
        write!(out, "${}\r\n", arg.len()).unwrap();
        out.extend_from_slice(arg);
        out.extend_from_slice(b"\r\n");
    }
}

// Return the end of one complete RESP2 value or HELLO RESP3 map, or None for a
// fragmented value. Nested errors fail just like a top-level -ERR.
fn reply(input: &[u8], depth: usize) -> Result<Option<usize>> {
    if depth > 128 { return Err("RESP nesting exceeds 128".into()); }
    let Some(end) = input.windows(2).position(|w| w == b"\r\n") else {
        return Ok(None);
    };
    if end == 0 { return Err("empty RESP header".into()); }
    let body = &input[1..end];
    let mut pos = end + 2;
    match input[0] {
        b'+' => (),
        b'-' => return Err(format!("server error: {}", String::from_utf8_lossy(body)).into()),
        b':' => { std::str::from_utf8(body)?.parse::<i64>()?; }
        b'$' | b'*' | b'%' => {
            let count: i64 = std::str::from_utf8(body)?.parse()?;
            if count < -1 || (input[0] == b'%' && count < 0) {
                return Err("invalid RESP length".into());
            }
            if count == -1 { return Ok(Some(pos)); }
            let count = usize::try_from(count)?;
            if input[0] == b'$' {
                let end = pos.checked_add(count).and_then(|n| n.checked_add(2))
                    .ok_or("RESP length overflow")?;
                if input.len() < end { return Ok(None); }
                if &input[end - 2..end] != b"\r\n" { return Err("bad bulk terminator".into()); }
                pos = end;
            } else {
                let count = if input[0] == b'%' {
                    count.checked_mul(2).ok_or("RESP map length overflow")?
                } else { count };
                for _ in 0..count {
                    let Some(size) = reply(&input[pos..], depth + 1)? else { return Ok(None); };
                    pos += size;
                }
            }
        }
        _ => return Err("unsupported RESP type".into()),
    }
    Ok(Some(pos))
}

fn connect(port: u16) -> Result<TcpStream> {
    let socket = TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), TIMEOUT)?;
    socket.set_nodelay(true)?;
    socket.set_read_timeout(Some(TIMEOUT))?;
    socket.set_write_timeout(Some(TIMEOUT))?;
    Ok(socket)
}

fn read_more(socket: &mut TcpStream, input: &mut Vec<u8>) -> io::Result<usize> {
    let mut bytes = [0; 8192];
    let n = socket.read(&mut bytes)?;
    if n == 0 { return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "server closed connection")); }
    input.extend_from_slice(&bytes[..n]);
    Ok(n)
}

// --fill N means exactly N sessions, sess:0 through sess:N-1. Batches are
// bounded by both commands and bytes, and all replies are checked before return.
fn fill(opts: &Options, count: u64) -> Result<()> {
    let mut socket = connect(opts.port)?;
    let mut next = 0;
    let mut out = Vec::new();
    let mut input = Vec::new();
    while next < count {
        out.clear();
        let mut batch = 0;
        while next < count && batch < 256 && out.len() < 1_048_576 {
            if opts.workload == "evict-zipf" {
                let key = format!("key:{next}");
                command(&mut out, &[b"SET", key.as_bytes(), &opts.value]);
            } else {
                let key = format!("sess:{next}");
                command(&mut out, &[b"SET", key.as_bytes(), &opts.value, b"EX", b"86400"]);
            }
            next += 1;
            batch += 1;
        }
        socket.write_all(&out)?;
        for _ in 0..batch {
            let size = loop {
                if let Some(size) = reply(&input, 0)? { break size; }
                read_more(&mut socket, &mut input)?;
            };
            if &input[..size] != b"+OK\r\n" { return Err("fill SET did not return OK".into()); }
            input.drain(..size);
        }
        if !input.is_empty() { return Err("extra fill reply".into()); }
    }
    Ok(())
}

// SplitMix64 per connection, fixed seeds for repeatable comparisons. Rejection
// removes modulo bias so every key in 0..keys has equal probability.
fn random_u64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e3779b97f4a7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    z ^ (z >> 31)
}

fn random_key(state: &mut u64, keys: u64) -> u64 {
    loop {
        let z = random_u64(state);
        if z >= keys.wrapping_neg() % keys { return z % keys; }
    }
}

fn request(out: &mut Vec<u8>, opts: &Options, rng: &mut u64, sha: &[u8]) -> usize {
    out.clear();
    if opts.workload == "handshake" { return handshake::request(out); }
    if opts.workload == "client-list" {
        command(out, &[b"CLIENT", b"LIST"]);
        return 1;
    }
    let prefix = if opts.workload.starts_with("session-") { "sess" } else { "key" };
    let key = format!("{prefix}:{}", random_key(rng, opts.keys));
    let key = key.as_bytes();
    match opts.workload.as_str() {
        "limiter-script" => {
            // _upsert passes points, secDuration, this.points, this.duration.
            // The last two are unused by the stock script; use a 1-point/60s limiter.
            command(out, &[b"EVALSHA", sha, b"1", key, b"1", b"60", b"1", b"60"]);
            1
        }
        "limiter-tx" => {
            command(out, &[b"MULTI"]);
            command(out, &[b"INCRBY", key, b"1"]);
            command(out, &[b"PTTL", key]);
            command(out, &[b"EXEC"]);
            4
        }
        "setmany-tx" => {
            let key2 = format!("key:{}", random_key(rng, opts.keys));
            let key3 = format!("key:{}", random_key(rng, opts.keys));
            command(out, &[b"MULTI"]);
            command(out, &[b"MSET", key, &opts.value, key2.as_bytes(), &opts.value,
                key3.as_bytes(), &opts.value]);
            for key in [key, key2.as_bytes(), key3.as_bytes()] {
                command(out, &[b"EXPIRE", key, b"300"]);
            }
            command(out, &[b"EXEC"]);
            6
        }
        "session-set" => {
            command(out, &[b"SET", key, &opts.value, b"EX", b"86400"]);
            1
        }
        "session-get" => { command(out, &[b"GET", key]); 1 }
        _ => unreachable!(),
    }
}

struct Connection {
    socket: Option<TcpStream>,
    sha: Vec<u8>,
    rng: u64,
    remaining: u64,
    out: Vec<u8>,
    sent: usize,
    input: Vec<u8>,
    replies: usize,
    started: Instant,
    cache_key: Vec<u8>,
    cache_set: bool,
    hit: bool,
    wrote: bool,
    refused_sets: u64,
    // Whether the last advance wrote or read bytes.
    moved: bool,
}

impl Connection {
    fn new(opts: &Options, id: usize) -> Result<Self> {
        // Handshake slots connect only inside the timed operation, and have no
        // idle socket between operations. Every other workload keeps its socket.
        let mut socket = if opts.workload == "handshake" { None } else { Some(connect(opts.port)?) };
        let mut sha = Vec::new();
        if opts.workload == "limiter-script" {
            let socket = socket.as_mut().unwrap();
            let mut out = Vec::new();
            command(&mut out, &[b"SCRIPT", b"LOAD", limiter::SCRIPT.as_bytes()]);
            socket.write_all(&out)?;
            let mut input = Vec::new();
            while reply(&input, 0)?.is_none() { read_more(socket, &mut input)?; }
            if input.len() != 47 || !input.starts_with(b"$40\r\n")
                || !input[5..45].iter().all(u8::is_ascii_hexdigit) {
                return Err("SCRIPT LOAD did not return a SHA1 bulk string".into());
            }
            sha.extend_from_slice(&input[5..45]);
        }
        if opts.workload == "client-list" { handshake::setup(socket.as_mut().unwrap())?; }
        if let Some(socket) = &socket { socket.set_nonblocking(true)?; }
        let remaining = opts.requests.map_or(u64::MAX, |n| share(n, opts.connections, id));
        // The default preserves the original uniform workload's sequence.
        Ok(Self { socket, sha, rng: (id as u64) ^ opts.seed.wrapping_sub(1), remaining,
            out: Vec::new(), sent: 0, input: Vec::new(), replies: 0,
            started: Instant::now(), cache_key: Vec::new(), cache_set: false, hit: false, wrote: false,
            refused_sets: 0, moved: false })
    }

    // One whole transaction is buffered as a single logical write. Partial TCP
    // writes resume from sent; no next request is queued until every reply arrives.
    fn advance(&mut self, opts: &Options) -> Result<bool> {
        if self.started.elapsed() > TIMEOUT { return Err("request exceeded 30 seconds".into()); }
        self.moved = false;
        let socket = self.socket.as_mut().ok_or("active request has no socket")?;
        if self.sent < self.out.len() {
            match socket.write(&self.out[self.sent..]) {
                Ok(0) => return Err("zero-byte socket write".into()),
                Ok(n) => { self.sent += n; self.moved = true; }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::Interrupted => (),
                Err(e) => return Err(e.into()),
            }
        }
        // Read even while a large write is incomplete: transaction QUEUED
        // replies may already be available, and must not back up the server.
        match read_more(socket, &mut self.input) {
            Ok(_) => self.moved = true,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::Interrupted => (),
            Err(e) => return Err(e.into()),
        }
        while self.replies > 0 {
            // Only a cache-aside miss SET may be refused. Fill, GET, control
            // commands and every other error retain the fatal RESP path.
            let refused = opts.workload == "evict-zipf" && self.cache_set
                && self.input.starts_with(eviction::OOM_REPLY);
            let size = if refused { eviction::OOM_REPLY.len() } else {
                let Some(size) = reply(&self.input, 0)? else { break; };
                size
            };
            if opts.workload == "handshake" {
                handshake::check_reply(&self.input[..size], 4 - self.replies)?;
            } else if opts.workload == "client-list" {
                handshake::check_list(&self.input[..size], opts.connections)?;
            } else if opts.workload == "evict-zipf" {
                if self.cache_set {
                    if refused {
                        self.refused_sets += 1;
                    } else if &self.input[..size] != b"+OK\r\n" {
                        return Err("cache SET did not return OK".into());
                    }
                    // Participation counts a completed SET attempt, including
                    // refusal; the preceding GET remains a miss either way.
                    self.wrote = true;
                } else {
                    self.hit = eviction::get_hit(&self.input[..size], &opts.value)?;
                }
            } else if opts.maxmemory.is_some() {
                if opts.workload == "session-get" {
                    if !eviction::get_hit(&self.input[..size], &opts.value)? {
                        return Err("prefilled session GET missed".into());
                    }
                } else if &self.input[..size] != b"+OK\r\n" {
                    return Err("session SET did not return OK".into());
                }
            }
            self.input.drain(..size);
            self.replies -= 1;
        }
        if self.replies == 0 {
            if self.sent != self.out.len() || !self.input.is_empty() {
                return Err("unexpected extra or premature reply".into());
            }
            if opts.workload == "evict-zipf" && !self.cache_set && !self.hit {
                self.out.clear();
                command(&mut self.out, &[b"SET", &self.cache_key, &opts.value]);
                self.sent = 0;
                self.replies = 1;
                self.cache_set = true;
                return Ok(false);
            }
            return Ok(true);
        }
        Ok(false)
    }
}

fn share(n: u64, total: usize, id: usize) -> u64 {
    n / total as u64 + u64::from((id as u64) < n % total as u64)
}

// Exact, sparse one-microsecond bins, with no clipped tail or averaged percentiles.
type Histogram = HashMap<u64, u64>;
fn worker(mut connections: Vec<Connection>, opts: &Options, start: Instant,
          cancelled: &AtomicBool, zipf: Option<&eviction::Zipf>) -> Result<(Histogram, Instant, u64, usize, u64, Duration, Duration)> {
    let mut histogram = Histogram::new();
    // Client accounting: a scan is work when any connection wrote bytes, read
    // bytes, connected or completed a request; every other scan only polls.
    let (loop_start, mut work) = (Instant::now(), Duration::ZERO);
    let mut hits = 0;
    loop {
        if cancelled.load(Ordering::Relaxed) { return Err("another worker failed".into()); }
        let scan_start = Instant::now();
        let (mut active, mut moved) = (false, false);
        for c in &mut connections {
            if c.replies == 0 {
                if c.remaining == 0 || opts.seconds.is_some_and(|s| start.elapsed() >= s) { continue; }
                if let Some(zipf) = zipf {
                    c.out.clear();
                    c.cache_key = format!("key:{}", zipf.draw(&mut c.rng)).into_bytes();
                    command(&mut c.out, &[b"GET", &c.cache_key]);
                    c.replies = 1;
                    c.cache_set = false;
                    c.hit = false;
                } else {
                    c.replies = request(&mut c.out, opts, &mut c.rng, &c.sha);
                }
                c.sent = 0;
                c.started = Instant::now();
                if opts.workload == "handshake" {
                    // std's connect_timeout blocks this multiplexing worker;
                    // connect time belongs to this slot's operation latency.
                    let socket = connect(opts.port)?;
                    socket.set_nonblocking(true)?;
                    c.socket = Some(socket);
                    moved = true;
                }
                c.remaining -= 1;
            }
            active = true;
            let done = c.advance(opts)?;
            moved |= c.moved;
            if done {
                let us = c.started.elapsed().as_nanos().div_ceil(1000) as u64;
                // Capture the fourth reply's latency before close overhead, but
                // close before starting another operation in this slot.
                if opts.workload == "handshake" { c.socket.take(); }
                *histogram.entry(us).or_insert(0) += 1;
                hits += u64::from(c.hit);
                moved = true;
            }
        }
        if moved { work += scan_start.elapsed(); }
        if !active { break; }
        // No sleeping timer imposes a latency floor; idle scans yield to peers.
        thread::yield_now();
    }
    let total = loop_start.elapsed();
    Ok((histogram, Instant::now(), hits, connections.iter().filter(|c| c.wrote).count(),
        connections.iter().map(|c| c.refused_sets).sum(), work, total))
}

fn percentile(histogram: &BTreeMap<u64, u64>, count: u64, percent: u64) -> f64 {
    let rank = (u128::from(count) * u128::from(percent)).div_ceil(100) as u64;
    let mut seen = 0;
    for (&us, &n) in histogram {
        seen += n;
        if seen >= rank { return us as f64 / 1000.0; }
    }
    unreachable!()
}

struct Measurement {
    count: u64,
    hits: u64,
    writers: usize,
    refused_sets: u64,
    // Summed over workers: scans with progress, and the whole worker loop.
    work: Duration,
    total: Duration,
    seconds: f64,
    p50: f64,
    p99: f64,
}

// Share of the client's loop time spent in scans that moved bytes or finished
// a request; the rest is idle scanning for replies.
fn work_fraction(work: Duration, total: Duration) -> f64 {
    if total.is_zero() { 0.0 } else { work.as_secs_f64() / total.as_secs_f64() }
}

impl Measurement {
    fn client_work_fraction(&self) -> f64 { work_fraction(self.work, self.total) }
    fn csv(&self, opts: &Options) -> String {
        format!("{},{},{},{:.6},{:.3},{:.3},{:.3}", opts.workload, opts.connections,
            self.count, self.seconds, self.count as f64 / self.seconds, self.p50, self.p99)
    }
}

fn measure(opts: &Options, zipf: Option<Arc<eviction::Zipf>>,
           mut monitor: Option<&mut eviction::Monitor>) -> Result<Measurement> {
    let mut groups: Vec<Vec<Connection>> = (0..opts.threads).map(|_| Vec::new()).collect();
    let mut retained_sockets = Vec::new();
    // Persistent connections, SCRIPT LOAD and CLIENT SETINFO finish before the
    // shared clock. Handshake slots have no connection setup outside that clock.
    for id in 0..opts.connections {
        let connection = Connection::new(&opts, id)?;
        if opts.workload == "client-list" {
            // Cloned descriptors hold the same connections open when a worker
            // has a shorter request share, including a share of zero.
            retained_sockets.push(connection.socket.as_ref().unwrap().try_clone()?);
        }
        groups[id % opts.threads].push(connection);
    }
    // RESETSTAT and the initial INFO happen with all measured connections open,
    // before releasing any worker. A failed setup cannot strand a barrier waiter.
    if let Some(m) = monitor.as_deref_mut() { m.start()?; }
    let opts = Arc::new(opts.clone());
    let barrier = Arc::new(Barrier::new(opts.threads + 1));
    let cancelled = Arc::new(AtomicBool::new(false));
    let start = Arc::new(std::sync::OnceLock::new());
    let mut handles = Vec::new();
    for connections in groups {
        let zipf = zipf.clone();
        let (opts, barrier, cancelled, start) =
            (opts.clone(), barrier.clone(), cancelled.clone(), start.clone());
        handles.push(thread::spawn(move || {
            barrier.wait();
            let result = worker(connections, &opts, *start.get().unwrap(), &cancelled, zipf.as_deref());
            if result.is_err() { cancelled.store(true, Ordering::Relaxed); }
            result
        }));
    }
    let begin = Instant::now();
    start.set(begin).unwrap();
    barrier.wait();
    let mut failure = None;
    if let Some(m) = monitor.as_deref_mut() {
        while handles.iter().any(|h| !h.is_finished()) {
            thread::sleep(opts.sample_interval);
            if let Err(e) = m.sample() {
                cancelled.store(true, Ordering::Relaxed);
                failure = Some(e);
                break;
            }
        }
    }
    let mut histogram = BTreeMap::new();
    let mut end = begin;
    let mut hits = 0;
    let mut writers = 0;
    let mut refused_sets = 0;
    let (mut work, mut total) = (Duration::ZERO, Duration::ZERO);
    for handle in handles {
        match handle.join().unwrap_or_else(|_| Err("worker panicked".into())) {
            Ok((bins, ended, found, wrote, refused, worked, looped)) => {
                work += worked;
                total += looped;
                end = end.max(ended);
                hits += found;
                writers += wrote;
                refused_sets += refused;
                for (us, n) in bins { *histogram.entry(us).or_insert(0) += n; }
            }
            Err(e) => { eprintln!("{e}"); failure = Some(e); }
        }
    }
    drop(retained_sockets);
    if let Some(e) = failure { return Err(e); }
    if let Some(m) = monitor { m.sample()?; }
    let count: u64 = histogram.values().sum();
    if count == 0 { return Err("no requests completed".into()); }
    if opts.requests.is_some_and(|n| count != n) { return Err("request count mismatch".into()); }
    // Timed runs stop issuing at the deadline and include draining in-flight work
    // in both the count and elapsed time; setup and histogram merging are excluded.
    let seconds = end.duration_since(begin).as_secs_f64();
    Ok(Measurement { count, hits, writers, refused_sets, work, total, seconds,
        p50: percentile(&histogram, count, 50), p99: percentile(&histogram, count, 99) })
}

fn run(opts: Options) -> Result<()> {
    if let Some(count) = opts.fill { return fill(&opts, count); }
    if opts.workload == "evict-zipf" { return eviction::run(&opts); }
    if opts.maxmemory.is_some() { return eviction::session_limit(&opts); }
    if pubsub::is_pubsub(&opts.workload) { return pubsub::run(&opts); }
    if matches!(opts.workload.as_str(), "handshake" | "client-list") && !opts.warmup.is_zero() {
        let mut warm = opts.clone();
        warm.seconds = Some(opts.warmup);
        warm.requests = None;
        measure(&warm, None, None)?;
    }
    let measured = measure(&opts, None, None)?;
    if let Some(path) = &opts.work_file {
        std::fs::write(path, format!("{:.4}\n", measured.client_work_fraction()))?;
    }
    println!("{}", measured.csv(&opts));
    Ok(())
}

fn main() {
    if let Err(e) = Options::parse().and_then(run) {
        eprintln!("firn-workload: {e}");
        // Exit status 3 says that the server lacks the workload's commands, so
        // that the harness can record a skip; every other failure is 1.
        std::process::exit(if e.downcast_ref::<pubsub::Unsupported>().is_some() { 3 } else { 1 });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn options(port: u16, workload: &str) -> Options {
        Options { port, connections: 2, threads: 2, seconds: None, requests: Some(3),
            keys: 1, value: vec![b'x'], workload: workload.into(), fill: None,
            seed: 1, zipf_s: 0.99, warmup: Duration::ZERO,
            sample_interval: Duration::from_millis(10), maxmemory: None, work_file: None,
            pipeline: 1, window: 1024 }
    }

    pub(super) fn expect_command(socket: &mut TcpStream, args: &[&[u8]]) {
        let mut expected = Vec::new();
        command(&mut expected, args);
        let mut actual = vec![0; expected.len()];
        socket.read_exact(&mut actual).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn work_fraction_is_work_over_loop_time() {
        let s = Duration::from_secs;
        assert_eq!(work_fraction(s(3), s(4)), 0.75);
        assert_eq!(work_fraction(Duration::ZERO, s(4)), 0.0);
        assert_eq!(work_fraction(s(4), s(4)), 1.0);
        assert_eq!(work_fraction(s(1), Duration::ZERO), 0.0);
    }

    #[test]
    fn workloads_send_consumer_forms_and_count_transactions_once() {
        use std::net::TcpListener;
        for &workload in WORKLOADS {
            // Cache-aside, connection and pub/sub workloads have separate checked cases.
            if matches!(workload, "evict-zipf" | "handshake" | "client-list"
                | "pubsub-channel" | "pubsub-pattern") { continue; }
            let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let opts = options(listener.local_addr().unwrap().port(), workload);
            let server = thread::spawn(move || {
                let mut clients = Vec::new();
                for id in 0..2 {
                    let (mut socket, _) = listener.accept().unwrap();
                    clients.push(thread::spawn(move || {
                        socket.set_read_timeout(Some(TIMEOUT)).unwrap();
                        let sha = b"0123456789012345678901234567890123456789";
                        if workload == "limiter-script" {
                            expect_command(&mut socket, &[b"SCRIPT", b"LOAD", limiter::SCRIPT.as_bytes()]);
                            socket.write_all(b"$40\r\n0123456789012345678901234567890123456789\r\n").unwrap();
                        }
                        for _ in 0..share(3, 2, id) {
                            let response: &[u8] = match workload {
                                "limiter-script" => {
                                    expect_command(&mut socket, &[b"EVALSHA", sha, b"1", b"key:0", b"1", b"60", b"1", b"60"]);
                                    b"*2\r\n:1\r\n:60000\r\n"
                                }
                                "limiter-tx" => {
                                    expect_command(&mut socket, &[b"MULTI"]);
                                    expect_command(&mut socket, &[b"INCRBY", b"key:0", b"1"]);
                                    expect_command(&mut socket, &[b"PTTL", b"key:0"]);
                                    expect_command(&mut socket, &[b"EXEC"]);
                                    b"+OK\r\n+QUEUED\r\n+QUEUED\r\n*2\r\n:1\r\n:-1\r\n"
                                }
                                "setmany-tx" => {
                                    expect_command(&mut socket, &[b"MULTI"]);
                                    expect_command(&mut socket, &[b"MSET", b"key:0", b"x", b"key:0", b"x", b"key:0", b"x"]);
                                    for _ in 0..3 { expect_command(&mut socket, &[b"EXPIRE", b"key:0", b"300"]); }
                                    expect_command(&mut socket, &[b"EXEC"]);
                                    b"+OK\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n+QUEUED\r\n*4\r\n+OK\r\n:1\r\n:1\r\n:1\r\n"
                                }
                                "session-set" => {
                                    expect_command(&mut socket, &[b"SET", b"sess:0", b"x", b"EX", b"86400"]);
                                    b"+OK\r\n"
                                }
                                "session-get" => {
                                    expect_command(&mut socket, &[b"GET", b"sess:0"]);
                                    b"$1\r\nx\r\n"
                                }
                                _ => unreachable!(),
                            };
                            socket.write_all(response).unwrap();
                        }
                    }));
                }
                for client in clients { client.join().unwrap(); }
            });
            let result = run(opts);
            server.join().unwrap();
            result.unwrap();
        }
    }

    #[test]
    fn fill_crosses_batch_boundary_and_checks_errors() {
        use std::net::TcpListener;
        for (workload, last_reply, error) in [
            ("session-get", b"+OK\r\n".as_slice(), None),
            ("session-get", b"-ERR rejected\r\n".as_slice(), Some("ERR rejected")),
            ("evict-zipf", b"+OK\r\n".as_slice(), None),
            ("evict-zipf", b"-ERR rejected\r\n".as_slice(), Some("ERR rejected")),
            ("evict-zipf", eviction::OOM_REPLY, Some("server error: OOM")),
        ] {
            let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let opts = options(listener.local_addr().unwrap().port(), workload);
            let server = thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                socket.set_read_timeout(Some(TIMEOUT)).unwrap();
                for id in 0..257 {
                    if workload == "evict-zipf" {
                        let key = format!("key:{id}");
                        expect_command(&mut socket, &[b"SET", key.as_bytes(), b"x"]);
                    } else {
                        let key = format!("sess:{id}");
                        expect_command(&mut socket, &[b"SET", key.as_bytes(), b"x", b"EX", b"86400"]);
                    }
                    socket.write_all(if id == 256 { last_reply } else { b"+OK\r\n" }).unwrap();
                }
            });
            let result = fill(&opts, 257);
            server.join().unwrap();
            if let Some(message) = error { assert!(result.unwrap_err().to_string().contains(message)); }
            else { result.unwrap(); }
        }
    }

    #[test]
    fn fragmented_and_nested_resp() {
        let wire = b"*4\r\n:1\r\n$5\r\na\r\n\0b\r\n$-1\r\n*2\r\n+OK\r\n*-1\r\n";
        for n in 0..wire.len() { assert_eq!(reply(&wire[..n], 0).unwrap(), None); }
        assert_eq!(reply(wire, 0).unwrap(), Some(wire.len()));
        assert_eq!(reply(b"+OK\r\n:2\r\n", 0).unwrap(), Some(5));
        for bad in [b"-ERR failure\r\n".as_slice(), b"*2\r\n:1\r\n*1\r\n-WRONGTYPE bad\r\n",
            b"$-2\r\n", b"$1\r\nx!!", b":not-an-int\r\n"] {
            assert!(reply(bad, 0).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn exact_counts_and_microsecond_tail() {
        for count in [1, 49, 50, 51, 103] {
            assert_eq!((0..50).map(|id| share(count, 50, id)).sum::<u64>(), count);
        }
        let bins = BTreeMap::from([(1, 50), (2, 48), (1_000_001, 2)]);
        assert_eq!(percentile(&bins, 100, 50), 0.001);
        assert_eq!(percentile(&bins, 100, 99), 1000.001);
    }

    #[test]
    fn command_framing_preserves_binary_values() {
        let mut out = Vec::new();
        command(&mut out, &[b"SET", b"sess:0", b"a\r\n\0b", b"EX", b"86400"]);
        assert_eq!(out, b"*5\r\n$3\r\nSET\r\n$6\r\nsess:0\r\n$5\r\na\r\n\0b\r\n$2\r\nEX\r\n$5\r\n86400\r\n");
    }
}
