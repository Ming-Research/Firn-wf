//! Shows whether a child's exit can interrupt another thread's timed socket
//! read with EINTR, and whether blocking SIGCHLD in every thread prevents it.
//!
//! One process: a spawner thread starts /bin/true children, and a reader
//! waits in timed reads on an idle loopback connection and counts how each
//! read ends. README.md beside this file gives the question, the predictions
//! and the runs.
//!
//!     sigchld_eintr run MAIN SPAWN SIGCHLD SECONDS
//!     sigchld_eintr child-mask SIGCHLD
//!
//! MAIN is what the process's first thread does: `reads` (it is the reader),
//! `waits` (it joins a thread, as libtest's first thread waits for a result)
//! or `starts-threads` (it starts and joins threads without pause, as
//! libtest's first thread starts one for each test). SPAWN is how the spawner
//! works: `nowait` (starts the next child while the last one is exiting),
//! `wait` (waits for each child before the next) or `fork-nowait` (as
//! `nowait`, through fork and exec instead of posix_spawn). SIGCHLD is `open`
//! or `blocked` (blocked in the first thread before any other exists, so in
//! all of them).

use std::collections::VecDeque;
use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

// glibc's sigset_t on Linux: 1024 bits.
type SignalSet = [u64; 16];

unsafe extern "C" {
    fn sigemptyset(set: *mut SignalSet) -> i32;
    fn sigaddset(set: *mut SignalSet, signal: i32) -> i32;
    fn pthread_sigmask(how: i32, set: *const SignalSet, old: *mut SignalSet) -> i32;
}

const SIG_BLOCK: i32 = 0;
const SIGCHLD: i32 = 17;

fn block_sigchld() {
    let mut set: SignalSet = [0; 16];
    // SAFETY: the set outlives each call, and the old mask is not asked for.
    unsafe {
        assert_eq!(sigemptyset(&mut set), 0);
        assert_eq!(sigaddset(&mut set, SIGCHLD), 0);
        assert_eq!(pthread_sigmask(SIG_BLOCK, &set, std::ptr::null_mut()), 0);
    }
}

#[derive(Default)]
struct Counts {
    spawns: AtomicU64,
    threads: AtomicU64,
    timed_out: AtomicU64,
    interrupted: AtomicU64,
}

/// Reads an idle connection with a 20 ms bound until told to stop. A read
/// can end only by its bound or by an interruption: the peer sends nothing
/// and stays open.
fn read_loop(stop: &AtomicBool, counts: &Counts) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("listen");
    let mut client = TcpStream::connect(listener.local_addr().expect("address")).expect("connect");
    let (_peer, _) = listener.accept().expect("accept");
    client.set_read_timeout(Some(Duration::from_millis(20))).expect("bound the read");
    let mut byte = [0_u8; 1];
    while !stop.load(Ordering::Relaxed) {
        match client.read(&mut byte) {
            Err(error) if matches!(error.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                counts.timed_out.fetch_add(1, Ordering::Relaxed);
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                counts.interrupted.fetch_add(1, Ordering::Relaxed);
            }
            other => panic!("the idle read ended otherwise: {other:?}"),
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Spawn {
    NoWait,
    Wait,
    ForkNoWait,
}

fn child_command(program: &str, fork: bool) -> Command {
    let mut command = Command::new(program);
    command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    if fork {
        // With a pre_exec closure std forks and execs; without one it calls
        // posix_spawn.
        // SAFETY: the closure does nothing.
        unsafe {
            command.pre_exec(|| Ok(()));
        }
    }
    command
}

fn spawn_loop(stop: &AtomicBool, counts: &Counts, spawn: Spawn) {
    let mut running: VecDeque<Child> = VecDeque::new();
    while !stop.load(Ordering::Relaxed) {
        let mut child = child_command("/bin/true", spawn == Spawn::ForkNoWait).spawn().expect("spawn /bin/true");
        counts.spawns.fetch_add(1, Ordering::Relaxed);
        if spawn == Spawn::Wait {
            child.wait().expect("wait");
        } else {
            // Reap a child started 64 spawns ago, which has exited long since.
            running.push_back(child);
            if running.len() > 64 {
                running.pop_front().expect("oldest child").wait().expect("wait");
            }
        }
    }
    for mut child in running {
        child.wait().expect("wait");
    }
}

fn status_line(pid: u32, name: &str) -> String {
    std::fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()
        .and_then(|status| status.lines().find(|line| line.starts_with(name)).map(str::to_owned))
        .unwrap_or_else(|| format!("{name} unreadable"))
}

/// Prints the signal mask a child holds after its exec, for both ways std
/// starts one, beside this process's own.
fn child_mask(sigchld: &str) {
    println!("child-mask parent sigchld={sigchld} {}", status_line(std::process::id(), "SigBlk:"));
    for (path, fork) in [("posix_spawn", false), ("fork", true)] {
        let mut command = child_command("/bin/sleep", fork);
        let mut child = command.arg("30").spawn().expect("spawn /bin/sleep");
        // Before its exec a forked child still shows the image and the mask
        // it was forked with.
        let exec_done = (0..2000).any(|_| {
            let comm = std::fs::read_to_string(format!("/proc/{}/comm", child.id())).unwrap_or_default();
            comm.trim() == "sleep" || {
                std::thread::sleep(Duration::from_millis(1));
                false
            }
        });
        println!(
            "child-mask path={path} parent sigchld={sigchld} exec_done={exec_done} {}",
            status_line(child.id(), "SigBlk:")
        );
        child.kill().expect("stop the child");
        child.wait().expect("wait");
    }
}

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let texts: Vec<&str> = arguments.iter().map(String::as_str).collect();
    match texts.as_slice() {
        ["child-mask", sigchld @ ("open" | "blocked")] => {
            if *sigchld == "blocked" {
                block_sigchld();
            }
            child_mask(sigchld);
        }
        ["run", main @ ("reads" | "waits" | "starts-threads"), spawn, sigchld @ ("open" | "blocked"), seconds] => {
            let spawn_kind = match *spawn {
                "nowait" => Spawn::NoWait,
                "wait" => Spawn::Wait,
                "fork-nowait" => Spawn::ForkNoWait,
                other => panic!("SPAWN is nowait, wait or fork-nowait, not {other}"),
            };
            let seconds: u64 = seconds.parse().expect("SECONDS is a whole number");
            // Before any other thread exists, so that every thread inherits it.
            if *sigchld == "blocked" {
                block_sigchld();
            }
            let stop = Arc::new(AtomicBool::new(false));
            let counts = Arc::new(Counts::default());
            let mut threads = Vec::new();
            // The reader is the oldest thread after the first, as a
            // long-running test's thread is among libtest's.
            if *main != "reads" {
                let (stop, counts) = (Arc::clone(&stop), Arc::clone(&counts));
                threads.push(std::thread::spawn(move || read_loop(&stop, &counts)));
            }
            {
                let (stop, counts) = (Arc::clone(&stop), Arc::clone(&counts));
                threads.push(std::thread::spawn(move || spawn_loop(&stop, &counts, spawn_kind)));
            }
            let timer = {
                let stop = Arc::clone(&stop);
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_secs(seconds));
                    stop.store(true, Ordering::Relaxed);
                })
            };
            match *main {
                "reads" => read_loop(&stop, &counts),
                "waits" => {}
                _ => {
                    while !stop.load(Ordering::Relaxed) {
                        std::thread::spawn(|| {}).join().expect("join");
                        counts.threads.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
            timer.join().expect("join the timer");
            for thread in threads {
                thread.join().expect("join");
            }
            println!(
                "run pid={} main={main} spawn={spawn} sigchld={sigchld} seconds={seconds} spawns={} threads_started={} reads_timed_out={} reads_interrupted={}",
                std::process::id(),
                counts.spawns.load(Ordering::Relaxed),
                counts.threads.load(Ordering::Relaxed),
                counts.timed_out.load(Ordering::Relaxed),
                counts.interrupted.load(Ordering::Relaxed),
            );
        }
        _ => {
            eprintln!("usage: sigchld_eintr run MAIN SPAWN SIGCHLD SECONDS | sigchld_eintr child-mask SIGCHLD");
            std::process::exit(2);
        }
    }
}
