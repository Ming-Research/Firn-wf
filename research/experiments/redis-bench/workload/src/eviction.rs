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

struct Control {
    socket: TcpStream,
}

impl Control {
    fn new(port: u16) -> Result<Self> { Ok(Self { socket: connect(port)? }) }

    fn call(&mut self, args: &[&[u8]]) -> Result<Vec<u8>> {
        self.call_before(args, None)
    }

    fn call_before(&mut self, args: &[&[u8]], deadline: Option<Instant>) -> Result<Vec<u8>> {
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

    fn config(&mut self, key: &[u8], value: &[u8]) -> Result<()> {
        self.ok(&[b"CONFIG", b"SET", key, value])
    }

    fn info(&mut self) -> Result<Memory> {
        Ok(self.observation(false)?.0)
    }

    fn observation(&mut self, persistence: bool) -> Result<(Memory, Option<Persistence>)> {
        let wire = self.call(&[b"INFO"])?;
        let body = info_body(&wire)?;
        Ok((Memory::parse(body)?, if persistence { Some(Persistence::parse(body)?) } else { None }))
    }

    fn dbsize(&mut self) -> Result<u64> {
        parse_dbsize(&self.call(&[b"DBSIZE"])?)
    }

    /// Firn replays AOF files into private keyspaces in the process heap during
    /// rewrite; Redis rewrites in a fork child. A fill reaching the automatic
    /// rewrite minimum (64 MiB) can therefore inflate firn's half-dataset limit:
    /// the 14900K probe (Firn-wf run 37996029478) read 390 MB with AOF versus
    /// 226 MB without. Wait for rewrite quiescence, then discard the first of
    /// two memory readings 200 ms apart before selecting filled_used_memory.
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
fn info_body(wire: &[u8]) -> Result<&str> {
    if wire.first() != Some(&b'$') || wire == b"$-1\r\n" { return Err("INFO is not a bulk string".into()); }
    let start = wire.windows(2).position(|w| w == b"\r\n").ok_or("missing INFO header")? + 2;
    Ok(std::str::from_utf8(&wire[start..wire.len() - 2])?)
}

fn rewrite_idle(info: &str) -> Result<bool> {
    let fields: HashMap<&str, &str> = info.lines().filter_map(|line| line.split_once(':')).collect();
    let in_progress = persistence_flag(&fields, "aof_rewrite_in_progress", false)?;
    let scheduled = persistence_flag(&fields, "aof_rewrite_scheduled", true)?;
    Ok(!in_progress && !scheduled)
}

fn persistence_flag(fields: &HashMap<&str, &str>, name: &str, optional: bool) -> Result<bool> {
    match fields.get(name).copied() {
        Some("0") => Ok(false),
        Some("1") => Ok(true),
        None if optional => Ok(false),
        None => Err(format!("INFO persistence missing {name}").into()),
        Some(value) => Err(format!("INFO persistence invalid {name}: {value}").into()),
    }
}

#[derive(Clone, Copy)]
struct Persistence {
    enabled: bool,
    in_progress: bool,
    scheduled: bool,
    last_ok: bool,
}

impl Persistence {
    fn parse(info: &str) -> Result<Self> {
        let fields: HashMap<&str, &str> = info.lines().filter_map(|line| line.split_once(':')).collect();
        let enabled = persistence_flag(&fields, "aof_enabled", false)?;
        let last_ok = match fields.get("aof_last_bgrewrite_status").copied() {
            Some("ok") => true,
            Some("err") => false,
            None if !enabled => false,
            _ => return Err("INFO persistence missing or invalid aof_last_bgrewrite_status".into()),
        };
        Ok(Self { enabled, last_ok,
            in_progress: persistence_flag(&fields, "aof_rewrite_in_progress", false)?,
            scheduled: persistence_flag(&fields, "aof_rewrite_scheduled", true)? })
    }
}

fn rewrite_started(wire: &[u8]) -> Result<()> {
    if wire != b"+Background append only file rewriting started\r\n" {
        return Err(format!("BGREWRITEAOF did not start: {}", String::from_utf8_lossy(wire)).into());
    }
    Ok(())
}

fn rewrite_summary(samples: &[(Instant, Persistence)], end: Instant, acknowledged: Instant)
    -> (Duration, bool) {
    let mut overlap = Duration::ZERO;
    let mut seen_active = false;
    let mut completed = false;
    // Hold each sampled flag until the next observation, clipped to the last
    // worker's end. This is a sampled estimate, not exact rewrite timing.
    for (i, &(at, p)) in samples.iter().enumerate() {
        if at > end { break; }
        if p.in_progress {
            seen_active = true;
            let next = samples.get(i + 1).map_or(end, |s| s.0.min(end));
            overlap += next.duration_since(at);
        }
        // A fast rewrite can finish before the first poll. Idle before either
        // observed activity or the successful start reply proves nothing.
        completed = (seen_active || at >= acknowledged)
            && !p.in_progress && !p.scheduled && p.last_ok;
    }
    (overlap, completed)
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
    fn adjusted(&self) -> u64 { self.used.saturating_sub(self.excluded) }

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

pub(super) struct Monitor {
    control: Control,
    first: Memory,
    last: Memory,
    sampled_max: u64,
    sampled_max_adjusted: u64,
    samples: u64,
    last_sample: Instant,
    max_gap: Duration,
    keys_at_start: u64,
    keys_at_end: u64,
    rewrite_control: Option<Control>,
    rewrite_handle: Option<thread::JoinHandle<Result<Instant>>>,
    rewrite_samples: Vec<(Instant, Persistence)>,
    rewrite_requested: bool,
    rewrite_overlap: Duration,
    rewrite_completed: bool,
}

impl Monitor {
    fn new(control: Control) -> Self {
        Self { control, first: Memory::default(), last: Memory::default(),
            sampled_max: 0, sampled_max_adjusted: 0, samples: 0,
            last_sample: Instant::now(), max_gap: Duration::ZERO,
            keys_at_start: 0, keys_at_end: 0, rewrite_control: None, rewrite_handle: None,
            rewrite_samples: Vec::new(), rewrite_requested: false,
            rewrite_overlap: Duration::ZERO, rewrite_completed: false }
    }

    pub(super) fn start(&mut self) -> Result<()> {
        self.control.ok(&[b"CONFIG", b"RESETSTAT"])?;
        let (memory, persistence) = self.control.observation(self.rewrite_control.is_some())?;
        self.first = memory;
        if let Some(p) = persistence {
            if !p.enabled {
                // Redis can rewrite even with appendonly off; this experiment must not.
                self.rewrite_control = None;
            } else if p.in_progress || p.scheduled {
                return Err("rewrite already active at measurement baseline; retry the run".into());
            }
        }
        self.keys_at_start = self.control.dbsize()?;
        self.last = self.first;
        self.sampled_max = self.first.used;
        self.sampled_max_adjusted = self.first.adjusted();
        self.samples = 1;
        self.last_sample = Instant::now();
        self.max_gap = Duration::ZERO;
        Ok(())
    }

    pub(super) fn sample(&mut self) -> Result<()> {
        let (memory, persistence) = self.control.observation(self.rewrite_requested)?;
        self.last = memory;
        let now = Instant::now();
        if let Some(p) = persistence { self.rewrite_samples.push((now, p)); }
        self.max_gap = self.max_gap.max(now.duration_since(self.last_sample));
        self.last_sample = now;
        self.sampled_max = self.sampled_max.max(self.last.used);
        self.sampled_max_adjusted = self.sampled_max_adjusted.max(self.last.adjusted());
        self.samples += 1;
        Ok(())
    }

    // Called after the workers' barrier opens. Firn may await close-tail replay
    // before replying, so the requester must not block the INFO sampler.
    pub(super) fn request_rewrite(&mut self) {
        if let Some(mut control) = self.rewrite_control.take() {
            self.rewrite_requested = true;
            self.rewrite_handle = Some(thread::spawn(move || {
                rewrite_started(&control.call(&[b"BGREWRITEAOF"])?)?;
                Ok(Instant::now())
            }));
        }
    }

    pub(super) fn finish(&mut self, end: Instant) -> Result<()> {
        // Capture ending keys before a slow rewrite acknowledgement can delay
        // reporting beyond the final memory observation.
        self.keys_at_end = self.control.dbsize()?;
        if let Some(handle) = self.rewrite_handle.take() {
            let acknowledged = handle.join().map_err(|_| "rewrite requester panicked")??;
            (self.rewrite_overlap, self.rewrite_completed) =
                rewrite_summary(&self.rewrite_samples, end, acknowledged);
        }
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
    control.config(b"maxmemory", b"0")?;
    control.config(b"maxmemory-policy", b"allkeys-lru")?;
    // Redis 7.0.15 defaults, explicit so both servers use the same settings.
    control.config(b"maxmemory-samples", b"5")?;
    control.config(b"maxmemory-eviction-tenacity", b"10")?;
    let before = control.info()?.used;
    fill(opts, opts.keys)?;
    let filled = control.filled_memory()?;
    let limit = half_dataset_limit(before, filled)?;
    control.config(b"maxmemory", limit.to_string().as_bytes())?;
    warmup(opts, Some(zipf.clone()))?;
    let mut monitor = Monitor::new(control);
    if opts.rewrite_during_measure { monitor.rewrite_control = Some(Control::new(opts.port)?); }
    let measured = measure(opts, Some(zipf), Some(&mut monitor))?;
    if measured.writers != opts.connections {
        return Err("not every connection completed a miss SET attempt; increase the run duration or dataset".into());
    }
    if monitor.first.limit != limit || monitor.last.limit != limit {
        return Err("INFO maxmemory differs from the configured limit".into());
    }
    let sampled_excess = monitor.sampled_max_adjusted.saturating_sub(limit);
    let lifetime_excess = monitor.last.peak.saturating_sub(limit);
    // The shell copies the leading seven columns to workloads.csv; the full
    // row goes to evict-zipf.csv under its own explicit header.
    println!("{},{},{},{},{},{},{:.3},{},{},{},{},{:.9},{},{},{},{},{},{},{},{},{:.9},{},{},{:.9},{},{:.3},{},{},{},{},{},{},{:.3},{}",
        measured.csv(opts), opts.keys, opts.zipf_s, opts.value.len(), opts.seed,
        opts.warmup.as_secs_f64(), opts.sample_interval.as_secs_f64() * 1000.0,
        measured.count, measured.hits, measured.count - measured.hits, measured.refused_sets,
        measured.hits as f64 / measured.count as f64, monitor.evicted_delta()?,
        monitor.last.used, monitor.last.peak, limit, before, filled, monitor.first.peak,
        lifetime_excess, lifetime_excess as f64 / limit as f64,
        monitor.sampled_max, sampled_excess, sampled_excess as f64 / limit as f64,
        monitor.samples, monitor.max_gap.as_secs_f64() * 1000.0, monitor.last.excluded, measured.writers,
        monitor.keys_at_start, monitor.keys_at_end, monitor.sampled_max_adjusted,
        u8::from(monitor.rewrite_requested), monitor.rewrite_overlap.as_secs_f64() * 1000.0,
        u8::from(monitor.rewrite_completed));
    Ok(())
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
        assert_eq!(memory.adjusted(), 190);
        assert_eq!(Memory::parse(&info.replace("evict:10", "evict:250")).unwrap().adjusted(), 0);
        for value in ["-1", "garbage", "18446744073709551616"] {
            assert!(Memory::parse(&info.replace("evict:10", &format!("evict:{value}"))).is_err());
        }
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
    fn rewrite_observations_require_valid_flags_and_status() {
        let info = "# Persistence\r\naof_enabled:1\r\naof_rewrite_in_progress:1\r\naof_rewrite_scheduled:0\r\naof_last_bgrewrite_status:ok\r\n";
        let p = Persistence::parse(info).unwrap();
        assert!(p.enabled && p.in_progress && !p.scheduled && p.last_ok);
        assert!(!Persistence::parse(&info.replace("status:ok", "status:err")).unwrap().last_ok);
        assert!(Persistence::parse(&info.replace("scheduled:0", "scheduled:1")).unwrap().scheduled);
        let disabled = Persistence::parse("aof_enabled:0\r\naof_rewrite_in_progress:0\r\n").unwrap();
        assert!(!disabled.enabled && !disabled.in_progress && !disabled.last_ok);
        for field in ["aof_enabled:1", "aof_rewrite_in_progress:1", "aof_last_bgrewrite_status:ok"] {
            assert!(Persistence::parse(&info.replace(&format!("{field}\r\n"), "")).is_err());
        }
        for field in ["aof_enabled:1", "aof_rewrite_in_progress:1", "aof_rewrite_scheduled:0",
                      "aof_last_bgrewrite_status:ok"] {
            let name = field.split_once(':').unwrap().0;
            for invalid in ["", "2", "-1", "garbage"] {
                assert!(Persistence::parse(&info.replace(field, &format!("{name}:{invalid}"))).is_err());
            }
        }
        assert!(rewrite_started(b"+Background append only file rewriting started\r\n").is_ok());
        for wire in [b"+OK\r\n".as_slice(), b"-ERR already in progress\r\n",
                     b"+Background append only file rewriting scheduled\r\n", b":1\r\n"] {
            assert!(rewrite_started(wire).is_err());
        }
    }

    #[test]
    fn rewrite_overlap_and_completion_stop_at_measured_end() {
        let begin = Instant::now();
        let at = |ms| begin + Duration::from_millis(ms);
        let active = Persistence { enabled: true, in_progress: true, scheduled: false, last_ok: true };
        let idle = Persistence { in_progress: false, ..active };
        // Two active intervals, 10..20 and 20..35, not baseline..10.
        let samples = [(at(10), active), (at(20), active), (at(35), idle)];
        assert_eq!(rewrite_summary(&samples, at(40), at(5)), (Duration::from_millis(25), true));
        // Completion after the measured end cannot turn an unfinished run into a pass.
        assert_eq!(rewrite_summary(&samples, at(30), at(5)), (Duration::from_millis(20), false));
        assert_eq!(rewrite_summary(&[(at(10), idle)], at(40), at(5)), (Duration::ZERO, true));
        assert_eq!(rewrite_summary(&[(at(10), idle)], at(40), at(15)), (Duration::ZERO, false));
        assert_eq!(rewrite_summary(&[], at(40), at(5)), (Duration::ZERO, false));
        for incomplete in [Persistence { last_ok: false, ..idle }, Persistence { scheduled: true, ..idle }] {
            assert_eq!(rewrite_summary(&[(at(10), active), (at(20), incomplete)], at(40), at(5)),
                (Duration::from_millis(10), false));
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
            for (used, excluded, evicted) in [(120, 0, 2), (180, 80, 5), (170, 10, 7)] {
                super::super::tests::expect_command(&mut socket, &[b"INFO"]);
                let info = format!("used_memory:{used}\r\nused_memory_peak:400\r\nevicted_keys:{evicted}\r\nmaxmemory:150\r\nmem_not_counted_for_evict:{excluded}\r\n");
                write!(socket, "${}\r\n{info}\r\n", info.len()).unwrap();
                if evicted == 2 {
                    super::super::tests::expect_command(&mut socket, &[b"DBSIZE"]);
                    socket.write_all(b":17\r\n").unwrap();
                }
            }
            super::super::tests::expect_command(&mut socket, &[b"DBSIZE"]);
            socket.write_all(b":13\r\n").unwrap();
        });
        let mut monitor = Monitor::new(Control::new(port).unwrap());
        monitor.start().unwrap();
        monitor.sample().unwrap();
        monitor.sample().unwrap();
        monitor.finish(Instant::now()).unwrap();
        server.join().unwrap();
        assert_eq!(monitor.evicted_delta().unwrap(), 5);
        assert_eq!(monitor.first.peak, 400);
        assert_eq!(monitor.last.peak.saturating_sub(monitor.last.limit), 250);
        assert_eq!(monitor.sampled_max.saturating_sub(monitor.last.limit), 30);
        // The largest raw sample is not the largest admission sample. Nor can
        // the last exclusion be subtracted from the raw maximum after sampling.
        assert_eq!(monitor.sampled_max_adjusted, 160);
        assert_eq!(monitor.sampled_max_adjusted.saturating_sub(monitor.last.limit), 10);
        assert!(!monitor.rewrite_requested);
        assert_eq!(monitor.samples, 3);
        assert_eq!(monitor.keys_at_start, 17);
        assert_eq!(monitor.keys_at_end, 13);
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
