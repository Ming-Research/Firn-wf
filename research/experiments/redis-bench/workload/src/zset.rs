//! Sorted-set workloads of board item 171, whose definitions
//! `research/investigations/firn/zset-algebra-bench.md` fixes before any
//! measurement:
//!
//! * `zunionstore` and `zinterstore`: `ZUNIONSTORE zd:<c> 2 <a> <b> WEIGHTS 1 2`
//!   (or `ZINTERSTORE`) over two sorted sets of 1000 members each, 500 of them in
//!   both, so that the union holds 1500 members and the intersection 500; the
//!   destination belongs to the connection `<c>`, and the sources are either one
//!   pair every connection reads (`--layout shared`) or one pair for each
//!   connection (`--layout per-connection`).
//! * `zmpop`: `ZMPOP 2 zp:<i> zp:<j> MIN COUNT <n>` over two distinct keys drawn
//!   from 100, each holding `--prefill` members before the run, so that no
//!   reply is empty; `--pop-count` is n, 1 unless the cell says otherwise.
//!
//! Every connection sends `--pipeline` commands and waits for all their
//! replies, as redis-benchmark -P does, and a batch's latency runs from its
//! first byte written to its last reply read. Each reply is checked: the size a
//! store command answers, and for ZMPOP an array of the key and `n` pairs; a
//! null array, an error or any other size fails the run for every server alike.
//! The sets are written before the connections open, outside the measured span.
use super::*;

pub(super) const NAMES: &[&str] = &["zunionstore", "zinterstore", "zmpop"];

/// Members of each source set, and of the union and the intersection.
const SOURCE_MEMBERS: u64 = 1000;
const OVERLAP: u64 = 500;
pub(super) const UNION_SIZE: u64 = 2 * SOURCE_MEMBERS - OVERLAP;
pub(super) const INTER_SIZE: u64 = OVERLAP;
/// Keys ZMPOP draws from, and the pairs of one ZADD that fills a set.
pub(super) const POP_KEYS: u64 = 100;
const PAIRS_PER_ZADD: u64 = 500;
/// Commands written before their replies are read when a set is filled.
const SETUP_BATCH: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Layout {
    Shared,
    PerConnection,
}

impl Layout {
    pub(super) fn parse(text: &str) -> Result<Self> {
        match text {
            "shared" => Ok(Self::Shared),
            "per-connection" => Ok(Self::PerConnection),
            _ => Err("--layout is shared or per-connection".into()),
        }
    }
}

pub(super) fn is_zset(workload: &str) -> bool {
    NAMES.contains(&workload)
}

fn sources(layout: Layout, id: usize) -> (String, String) {
    match layout {
        Layout::Shared => ("za".to_owned(), "zb".to_owned()),
        Layout::PerConnection => (format!("za:{id}"), format!("zb:{id}")),
    }
}

/// The commands of one batch, `opts.pipeline` of them, as connection `id` draws
/// them, returning how many replies follow.
pub(super) fn request(out: &mut Vec<u8>, opts: &Options, rng: &mut u64, id: usize) -> usize {
    for _ in 0..opts.pipeline {
        match opts.workload.as_str() {
            "zunionstore" | "zinterstore" => {
                let name: &[u8] = if opts.workload == "zunionstore" { b"ZUNIONSTORE" } else { b"ZINTERSTORE" };
                let (a, b) = sources(opts.layout, id);
                let destination = format!("zd:{id}");
                command(out, &[name, destination.as_bytes(), b"2", a.as_bytes(), b.as_bytes(),
                    b"WEIGHTS", b"1", b"2"]);
            }
            "zmpop" => {
                let first = random_key(rng, POP_KEYS);
                let second = (first + 1 + random_key(rng, POP_KEYS - 1)) % POP_KEYS;
                let (first, second) = (format!("zp:{first}"), format!("zp:{second}"));
                let count = opts.pop_count.to_string();
                command(out, &[b"ZMPOP", b"2", first.as_bytes(), second.as_bytes(), b"MIN", b"COUNT", count.as_bytes()]);
            }
            _ => unreachable!(),
        }
    }
    opts.pipeline
}

// The end of the bulk string that starts at `at`, or an error.
fn bulk_end(input: &[u8], at: usize) -> Result<usize> {
    if input.get(at) != Some(&b'$') { return Err("a bulk string was expected".into()); }
    let line = input[at..].windows(2).position(|w| w == b"\r\n").ok_or("an unterminated header")?;
    let length: usize = std::str::from_utf8(&input[at + 1..at + line])?.parse()?;
    let end = at + line + 2 + length + 2;
    if input.len() < end { return Err("a short bulk string".into()); }
    Ok(end)
}

/// Whether one reply is the one this workload's command must answer.
pub(super) fn check_reply(opts: &Options, reply: &[u8]) -> Result<()> {
    match opts.workload.as_str() {
        "zunionstore" | "zinterstore" => {
            let size = if opts.workload == "zunionstore" { UNION_SIZE } else { INTER_SIZE };
            if reply != format!(":{size}\r\n").as_bytes() {
                return Err(format!("{} must answer {size}, not {:?}", opts.workload.to_uppercase(),
                    String::from_utf8_lossy(reply)).into());
            }
        }
        "zmpop" => {
            // *2, the key, then *n and n pairs of a member and its score.
            let key_end = if reply.starts_with(b"*2\r\n") { bulk_end(reply, 4).ok() } else { None };
            let header = format!("*{}\r\n", opts.pop_count);
            let mut at = match key_end {
                Some(end) if reply[end..].starts_with(header.as_bytes()) => end + header.len(),
                _ => return Err(format!("ZMPOP must answer a key and {} pair(s), not {:?}",
                    opts.pop_count, String::from_utf8_lossy(&reply[..reply.len().min(60)])).into()),
            };
            for _ in 0..opts.pop_count {
                if !reply[at..].starts_with(b"*2\r\n") { return Err("a ZMPOP pair is an array of two".into()); }
                at = bulk_end(reply, at + 4)?;
                at = bulk_end(reply, at)?;
            }
            if at != reply.len() { return Err("extra bytes after the ZMPOP reply".into()); }
        }
        _ => unreachable!(),
    }
    Ok(())
}

// Writes commands in batches and checks that each is answered by an integer,
// the setup connection's way of writing the sets.
struct Writer {
    socket: TcpStream,
    input: Vec<u8>,
    out: Vec<u8>,
    pending: Vec<u64>,
}

impl Writer {
    fn new(port: u16) -> Result<Self> {
        Ok(Self { socket: connect(port)?, input: Vec::new(), out: Vec::new(), pending: Vec::new() })
    }

    // Queues one command whose reply must be the integer `expected`, or any
    // integer when it is None.
    fn queue(&mut self, args: &[&[u8]], expected: Option<u64>) -> Result<()> {
        command(&mut self.out, args);
        self.pending.push(expected.unwrap_or(u64::MAX));
        if self.pending.len() >= SETUP_BATCH { self.flush()?; }
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        self.socket.write_all(&self.out)?;
        self.out.clear();
        for expected in std::mem::take(&mut self.pending) {
            let size = loop {
                if let Some(size) = reply(&self.input, 0)? { break size; }
                read_more(&mut self.socket, &mut self.input)?;
            };
            let text = String::from_utf8_lossy(&self.input[..size]).into_owned();
            let value = text.strip_prefix(':').and_then(|n| n.trim_end().parse::<u64>().ok())
                .ok_or_else(|| format!("setup expected an integer reply, got {text:?}"))?;
            if expected != u64::MAX && value != expected {
                return Err(format!("setup expected {expected}, got {value}").into());
            }
            self.input.drain(..size);
        }
        if !self.input.is_empty() { return Err("extra setup reply".into()); }
        Ok(())
    }
}

// ZADD key score member [score member ...] for members first..first+count, each
// scored by `scale` times its index.
fn fill_set(writer: &mut Writer, key: &str, first: u64, count: u64, scale: u64) -> Result<()> {
    let mut next = first;
    while next < first + count {
        let pairs = PAIRS_PER_ZADD.min(first + count - next);
        let words: Vec<String> = (next..next + pairs)
            .flat_map(|index| [(index * scale).to_string(), format!("m{index}")]).collect();
        let mut args: Vec<&[u8]> = Vec::new();
        args.push(b"ZADD");
        args.push(key.as_bytes());
        args.extend(words.iter().map(|word| word.as_bytes()));
        writer.queue(&args, Some(pairs))?;
        next += pairs;
    }
    Ok(())
}

/// Writes the sets the workload reads, over a connection of its own that ends
/// before any measured connection opens: leftovers of an earlier run on the same
/// server are deleted first.
pub(super) fn setup(opts: &Options) -> Result<()> {
    let mut writer = Writer::new(opts.port)?;
    if opts.workload == "zmpop" {
        for key in 0..POP_KEYS {
            let name = format!("zp:{key}");
            writer.queue(&[b"DEL", name.as_bytes()], None)?;
            fill_set(&mut writer, &name, 0, opts.prefill, 1)?;
        }
        return writer.flush();
    }
    for key in ["za", "zb"] { writer.queue(&[b"DEL", key.as_bytes()], None)?; }
    let sets: Vec<(String, String)> = match opts.layout {
        Layout::Shared => vec![sources(opts.layout, 0)],
        Layout::PerConnection => (0..opts.connections).map(|id| sources(opts.layout, id)).collect(),
    };
    for (a, b) in &sets {
        for key in [a, b] { writer.queue(&[b"DEL", key.as_bytes()], None)?; }
        // The first set holds m0..m999 at scores 0..999, the second m500..m1499
        // at twice their index, so that the intersection is m500..m999.
        fill_set(&mut writer, a, 0, SOURCE_MEMBERS, 1)?;
        fill_set(&mut writer, b, SOURCE_MEMBERS - OVERLAP, SOURCE_MEMBERS, 2)?;
    }
    for id in 0..opts.connections {
        let destination = format!("zd:{id}");
        writer.queue(&[b"DEL", destination.as_bytes()], None)?;
    }
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn options(workload: &str, layout: Layout) -> Options {
        let mut opts = super::super::tests::options(0, workload);
        opts.layout = layout;
        opts.pipeline = 3;
        opts.prefill = 1200;
        opts
    }

    fn words(command: &[Vec<u8>]) -> Vec<String> {
        command.iter().map(|word| String::from_utf8_lossy(word).into_owned()).collect()
    }

    fn built(opts: &Options, id: usize) -> Vec<u8> {
        let mut out = Vec::new();
        let mut rng = 7;
        assert_eq!(request(&mut out, opts, &mut rng, id), opts.pipeline);
        out
    }

    fn expected(commands: &[&[&str]]) -> Vec<u8> {
        let mut out = Vec::new();
        for words in commands {
            let args: Vec<&[u8]> = words.iter().map(|word| word.as_bytes()).collect();
            command(&mut out, &args);
        }
        out
    }

    #[test]
    fn stores_name_the_layout_destination_and_weights() {
        let union = ["ZUNIONSTORE", "zd:4", "2", "za", "zb", "WEIGHTS", "1", "2"];
        assert_eq!(built(&options("zunionstore", Layout::Shared), 4), expected(&[&union, &union, &union]));
        let inter = ["ZINTERSTORE", "zd:4", "2", "za:4", "zb:4", "WEIGHTS", "1", "2"];
        assert_eq!(built(&options("zinterstore", Layout::PerConnection), 4), expected(&[&inter, &inter, &inter]));
    }

    #[test]
    fn pops_name_two_distinct_keys_and_the_count() {
        let mut opts = options("zmpop", Layout::Shared);
        opts.pipeline = 200;
        opts.pop_count = 10;
        let mut out = built(&opts, 0);
        let mut seen = std::collections::HashSet::new();
        let mut commands = 0;
        while !out.is_empty() {
            let size = reply(&out, 0).unwrap().unwrap();
            let text = String::from_utf8_lossy(&out[..size]).into_owned();
            let parts: Vec<&str> = text.split("\r\n").collect();
            // *7, then each argument's length and text.
            assert_eq!((parts[0], parts[2], parts[4]), ("*7", "ZMPOP", "2"), "{text:?}");
            let (first, second) = (parts[6], parts[8]);
            assert!(first.starts_with("zp:") && second.starts_with("zp:") && first != second, "{text:?}");
            for key in [first, second] {
                assert!(key[3..].parse::<u64>().unwrap() < POP_KEYS, "{text:?}");
            }
            assert_eq!((parts[10], parts[12], parts[14]), ("MIN", "COUNT", "10"), "{text:?}");
            seen.insert(first.to_owned());
            commands += 1;
            out.drain(..size);
        }
        assert_eq!(commands, 200);
        assert!(seen.len() > 50, "the first key varies: {}", seen.len());
    }

    #[test]
    fn replies_are_checked_against_what_the_command_must_answer() {
        let union = options("zunionstore", Layout::Shared);
        let inter = options("zinterstore", Layout::Shared);
        check_reply(&union, b":1500\r\n").unwrap();
        check_reply(&inter, b":500\r\n").unwrap();
        for (opts, bad) in [(&union, b":500\r\n".as_slice()), (&union, b":0\r\n".as_slice()),
            (&inter, b":1500\r\n".as_slice()), (&inter, b"+OK\r\n".as_slice())] {
            assert!(check_reply(opts, bad).is_err(), "{bad:?}");
        }
        let mut pop = options("zmpop", Layout::Shared);
        let one = b"*2\r\n$4\r\nzp:7\r\n*1\r\n*2\r\n$2\r\nm0\r\n$1\r\n0\r\n";
        check_reply(&pop, one).unwrap();
        let bad_pops: [&[u8]; 7] = [
            b"*-1\r\n", b"*0\r\n", b"*2\r\n$4\r\nzp:7\r\n*0\r\n",
            b"*2\r\n$4\r\nzp:7\r\n*2\r\n*2\r\n$2\r\nm0\r\n$1\r\n0\r\n*2\r\n$2\r\nm1\r\n$1\r\n1\r\n",
            b"*2\r\n$4\r\nzp:7\r\n*1\r\n*2\r\n$2\r\nm0\r\n$1\r\n0\r\n+extra\r\n",
            b"*2\r\n$4\r\nzp:7\r\n*1\r\n*2\r\n$2\r\nm0\r\n", b"+OK\r\n"];
        for bad in bad_pops {
            assert!(check_reply(&pop, bad).is_err(), "{bad:?}");
        }
        pop.pop_count = 2;
        let two = b"*2\r\n$4\r\nzp:7\r\n*2\r\n*2\r\n$2\r\nm0\r\n$1\r\n0\r\n*2\r\n$2\r\nm1\r\n$1\r\n1\r\n";
        check_reply(&pop, two).unwrap();
        assert!(check_reply(&pop, one).is_err());
    }

    // A server that answers the setup commands as Redis does and every measured
    // command with `answer`, and keeps each command it received.
    fn serve(workload: &'static str, measured: &'static [u8]) -> (u16, Arc<Mutex<Vec<Vec<String>>>>, thread::JoinHandle<()>, usize) {
        use std::net::TcpListener;
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let received = Arc::new(Mutex::new(Vec::new()));
        let log = received.clone();
        let connections = 3;
        let handle = thread::spawn(move || {
            let mut handlers = Vec::new();
            // The setup connection, then the measured ones.
            for _ in 0..=connections {
                let (mut socket, _) = listener.accept().unwrap();
                let log = log.clone();
                handlers.push(thread::spawn(move || {
                    socket.set_read_timeout(Some(TIMEOUT)).unwrap();
                    let mut input = Vec::new();
                    loop {
                        let mut chunk = [0; 65536];
                        match socket.read(&mut chunk) {
                            Ok(0) | Err(_) => return,
                            Ok(n) => input.extend_from_slice(&chunk[..n]),
                        }
                        // Whole commands are arrays of bulk strings.
                        while let Some(size) = reply(&input, 0).unwrap() {
                            let mut parts = Vec::new();
                            let mut at = input[..size].windows(2).position(|w| w == b"\r\n").unwrap() + 2;
                            while at < size {
                                let line = input[at..].windows(2).position(|w| w == b"\r\n").unwrap();
                                let length: usize = std::str::from_utf8(&input[at + 1..at + line]).unwrap().parse().unwrap();
                                parts.push(input[at + line + 2..at + line + 2 + length].to_vec());
                                at += line + 2 + length + 2;
                            }
                            let command = words(&parts);
                            let answer: Vec<u8> = match command[0].as_str() {
                                "DEL" => b":0\r\n".to_vec(),
                                "ZADD" => format!(":{}\r\n", (command.len() - 2) / 2).into_bytes(),
                                name if name == workload.to_uppercase() => measured.to_vec(),
                                _ => b"-ERR unexpected\r\n".to_vec(),
                            };
                            log.lock().unwrap().push(command);
                            input.drain(..size);
                            // A client that failed the run has closed its end.
                            if socket.write_all(&answer).is_err() { return; }
                        }
                    }
                }));
            }
            for handler in handlers { handler.join().unwrap(); }
        });
        (port, received, handle, connections)
    }

    fn run_against(workload: &'static str, layout: Layout, measured: &'static [u8]) -> (Result<()>, Vec<Vec<String>>) {
        let (port, received, handle, connections) = serve(workload, measured);
        let mut opts = super::super::tests::options(port, workload);
        opts.layout = layout;
        opts.pipeline = 4;
        opts.prefill = 1200;
        opts.connections = connections;
        // One worker, so that the error a failed run returns is the first one's.
        opts.threads = 1;
        opts.requests = Some(6);
        let result = super::super::run(opts);
        handle.join().unwrap();
        let commands = received.lock().unwrap().clone();
        (result, commands)
    }

    #[test]
    fn stores_run_with_pipelined_batches_after_the_sets_are_written() {
        for (workload, layout, size) in [("zunionstore", Layout::Shared, b":1500\r\n".as_slice()),
            ("zinterstore", Layout::PerConnection, b":500\r\n".as_slice())] {
            let (result, commands) = run_against(workload, layout, size);
            result.unwrap();
            let stores: Vec<&Vec<String>> = commands.iter().filter(|c| c[0] == workload.to_uppercase()).collect();
            assert_eq!(stores.len(), 6 * 4, "six batches of four commands");
            let sources = if layout == Layout::Shared { 1 } else { 3 };
            // Each set is filled with 1000 members, 500 pairs to a command; the
            // connection that sets up comes before every measured command.
            let adds: Vec<&Vec<String>> = commands.iter().filter(|c| c[0] == "ZADD").collect();
            assert_eq!(adds.len(), sources * 2 * 2, "{commands:?}");
            assert!(adds.iter().all(|c| c.len() == 2 + 2 * 500));
            let first_store = commands.iter().position(|c| c[0] == workload.to_uppercase()).unwrap();
            assert!(commands[..first_store].iter().any(|c| c[0] == "ZADD"));
            assert!(commands[first_store..].iter().all(|c| c[0] == workload.to_uppercase()), "no fill after the run starts");
            for store in stores {
                let id: usize = store[1][3..].parse().unwrap();
                assert!(id < 3 && store[2] == "2" && store[5] == "WEIGHTS" && store[6] == "1" && store[7] == "2", "{store:?}");
                let (a, b) = if layout == Layout::Shared { ("za".to_owned(), "zb".to_owned()) }
                    else { (format!("za:{id}"), format!("zb:{id}")) };
                assert_eq!((&store[3], &store[4]), (&a, &b));
            }
        }
    }

    #[test]
    fn pops_fill_every_key_and_a_wrong_reply_fails_the_run() {
        let pair: &'static [u8] = b"*2\r\n$4\r\nzp:7\r\n*1\r\n*2\r\n$2\r\nm0\r\n$1\r\n0\r\n";
        let (result, commands) = run_against("zmpop", Layout::Shared, pair);
        result.unwrap();
        let adds = commands.iter().filter(|c| c[0] == "ZADD").count();
        assert_eq!(adds as u64, POP_KEYS * 3, "1200 members make three ZADDs of 500, 500 and 200 pairs a key");
        assert_eq!(commands.iter().filter(|c| c[0] == "ZMPOP").count(), 6 * 4);
        // An empty reply, a wrong size and an error each fail the run, naming why.
        for (workload, reply, why) in [
            ("zmpop", b"*-1\r\n".as_slice(), "ZMPOP must answer a key"),
            ("zunionstore", b":500\r\n".as_slice(), "ZUNIONSTORE must answer 1500"),
            ("zinterstore", b":1500\r\n".as_slice(), "ZINTERSTORE must answer 500"),
            ("zinterstore", b"-ERR boom\r\n".as_slice(), "server error: ERR boom"),
        ] {
            let (result, _) = run_against(workload, Layout::Shared, reply);
            let message = result.unwrap_err().to_string();
            assert!(message.contains(why), "{workload}: {message}");
        }
    }
}
