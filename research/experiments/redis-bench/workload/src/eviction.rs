//! Cache-aside Zipf traffic and matched session admission-check measurements.
//! The lifetime INFO peak includes the unlimited prefill; sampled memory is a
//! separate, explicitly lower-bound observation of excess during measurement.
use super::*;

pub(super) const OOM_REPLY: &[u8] = b"-OOM command not allowed when used memory > 'maxmemory'.\r\n";

pub(super) struct Zipf {
    cumulative: Vec<f64>,
    permutation: Vec<u64>,
}

impl Zipf {
    fn new(keys: u64, exponent: f64, seed: u64) -> Result<Self> {
        let count = usize::try_from(keys)?;
        if count == 0 || !exponent.is_finite() || exponent < 0.0 {
            return Err("Zipf needs positive keys and a finite, nonnegative exponent".into());
        }
        let mut cumulative = Vec::with_capacity(count);
        let mut total = 0.0;
        for rank in 1..=count {
            total += (rank as f64).powf(-exponent);
            cumulative.push(total);
        }
        for p in &mut cumulative { *p /= total; }
        *cumulative.last_mut().unwrap() = 1.0;
        let mut permutation: Vec<u64> = (0..keys).collect();
        let mut state = seed;
        // Fisher-Yates: popularity rank is independent of the key's spelling.
        for i in (1..count).rev() {
            let j = random_key(&mut state, (i + 1) as u64) as usize;
            permutation.swap(i, j);
        }
        Ok(Self { cumulative, permutation })
    }

    pub(super) fn draw(&self, state: &mut u64) -> u64 {
        // Uniform [0, 1), with 53 random bits; binary-search the exact finite
        // Zipf CDF rather than an approximation to the unbounded distribution.
        let p = (random_u64(state) >> 11) as f64 / 9_007_199_254_740_992.0;
        self.permutation[self.cumulative.partition_point(|&end| end <= p)]
    }
}

// Called only after the RESP framing parser has accepted the whole reply.
pub(super) fn get_hit(wire: &[u8], expected: &[u8]) -> Result<bool> {
    if wire == b"$-1\r\n" { return Ok(false); }
    let header = format!("${}\r\n", expected.len());
    if !wire.starts_with(header.as_bytes())
        || wire.len() != header.len() + expected.len() + 2
        || &wire[header.len()..wire.len() - 2] != expected {
        return Err("GET hit did not return the configured value".into());
    }
    Ok(true)
}

pub(super) struct Control {
    socket: TcpStream,
}

impl Control {
    pub(super) fn new(port: u16) -> Result<Self> { Ok(Self { socket: connect(port)? }) }

    fn call(&mut self, args: &[&[u8]]) -> Result<Vec<u8>> {
        self.call_before(args, None)
    }

    pub(super) fn call_before(&mut self, args: &[&[u8]], deadline: Option<Instant>) -> Result<Vec<u8>> {
        let remaining = || -> Result<Duration> {
            match deadline {
                Some(end) => end.checked_duration_since(Instant::now()).filter(|d| !d.is_zero())
                    .ok_or_else(|| "timed out after 120 s waiting for AOF rewrite to finish (in progress or scheduled)".into()),
                None => Ok(TIMEOUT),
            }
        };
        let mut out = Vec::new();
        command(&mut out, args);
        self.socket.set_write_timeout(Some(remaining()?))?;
        let result = self.socket.write_all(&out);
        remaining()?;
        result?;
        let mut input = Vec::new();
        loop {
            self.socket.set_read_timeout(Some(remaining()?))?;
            if let Some(size) = reply(&input, 0)? {
                if size != input.len() { return Err("extra control reply".into()); }
                return Ok(input);
            }
            let result = read_more(&mut self.socket, &mut input);
            remaining()?;
            result?;
        }
    }

    fn ok(&mut self, args: &[&[u8]]) -> Result<()> {
        if self.call(args)? != b"+OK\r\n" { return Err("control command did not return OK".into()); }
        Ok(())
    }

    pub(super) fn config(&mut self, key: &[u8], value: &[u8]) -> Result<()> {
        self.ok(&[b"CONFIG", b"SET", key, value])
    }

    fn info(&mut self) -> Result<Memory> {
        let wire = self.call(&[b"INFO"])?;
        Memory::parse(info_body(&wire)?)
    }

    fn dbsize(&mut self) -> Result<u64> {
        parse_dbsize(&self.call(&[b"DBSIZE"])?)
    }

    /// The original closed-log rewrite replayed AOF files into private
    /// keyspaces in firn's process heap; Redis rewrote in a fork child. A fill
    /// reaching the automatic rewrite minimum (64 MiB) inflated firn's memory
    /// reading in the 14900K probe (Firn-wf run 37996029478): 390 MB with AOF
    /// versus 226 MB without. Older compared images still need this wait, which
    /// excludes active rewrite allocations from the cache limit. Wait for
    /// quiescence, then discard the first of two memory readings 200 ms apart
    /// before selecting filled_used_memory.
    fn filled_memory(&mut self) -> Result<u64> {
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            let wire = self.call_before(&[b"INFO", b"persistence"], Some(deadline))?;
            if rewrite_idle(info_body(&wire)?)? { break; }
            thread::sleep(Duration::from_millis(100).min(deadline.saturating_duration_since(Instant::now())));
        }
        self.info()?;
        thread::sleep(Duration::from_millis(200));
        Ok(self.info()?.used)
    }
}

// Control::call has already validated RESP framing before these parsers run.
pub(super) fn info_body(wire: &[u8]) -> Result<&str> {
    if wire.first() != Some(&b'$') || wire == b"$-1\r\n" { return Err("INFO is not a bulk string".into()); }
    let start = wire.windows(2).position(|w| w == b"\r\n").ok_or("missing INFO header")? + 2;
    Ok(std::str::from_utf8(&wire[start..wire.len() - 2])?)
}

fn rewrite_idle(info: &str) -> Result<bool> {
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
    let in_progress = flag("aof_rewrite_in_progress", false)?;
    let scheduled = flag("aof_rewrite_scheduled", true)?;
    Ok(!in_progress && !scheduled)
}

fn parse_dbsize(wire: &[u8]) -> Result<u64> {
    let value = wire.strip_prefix(b":").and_then(|s| s.strip_suffix(b"\r\n"))
        .ok_or("DBSIZE is not an integer reply")?;
    let count = std::str::from_utf8(value)?.parse::<i64>()?;
    Ok(u64::try_from(count).map_err(|_| "DBSIZE returned a negative key count")?)
}

#[derive(Clone, Copy, Default)]
struct Memory {
    used: u64,
    peak: u64,
    evicted: u64,
    limit: u64,
    excluded: u64,
}

impl Memory {
    fn parse(info: &str) -> Result<Self> {
        let fields: HashMap<&str, &str> = info.lines().filter_map(|line| line.split_once(':')).collect();
        let read = |name| -> Result<u64> {
            Ok(fields.get(name).ok_or_else(|| format!("INFO missing {name}"))?.parse()?)
        };
        Ok(Self { used: read("used_memory")?, peak: read("used_memory_peak")?,
            evicted: read("evicted_keys")?, limit: read("maxmemory")?,
            excluded: read("mem_not_counted_for_evict")? })
    }
}

/// A terminal outcome is retained in the CSV even when the run exits nonzero.
/// Unknown completion times stay empty; the cap is not a rewrite duration.
struct RewriteOutcome {
    duration: Option<Duration>,
    status: &'static str,
    abort_reason: String,
}

impl RewriteOutcome {
    fn csv(&self) -> String {
        format!("{},{},\"{}\"", self.duration.map(|d| format!("{:.9}", d.as_secs_f64())).unwrap_or_default(),
            self.status, self.abort_reason.replace('"', "\"\""))
    }

    fn failed(&self) -> bool { !matches!(self.status, "ok" | "not_requested") }
}

fn write_result(row: &str, outcome: Option<&RewriteOutcome>, out: &mut impl Write) -> Result<()> {
    if let Some(outcome) = outcome {
        writeln!(out, "{row},{}", outcome.csv())?;
        out.flush()?;
        if outcome.failed() { return Err(format!("cache rewrite ended with {}", outcome.status).into()); }
    } else { writeln!(out, "{row}")?; }
    Ok(())
}

pub(super) struct RewriteWindow {
    control: Control,
    requester: Option<Control>,
    request: Option<thread::JoinHandle<Result<()>>>,
    sent: Arc<std::sync::OnceLock<Instant>>,
    baseline: u64,
    nominal: Duration,
    bound: Duration,
    deadline: Option<Instant>,
    acknowledged: bool,
    observed: Option<(Instant, bool)>,
    abort_reason: String,
    outcome: Option<RewriteOutcome>,
    final_checked: bool,
}

impl RewriteWindow {
    fn persistence(control: &mut Control, deadline: Instant) -> Result<(rewrite::Persistence, String)> {
        let wire = control.call_before(&[b"INFO", b"persistence"], Some(deadline))?;
        let info = info_body(&wire)?;
        let reason = info.lines().filter_map(|line| line.split_once(':'))
            .find_map(|(key, value)| (key == "firn_aof_rewrite_abort_reason").then(|| value.to_string()))
            .unwrap_or_default();
        Ok((rewrite::Persistence::parse(info)?, reason))
    }

    fn configure(control: &mut Control, rewrite: bool) -> Result<()> {
        control.config(b"auto-aof-rewrite-percentage", b"0")?;
        let (p, _) = Self::persistence(control, Instant::now() + TIMEOUT)?;
        if p.enabled != rewrite || !p.idle() {
            return Err("cache rewrite mode needs enabled, idle AOF; its control needs disabled, idle AOF".into());
        }
        Ok(())
    }

    fn new(port: u16, nominal: Duration, request_rewrite: bool) -> Result<Self> {
        let mut control = Control::new(port)?;
        let (p, _) = Self::persistence(&mut control, Instant::now() + TIMEOUT)?;
        if p.enabled != request_rewrite || !p.idle() {
            return Err("cache AOF state changed during setup".into());
        }
        if request_rewrite && p.rewrites == u64::MAX {
            return Err("cache rewrite count cannot advance without overflow".into());
        }
        Ok(Self { control, requester: if request_rewrite { Some(Control::new(port)?) } else { None },
            request: None, sent: Arc::new(std::sync::OnceLock::new()), baseline: p.rewrites,
            nominal, bound: Duration::from_secs(120), deadline: None, acknowledged: false,
            observed: None, abort_reason: String::new(),
            final_checked: false,
            outcome: (!request_rewrite).then(|| RewriteOutcome { duration: None,
                status: "not_requested", abort_reason: String::new() }) })
    }

    pub(super) fn deadline(&self) -> Option<Instant> {
        if self.outcome.is_none() { self.deadline } else { None }
    }

    pub(super) fn pause(&self, begin: Instant, sample: Duration) -> Duration {
        if self.outcome.as_ref().is_some_and(|o| o.failed() || begin.elapsed() >= self.nominal) {
            return sample;
        }
        let next = if let Some(deadline) = self.deadline() { deadline }
            else if self.outcome.is_none() { begin + self.nominal / 2 }
            else { begin + self.nominal };
        sample.min(next.saturating_duration_since(Instant::now()))
    }

    pub(super) fn fail(&mut self, error: impl std::fmt::Display) {
        eprintln!("cache rewrite: {error}");
        let status = if self.deadline().is_some_and(|d| Instant::now() >= d) { "timeout" } else { "protocol_error" };
        self.finish(status, None);
    }

    fn finish(&mut self, status: &'static str, at: Option<Instant>) {
        self.outcome = Some(RewriteOutcome { duration: at.and_then(|at|
            self.sent.get().and_then(|sent| at.checked_duration_since(*sent))),
            status, abort_reason: self.abort_reason.clone() });
    }

    fn poll(&mut self) -> Result<()> {
        let deadline = self.deadline.unwrap();
        if Instant::now() >= deadline {
            self.finish("timeout", None);
            return Ok(());
        }
        let mut request_error = None;
        if self.request.as_ref().is_some_and(|h| h.is_finished()) {
            match self.request.take().unwrap().join().unwrap_or_else(|_| Err("cache rewrite requester panicked".into())) {
                Ok(()) => self.acknowledged = true,
                Err(error) => request_error = Some(error),
            }
        }
        // Poll even after a rejected request to retain firn's admission reason.
        let (p, reason) = Self::persistence(&mut self.control, deadline)?;
        self.abort_reason = reason;
        if let Some(error) = request_error {
            eprintln!("cache BGREWRITEAOF: {error}");
            self.finish("request_error", None);
            return Ok(());
        }
        if !p.enabled || p.rewrites < self.baseline || p.rewrites > self.baseline.saturating_add(1) {
            return Err("cache rewrite AOF state or rewrite count changed unexpectedly".into());
        }
        if p.idle() && p.rewrites == self.baseline + 1 {
            if self.observed.is_some_and(|(_, ok)| ok != p.last_ok) {
                return Err("cache rewrite status changed after completion".into());
            }
            self.observed.get_or_insert((Instant::now(), p.last_ok));
        } else if self.observed.is_some() {
            return Err("cache rewrite became active after completion".into());
        }
        if self.acknowledged {
            if let Some((at, ok)) = self.observed { self.finish(if ok { "ok" } else { "err" }, Some(at)); }
        }
        Ok(())
    }

    pub(super) fn tick(&mut self, begin: Instant) -> bool {
        if self.outcome.is_none() {
            if self.deadline.is_none() && begin.elapsed() >= self.nominal / 2 {
                let started = (|| -> Result<()> {
                    let (p, reason) = Self::persistence(&mut self.control, Instant::now() + TIMEOUT)?;
                    self.abort_reason = reason;
                    if !p.enabled || !p.idle() || p.rewrites != self.baseline {
                        return Err("cache rewrite state changed before the trigger".into());
                    }
                    let deadline = Instant::now() + self.bound;
                    self.deadline = Some(deadline);
                    let mut requester = self.requester.take().ok_or("cache rewrite already requested")?;
                    let sent = self.sent.clone();
                    self.request = Some(thread::spawn(move || {
                        sent.set(Instant::now()).unwrap();
                        rewrite::rewrite_started(&requester.call_before(&[b"BGREWRITEAOF"], Some(deadline))?)
                    }));
                    Ok(())
                })();
                if let Err(error) = started { self.fail(error); }
            }
            if self.outcome.is_none() && self.deadline.is_some() {
                if let Err(error) = self.poll() { self.fail(error); }
            }
        }
        if !self.final_checked && begin.elapsed() >= self.nominal
            && self.outcome.as_ref().is_some_and(|o| !o.failed()) {
            self.final_checked = true;
            let checked = (|| -> Result<()> {
                let (p, reason) = Self::persistence(&mut self.control, Instant::now() + TIMEOUT)?;
                self.abort_reason = reason;
                let requested = self.outcome.as_ref().unwrap().status == "ok";
                if p.enabled != requested || !p.idle()
                    || p.rewrites != self.baseline + u64::from(requested)
                    || (requested && !p.last_ok) {
                    return Err("cache AOF state changed after the measured rewrite or in the no-AOF control".into());
                }
                Ok(())
            })();
            if let Err(error) = checked { self.fail(error); }
        }
        // Errors end traffic explicitly; successful and no-AOF runs retain the
        // whole nominal window, extending it if the rewrite takes longer.
        self.outcome.as_ref().is_some_and(|o| o.failed() || begin.elapsed() >= self.nominal)
    }

    fn finish_request(&mut self) {
        if let Some(request) = self.request.take() {
            // Every operation uses the original absolute deadline.
            if let Err(error) = request.join().unwrap_or_else(|_| Err("cache rewrite requester panicked".into())) {
                eprintln!("cache BGREWRITEAOF: {error}");
            }
        }
    }

    pub(super) fn failed(&self) -> bool { self.outcome.as_ref().is_some_and(RewriteOutcome::failed) }
}

impl Drop for RewriteWindow {
    fn drop(&mut self) { self.finish_request(); }
}

pub(super) struct Monitor {
    control: Control,
    first: Memory,
    last: Memory,
    sampled_max: u64,
    samples: u64,
    last_sample: Instant,
    max_gap: Duration,
    keys_at_start: u64,
}

impl Monitor {
    fn new(control: Control) -> Self {
        Self { control, first: Memory::default(), last: Memory::default(),
            sampled_max: 0, samples: 0, last_sample: Instant::now(), max_gap: Duration::ZERO,
            keys_at_start: 0 }
    }

    pub(super) fn start(&mut self) -> Result<()> {
        self.control.ok(&[b"CONFIG", b"RESETSTAT"])?;
        self.first = self.control.info()?;
        self.keys_at_start = self.control.dbsize()?;
        self.last = self.first;
        self.sampled_max = self.first.used;
        self.samples = 1;
        self.last_sample = Instant::now();
        self.max_gap = Duration::ZERO;
        Ok(())
    }

    pub(super) fn sample(&mut self) -> Result<()> {
        self.sample_before(None)
    }

    pub(super) fn sample_before(&mut self, deadline: Option<Instant>) -> Result<()> {
        let wire = self.control.call_before(&[b"INFO"], deadline)?;
        self.last = Memory::parse(info_body(&wire)?)?;
        let now = Instant::now();
        self.max_gap = self.max_gap.max(now.duration_since(self.last_sample));
        self.last_sample = now;
        self.sampled_max = self.sampled_max.max(self.last.used);
        self.samples += 1;
        Ok(())
    }

    fn evicted_delta(&self) -> Result<u64> {
        self.last.evicted.checked_sub(self.first.evicted).ok_or_else(|| "eviction counter went backwards".into())
    }
}

fn half_dataset_limit(before: u64, filled: u64) -> Result<u64> {
    let dataset = filled.checked_sub(before).filter(|&n| n >= 2)
        .ok_or("prefill did not grow used_memory by at least two bytes")?;
    Ok(before + dataset / 2)
}

fn warmup(opts: &Options, zipf: Option<Arc<Zipf>>) -> Result<()> {
    if !opts.warmup.is_zero() {
        let mut warm = opts.clone();
        warm.seconds = Some(opts.warmup);
        warm.requests = None;
        // Separate streams: a faster warm-up must not advance the measured
        // sequence. Each server starts the same per-connection measured prefix.
        warm.seed ^= 0xd1b54a32d192ed03;
        measure(&warm, zipf, None)?;
    }
    Ok(())
}

pub(super) fn run(opts: &Options) -> Result<()> {
    let zipf = Arc::new(Zipf::new(opts.keys, opts.zipf_s, opts.seed)?);
    let mut control = Control::new(opts.port)?;
    let controlled = opts.workload != "evict-zipf";
    if controlled { RewriteWindow::configure(&mut control, opts.workload == "evict-zipf-rewrite")?; }
    control.config(b"maxmemory", b"0")?;
    control.config(b"maxmemory-policy", b"allkeys-lru")?;
    // Redis 7.2.16 defaults, explicit so both servers use the same settings.
    control.config(b"maxmemory-samples", b"5")?;
    control.config(b"maxmemory-eviction-tenacity", b"10")?;
    let before = control.info()?.used;
    fill(opts, opts.keys)?;
    let filled = control.filled_memory()?;
    let limit = half_dataset_limit(before, filled)?;
    control.config(b"maxmemory", limit.to_string().as_bytes())?;
    warmup(opts, Some(zipf.clone()))?;
    let mut window = if controlled { Some(RewriteWindow::new(opts.port, opts.seconds.unwrap(),
        opts.workload == "evict-zipf-rewrite")?) } else { None };
    let mut monitor = Monitor::new(control);
    let measured = measure_controlled(opts, Some(zipf), Some(&mut monitor), window.as_mut())?;
    if let Some(w) = window.as_mut() { w.finish_request(); }
    let failed = window.as_ref().is_some_and(RewriteWindow::failed);
    let keys_at_end = if failed { String::new() } else { monitor.control.dbsize()?.to_string() };
    if !failed && measured.writers != opts.connections {
        return Err("not every connection completed a miss SET attempt; increase the run duration or dataset".into());
    }
    if monitor.first.limit != limit || monitor.last.limit != limit {
        return Err("INFO maxmemory differs from the configured limit".into());
    }
    let sampled_excess = monitor.sampled_max.saturating_sub(limit);
    let lifetime_excess = monitor.last.peak.saturating_sub(limit);
    // The shell copies the leading seven columns to workloads.csv; the full
    // row goes to evict-zipf.csv under its own explicit header.
    let row = format!("{},{},{},{},{},{},{:.3},{},{},{},{},{:.9},{},{},{},{},{},{},{},{},{:.9},{},{},{:.9},{},{:.3},{},{},{},{}",
        measured.csv(opts), opts.keys, opts.zipf_s, opts.value.len(), opts.seed,
        opts.warmup.as_secs_f64(), opts.sample_interval.as_secs_f64() * 1000.0,
        measured.count, measured.hits, measured.count - measured.hits, measured.refused_sets,
        measured.hits as f64 / measured.count as f64, monitor.evicted_delta()?,
        monitor.last.used, monitor.last.peak, limit, before, filled, monitor.first.peak,
        lifetime_excess, lifetime_excess as f64 / limit as f64,
        monitor.sampled_max, sampled_excess, sampled_excess as f64 / limit as f64,
        monitor.samples, monitor.max_gap.as_secs_f64() * 1000.0, monitor.last.excluded, measured.writers,
        monitor.keys_at_start, keys_at_end);
    if let Some(w) = &window {
        let outcome = w.outcome.as_ref().ok_or("cache rewrite measurement has no terminal status")?;
        write_result(&row, Some(outcome), &mut io::stdout().lock())
    } else { write_result(&row, None, &mut io::stdout().lock()) }
}

pub(super) fn session_limit(opts: &Options) -> Result<()> {
    let limit = opts.maxmemory.unwrap();
    let mut control = Control::new(opts.port)?;
    control.config(b"maxmemory", b"0")?;
    control.config(b"maxmemory-policy", b"noeviction")?;
    // Both SET and GET begin with the same full, bounded set of session keys.
    fill(opts, opts.keys)?;
    if limit != 0 && control.info()?.used >= limit {
        return Err("session dataset already reaches the nonbinding limit".into());
    }
    control.config(b"maxmemory", limit.to_string().as_bytes())?;
    warmup(opts, None)?;
    let mut monitor = Monitor::new(control);
    let measured = measure(opts, None, Some(&mut monitor))?;
    if monitor.first.limit != limit || monitor.last.limit != limit
        || monitor.evicted_delta()? != 0
        || (limit != 0 && monitor.last.peak >= limit) {
        return Err("session comparison did not retain a nonbinding limit without eviction".into());
    }
    println!("{},{},{},{},{},{:.3},{}", measured.csv(opts), opts.keys, opts.value.len(),
        opts.seed, opts.warmup.as_secs_f64(), opts.sample_interval.as_secs_f64() * 1000.0, limit);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn persistence_body(enabled: bool, active: bool, rewrites: u64, status: &str, reason: &str) -> String {
        format!("aof_enabled:{}\r\naof_rewrite_in_progress:{}\r\naof_rewrite_scheduled:0\r\naof_rewrites:{rewrites}\r\naof_last_bgrewrite_status:{status}\r\nfirn_aof_rewrite_abort_reason:{reason}\r\n",
            u8::from(enabled), u8::from(active))
    }

    #[test]
    fn cache_rewrite_modes_require_time_and_keep_cache_defaults() {
        for mode in ["evict-zipf-rewrite", "evict-zipf-control"] {
            let args = ["--port", "6379", "--workload", mode, "--seconds", "1"];
            let opts = Options::parse_args(args.into_iter().map(str::to_string), 2).unwrap();
            assert_eq!((opts.keys, opts.value.len(), opts.connections), (1_000_000, 64, 50));
            assert!(Options::parse_args(["--port", "6379", "--workload", mode, "--requests", "2"]
                .into_iter().map(str::to_string), 2).is_err());
        }
    }

    #[test]
    fn cache_rewrite_configuration_disables_automatic_rewrites_and_checks_aof() {
        use std::net::TcpListener;
        for (requested, enabled, active, accepted) in [
            (true, true, false, true), (false, false, false, true),
            (true, false, false, false), (false, true, false, false),
            (true, true, true, false),
        ] {
            let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let port = listener.local_addr().unwrap().port();
            let server = thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                socket.set_read_timeout(Some(TIMEOUT)).unwrap();
                super::super::tests::expect_command(&mut socket, &[b"CONFIG", b"SET", b"auto-aof-rewrite-percentage", b"0"]);
                socket.write_all(b"+OK\r\n").unwrap();
                super::super::tests::expect_command(&mut socket, &[b"INFO", b"persistence"]);
                let body = persistence_body(enabled, active, 0, "ok", "");
                write!(socket, "${}\r\n{body}\r\n", body.len()).unwrap();
            });
            assert_eq!(RewriteWindow::configure(&mut Control::new(port).unwrap(), requested).is_ok(), accepted);
            server.join().unwrap();
        }
    }

    #[test]
    fn expired_cache_rewrite_deadline_stops_without_more_requests_or_invented_duration() {
        use std::net::TcpListener;
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (mut info, _) = listener.accept().unwrap();
            info.set_read_timeout(Some(TIMEOUT)).unwrap();
            super::super::tests::expect_command(&mut info, &[b"INFO", b"persistence"]);
            let body = persistence_body(true, false, 0, "ok", "");
            write!(info, "${}\r\n{body}\r\n", body.len()).unwrap();
            let (mut request, _) = listener.accept().unwrap();
            request.set_read_timeout(Some(TIMEOUT)).unwrap();
            assert_eq!(info.read(&mut [0]).unwrap(), 0, "an expired cap must not poll again");
            assert_eq!(request.read(&mut [0]).unwrap(), 0, "an expired cap must not issue a request");
        });
        let nominal = Duration::from_secs(1);
        let mut window = RewriteWindow::new(port, nominal, true).unwrap();
        // Enter the state of an outstanding request whose absolute deadline
        // already passed. No wall-clock wait or scheduler race is needed to
        // verify the same cap branch used by an unfinished server rewrite.
        let sent = Instant::now() - Duration::from_secs(121);
        window.sent.set(sent).unwrap();
        window.deadline = Some(sent + Duration::from_secs(120));
        assert!(window.tick(sent));
        let outcome = window.outcome.as_ref().unwrap();
        assert_eq!(outcome.status, "timeout");
        assert!(outcome.duration.is_none());
        assert_eq!(outcome.csv(), ",timeout,\"\"");
        assert_eq!(window.pause(sent, Duration::from_millis(10)), Duration::from_millis(10));
        // An expired stored deadline no longer classifies unrelated errors
        // after successful completion as rewrite timeouts.
        window.outcome = Some(RewriteOutcome { duration: Some(nominal), status: "ok", abort_reason: String::new() });
        window.fail("final verification failed after the rewrite completed");
        assert_eq!(window.outcome.as_ref().unwrap().status, "protocol_error");
        drop(window);
        server.join().unwrap();
    }

    #[test]
    fn cache_rewrite_records_one_request_completion_failure_and_control_with_counts() {
        use std::net::TcpListener;
        use std::sync::atomic::AtomicU64;
        for finish in ["ok", "err", "control", "extra", "rejected"] {
            let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let port = listener.local_addr().unwrap().port();
            let rewriting = finish != "control";
            let triggered = Arc::new(AtomicU64::new(0));
            let counts = Arc::new(AtomicU64::new(0));
            let traffic_ready = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
            let started = Arc::new(AtomicBool::new(false));
            let (server_triggered, server_counts, server_started) = (triggered.clone(), counts.clone(), started.clone());
            let nominal = Duration::from_millis(20);
            let server = thread::spawn(move || {
                let (mut info, _) = listener.accept().unwrap();
                info.set_read_timeout(Some(TIMEOUT)).unwrap();
                let ready = traffic_ready.clone();
                let persistence = thread::spawn(move || {
                    let mut expected = Vec::new();
                    command(&mut expected, &[b"INFO", b"persistence"]);
                    let mut polls = 0;
                    let mut active_polls = 0;
                    loop {
                        let mut wire = vec![0; expected.len()];
                        match info.read_exact(&mut wire) {
                            Ok(()) => (),
                            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
                            Err(e) => panic!("{e}"),
                        }
                        assert_eq!(wire, expected);
                        if polls > 0 {
                            // The pretrigger check (or control's final check)
                            // waits for a real hit and refused miss SET. Setup's
                            // first INFO must not wait for unstarted traffic.
                            let (lock, wake) = &*ready;
                            let (arrived, _) = wake.wait_timeout_while(lock.lock().unwrap(), TIMEOUT,
                                |arrived| !*arrived).unwrap();
                            assert!(*arrived, "traffic did not complete a hit and a refused miss");
                        }
                        polls += 1;
                        let begun = server_started.load(Ordering::Relaxed);
                        if begun { active_polls += 1; }
                        let done = active_polls >= 2;
                        if active_polls == 2 {
                            // Keep traffic alive past its nominal window. There
                            // is no short success deadline: delayed scheduling
                            // cannot turn a valid response into a test timeout.
                            thread::sleep(nominal);
                        }
                        let body = persistence_body(rewriting, begun && !done,
                            if finish == "extra" && done { 2 } else { u64::from(begun) },
                            if done && finish == "err" { "err" } else { "ok" },
                            if done && finish == "err" { "reserve,\"full\"" } else if finish == "rejected" { "admission" } else { "" });
                        if let Err(e) = write!(info, "${}\r\n{body}\r\n", body.len()) {
                            if matches!(e.kind(), io::ErrorKind::BrokenPipe | io::ErrorKind::ConnectionReset) { break; }
                            panic!("{e}");
                        }
                    }
                });
                let request = if rewriting {
                    let (mut socket, _) = listener.accept().unwrap();
                    socket.set_read_timeout(Some(TIMEOUT)).unwrap();
                    let started = started.clone();
                    Some(thread::spawn(move || {
                        super::super::tests::expect_command(&mut socket, &[b"BGREWRITEAOF"]);
                        server_triggered.fetch_add(1, Ordering::Relaxed);
                        started.store(true, Ordering::Relaxed);
                        socket.write_all(if finish == "rejected" { b"-ERR rejected\r\n" }
                            else { b"+Background append only file rewriting started\r\n" }).unwrap();
                        // Any second request on this connection fails this assertion.
                        assert_eq!(socket.read(&mut [0]).unwrap(), 0);
                    }))
                } else { None };
                let (mut socket, _) = listener.accept().unwrap();
                socket.set_read_timeout(Some(TIMEOUT)).unwrap();
                let mut get = Vec::new();
                command(&mut get, &[b"GET", b"key:0"]);
                loop {
                    let mut wire = vec![0; get.len()];
                    match socket.read_exact(&mut wire) {
                        Ok(()) => (),
                        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
                        Err(e) => panic!("{e}"),
                    }
                    assert_eq!(wire, get, "control and traffic connections must send only cache commands");
                    let count = server_counts.load(Ordering::Relaxed);
                    if count % 2 == 0 {
                        socket.write_all(b"$1\r\nx\r\n").unwrap();
                    } else {
                        socket.write_all(b"$-1\r\n").unwrap();
                        super::super::tests::expect_command(&mut socket, &[b"SET", b"key:0", b"x"]);
                        socket.write_all(OOM_REPLY).unwrap();
                    }
                    if server_counts.fetch_add(1, Ordering::Relaxed) == 1 {
                        let (ready, wake) = &*traffic_ready;
                        *ready.lock().unwrap() = true;
                        wake.notify_all();
                    }
                }
                if let Some(request) = request { request.join().unwrap(); }
                persistence.join().unwrap();
            });
            let mut opts = super::super::tests::options(port, if rewriting { "evict-zipf-rewrite" } else { "evict-zipf-control" });
            opts.connections = 1;
            opts.threads = 1;
            opts.seconds = Some(nominal);
            opts.requests = None;
            opts.sample_interval = Duration::from_millis(1);
            let mut window = RewriteWindow::new(port, opts.seconds.unwrap(), rewriting).unwrap();
            window.bound = TIMEOUT;
            let measured = measure_controlled(&opts, Some(Arc::new(Zipf::new(1, 0.99, 1).unwrap())),
                None, Some(&mut window)).unwrap();
            window.finish_request();
            let outcome = window.outcome.take().unwrap();
            drop(window);
            server.join().unwrap();
            assert_eq!(triggered.load(Ordering::Relaxed), u64::from(rewriting));
            let count = counts.load(Ordering::Relaxed);
            assert_eq!(measured.count, count);
            assert_eq!(measured.hits, count.div_ceil(2));
            assert_eq!(measured.refused_sets, count / 2);
            assert_eq!(measured.writers, 1);
            assert!(count >= 2);
            assert_eq!(outcome.status, match finish { "control" => "not_requested", "extra" => "protocol_error", "rejected" => "request_error", other => other });
            if matches!(finish, "ok" | "err") {
                assert!(outcome.duration.is_some());
                assert!(measured.seconds > opts.seconds.unwrap().as_secs_f64(), "traffic must extend past the nominal window");
            } else { assert!(outcome.duration.is_none()); }
            if finish == "err" {
                assert_eq!(outcome.abort_reason, "reserve,\"full\"");
                assert!(outcome.csv().ends_with(",err,\"reserve,\"\"full\"\"\""));
            }
            if finish == "rejected" { assert_eq!(outcome.abort_reason, "admission"); }
            // Exercise the production output-before-error path with the full
            // 36-field cache prefix. The memory columns are dummy values here;
            // traffic counts come from the measured fake-server exchange.
            let mut cache_columns = vec!["0".to_string(); 29];
            cache_columns[6] = measured.count.to_string();
            cache_columns[7] = measured.hits.to_string();
            cache_columns[8] = (measured.count - measured.hits).to_string();
            cache_columns[9] = measured.refused_sets.to_string();
            let row = format!("{},{}", measured.csv(&opts), cache_columns.join(","));
            let mut output = Vec::new();
            let result = write_result(&row, Some(&outcome), &mut output);
            assert_eq!(result.is_err(), outcome.failed());
            let output = String::from_utf8(output).unwrap();
            assert_eq!(output, format!("{row},{}\n", outcome.csv()));
            let columns: Vec<_> = output.split(',').collect();
            assert_eq!(columns[14], measured.hits.to_string());
            assert_eq!(columns[16], measured.refused_sets.to_string());
            assert_eq!(columns[37], outcome.status);
            if finish == "err" { assert!(output.ends_with(",err,\"reserve,\"\"full\"\"\"\n")); }
        }
    }

    #[test]
    fn finite_zipf_weights_and_permutation() {
        let zipf = Zipf::new(4, 1.0, 7).unwrap();
        // Independent finite Zipf oracle: weights 1, 1/2, 1/3, 1/4 sum to 25/12.
        for (actual, expected) in zipf.cumulative.iter().zip([12.0/25.0, 18.0/25.0, 22.0/25.0, 1.0]) {
            assert!((actual - expected).abs() < 1e-12);
        }
        let mut keys = zipf.permutation.clone();
        keys.sort_unstable();
        assert_eq!(keys, vec![0, 1, 2, 3]);
        let other = Zipf::new(4, 1.0, 7).unwrap();
        let (mut a, mut b) = (17, 17);
        for _ in 0..100 { assert_eq!(zipf.draw(&mut a), other.draw(&mut b)); }
        assert!(Zipf::new(0, 1.0, 1).is_err());
        assert!(Zipf::new(4, f64::NAN, 1).is_err());
        assert!(Zipf::new(4, -1.0, 1).is_err());
    }

    #[test]
    fn replies_and_memory_require_evidence() {
        assert!(!get_hit(b"$-1\r\n", b"xx").unwrap());
        assert!(get_hit(b"$2\r\nxx\r\n", b"xx").unwrap());
        for wire in [b"$1\r\nx\r\n".as_slice(), b"+OK\r\n", b"$2\r\nxy\r\n"] {
            assert!(get_hit(wire, b"xx").is_err());
        }
        assert_eq!(half_dataset_limit(100, 301).unwrap(), 200);
        assert!(half_dataset_limit(100, 99).is_err());
        assert!(half_dataset_limit(100, 101).is_err());
        let info = "used_memory:200\r\nused_memory_peak:400\r\nevicted_keys:7\r\nmaxmemory:150\r\nmem_not_counted_for_evict:10\r\n";
        let memory = Memory::parse(info).unwrap();
        assert_eq!((memory.used, memory.peak, memory.evicted), (200, 400, 7));
        for line in info.lines() {
            assert!(Memory::parse(&info.replace(&format!("{line}\r\n"), "")).is_err());
        }
    }

    #[test]
    fn persistence_requires_idle_rewrite_and_optional_schedule() {
        for (fields, idle) in [
            ("aof_rewrite_in_progress:0\r\n", true),
            ("aof_rewrite_in_progress:1\r\n", false),
            ("aof_rewrite_in_progress:0\r\naof_rewrite_scheduled:0\r\n", true),
            ("aof_rewrite_in_progress:1\r\naof_rewrite_scheduled:0\r\n", false),
            ("aof_rewrite_in_progress:0\r\naof_rewrite_scheduled:1\r\n", false),
            ("aof_rewrite_scheduled:1\r\naof_rewrite_in_progress:1\r\n", false),
        ] {
            let info = format!("# Persistence\r\naof_enabled:1\r\n{fields}aof_last_bgrewrite_status:ok\r\n");
            assert_eq!(rewrite_idle(&info).unwrap(), idle, "{fields}");
        }
        for info in [
            "", "aof_rewrite_scheduled:0\r\n", "aof_rewrite_in_progress:2\r\n",
            "aof_rewrite_in_progress:-1\r\n", "aof_rewrite_in_progress:\r\n",
            "aof_rewrite_in_progress:garbage\r\n",
            "aof_rewrite_in_progress:0\r\naof_rewrite_scheduled:2\r\n",
            "aof_rewrite_in_progress:0\r\naof_rewrite_scheduled:\r\n",
            "aof_rewrite_in_progress:1\r\naof_rewrite_scheduled:garbage\r\n",
        ] {
            assert!(rewrite_idle(info).is_err(), "{info}");
        }
    }

    #[test]
    fn dbsize_requires_a_nonnegative_integer_reply() {
        for (wire, count) in [(b":0\r\n".as_slice(), 0), (b":42\r\n", 42),
                              (b":9223372036854775807\r\n", i64::MAX as u64)] {
            assert_eq!(parse_dbsize(wire).unwrap(), count);
        }
        for wire in [b":-1\r\n".as_slice(), b"+OK\r\n", b"$1\r\n0\r\n",
                     b":\r\n", b":abc\r\n", b":1", b":9223372036854775808\r\n"] {
            assert!(parse_dbsize(wire).is_err(), "{wire:?}");
        }
    }

    #[test]
    fn reset_keeps_lifetime_peak_separate_from_sampled_memory() {
        use std::net::TcpListener;
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket.set_read_timeout(Some(TIMEOUT)).unwrap();
            super::super::tests::expect_command(&mut socket, &[b"CONFIG", b"RESETSTAT"]);
            socket.write_all(b"+OK\r\n").unwrap();
            for (used, evicted) in [(120, 2), (180, 5), (130, 7)] {
                super::super::tests::expect_command(&mut socket, &[b"INFO"]);
                let info = format!("used_memory:{used}\r\nused_memory_peak:400\r\nevicted_keys:{evicted}\r\nmaxmemory:150\r\nmem_not_counted_for_evict:0\r\n");
                write!(socket, "${}\r\n{info}\r\n", info.len()).unwrap();
                if evicted == 2 {
                    super::super::tests::expect_command(&mut socket, &[b"DBSIZE"]);
                    socket.write_all(b":17\r\n").unwrap();
                }
            }
        });
        let mut monitor = Monitor::new(Control::new(port).unwrap());
        monitor.start().unwrap();
        monitor.sample().unwrap();
        monitor.sample().unwrap();
        server.join().unwrap();
        assert_eq!(monitor.evicted_delta().unwrap(), 5);
        assert_eq!(monitor.first.peak, 400);
        assert_eq!(monitor.last.peak.saturating_sub(monitor.last.limit), 250);
        assert_eq!(monitor.sampled_max.saturating_sub(monitor.last.limit), 30);
        assert_eq!(monitor.samples, 3);
        assert_eq!(monitor.keys_at_start, 17);
        monitor.last.evicted = 1;
        assert!(monitor.evicted_delta().is_err());
    }

    #[test]
    fn cache_aside_counts_one_get_and_checks_miss_set_and_hit() {
        use std::net::TcpListener;
        for (set_reply, bad) in [(b"+OK\r\n".as_slice(), false), (OOM_REPLY, false),
                                 (b"+OK\r\n".as_slice(), true)] {
            let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let mut opts = super::super::tests::options(listener.local_addr().unwrap().port(), "evict-zipf");
            opts.connections = 1;
            opts.threads = 1;
            opts.requests = Some(2);
            let server = thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                socket.set_read_timeout(Some(TIMEOUT)).unwrap();
                super::super::tests::expect_command(&mut socket, &[b"GET", b"key:0"]);
                // Fragment the miss to exercise the ordinary RESP buffering.
                socket.write_all(b"$-1\r").unwrap();
                socket.write_all(b"\n").unwrap();
                super::super::tests::expect_command(&mut socket, &[b"SET", b"key:0", b"x"]);
                socket.write_all(&set_reply[..set_reply.len() - 1]).unwrap();
                socket.write_all(&set_reply[set_reply.len() - 1..]).unwrap();
                super::super::tests::expect_command(&mut socket, &[b"GET", b"key:0"]);
                socket.write_all(if bad { b"$1\r\ny\r\n" } else { b"$1\r\nx\r\n" }).unwrap();
            });
            let result = measure(&opts, Some(Arc::new(Zipf::new(1, 0.99, 1).unwrap())), None);
            server.join().unwrap();
            if bad {
                assert!(result.err().unwrap().to_string().contains("configured value"));
            } else {
                let measured = result.unwrap();
                assert_eq!((measured.count, measured.hits), (2, 1));
                assert_eq!(measured.refused_sets, u64::from(set_reply == OOM_REPLY));
                assert_eq!(measured.writers, 1);
            }
        }
    }

    #[test]
    fn cache_aside_rejects_other_set_errors_and_get_oom() {
        use std::net::TcpListener;
        for (on_set, response, message) in [
            (true, b"-ERR rejected\r\n".as_slice(), "server error: ERR rejected"),
            (true, b"-OOM different error\r\n".as_slice(), "server error: OOM different error"),
            (true, b":1\r\n".as_slice(), "cache SET did not return OK"),
            (false, OOM_REPLY, "server error: OOM"),
        ] {
            let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let mut opts = super::super::tests::options(listener.local_addr().unwrap().port(), "evict-zipf");
            opts.connections = 1;
            opts.threads = 1;
            opts.requests = Some(1);
            let server = thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                socket.set_read_timeout(Some(TIMEOUT)).unwrap();
                super::super::tests::expect_command(&mut socket, &[b"GET", b"key:0"]);
                if on_set {
                    socket.write_all(b"$-1\r\n").unwrap();
                    super::super::tests::expect_command(&mut socket, &[b"SET", b"key:0", b"x"]);
                }
                socket.write_all(response).unwrap();
            });
            let result = measure(&opts, Some(Arc::new(Zipf::new(1, 0.99, 1).unwrap())), None);
            server.join().unwrap();
            assert!(result.err().unwrap().to_string().contains(message));
        }
    }
}
