//! Blocking hand-off: `--connections` consumers each hold a `BLPOP key 0` on a
//! key of their own, and one producer pushes (`LPUSH`) an element to a
//! consumer's key whenever that consumer has received its previous element.
//! A consumer blocks again as soon as it has an element. RESP2 over loopback.
//! The rate is the elements received per second; an element's latency runs
//! from just before the producer writes the batch holding it to its arrival at
//! its consumer.
//!
//! The producer is closed-loop: it sends `--pipeline` pushes to as many
//! different consumers' keys and waits for all their replies before the next
//! batch, as redis-benchmark -P does. A key never holds more than one element,
//! so no push finds a backlog and every element is handed to a consumer that
//! the server either holds blocked or is about to block again. A push can reach
//! the server before the consumer's new `BLPOP` does; the server then keeps
//! the element and the `BLPOP` returns at once. The client cannot tell the two
//! cases apart, and every server runs the same protocol.
//!
//! A run ends with one more element for each key, the drain, after which its
//! consumer does not block again. No `BLPOP` is left on the server when the
//! connections close, so that a server which keeps the blocked commands of
//! closed connections (Garnet 2.2.3 does) cannot hand the next run's first
//! element on the same keys to a connection that is gone.
use super::pubsub::{read_available, Accounting, Counter, Unsupported};
use super::*;
use std::sync::atomic::AtomicU64;
use std::sync::OnceLock;

// An element starts with its number among the elements of its key and the time
// the batch holding it was built, both little-endian u64, then padding.
const HEADER: usize = 16;
// The number of the drain, which no key reaches.
const DRAIN: u64 = u64::MAX;
const READ_CHUNK: usize = 65536;
// How long the consumers' first BLPOP commands have to be refused before the
// run starts: no element exists yet, so any reply then is a refusal.
const SETTLE: Duration = Duration::from_millis(200);

/// The consumers waited `Shared::stall` for an element that never came: the
/// server lost it or stopped handing elements out. The workload does not
/// complete on that server, which is a result for a competitor and a failure
/// for firn.
#[derive(Debug)]
pub(super) struct Stalled(String);

impl std::fmt::Display for Stalled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Stalled {}

pub(super) fn is_handoff(workload: &str) -> bool {
    workload == "blocking-handoff"
}

fn key(slot: usize) -> Vec<u8> {
    format!("bench:q:{slot}").into_bytes()
}

// What the reply that serves one consumer looks like: all of them have one
// length.
struct Reply {
    // Every byte before the payload: the array header, the key and the
    // payload's length.
    head: Vec<u8>,
    payload_len: usize,
    padding: Vec<u8>,
}

impl Reply {
    fn new(slot: usize, payload_len: usize) -> Self {
        let key = key(slot);
        let mut head = format!("*2\r\n${}\r\n", key.len()).into_bytes();
        head.extend_from_slice(&key);
        head.extend_from_slice(format!("\r\n${payload_len}\r\n").as_bytes());
        Reply { head, payload_len, padding: vec![b'x'; payload_len - HEADER] }
    }

    fn frame_len(&self) -> usize { self.head.len() + self.payload_len + 2 }

    // The length of the reply at the start of `input`, or None while it is
    // incomplete. Anything but the reply this consumer awaits is an error, also
    // while the reply is still incomplete.
    fn parse(&self, input: &[u8]) -> Result<Option<usize>> {
        let seen = input.len().min(self.head.len());
        if input[..seen] != self.head[..seen] {
            return Err(format!("unexpected reply to BLPOP: {:?}",
                String::from_utf8_lossy(&input[..input.len().min(80)])).into());
        }
        let frame = self.frame_len();
        if input.len() < frame { return Ok(None); }
        if &input[frame - 2..frame] != b"\r\n" {
            return Err("reply to BLPOP lacks its closing CRLF".into());
        }
        Ok(Some(frame))
    }

    // The element's number and send time in a complete reply.
    fn decode(&self, frame: &[u8]) -> Result<(u64, u64)> {
        let payload = &frame[self.head.len()..self.head.len() + self.payload_len];
        if payload[HEADER..] != self.padding[..] { return Err("element payload was altered".into()); }
        let number = u64::from_le_bytes(payload[..8].try_into().unwrap());
        let sent = u64::from_le_bytes(payload[8..HEADER].try_into().unwrap());
        Ok((number, sent))
    }
}

struct Consumer {
    socket: TcpStream,
    input: Vec<u8>,
    // The BLPOP command, and the bytes of it still to write.
    blpop: Vec<u8>,
    pending: Vec<u8>,
    reply: Reply,
    // Elements received so far, which is also the number due next.
    count: u64,
    slot: usize,
    // The drain has arrived: the consumer holds no BLPOP and expects nothing.
    drained: bool,
}

// Connects every consumer, blocks it, and checks that the server accepted the
// command: before the shared clock.
fn consumers(opts: &Options) -> Result<Vec<Consumer>> {
    let mut all = Vec::new();
    for slot in 0..opts.connections {
        let mut socket = connect(opts.port)?;
        let mut blpop = Vec::new();
        command(&mut blpop, &[b"BLPOP", &key(slot), b"0"]);
        socket.write_all(&blpop)?;
        all.push(Consumer { socket, input: Vec::new(), blpop, pending: Vec::new(),
            reply: Reply::new(slot, opts.value.len()), count: 0, slot, drained: false });
    }
    thread::sleep(SETTLE);
    let mut scratch = vec![0; 4096];
    for c in &mut all {
        c.socket.set_nonblocking(true)?;
        if read_available(&mut c.socket, &mut c.input, &mut scratch)? {
            let text = String::from_utf8_lossy(&c.input).trim_end().to_string();
            let text = text.strip_prefix('-').unwrap_or(&text).to_string();
            if text.starts_with("ERR unknown command") { return Err(Unsupported(text).into()); }
            return Err(format!("BLPOP replied {text:?} with nothing pushed").into());
        }
    }
    Ok(all)
}

// Writes what a consumer has pending, as much as the socket takes.
fn flush(c: &mut Consumer) -> Result<()> {
    while !c.pending.is_empty() {
        match c.socket.write(&c.pending) {
            Ok(0) => return Err("zero-byte socket write".into()),
            Ok(n) => { c.pending.drain(..n); }
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted) => break,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

struct Producer {
    socket: TcpStream,
    input: Vec<u8>,
    out: Vec<u8>,
    sent: usize,
    // LPUSH replies still due for the batch in `out`.
    awaiting: usize,
    // Per key, the elements pushed so far, which is also the number due next.
    pushed: Vec<u64>,
}

struct Shared {
    // Per consumer, the elements it has received.
    received: Vec<Counter>,
    cancelled: AtomicBool,
    // How long the consumers of a thread may go without an element before the
    // run fails: an element the server never delivers is found this way.
    stall: Duration,
}

struct Context {
    payload_len: usize,
    depth: usize,
}

// The keys, from `cursor` on and wrapping, whose consumer has received every
// element pushed to it, up to `depth` of them; fewer when fewer qualify.
fn ready_keys(received: impl Fn(usize) -> u64, pushed: &[u64], cursor: usize, depth: usize) -> Vec<usize> {
    let n = pushed.len();
    (0..n).map(|i| (cursor + i) % n).filter(|&k| received(k) == pushed[k]).take(depth).collect()
}

fn producer_worker(mut p: Producer, ctx: &Context, begin: Instant, run_for: Duration,
                   shared: &Shared) -> Result<(Accounting, u64)> {
    let mut scratch = vec![0; READ_CHUNK];
    let mut payload = vec![b'x'; ctx.payload_len];
    let keys: Vec<Vec<u8>> = (0..p.pushed.len()).map(key).collect();
    let (loop_start, mut work) = (Instant::now(), Duration::ZERO);
    let (mut cursor, mut stopping, mut draining) = (0, false, false);
    loop {
        if shared.cancelled.load(Ordering::Relaxed) { return Err("another worker failed".into()); }
        let scan_start = Instant::now();
        let mut moved = read_available(&mut p.socket, &mut p.input, &mut scratch)?;
        while p.awaiting > 0 {
            let Some(size) = reply(&p.input, 0)? else { break; };
            if p.input[0] != b':' {
                return Err(format!("LPUSH replied {:?}", String::from_utf8_lossy(&p.input[..size])).into());
            }
            p.input.drain(..size);
            p.awaiting -= 1;
        }
        if p.awaiting == 0 && p.sent == p.out.len() {
            if !p.input.is_empty() { return Err("extra bytes on the producer's connection".into()); }
            // The drain's replies are in: the run is over.
            if draining { break; }
            if begin.elapsed() >= run_for { stopping = true; }
            // Everything pushed has been received once every key is ready.
            let want = if stopping { p.pushed.len() } else { ctx.depth };
            let ready = ready_keys(|k| shared.received[k].0.load(Ordering::Relaxed), &p.pushed, cursor, want);
            if stopping {
                if ready.len() == want {
                    // The consumers hold everything pushed: end the run with
                    // an element for each, after which none blocks again.
                    payload[..8].copy_from_slice(&DRAIN.to_le_bytes());
                    payload[8..HEADER].fill(0);
                    p.out.clear();
                    p.sent = 0;
                    for key in &keys { command(&mut p.out, &[b"LPUSH", key, &payload]); }
                    p.awaiting = keys.len();
                    draining = true;
                }
            } else if ready.len() == want {
                let sent = begin.elapsed().as_nanos() as u64;
                p.out.clear();
                p.sent = 0;
                for &k in &ready {
                    payload[..8].copy_from_slice(&p.pushed[k].to_le_bytes());
                    payload[8..HEADER].copy_from_slice(&sent.to_le_bytes());
                    command(&mut p.out, &[b"LPUSH", &keys[k], &payload]);
                    p.pushed[k] += 1;
                }
                cursor = (ready[ready.len() - 1] + 1) % p.pushed.len();
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
    Ok((Accounting { work, total: loop_start.elapsed() }, p.pushed.iter().sum()))
}

struct ConsumerResult {
    // Elements by latency in microseconds, at least 1.
    histogram: BTreeMap<u64, u64>,
    // The time, since the run began, of this thread's last element.
    last_message: u64,
    accounting: Accounting,
}

fn consumer_worker(mut group: Vec<Consumer>, begin: Instant, shared: &Shared) -> Result<ConsumerResult> {
    let mut scratch = vec![0; READ_CHUNK];
    let mut histogram = BTreeMap::new();
    let (loop_start, mut work) = (Instant::now(), Duration::ZERO);
    let (mut last_message, mut last_progress) = (0, Instant::now());
    loop {
        if shared.cancelled.load(Ordering::Relaxed) { return Err("another worker failed".into()); }
        let mut moved = false;
        for c in &mut group {
            let read_start = Instant::now();
            flush(c)?;
            if !read_available(&mut c.socket, &mut c.input, &mut scratch)? { continue; }
            moved = true;
            // Elements that arrive in one read share the time it returned.
            let now = begin.elapsed().as_nanos() as u64;
            let before = c.count;
            let mut pos = 0;
            while let Some(frame) = c.reply.parse(&c.input[pos..])? {
                if c.drained { return Err(format!("consumer {} received an element after the drain", c.slot).into()); }
                let (number, sent) = c.reply.decode(&c.input[pos..pos + frame])?;
                pos += frame;
                if number == DRAIN {
                    // The producer's last element for this key; the consumer
                    // stays unblocked.
                    c.drained = true;
                    continue;
                }
                if number != c.count {
                    return Err(format!("consumer {} received element {number} where {} was due",
                        c.slot, c.count).into());
                }
                let us = now.saturating_sub(sent).div_ceil(1000).max(1);
                *histogram.entry(us).or_insert(0) += 1;
                c.count += 1;
                // Block again before the producer learns that this element
                // has arrived.
                c.pending.extend_from_slice(&c.blpop);
            }
            c.input.drain(..pos);
            flush(c)?;
            if c.count != before {
                shared.received[c.slot].0.store(c.count, Ordering::Relaxed);
                last_message = now;
            }
            work += read_start.elapsed();
        }
        if moved { last_progress = Instant::now(); }
        if group.iter().all(|c| c.drained) { break; }
        if last_progress.elapsed() > shared.stall {
            return Err(Stalled(format!("no element for {} seconds", shared.stall.as_secs_f64())).into());
        }
        thread::yield_now();
    }
    Ok(ConsumerResult { histogram, last_message,
        accounting: Accounting { work, total: loop_start.elapsed() } })
}

struct Measured {
    pushed: u64,
    seconds: f64,
    p50: f64,
    p99: f64,
    // The larger of the producer's and the consumers' work fractions: either
    // side at its limit makes the client the limit.
    work_fraction: f64,
}

impl Measured {
    // workload,connections,pipeline,published,delivered,seconds,rate,p50_ms,p99_ms:
    // the columns of the pub/sub workloads, whose summary reads them.
    fn csv(&self, opts: &Options) -> String {
        format!("{},{},{},{},{},{:.6},{:.3},{:.3},{:.3}", opts.workload, opts.connections,
            opts.pipeline, self.pushed, self.pushed, self.seconds,
            self.pushed as f64 / self.seconds, self.p50, self.p99)
    }
}

// One run of `run_for`: block the consumers, push until the time is up, then
// wait until every consumer has every element. The span counts from the start
// of the run to the last arrival; setup and the merging of results are outside
// it. A consumer thread that receives nothing for `stall` fails the run.
fn measure(opts: &Options, run_for: Duration, stall: Duration) -> Result<Measured> {
    let consumers = consumers(opts)?;
    let producer = Producer { socket: connect(opts.port)?, input: Vec::new(), out: Vec::new(),
        sent: 0, awaiting: 0, pushed: vec![0; opts.connections] };
    producer.socket.set_nonblocking(true)?;
    // The producer has a thread of its own; the other client threads share the
    // consumers.
    let threads = opts.threads.saturating_sub(1).max(1).min(opts.connections);
    let mut groups: Vec<Vec<Consumer>> = (0..threads).map(|_| Vec::new()).collect();
    for c in consumers { groups[c.slot % threads].push(c); }
    let shared = Arc::new(Shared {
        received: (0..opts.connections).map(|_| Counter(AtomicU64::new(0))).collect(),
        cancelled: AtomicBool::new(false),
        stall,
    });
    let ctx = Arc::new(Context { payload_len: opts.value.len(), depth: opts.pipeline });
    let barrier = Arc::new(Barrier::new(threads + 2));
    let start = Arc::new(OnceLock::new());
    let mut handles = Vec::new();
    for group in groups {
        let (shared, barrier, start) = (shared.clone(), barrier.clone(), start.clone());
        handles.push(thread::spawn(move || {
            barrier.wait();
            let result = consumer_worker(group, *start.get().unwrap(), &shared);
            if result.is_err() { shared.cancelled.store(true, Ordering::Relaxed); }
            result
        }));
    }
    let producing = {
        let (ctx, shared, barrier, start) = (ctx.clone(), shared.clone(), barrier.clone(), start.clone());
        thread::spawn(move || {
            barrier.wait();
            let result = producer_worker(producer, &ctx, *start.get().unwrap(), run_for, &shared);
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
    let produced = match producing.join().unwrap_or_else(|_| Err("worker panicked".into())) {
        Ok(r) => Some(r),
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
    let (producer_accounting, pushed) = produced.unwrap();
    if pushed == 0 { return Err("no element was pushed".into()); }
    let mut histogram = BTreeMap::new();
    for r in &results {
        for (&us, &n) in &r.histogram { *histogram.entry(us).or_insert(0) += n; }
    }
    if histogram.values().sum::<u64>() != pushed {
        return Err("the consumers received a different number of elements than were pushed".into());
    }
    let end = results.iter().map(|r| r.last_message).max().unwrap();
    let consumers_work = results.iter().map(|r| r.accounting.work).sum::<Duration>();
    let consumers_total = results.iter().map(|r| r.accounting.total).sum::<Duration>();
    Ok(Measured {
        pushed,
        seconds: end as f64 / 1e9,
        p50: percentile(&histogram, pushed, 50),
        p99: percentile(&histogram, pushed, 99),
        work_fraction: work_fraction(consumers_work, consumers_total)
            .max(work_fraction(producer_accounting.work, producer_accounting.total)),
    })
}

pub(super) fn run(opts: &Options) -> Result<()> {
    let seconds = opts.seconds.ok_or("the blocking-handoff workload takes --seconds")?;
    if !opts.warmup.is_zero() { measure(opts, opts.warmup, TIMEOUT)?; }
    let measured = measure(opts, seconds, TIMEOUT)?;
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
    use std::sync::atomic::AtomicUsize;
    use std::sync::{mpsc, Mutex};

    // The payload length every test pushes.
    const PAYLOAD: usize = 24;

    #[derive(Clone, Copy, PartialEq)]
    enum Fault { None, Unsupported, Refused, ServeTwice, LoseOne }

    fn frame(slot: usize, payload: &[u8]) -> Vec<u8> {
        let mut frame = Reply::new(slot, PAYLOAD).head;
        frame.extend_from_slice(payload);
        frame.extend_from_slice(b"\r\n");
        frame
    }

    // What a run did to the fake server.
    #[derive(Default)]
    struct Counts {
        // Pushes that reached a key before the server had delivered every
        // earlier element of it, which the client must never send.
        pushes_before_delivery: AtomicUsize,
        // Elements `Fault::LoseOne` did not deliver.
        lost: AtomicUsize,
        // Consumers that received the drain.
        drained: AtomicUsize,
        // Bytes a consumer sent after the drain: a BLPOP the server would
        // keep blocked when the connection closes.
        after_drain: AtomicUsize,
    }

    // A server with the behaviour the client relies on: a consumer's BLPOP
    // holds its connection until an LPUSH to its key sends it the element as
    // the two-element reply, and every LPUSH answers with the list length 1.
    fn serve(listener: TcpListener, consumers: usize, fault: Fault, counts: Arc<Counts>) -> thread::JoinHandle<()> {
        thread::spawn(move || {
            let mut senders = Vec::new();
            let delivered = Arc::new(Mutex::new(vec![0u64; consumers]));
            let mut workers = Vec::new();
            let mut refused = Vec::new();
            for slot in 0..consumers {
                let (mut socket, _) = listener.accept().unwrap();
                socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                super::super::tests::expect_command(&mut socket, &[b"BLPOP", &key(slot), b"0"]);
                // Every consumer's BLPOP is refused; the client stops at the
                // first reply it reads and never connects a producer.
                if fault == Fault::Unsupported {
                    socket.write_all(b"-ERR unknown command 'BLPOP', with args beginning with: 'bench:q:0' '0' \r\n").unwrap();
                    refused.push(socket);
                    continue;
                }
                if fault == Fault::Refused {
                    socket.write_all(b"-NOPERM no permission\r\n").unwrap();
                    refused.push(socket);
                    continue;
                }
                let (tx, rx) = mpsc::channel::<Vec<u8>>();
                senders.push(tx);
                let (delivered, counts) = (delivered.clone(), counts.clone());
                workers.push(thread::spawn(move || {
                    let mut drained = false;
                    while let Ok(payload) = rx.recv() {
                        // The push is answered and the element is dropped: the
                        // consumer is left blocked on a key that holds nothing.
                        if fault == Fault::LoseOne && slot == 0 && payload[..8] == 2u64.to_le_bytes() {
                            counts.lost.fetch_add(1, Ordering::Relaxed);
                            continue;
                        }
                        let reply = frame(slot, &payload);
                        let drain = payload[..8] == DRAIN.to_le_bytes();
                        // Counted before the write, so that a push sent after
                        // the consumer received the element finds it counted.
                        if !drain { delivered.lock().unwrap()[slot] += 1; }
                        // A client that has failed has closed its sockets.
                        if socket.write_all(&reply).is_err() { return; }
                        if fault == Fault::ServeTwice && slot == 0 && payload[..8] == 2u64.to_le_bytes() {
                            let _ = socket.write_all(&reply);
                        }
                        // The consumer does not block again.
                        if drain {
                            counts.drained.fetch_add(1, Ordering::Relaxed);
                            drained = true;
                            break;
                        }
                        // The consumer blocks again.
                        let mut expected = Vec::new();
                        command(&mut expected, &[b"BLPOP", &key(slot), b"0"]);
                        let mut got = vec![0; expected.len()];
                        if socket.read_exact(&mut got).is_err() { return; }
                        assert_eq!(got, expected);
                    }
                    // A server does not close its blocked clients when the
                    // producer goes: wait for the client to close them, or a
                    // consumer that has yet to see the run end could meet the
                    // close first.
                    let mut sink = [0; 64];
                    while let Ok(n) = socket.read(&mut sink) {
                        if n == 0 { break; }
                        if drained { counts.after_drain.fetch_add(n, Ordering::Relaxed); }
                    }
                }));
            }
            if !refused.is_empty() {
                // Wait for the client to close the sockets, as a server would.
                let mut sink = [0; 64];
                for socket in &mut refused {
                    while matches!(socket.read(&mut sink), Ok(n) if n > 0) {}
                }
                return;
            }
            let (mut producer, _) = listener.accept().unwrap();
            let mut pushed = vec![0u64; consumers];
            let (mut buffer, mut chunk) = (Vec::new(), [0; 4096]);
            'serving: loop {
                match producer.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => buffer.extend_from_slice(&chunk[..n]),
                }
                // *3 $5 LPUSH $<key length> <key> $<payload length> <payload>
                loop {
                    let Some(end) = buffer.windows(2).position(|w| w == b"\r\n") else { break };
                    assert_eq!(&buffer[..end], b"*3");
                    let mut at = end + 2;
                    let mut fields = Vec::new();
                    let mut complete = true;
                    for _ in 0..3 {
                        let Some(e) = buffer[at..].windows(2).position(|w| w == b"\r\n") else { complete = false; break };
                        let length: usize = std::str::from_utf8(&buffer[at + 1..at + e]).unwrap().parse().unwrap();
                        let (from, to) = (at + e + 2, at + e + 2 + length);
                        if buffer.len() < to + 2 { complete = false; break; }
                        fields.push(buffer[from..to].to_vec());
                        at = to + 2;
                    }
                    if !complete { break; }
                    buffer.drain(..at);
                    assert_eq!(fields[0], b"LPUSH");
                    let slot: usize = String::from_utf8(fields[1][8..].to_vec()).unwrap().parse().unwrap();
                    let number = u64::from_le_bytes(fields[2][..8].try_into().unwrap());
                    if number != DRAIN {
                        assert_eq!(number, pushed[slot]);
                        // The consumer has received element `number - 1` before
                        // this push, and the server counted it before writing it.
                        if delivered.lock().unwrap()[slot] != number {
                            counts.pushes_before_delivery.fetch_add(1, Ordering::Relaxed);
                        }
                        pushed[slot] += 1;
                    }
                    // A failed client has closed its sockets and the workers.
                    let _ = senders[slot].send(fields[2].clone());
                    if producer.write_all(b":1\r\n").is_err() { break 'serving; }
                }
            }
            drop(senders);
            for w in workers { let _ = w.join(); }
        })
    }

    fn options(port: u16, consumers: usize, pipeline: usize) -> Options {
        let mut opts = super::super::tests::options(port, "blocking-handoff");
        opts.connections = consumers;
        opts.threads = 3;
        opts.requests = None;
        opts.seconds = Some(Duration::from_millis(300));
        opts.value = vec![b'x'; PAYLOAD];
        opts.pipeline = pipeline;
        opts
    }

    // The result of the run and what it did to the server. Only the run that
    // loses an element shortens the stall limit, so that a healthy run on a
    // loaded machine does not meet it.
    fn run_against(fault: Fault, consumers: usize, pipeline: usize) -> (Result<Measured>, Arc<Counts>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let opts = options(listener.local_addr().unwrap().port(), consumers, pipeline);
        let counts = Arc::new(Counts::default());
        let server = serve(listener, consumers, fault, counts.clone());
        let stall = if fault == Fault::LoseOne { Duration::from_secs(1) } else { TIMEOUT };
        let result = measure(&opts, opts.seconds.unwrap(), stall);
        server.join().unwrap();
        (result, counts)
    }

    #[test]
    fn every_element_reaches_its_consumer_and_no_push_finds_a_backlog() {
        for pipeline in [1, 4] {
            let (measured, counts) = run_against(Fault::None, 6, pipeline);
            let measured = measured.unwrap();
            assert!(measured.pushed > 0);
            assert!(measured.seconds > 0.0 && measured.p50 > 0.0 && measured.p99 >= measured.p50);
            assert!((0.0..=1.0).contains(&measured.work_fraction));
            assert_eq!(counts.pushes_before_delivery.load(Ordering::Relaxed), 0);
            assert_eq!(counts.lost.load(Ordering::Relaxed), 0);
        }
    }

    #[test]
    fn a_run_leaves_no_consumer_blocked() {
        // Every consumer receives the drain and sends nothing after it, so no
        // BLPOP is left for a server that keeps the commands of closed
        // connections to serve to a connection that is gone.
        for pipeline in [1, 4] {
            let (measured, counts) = run_against(Fault::None, 6, pipeline);
            measured.unwrap();
            assert_eq!(counts.drained.load(Ordering::Relaxed), 6);
            assert_eq!(counts.after_drain.load(Ordering::Relaxed), 0);
        }
    }

    #[test]
    fn a_repeated_element_fails_the_run() {
        let (result, _) = run_against(Fault::ServeTwice, 3, 1);
        let error = result.err().unwrap();
        assert!(error.to_string().contains("consumer 0 received element 2 where 3 was due"), "{error}");
    }

    #[test]
    fn a_lost_element_fails_the_run() {
        // The producer never pushes to a key whose consumer is behind, so after
        // the loss the consumers go quiet and the run ends in the stall check.
        // Were there none, the run would never end: the watchdog fails the
        // test instead of leaving it hanging.
        let (done, outcome) = mpsc::channel();
        thread::spawn(move || { let _ = done.send(run_against(Fault::LoseOne, 3, 1)); });
        let (result, counts) = outcome.recv_timeout(Duration::from_secs(20))
            .expect("the run neither finished nor failed");
        assert_eq!(counts.lost.load(Ordering::Relaxed), 1);
        let error = result.err().unwrap();
        assert!(error.downcast_ref::<Stalled>().is_some(), "{error}");
        assert!(error.to_string().starts_with("no element for 1 seconds"), "{error}");
    }

    #[test]
    fn unknown_blpop_is_reported_as_unsupported() {
        let (result, _) = run_against(Fault::Unsupported, 2, 1);
        assert!(result.err().unwrap().downcast_ref::<Unsupported>().is_some());
    }

    #[test]
    fn other_refusals_fail() {
        let (result, _) = run_against(Fault::Refused, 2, 1);
        let error = result.err().unwrap();
        assert!(error.downcast_ref::<Unsupported>().is_none());
        assert!(error.to_string().contains("NOPERM"), "{error}");
    }

    #[test]
    fn a_ready_key_is_one_whose_consumer_has_everything_pushed_to_it() {
        let received = [3, 2, 5, 0];
        let pushed = [3, 3, 5, 0];
        let ready = |cursor, depth| ready_keys(|k| received[k], &pushed, cursor, depth);
        // Key 1 has an element its consumer has not received.
        assert_eq!(ready(0, 4), vec![0, 2, 3]);
        // From the cursor on, wrapping, and at most `depth`.
        assert_eq!(ready(2, 2), vec![2, 3]);
        assert_eq!(ready(3, 3), vec![3, 0, 2]);
        assert_eq!(ready(0, 1), vec![0]);
    }

    #[test]
    fn replies_parse_when_fragmented_and_nothing_else_does() {
        let reply = Reply::new(7, PAYLOAD);
        let mut payload = vec![b'x'; PAYLOAD];
        payload[..8].copy_from_slice(&5u64.to_le_bytes());
        payload[8..HEADER].copy_from_slice(&9u64.to_le_bytes());
        let whole = frame(7, &payload);
        for n in 0..whole.len() { assert_eq!(reply.parse(&whole[..n]).unwrap(), None, "{n}"); }
        assert_eq!(reply.parse(&whole).unwrap(), Some(whole.len()));
        // A second reply after the first is not part of it.
        let two = [whole.clone(), whole.clone()].concat();
        assert_eq!(reply.parse(&two).unwrap(), Some(whole.len()));
        assert_eq!(reply.decode(&whole).unwrap(), (5, 9));
        // Another consumer's key, another length, no closing CRLF, an error
        // reply and an altered payload are all errors.
        let other_key = frame(8, &payload);
        let other_length = frame_with_length(7, PAYLOAD + 1, &[payload.clone(), vec![b'x']].concat());
        let mut no_crlf = whole.clone();
        *no_crlf.last_mut().unwrap() = b'!';
        for bad in [other_key, other_length, no_crlf, b"-ERR boom\r\n".to_vec()] {
            assert!(reply.parse(&bad).is_err(), "{:?}", String::from_utf8_lossy(&bad));
        }
        let mut altered = payload.clone();
        altered[HEADER] = b'y';
        assert!(reply.decode(&frame(7, &altered)).is_err());
    }

    fn frame_with_length(slot: usize, length: usize, payload: &[u8]) -> Vec<u8> {
        let mut frame = Reply::new(slot, length).head;
        frame.extend_from_slice(payload);
        frame.extend_from_slice(b"\r\n");
        frame
    }
}
