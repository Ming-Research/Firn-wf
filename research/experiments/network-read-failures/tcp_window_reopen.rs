//! Shows whether a loopback TCP connection resumes at once when a reader that
//! left it full for a long time starts to read, with no server of ours in it.
//!
//! Each pair of sockets follows the soft-limit case: a writer thread writes a
//! 14 MiB frame with one blocking `write_all` on a socket with TCP_NODELAY,
//! as a server's send does, and the reader waits without reading, then reads
//! the frame as the case does (its first 40 bytes one at a time, the rest into
//! one buffer) with one-second bounds; then a second frame the same way. For
//! a second without a byte it prints what the kernel says of both sockets
//! (`ss -tinm`). README.md beside this file gives the question and the runs.
//!
//!     tcp_window_reopen SECONDS INTERVAL_MS WAITS
//!
//! Starts a pair every INTERVAL_MS while one can still end within SECONDS.
//! WAITS is how many seconds each frame of a pair waits unread, separated by
//! commas: `35,33` is the case's two frames.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

const HEAD: usize = 40;
const FRAME: usize = 14 * 1024 * 1024 + HEAD;

/// The counters of /proc/net/netstat's TcpExt line that say what TCP did
/// when a receiver was full.
const COUNTERS: [&str; 12] = [
    "PruneCalled",
    "RcvPruned",
    "TCPRcvCollapsed",
    "TCPRcvQDrop",
    "TCPBacklogDrop",
    "TCPZeroWindowDrop",
    "TCPWantZeroWindowAdv",
    "TCPToZeroWindowAdv",
    "TCPFromZeroWindowAdv",
    "TCPTimeouts",
    "TCPLostRetransmit",
    "TCPMemoryPressures",
];

fn counters() -> BTreeMap<String, u64> {
    let text = std::fs::read_to_string("/proc/net/netstat").unwrap_or_default();
    let lines: Vec<&str> = text.lines().filter(|line| line.starts_with("TcpExt:")).collect();
    let mut found = BTreeMap::new();
    if let [names, values] = lines.as_slice() {
        for (name, value) in names.split_whitespace().zip(values.split_whitespace()).skip(1) {
            if COUNTERS.contains(&name) {
                found.insert(name.to_owned(), value.parse().unwrap_or(0));
            }
        }
    }
    found
}

fn sockets(pair: usize, port: u16) {
    let port_text = format!(":{port}");
    match std::process::Command::new("ss")
        .args(["-tinm", "sport", "=", &port_text, "or", "dport", "=", &port_text])
        .output()
    {
        Ok(output) => {
            for line in String::from_utf8_lossy(&output.stdout).lines().take(12) {
                println!("DUMP pair={pair} {line}");
            }
        }
        Err(error) => println!("DUMP pair={pair} ss failed: {error}"),
    }
}

struct FrameRead {
    milliseconds: u128,
    longest_wait_ms: u128,
    seconds_without_a_byte: u32,
}

fn read_frame(reader: &mut TcpStream, body: &mut [u8], pair: usize, frame: usize, port: u16) -> FrameRead {
    let start = Instant::now();
    let mut read = 0;
    let mut longest = Duration::ZERO;
    let mut last = start;
    let mut waited = 0_u32;
    let mut without = 0_u32;
    while read < body.len() {
        let end = if read < HEAD { read + 1 } else { body.len() };
        match reader.read(&mut body[read..end]) {
            Ok(0) => panic!("pair {pair}: the writer closed after {read} bytes"),
            Ok(count) => {
                let now = Instant::now();
                longest = longest.max(now - last);
                last = now;
                read += count;
                waited = 0;
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) if matches!(error.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                waited += 1;
                without += 1;
                println!(
                    "STALL pair={pair} frame={frame} second={waited} read={read} of={} since_read_start_ms={}",
                    body.len(),
                    start.elapsed().as_millis()
                );
                if matches!(waited, 1 | 5 | 12) {
                    sockets(pair, port);
                }
                assert!(waited < 90, "pair {pair}: no byte for 90 s after {read} bytes");
            }
            Err(error) => panic!("pair {pair}: {error}"),
        }
    }
    FrameRead {
        milliseconds: start.elapsed().as_millis(),
        longest_wait_ms: longest.as_millis(),
        seconds_without_a_byte: without,
    }
}

fn pair(number: usize, payload: Arc<Vec<u8>>, waits: Vec<u64>, sample: bool) -> Vec<FrameRead> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("listen");
    let port = listener.local_addr().expect("address").port();
    let mut reader = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    let (mut writer, _) = listener.accept().expect("accept");
    drop(listener);
    writer.set_nodelay(true).expect("no delay");
    reader.set_read_timeout(Some(Duration::from_secs(1))).expect("bound the reads");
    let (go, gone) = mpsc::channel::<()>();
    let writing = std::thread::spawn(move || {
        while gone.recv().is_ok() {
            writer.write_all(&payload).expect("write the frame");
        }
    });
    let mut body = vec![0_u8; FRAME];
    let mut reads = Vec::new();
    for (index, wait) in waits.iter().enumerate() {
        go.send(()).expect("start the frame");
        std::thread::sleep(Duration::from_secs(*wait));
        if sample && index + 1 == waits.len() {
            println!("SAMPLE pair={number}: a frame that has waited {wait} s, before its read");
            sockets(number, port);
        }
        let read = read_frame(&mut reader, &mut body, number, index + 1, port);
        println!(
            "FRAME pair={number} frame={} waited_s={wait} ms={} longest_wait_ms={} seconds_without_a_byte={}",
            index + 1,
            read.milliseconds,
            read.longest_wait_ms,
            read.seconds_without_a_byte
        );
        reads.push(read);
        // As the case does between its frames.
        std::thread::sleep(Duration::from_secs(2));
    }
    drop(go);
    writing.join().expect("the writer");
    reads
}

fn spread(values: &mut Vec<u128>) -> String {
    if values.is_empty() {
        return "none".to_owned();
    }
    values.sort_unstable();
    let at = |share: f64| values[((values.len() as f64 * share) as usize).min(values.len() - 1)];
    format!(
        "n={} min={} median={} p90={} p99={} max={}",
        values.len(),
        values[0],
        at(0.5),
        at(0.9),
        at(0.99),
        values[values.len() - 1]
    )
}

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let [seconds, interval, waits] = arguments.as_slice() else {
        eprintln!("usage: tcp_window_reopen SECONDS INTERVAL_MS WAITS");
        std::process::exit(2);
    };
    let seconds: u64 = seconds.parse().expect("SECONDS is a whole number");
    let interval: u64 = interval.parse().expect("INTERVAL_MS is a whole number");
    let waits: Vec<u64> = waits.split(',').map(|text| text.parse().expect("a wait in seconds")).collect();
    let one_pair = waits.iter().sum::<u64>() + 2 * waits.len() as u64 + 5;
    let payload = Arc::new(vec![b'x'; FRAME]);
    let before = counters();
    let begun = Instant::now();
    let mut pairs = Vec::new();
    while begun.elapsed().as_secs() + one_pair < seconds {
        let (number, payload, waits) = (pairs.len() + 1, Arc::clone(&payload), waits.clone());
        pairs.push(std::thread::spawn(move || pair(number, payload, waits, number == 1)));
        std::thread::sleep(Duration::from_millis(interval));
    }
    let mut failed = 0;
    let mut frames: BTreeMap<usize, (Vec<u128>, Vec<u128>, usize)> = BTreeMap::new();
    for thread in pairs {
        match thread.join() {
            Ok(reads) => {
                for (index, read) in reads.iter().enumerate() {
                    let (took, longest, stalled) = frames.entry(index + 1).or_default();
                    took.push(read.milliseconds);
                    longest.push(read.longest_wait_ms);
                    *stalled += usize::from(read.seconds_without_a_byte > 0);
                }
            }
            Err(_) => failed += 1,
        }
    }
    println!("tcp-window-reopen: pairs failed={failed} after {} s", begun.elapsed().as_secs());
    for (frame, (took, longest, stalled)) in frames.iter_mut() {
        println!("tcp-window-reopen: frame {frame} read ms: {}", spread(took));
        println!("tcp-window-reopen: frame {frame} longest wait for a byte ms: {}", spread(longest));
        println!("tcp-window-reopen: frame {frame} reads with a second or more without a byte: {stalled}");
    }
    let after = counters();
    let changes: Vec<String> = after
        .iter()
        .map(|(name, value)| format!("{name}=+{}", value - before.get(name).copied().unwrap_or(0)))
        .collect();
    println!("tcp-window-reopen: TcpExt counters over the run: {}", changes.join(" "));
}
