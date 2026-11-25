//! Shows whether a loopback TCP connection resumes at once when a reader that
//! left it full for a long time starts to read, with no server of ours in it.
//!
//! Each pair of sockets follows the soft-limit case: a writer thread writes a
//! 14 MiB frame with one blocking `write_all` on a socket with TCP_NODELAY,
//! as a server's send does, and the reader waits without reading, then reads
//! the frame as the case does (its first 40 bytes one at a time, the rest into
//! one buffer) with one-second bounds; then a second frame the same way. For
//! a second without a byte it prints what the kernel says of both sockets:
//! fields of their `struct tcp_info` and their memory counters, read with
//! getsockopt, so that it needs no other program and can be a machine's only
//! one. README.md beside this file gives the question and the runs.
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

unsafe extern "C" {
    fn getsockopt(socket: i32, level: i32, name: i32, value: *mut u8, length: *mut u32) -> i32;
}

/// What the kernel says of one socket. The offsets are those of Linux's
/// `struct tcp_info` (include/uapi/linux/tcp.h) and of SO_MEMINFO's array.
fn kernel_state(socket: &TcpStream) -> String {
    use std::os::fd::AsRawFd;
    const IPPROTO_TCP: i32 = 6;
    const TCP_INFO: i32 = 11;
    const SOL_SOCKET: i32 = 1;
    const SO_MEMINFO: i32 = 55;
    let descriptor = socket.as_raw_fd();
    let mut info = [0_u8; 256];
    let mut length = info.len() as u32;
    // SAFETY: the buffer and its length outlive the call.
    let read = unsafe { getsockopt(descriptor, IPPROTO_TCP, TCP_INFO, info.as_mut_ptr(), &mut length) };
    if read != 0 {
        return format!("tcp_info unreadable: {}", std::io::Error::last_os_error());
    }
    let word = |at: usize| u32::from_ne_bytes([info[at], info[at + 1], info[at + 2], info[at + 3]]);
    let long = |at: usize| u64::from(word(at)) | (u64::from(word(at + 4)) << 32);
    let mut memory = [0_u8; 36];
    let mut memory_length = memory.len() as u32;
    // SAFETY: as above.
    let counted = unsafe { getsockopt(descriptor, SOL_SOCKET, SO_MEMINFO, memory.as_mut_ptr(), &mut memory_length) };
    let counter = |index: usize| {
        let at = 4 * index;
        u32::from_ne_bytes([memory[at], memory[at + 1], memory[at + 2], memory[at + 3]])
    };
    format!(
        "backoff={} probes={} retransmits={} rto_us={} unacked={} lost={} last_data_sent_ms={} last_data_recv_ms={} \
         last_ack_recv_ms={} rcv_ssthresh={} snd_cwnd={} rcv_space={} total_retrans={} bytes_acked={} \
         bytes_received={} notsent={} snd_wnd={} rcv_wnd={} meminfo={} rmem_alloc={} rcvbuf={} sndbuf={} \
         wmem_queued={} drops={}",
        info[4],
        info[3],
        info[2],
        word(8),
        word(24),
        word(32),
        word(44),
        word(52),
        word(56),
        word(64),
        word(80),
        word(96),
        word(100),
        long(120),
        long(128),
        word(144),
        word(228),
        word(232),
        if counted == 0 { "read" } else { "unreadable" },
        counter(0),
        counter(1),
        counter(3),
        counter(5),
        counter(8),
    )
}

fn sockets(pair: usize, reader: &TcpStream, writer: &TcpStream) {
    println!("DUMP pair={pair} reader: {}", kernel_state(reader));
    println!("DUMP pair={pair} writer: {}", kernel_state(writer));
}

/// `ss`'s account of both sockets, where the machine has it, beside which
/// the fields above can be checked.
fn sockets_by_ss(pair: usize, port: u16) {
    let port_text = format!(":{port}");
    if let Ok(output) = std::process::Command::new("ss")
        .args(["-tinm", "sport", "=", &port_text, "or", "dport", "=", &port_text])
        .output()
    {
        for line in String::from_utf8_lossy(&output.stdout).lines().take(8) {
            println!("DUMP pair={pair} ss: {}", line.trim_end());
        }
    }
}

struct FrameRead {
    milliseconds: u128,
    longest_wait_ms: u128,
    seconds_without_a_byte: u32,
}

fn read_frame(reader: &mut TcpStream, writer: &TcpStream, body: &mut [u8], pair: usize, frame: usize) -> FrameRead {
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
                    sockets(pair, reader, writer);
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
    let writer_view = writer.try_clone().expect("a second handle of the writer's socket");
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
            sockets(number, &reader, &writer_view);
            sockets_by_ss(number, port);
        }
        let read = read_frame(&mut reader, &writer_view, &mut body, number, index + 1);
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
