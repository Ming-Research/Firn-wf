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
        let mut out = Vec::new();
        command(&mut out, args);
        self.socket.write_all(&out)?;
        let mut input = Vec::new();
        loop {
            if let Some(size) = reply(&input, 0)? {
                if size != input.len() { return Err("extra control reply".into()); }
                return Ok(input);
            }
            read_more(&mut self.socket, &mut input)?;
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
        let wire = self.call(&[b"INFO"])?;
        if wire.first() != Some(&b'$') || wire == b"$-1\r\n" { return Err("INFO is not a bulk string".into()); }
        let start = wire.windows(2).position(|w| w == b"\r\n").ok_or("missing INFO header")? + 2;
        Memory::parse(std::str::from_utf8(&wire[start..wire.len() - 2])?)
    }
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

pub(super) struct Monitor {
    control: Control,
    first: Memory,
    last: Memory,
    sampled_max: u64,
    samples: u64,
    last_sample: Instant,
    max_gap: Duration,
}

impl Monitor {
    fn new(control: Control) -> Self {
        Self { control, first: Memory::default(), last: Memory::default(),
            sampled_max: 0, samples: 0, last_sample: Instant::now(), max_gap: Duration::ZERO }
    }

    pub(super) fn start(&mut self) -> Result<()> {
        self.control.ok(&[b"CONFIG", b"RESETSTAT"])?;
        self.first = self.control.info()?;
        self.last = self.first;
        self.sampled_max = self.first.used;
        self.samples = 1;
        self.last_sample = Instant::now();
        self.max_gap = Duration::ZERO;
        Ok(())
    }

    pub(super) fn sample(&mut self) -> Result<()> {
        self.last = self.control.info()?;
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
    control.config(b"maxmemory", b"0")?;
    control.config(b"maxmemory-policy", b"allkeys-lru")?;
    // Redis 7.0.15 defaults, explicit so both servers use the same settings.
    control.config(b"maxmemory-samples", b"5")?;
    control.config(b"maxmemory-eviction-tenacity", b"10")?;
    let before = control.info()?.used;
    fill(opts, opts.keys)?;
    let filled = control.info()?.used;
    let limit = half_dataset_limit(before, filled)?;
    control.config(b"maxmemory", limit.to_string().as_bytes())?;
    warmup(opts, Some(zipf.clone()))?;
    let mut monitor = Monitor::new(control);
    let measured = measure(opts, Some(zipf), Some(&mut monitor))?;
    if measured.writers != opts.connections {
        return Err("not every connection completed a miss SET attempt; increase the run duration or dataset".into());
    }
    if monitor.first.limit != limit || monitor.last.limit != limit {
        return Err("INFO maxmemory differs from the configured limit".into());
    }
    let sampled_excess = monitor.sampled_max.saturating_sub(limit);
    let lifetime_excess = monitor.last.peak.saturating_sub(limit);
    // The shell copies the leading seven columns to workloads.csv; the full
    // row goes to evict-zipf.csv under its own explicit header.
    println!("{},{},{},{},{},{},{:.3},{},{},{},{},{:.9},{},{},{},{},{},{},{},{},{:.9},{},{},{:.9},{},{:.3},{},{}",
        measured.csv(opts), opts.keys, opts.zipf_s, opts.value.len(), opts.seed,
        opts.warmup.as_secs_f64(), opts.sample_interval.as_secs_f64() * 1000.0,
        measured.count, measured.hits, measured.count - measured.hits, measured.refused_sets,
        measured.hits as f64 / measured.count as f64, monitor.evicted_delta()?,
        monitor.last.used, monitor.last.peak, limit, before, filled, monitor.first.peak,
        lifetime_excess, lifetime_excess as f64 / limit as f64,
        monitor.sampled_max, sampled_excess, sampled_excess as f64 / limit as f64,
        monitor.samples, monitor.max_gap.as_secs_f64() * 1000.0, monitor.last.excluded, measured.writers);
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
        for line in info.lines() {
            assert!(Memory::parse(&info.replace(&format!("{line}\r\n"), "")).is_err());
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
