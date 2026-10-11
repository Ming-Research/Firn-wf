//! Continuous depth-one SET traffic before, during and after BGREWRITEAOF.
//! Persistence parsing and the asynchronous requester follow exp/rw-base-main;
//! completion is now bounded and rates use sampled completed-request counters.
use super::*;
use super::eviction::{Control, info_body};
use std::sync::atomic::AtomicU64;

const REWRITE_BOUND: Duration = Duration::from_secs(120);

#[derive(Default)]
pub(super) struct Progress {
    count: AtomicU64,
    stop: AtomicBool,
}

impl Progress {
    pub(super) fn completed(&self) { self.count.fetch_add(1, Ordering::Relaxed); }
    pub(super) fn stopped(&self) -> bool { self.stop.load(Ordering::Relaxed) }
}

#[derive(Clone, Copy, Debug)]
struct Persistence {
    enabled: bool,
    in_progress: bool,
    scheduled: bool,
    last_ok: bool,
    rewrites: u64,
    // Some older firn images omit this field; never substitute client wall time.
    last_seconds: Option<u64>,
}

impl Persistence {
    fn parse(info: &str) -> Result<Self> {
        let fields: HashMap<&str, &str> = info.lines().filter_map(|line| line.split_once(':')).collect();
        let flag = |name, optional| -> Result<bool> {
            match fields.get(name).copied() {
                Some("0") => Ok(false),
                Some("1") => Ok(true),
                None if optional => Ok(false),
                None => Err(format!("INFO persistence missing {name}").into()),
                Some(value) => Err(format!("INFO persistence invalid {name}: {value}").into()),
            }
        };
        let last_ok = match fields.get("aof_last_bgrewrite_status").copied() {
            Some("ok") => true,
            Some("err") => false,
            _ => return Err("INFO persistence missing or invalid aof_last_bgrewrite_status".into()),
        };
        let last_seconds = match fields.get("aof_last_rewrite_time_sec").copied() {
            None | Some("-1") => None,
            Some(value) => Some(value.parse()?),
        };
        Ok(Self {
            enabled: flag("aof_enabled", false)?,
            in_progress: flag("aof_rewrite_in_progress", false)?,
            scheduled: flag("aof_rewrite_scheduled", true)?,
            last_ok,
            rewrites: fields.get("aof_rewrites").ok_or("INFO persistence missing aof_rewrites")?.parse()?,
            last_seconds,
        })
    }

    fn idle(&self) -> bool { !self.in_progress && !self.scheduled }

    fn completed(&self, baseline: u64) -> Result<bool> {
        if !self.enabled { return Err("AOF disabled during rewrite workload".into()); }
        if self.rewrites < baseline { return Err("aof_rewrites went backwards".into()); }
        if self.idle() && self.rewrites > baseline {
            if self.rewrites != baseline + 1 { return Err("more than one rewrite during measurement".into()); }
            if !self.last_ok { return Err("aof_last_bgrewrite_status:err".into()); }
            return Ok(true);
        }
        Ok(false)
    }
}

fn observation(control: &mut Control, deadline: Instant) -> Result<Persistence> {
    let wire = control.call_before(&[b"INFO", b"persistence"], Some(deadline))?;
    Persistence::parse(info_body(&wire)?)
}

fn rewrite_started(wire: &[u8]) -> Result<()> {
    if wire != b"+Background append only file rewriting started\r\n" {
        return Err(format!("BGREWRITEAOF did not start: {}", String::from_utf8_lossy(wire)).into());
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct Snapshot {
    at: Instant,
    count: u64,
}

fn snapshot(progress: &[Arc<Progress>]) -> Snapshot {
    Snapshot { count: progress.iter().map(|p| p.count.load(Ordering::Relaxed)).sum(), at: Instant::now() }
}

fn rate(begin: Snapshot, end: Snapshot) -> Result<f64> {
    let count = end.count.checked_sub(begin.count).ok_or("request counter went backwards")?;
    let elapsed = end.at.checked_duration_since(begin.at).filter(|d| !d.is_zero())
        .ok_or("measurement phase has no elapsed time")?;
    // A short rewrite may complete between polls without any completed SET.
    Ok(count as f64 / elapsed.as_secs_f64())
}

fn check_workers(cancelled: &AtomicBool) -> Result<()> {
    if cancelled.load(Ordering::Relaxed) { return Err("rewrite workload worker failed".into()); }
    Ok(())
}

fn phase(seconds: Duration, sample: Duration, cancelled: &AtomicBool) -> Result<()> {
    let end = Instant::now() + seconds;
    while Instant::now() < end {
        check_workers(cancelled)?;
        thread::sleep(sample.min(end.saturating_duration_since(Instant::now())));
    }
    check_workers(cancelled)
}

struct Summary {
    before: f64,
    during: f64,
    after: f64,
    during_seconds: f64,
    persistence: Persistence,
}

fn wait_rewrite(control: &mut Control, mut requester: Control, deadline: Instant,
                baseline: u64, progress: &[Arc<Progress>], cancelled: &AtomicBool,
                sample: Duration) -> Result<(Persistence, Snapshot)> {
    // Firn experiment images can delay the start reply during close-tail replay.
    // Use a separate socket/thread so INFO and traffic continue meanwhile.
    let mut request = Some(thread::spawn(move || {
        rewrite_started(&requester.call_before(&[b"BGREWRITEAOF"], Some(deadline))?)
    }));
    let observed = (|| -> Result<(Persistence, Snapshot)> {
        let mut acknowledged = false;
        loop {
            check_workers(cancelled)?;
            if request.as_ref().is_some_and(|h| h.is_finished()) {
                request.take().unwrap().join().map_err(|_| "rewrite requester panicked")??;
                acknowledged = true;
            }
            let p = observation(control, deadline)?;
            if p.completed(baseline)? && acknowledged { return Ok((p, snapshot(progress))); }
            thread::sleep(sample.min(deadline.saturating_duration_since(Instant::now())));
        }
    })();
    // Joining is bounded by the same absolute deadline, even on a failed poll.
    if observed.is_err() { cancelled.store(true, Ordering::Relaxed); }
    if let Some(request) = request {
        request.join().map_err(|_| "rewrite requester panicked")??;
    }
    observed
}

fn phases(opts: &Options, control: &mut Control, requester: Control,
          progress: &[Arc<Progress>], cancelled: &AtomicBool, begin: Snapshot,
          baseline: u64) -> Result<Summary> {
    let seconds = opts.seconds.ok_or("rewrite-during requires --seconds")?;
    phase(seconds, opts.sample_interval, cancelled)?;
    // Establish idle immediately before the request, so stale success or a
    // rewrite during the baseline cannot be mistaken for this rewrite.
    let prior = observation(control, Instant::now() + REWRITE_BOUND)?;
    if !prior.enabled || !prior.idle() || prior.rewrites != baseline {
        return Err("rewrite state changed during the before phase".into());
    }
    let during = snapshot(progress);
    let (persistence, after) = wait_rewrite(control, requester, during.at + REWRITE_BOUND,
        baseline, progress, cancelled, opts.sample_interval)?;
    phase(seconds, opts.sample_interval, cancelled)?;
    let end = snapshot(progress);
    let final_state = observation(control, Instant::now() + TIMEOUT)?;
    if !final_state.completed(baseline)? {
        return Err("rewrite state changed during the after phase".into());
    }
    Ok(Summary { before: rate(begin, during)?, during: rate(during, after)?,
        after: rate(after, end)?, during_seconds: after.at.duration_since(during.at).as_secs_f64(),
        persistence })
}

fn configure(control: &mut Control, opts: &Options) -> Result<()> {
    // The explicit rewrite must be the only one, including during the prefill.
    control.config(b"auto-aof-rewrite-percentage", b"0")?;
    control.config(b"maxmemory-policy", b"noeviction")?;
    let maxmemory = opts.maxmemory.unwrap_or(0).to_string();
    control.config(b"maxmemory", maxmemory.as_bytes())?;
    eprintln!("rewrite-during settings: maxmemory={maxmemory} bytes, maxmemory-policy=noeviction (no keys evicted)");
    Ok(())
}

fn write_diagnostics(info: &str, out: &mut impl Write) -> Result<()> {
    for line in info.lines() {
        let Some((name, _)) = line.split_once(':') else { continue; };
        if name.starts_with("aof_") || name.starts_with("firn_aof_rewrite_")
            || matches!(name, "used_memory" | "maxmemory" | "maxmemory_policy") {
            writeln!(out, "{line}")?;
        }
    }
    Ok(())
}

fn diagnostics(port: u16, out: &mut impl Write) -> Result<()> {
    // A timeout may leave the polling connection with an unread reply. Use a
    // fresh connection and deadline, and one INFO for persistence and memory.
    let mut control = Control::new(port)?;
    let wire = control.call_before(&[b"INFO"], Some(Instant::now() + TIMEOUT))?;
    write_diagnostics(info_body(&wire)?, out)
}

pub(super) fn run(opts: &Options) -> Result<()> {
    let result = run_inner(opts);
    let outcome = if result.is_ok() { "success" } else { "failure" };
    eprintln!("rewrite-during INFO after {outcome}:");
    if let Err(e) = diagnostics(opts.port, &mut io::stderr().lock()) {
        // Diagnostics must not replace the workload's original failure.
        eprintln!("rewrite-during INFO unavailable: {e}");
    }
    result
}

fn run_inner(opts: &Options) -> Result<()> {
    let mut control = Control::new(opts.port)?;
    configure(&mut control, opts)?;
    let initial = observation(&mut control, Instant::now() + REWRITE_BOUND)?;
    if !initial.enabled || !initial.idle() { return Err("rewrite-during needs AOF on and an idle rewrite".into()); }
    fill(opts, opts.keys)?;
    if !opts.warmup.is_zero() {
        let mut warm = opts.clone();
        warm.seconds = Some(opts.warmup);
        warm.seed ^= 0xd1b54a32d192ed03;
        measure(&warm, None, None)?;
    }
    let prior = observation(&mut control, Instant::now() + REWRITE_BOUND)?;
    if !prior.enabled || !prior.idle() || prior.rewrites != initial.rewrites {
        return Err("unexpected rewrite during prefill/warmup".into());
    }
    let requester = Control::new(opts.port)?;
    let mut groups: Vec<Vec<Connection>> = (0..opts.threads).map(|_| Vec::new()).collect();
    for id in 0..opts.connections { groups[id % opts.threads].push(Connection::new(opts, id)?); }
    let progress: Vec<_> = (0..opts.threads).map(|_| Arc::new(Progress::default())).collect();
    let barrier = Arc::new(Barrier::new(opts.threads + 1));
    let start = Arc::new(std::sync::OnceLock::new());
    let cancelled = Arc::new(AtomicBool::new(false));
    let mut handles = Vec::new();
    // Stop is set after the three phases; it drains in-flight replies. The
    // ordinary per-request 30-second timeout and error checks still apply.
    let mut continuous = opts.clone();
    continuous.seconds = None;
    continuous.requests = None;
    for (connections, p) in groups.into_iter().zip(&progress) {
        let (opts, p, barrier, start, cancelled) =
            (continuous.clone(), p.clone(), barrier.clone(), start.clone(), cancelled.clone());
        handles.push(thread::spawn(move || {
            barrier.wait();
            let result = worker(connections, &opts, *start.get().unwrap(), &cancelled, None, Some(&p));
            if result.is_err() { cancelled.store(true, Ordering::Relaxed); }
            result
        }));
    }
    let begin = Snapshot { at: Instant::now(), count: 0 };
    start.set(begin.at).unwrap();
    barrier.wait();
    let summary = phases(opts, &mut control, requester, &progress, &cancelled, begin, prior.rewrites);
    for p in &progress { p.stop.store(true, Ordering::Relaxed); }
    if summary.is_err() { cancelled.store(true, Ordering::Relaxed); }
    let mut worker_failure = None;
    for handle in handles {
        if let Err(e) = handle.join().unwrap_or_else(|_| Err("worker panicked".into())) {
            eprintln!("{e}");
            worker_failure = Some(e);
        }
    }
    let summary = summary?;
    if let Some(e) = worker_failure { return Err(e); }
    let duration = summary.persistence.last_seconds.map_or_else(|| "unavailable".into(), |s| s.to_string());
    println!("rewrite-during,{},{},{},{},{:.3},{:.3},{:.3},{:.6},{},ok,1",
        opts.connections, opts.keys, opts.value.len(), opts.seed,
        summary.before, summary.during, summary.after, summary.during_seconds, duration);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> &'static str {
        "aof_enabled:1\r\naof_rewrite_in_progress:1\r\naof_rewrite_scheduled:0\r\naof_last_bgrewrite_status:ok\r\naof_rewrites:1\r\naof_last_rewrite_time_sec:2\r\n"
    }

    #[test]
    fn rewrite_configuration_sets_noeviction_and_optional_limit() {
        use std::net::TcpListener;
        for limit in [None, Some(0), Some(1_073_741_824)] {
            let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let mut opts = super::super::tests::options(listener.local_addr().unwrap().port(), "rewrite-during");
            opts.maxmemory = limit;
            let server = thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                socket.set_read_timeout(Some(TIMEOUT)).unwrap();
                for (key, value) in [("auto-aof-rewrite-percentage", "0".to_string()),
                                     ("maxmemory-policy", "noeviction".to_string()),
                                     ("maxmemory", limit.unwrap_or(0).to_string())] {
                    super::super::tests::expect_command(&mut socket,
                        &[b"CONFIG", b"SET", key.as_bytes(), value.as_bytes()]);
                    socket.write_all(b"+OK\r\n").unwrap();
                }
            });
            configure(&mut Control::new(opts.port).unwrap(), &opts).unwrap();
            server.join().unwrap();
        }
    }

    #[test]
    fn diagnostics_record_success_failure_and_active_rewrite_fields() {
        use std::net::TcpListener;
        for (status, active) in [("ok", "0"), ("err", "0"), ("ok", "1")] {
            let persistence = info().replace("status:ok", &format!("status:{status}"))
                .replace("in_progress:1", &format!("in_progress:{active}"));
            // Optional and future rewrite fields are printed verbatim, without
            // claiming that scan_peak measures the journal or total footprint.
            let fields = concat!("firn_aof_rewrite_reserve:53687091\r\n",
                "firn_aof_rewrite_scan_peak:1024\r\nfirn_aof_rewrite_cut:0\r\n",
                "firn_aof_rewrite_commit_seq:1000001\r\nfirn_aof_rewrite_future:7\r\n",
                "aof_current_size:90000000\r\naof_base_size:0\r\n",
                "used_memory:226000000\r\nmaxmemory:1073741824\r\nmaxmemory_policy:noeviction\r\n");
            let body = format!("# Persistence\r\n{persistence}{fields}total_commands_processed:42\r\n");
            let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let port = listener.local_addr().unwrap().port();
            let server = thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                socket.set_read_timeout(Some(TIMEOUT)).unwrap();
                super::super::tests::expect_command(&mut socket, &[b"INFO"]);
                write!(socket, "${}\r\n{body}\r\n", body.len()).unwrap();
            });
            let mut out = Vec::new();
            diagnostics(port, &mut out).unwrap();
            server.join().unwrap();
            let expected = format!("{persistence}{fields}").replace("\r\n", "\n");
            assert_eq!(String::from_utf8(out).unwrap(), expected);
        }
        // Redis/older images need no firn-only fields or fabricated defaults.
        let mut out = Vec::new();
        write_diagnostics(info(), &mut out).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), info().replace("\r\n", "\n"));
    }

    #[test]
    fn persistence_requires_valid_flags_status_count_and_duration() {
        let p = Persistence::parse(info()).unwrap();
        assert!(p.enabled && p.in_progress && !p.scheduled && p.last_ok);
        assert_eq!(p.last_seconds, Some(2));
        for field in ["aof_enabled:1", "aof_rewrite_in_progress:1", "aof_last_bgrewrite_status:ok", "aof_rewrites:1"] {
            assert!(Persistence::parse(&info().replace(&format!("{field}\r\n"), "")).is_err());
        }
        for field in ["aof_enabled:1", "aof_rewrite_in_progress:1", "aof_rewrite_scheduled:0", "aof_last_bgrewrite_status:ok"] {
            let name = field.split_once(':').unwrap().0;
            for invalid in ["", "2", "-1", "garbage"] {
                assert!(Persistence::parse(&info().replace(field, &format!("{name}:{invalid}"))).is_err());
            }
        }
        for field in ["aof_rewrites:1", "aof_last_rewrite_time_sec:2"] {
            for invalid in ["", "-2", "garbage", "18446744073709551616"] {
                assert!(Persistence::parse(&info().replace(field, &format!("{}:{invalid}", field.split_once(':').unwrap().0))).is_err());
            }
        }
        assert!(!Persistence::parse(&info().replace("scheduled:0", "scheduled:1")).unwrap().idle());
        assert!(Persistence::parse(&info().replace("aof_rewrite_scheduled:0\r\n", "")).is_ok());
        for missing in [info().replace("aof_last_rewrite_time_sec:2\r\n", ""),
                        info().replace("time_sec:2", "time_sec:-1")] {
            assert_eq!(Persistence::parse(&missing).unwrap().last_seconds, None);
        }
    }

    #[test]
    fn completion_requires_this_rewrite_not_stale_success() {
        let active = Persistence::parse(info()).unwrap();
        let idle = Persistence { in_progress: false, ..active };
        assert!(!idle.completed(1).unwrap());
        assert!(!active.completed(0).unwrap());
        // A fast rewrite between polls is valid if the counter advanced.
        assert!(idle.completed(0).unwrap());
        assert!(!Persistence { scheduled: true, ..idle }.completed(0).unwrap());
        for invalid in [Persistence { last_ok: false, ..idle },
                        Persistence { enabled: false, ..idle },
                        Persistence { rewrites: 2, ..idle }] {
            assert!(invalid.completed(0).is_err());
        }
        assert!(idle.completed(2).is_err());
        assert!(rewrite_started(b"+Background append only file rewriting started\r\n").is_ok());
        for wire in [b"+OK\r\n".as_slice(), b"-ERR already in progress\r\n",
                     b"+Background append only file rewriting scheduled\r\n", b":1\r\n"] {
            assert!(rewrite_started(wire).is_err());
        }
    }

    #[test]
    fn phase_rates_use_completed_request_deltas_and_elapsed_time() {
        let at = Instant::now();
        let a = Snapshot { at, count: 10 };
        let b = Snapshot { at: at + Duration::from_secs(2), count: 30 };
        let c = Snapshot { at: at + Duration::from_secs(3), count: 35 };
        assert_eq!(rate(a, b).unwrap(), 10.0);
        assert_eq!(rate(b, c).unwrap(), 5.0);
        assert!(rate(b, a).is_err());
        assert!(rate(a, a).is_err());
        assert_eq!(rate(a, Snapshot { count: 10, ..b }).unwrap(), 0.0);
    }

    #[test]
    fn rewrite_set_traffic_checks_replies_and_updates_progress() {
        use std::net::TcpListener;
        for (response, error) in [(b"+OK\r\n".as_slice(), None),
                                  (b"-ERR rejected\r\n", Some("server error: ERR rejected")),
                                  (eviction::OOM_REPLY, Some("server error: OOM")),
                                  (b":1\r\n", Some("rewrite workload SET did not return OK"))] {
            let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let mut opts = super::super::tests::options(listener.local_addr().unwrap().port(), "rewrite-during");
            opts.connections = 1;
            opts.threads = 1;
            opts.requests = Some(1);
            let server = thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                socket.set_read_timeout(Some(TIMEOUT)).unwrap();
                super::super::tests::expect_command(&mut socket, &[b"SET", b"key:0", b"x"]);
                socket.write_all(response).unwrap();
            });
            let p = Progress::default();
            let cancelled = AtomicBool::new(false);
            let result = worker(vec![Connection::new(&opts, 0).unwrap()], &opts, Instant::now(),
                &cancelled, None, Some(&p));
            server.join().unwrap();
            if let Some(message) = error {
                assert!(result.err().unwrap().to_string().contains(message));
                assert_eq!(p.count.load(Ordering::Relaxed), 0);
            } else {
                assert_eq!(result.unwrap().0.values().sum::<u64>(), 1);
                assert_eq!(p.count.load(Ordering::Relaxed), 1);
            }
        }
    }

    #[test]
    fn stop_prevents_new_requests_and_cancellation_fails_a_phase() {
        use std::net::TcpListener;
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let mut opts = super::super::tests::options(listener.local_addr().unwrap().port(), "rewrite-during");
        opts.connections = 1;
        opts.threads = 1;
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket.set_read_timeout(Some(TIMEOUT)).unwrap();
            assert_eq!(socket.read(&mut [0]).unwrap(), 0, "stop must issue no SET");
        });
        let p = Progress::default();
        p.stop.store(true, Ordering::Relaxed);
        let result = worker(vec![Connection::new(&opts, 0).unwrap()], &opts, Instant::now(),
            &AtomicBool::new(false), None, Some(&p)).unwrap();
        assert!(result.0.is_empty());
        server.join().unwrap();
        assert!(phase(Duration::ZERO, Duration::from_millis(1), &AtomicBool::new(true)).is_err());
        assert!(phase(Duration::ZERO, Duration::from_millis(1), &AtomicBool::new(false)).is_ok());
    }

    #[test]
    fn persistence_control_rejects_errors_and_expired_deadline() {
        use std::net::TcpListener;
        for wire in [b"-ERR persistence failed\r\n".as_slice(), b"+OK\r\n"] {
            let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let port = listener.local_addr().unwrap().port();
            let server = thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                super::super::tests::expect_command(&mut socket, &[b"INFO", b"persistence"]);
                socket.write_all(wire).unwrap();
            });
            let mut control = Control::new(port).unwrap();
            assert!(observation(&mut control, Instant::now() + TIMEOUT).is_err());
            assert!(observation(&mut control, Instant::now()).unwrap_err().to_string().contains("120 s"));
            server.join().unwrap();
        }
    }

    #[test]
    fn requester_does_not_block_polling_and_unfinished_rewrite_times_out() {
        use std::net::TcpListener;
        use std::sync::mpsc;
        // Completion can be missed between polls, but acknowledgement must be
        // checked. The other cases fail on a start error, rewrite error or bound.
        for (start_reply, finish, success) in [
            (b"+Background append only file rewriting started\r\n".as_slice(), "ok", true),
            (b"-ERR cannot rewrite\r\n".as_slice(), "ok", false),
            (b"+Background append only file rewriting started\r\n".as_slice(), "err", false),
            (b"+Background append only file rewriting started\r\n".as_slice(), "active", false),
        ] {
            let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let port = listener.local_addr().unwrap().port();
            let (polled, first_poll) = mpsc::channel();
            let server = thread::spawn(move || {
                let (mut info_socket, _) = listener.accept().unwrap();
                let (mut rewrite_socket, _) = listener.accept().unwrap();
                info_socket.set_read_timeout(Some(TIMEOUT)).unwrap();
                rewrite_socket.set_read_timeout(Some(TIMEOUT)).unwrap();
                let requester = thread::spawn(move || {
                    super::super::tests::expect_command(&mut rewrite_socket, &[b"BGREWRITEAOF"]);
                    // Waiting for INFO before replying would deadlock a sampler
                    // that sent BGREWRITEAOF on the INFO connection/thread.
                    first_poll.recv_timeout(TIMEOUT).unwrap();
                    rewrite_socket.write_all(start_reply).unwrap();
                });
                let mut expected = Vec::new();
                command(&mut expected, &[b"INFO", b"persistence"]);
                let mut first = Some(polled);
                loop {
                    let mut wire = vec![0; expected.len()];
                    match info_socket.read_exact(&mut wire) {
                        Ok(()) => (),
                        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
                        Err(e) => panic!("{e}"),
                    }
                    assert_eq!(wire, expected);
                    let body = if finish == "active" { info().to_string() } else {
                        info().replace("in_progress:1", "in_progress:0")
                            .replace("status:ok", &format!("status:{finish}"))
                    };
                    match write!(info_socket, "${}\r\n{body}\r\n", body.len()) {
                        Ok(()) => (),
                        Err(e) if matches!(e.kind(), io::ErrorKind::BrokenPipe | io::ErrorKind::ConnectionReset) => break,
                        Err(e) => panic!("{e}"),
                    }
                    if let Some(polled) = first.take() { polled.send(()).unwrap(); }
                }
                requester.join().unwrap();
            });
            let mut control = Control::new(port).unwrap();
            let requester = Control::new(port).unwrap();
            let bound = if finish == "active" { Duration::from_millis(100) } else { TIMEOUT };
            let result = wait_rewrite(&mut control, requester,
                Instant::now() + bound, 0, &[],
                &AtomicBool::new(false), Duration::from_millis(1));
            drop(control);
            server.join().unwrap();
            assert_eq!(result.is_ok(), success, "{finish}");
            if success {
                assert_eq!(result.unwrap().0.last_seconds, Some(2));
            } else {
                let error = result.err().unwrap().to_string();
                let expected = match finish {
                    "active" => "120 s",
                    "err" => "aof_last_bgrewrite_status:err",
                    _ => "server error: ERR cannot rewrite",
                };
                assert!(error.contains(expected), "{error}");
            }
        }
    }
}
