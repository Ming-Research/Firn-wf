//! The process and fixture helpers used by the network cases.

use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, TryLockError};
use std::time::{Duration, Instant};

static NEXT_EXECUTION: AtomicU64 = AtomicU64::new(0);
static LEASED_PORTS: OnceLock<Mutex<HashSet<u16>>> = OnceLock::new();

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

struct LeasedPort {
    port: u16,
}

impl Drop for LeasedPort {
    fn drop(&mut self) {
        locked(LEASED_PORTS.get_or_init(|| Mutex::new(HashSet::new()))).remove(&self.port);
    }
}

/// A loopback port reserved against reuse by another test in this process.
/// Clones share the reservation until the last owner drops it; the operating
/// system listener is released so the server can bind the port itself.
#[derive(Clone)]
pub struct PortLease {
    reservation: Arc<LeasedPort>,
}

impl PortLease {
    /// The port the server and its peers use while this lease is alive.
    pub fn port(&self) -> u16 {
        self.reservation.port
    }
}

/// Leases an operating-system-selected port not held by any other test.
/// Keep the lease across every server lifetime, including restarts on that
/// port; `ProgramChild::with_port` also retains it for a single child.
pub fn lease_port() -> PortLease {
    // Keep each candidate's listener bound until the registry has accepted
    // or refused it. A refused listener is dropped before choosing again.
    let mut listener = None;
    lease_from(|| {
        drop(listener.take());
        let bound = std::net::TcpListener::bind(("127.0.0.1", 0))
            .expect("bind loopback port for lease");
        let port = bound.local_addr().expect("leased listener address").port();
        listener = Some(bound);
        port
    })
}

/// Reserves a literal port used by a documented command, without rewriting
/// the example. A conflicting listener or in-process lease fails the case.
pub(super) fn lease_named_port(port: u16) -> PortLease {
    let mut ports = locked(LEASED_PORTS.get_or_init(|| Mutex::new(HashSet::new())));
    assert!(!ports.contains(&port), "documented port {port} is already leased");
    let listener = std::net::TcpListener::bind(("127.0.0.1", port))
        .unwrap_or_else(|error| panic!("reserve documented port {port}: {error}"));
    ports.insert(port);
    drop(listener);
    PortLease { reservation: Arc::new(LeasedPort { port }) }
}

/// Retries candidates until the registry grants an exclusive reservation.
fn lease_from(mut candidate: impl FnMut() -> u16) -> PortLease {
    loop {
        let port = candidate();
        if locked(LEASED_PORTS.get_or_init(|| Mutex::new(HashSet::new()))).insert(port) {
            return PortLease {
                reservation: Arc::new(LeasedPort { port }),
            };
        }
    }
}

fn select_route(command: &mut Command, native: bool) {
    command.env_remove("WF_REQUIRE_WINDOWS_IOCP");
    if native {
        command.env_remove("WF_IO_NO_NATIVE_RING");
    } else {
        command.env("WF_IO_NO_NATIVE_RING", "1");
    }
}

/// Preserves the raw argument bytes on the Linux hosts these cases use.
fn invocation_argument(bytes: &[u8]) -> OsString {
    OsStr::from_bytes(bytes).to_os_string()
}

/// One externally built executable that a case invokes repeatedly.
pub struct CompiledProgram {
    directory: PathBuf,
    executable: PathBuf,
}

impl CompiledProgram {
    pub fn from_environment() -> Self {
        Self::from_environment_under(&std::env::temp_dir())
    }

    /// The program `FIRN` names, run in a fresh directory under `parent`
    /// rather than the host's temporary directory, for a case whose files
    /// must lie on a particular filesystem.
    pub fn from_environment_under(parent: &Path) -> Self {
        let executable = std::env::var_os("FIRN")
            .map(PathBuf::from)
            .expect("FIRN must name the absolute path to a built firn executable");
        assert!(
            executable.is_absolute() && executable.is_file(),
            "FIRN must name an existing file by absolute path, got {}",
            executable.display()
        );
        let sequence = NEXT_EXECUTION.fetch_add(1, Ordering::Relaxed);
        let directory = parent.join(format!(
            "firn-tests-program-{}-{sequence}",
            std::process::id()
        ));
        std::fs::create_dir(&directory).expect("create unique program directory");
        Self {
            directory,
            executable,
        }
    }

    /// The directory the program runs in, where a file it names relative to
    /// its working directory lies, so that a case can read what it wrote.
    #[cfg(target_os = "linux")]
    pub(super) fn working_directory(&self) -> &Path {
        &self.directory
    }

    /// Runs the program in `working_directory` with `arguments` as argv[1..].
    ///
    /// Arguments are raw bytes, because the program reads them through the
    /// lossless host-string route and a case must be able to supply an
    /// argument that is not valid UTF-8.
    pub fn run(&self, working_directory: &Path, arguments: &[&[u8]]) -> Output {
        run_command(
            Command::new(&self.executable)
                .current_dir(working_directory)
                .args(arguments.iter().map(|bytes| invocation_argument(bytes))),
        )
    }

    /// Starts the program in `working_directory` with captured output, for a
    /// case that must observe the child while a peer runs concurrently.
    pub fn spawn_in_directory(
        &self,
        working_directory: &Path,
        arguments: &[&[u8]],
    ) -> ProgramChild {
        ProgramChild::spawn(
            Command::new(&self.executable)
                .current_dir(working_directory)
                .args(arguments.iter().map(|bytes| invocation_argument(bytes)))
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped()),
        )
        .expect("spawn compiled program")
    }

    /// Starts the program on one runtime route with raw invocation arguments
    /// and hands back the running child.
    /// Firn callers supply Redis-style directives explicitly, including
    /// `--firn-clients N` when the test ends after accepting N clients; the
    /// harness passes argv through unchanged.
    ///
    /// A loopback case has to play the peer while the program runs, so it
    /// needs the child rather than the finished output: the program is a
    /// server the case connects to, or a client the case accepts from, and
    /// either way both sides are alive at once. `native_ring` selects the
    /// route exactly as the standard-input cases do — `true` is the shipped
    /// default, `false` sets `WF_IO_NO_NATIVE_RING` so the same program runs
    /// through the shared file adapter instead of the kernel completion ring.
    pub fn spawn_on_route(&self, native_ring: bool, arguments: &[&[u8]]) -> ProgramChild {
        self.spawn_on_route_with(native_ring, &[], arguments)
    }

    /// Starts the program on one runtime route with the thread counts named.
    ///
    /// A case whose property is about several peers being served at once has
    /// to state the threads it is served by, because the shipped defaults size
    /// them to the machine: a host with many cores serves four peers on four
    /// workers whatever the runtime does with a wait, so the property would be
    /// proved by the runner rather than by the program, and a host with one
    /// core runs every context on one driver. `settings` names `WF_WORKERS`
    /// or `WF_DRIVERS` with the count a case pins; each one it does not name
    /// takes the shipped default, whatever the runner's environment holds.
    pub fn spawn_on_route_with(
        &self,
        native_ring: bool,
        settings: &[(&str, &str)],
        arguments: &[&[u8]],
    ) -> ProgramChild {
        self.spawn_on_route_options(native_ring, settings, arguments, false, PROGRAM_DEADLINE)
    }

    /// Starts the program as `spawn_on_route_with` does, stopping it after
    /// `limit` instead of `PROGRAM_DEADLINE`: for a case whose property is an
    /// interval the program itself keeps, such as a 60-second output limit,
    /// which the shared deadline would end first.
    pub fn spawn_on_route_with_limit(
        &self,
        native_ring: bool,
        settings: &[(&str, &str)],
        arguments: &[&[u8]],
        limit: Duration,
    ) -> ProgramChild {
        self.spawn_on_route_options(native_ring, settings, arguments, false, limit)
    }

    /// Linux append-failure cases inherit ignored SIGXFSZ so RLIMIT_FSIZE
    /// makes write return EFBIG instead of terminating the owned child.
    #[cfg(target_os = "linux")]
    pub fn spawn_on_route_ignoring_xfsz(
        &self,
        native_ring: bool,
        arguments: &[&[u8]],
    ) -> ProgramChild {
        self.spawn_on_route_options(native_ring, &[], arguments, true, PROGRAM_DEADLINE)
    }

    fn spawn_on_route_options(
        &self,
        native_ring: bool,
        settings: &[(&str, &str)],
        arguments: &[&[u8]],
        ignore_xfsz: bool,
        limit: Duration,
    ) -> ProgramChild {
        let mut command = Command::new(&self.executable);
        #[cfg(target_os = "linux")]
        if ignore_xfsz {
            use std::os::unix::process::CommandExt;
            // signal is async-signal-safe. SIG_IGN survives exec; no parent
            // disposition changes, so parallel tests remain independent.
            unsafe extern "C" {
                fn signal(signal: i32, handler: usize) -> usize;
            }
            unsafe {
                command.pre_exec(|| {
                    if signal(25, 1) == usize::MAX {
                        Err(std::io::Error::last_os_error())
                    } else {
                        Ok(())
                    }
                });
            }
        }
        #[cfg(not(target_os = "linux"))]
        let _ = ignore_xfsz;
        command
            .current_dir(&self.directory)
            .args(arguments.iter().map(|bytes| invocation_argument(bytes)))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        select_route(&mut command, native_ring);
        command.env_remove("WF_WORKERS").env_remove("WF_DRIVERS");
        for (name, count) in settings {
            command.env(name, count);
        }
        ProgramChild::spawn_with_limit(&mut command, limit).expect("spawn compiled program")
    }
}

impl Drop for CompiledProgram {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

/// One directory whose complete content a case fixes.
pub struct FixtureDirectory {
    path: PathBuf,
}

pub fn fixture_directory() -> FixtureDirectory {
    let sequence = NEXT_EXECUTION.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "firn-tests-fixtures-{}-{sequence}",
        std::process::id()
    ));
    std::fs::create_dir(&path).expect("create unique fixture directory");
    FixtureDirectory { path }
}

impl FixtureDirectory {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for FixtureDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// How long a test lets a program run before it stops the program and fails:
/// far past any test program's time, so it stops a program that never
/// finishes.
pub(crate) const PROGRAM_DEADLINE: Duration = Duration::from_secs(60);

const DIAGNOSTIC_TAIL_BYTES: usize = 4096;
const DIAGNOSTIC_LIMIT: Duration = Duration::from_secs(2);

/// A diagnostic never waits behind another thread's blocking cleanup.
fn locked_until<T>(mutex: &Mutex<T>, deadline: Instant) -> Option<MutexGuard<'_, T>> {
    loop {
        match mutex.try_lock() {
            Ok(guard) => return Some(guard),
            Err(TryLockError::Poisoned(error)) => return Some(error.into_inner()),
            Err(TryLockError::WouldBlock) => {}
        }
        if !pause_until(deadline) {
            return None;
        }
    }
}

/// Poll briefly without extending the shared diagnostic deadline.
fn pause_until(deadline: Instant) -> bool {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return false;
    }
    std::thread::sleep(remaining.min(Duration::from_millis(5)));
    true
}

/// Marks completion even if a reader returns an error or unwinds.
struct ReaderCompletion {
    complete: Arc<AtomicBool>,
    failure: Arc<Mutex<Option<String>>>,
}

impl Drop for ReaderCompletion {
    fn drop(&mut self) {
        if std::thread::panicking() {
            *locked(&self.failure) = Some("output reader thread panicked".to_owned());
        }
        self.complete.store(true, Ordering::Release);
    }
}

fn output_tail(bytes: &[u8]) -> String {
    String::from_utf8_lossy(&bytes[bytes.len().saturating_sub(DIAGNOSTIC_TAIL_BYTES)..])
        .into_owned()
}

struct CapturedOutput {
    bytes: Arc<Mutex<Vec<u8>>>,
    reader: Mutex<Option<std::thread::JoinHandle<std::io::Result<()>>>>,
    complete: Arc<AtomicBool>,
    failure: Arc<Mutex<Option<String>>>,
}

impl CapturedOutput {
    fn drain(reader: Option<impl Read + Send + 'static>) -> Self {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&bytes);
        let complete = Arc::new(AtomicBool::new(reader.is_none()));
        let failure = Arc::new(Mutex::new(None));
        let completion = ReaderCompletion {
            complete: Arc::clone(&complete),
            failure: Arc::clone(&failure),
        };
        let reader = reader.map(|mut reader| {
            std::thread::spawn(move || {
                let _completion = completion;
                let mut chunk = [0_u8; 4096];
                loop {
                    let count = match reader.read(&mut chunk) {
                        Ok(count) => count,
                        Err(error) => {
                            *locked(&_completion.failure) = Some(error.to_string());
                            return Err(error);
                        }
                    };
                    if count == 0 {
                        return Ok(());
                    }
                    locked(&captured).extend_from_slice(&chunk[..count]);
                }
            })
        });
        Self {
            bytes,
            reader: Mutex::new(reader),
            complete,
            failure,
        }
    }

    fn finish(&self) -> std::io::Result<Vec<u8>> {
        // Keep the handle lock through joining so another normal finisher
        // cannot return before this reader has finished draining.
        let mut reader = locked(&self.reader);
        if let Some(thread) = reader.take() {
            thread
                .join()
                .map_err(|_| std::io::Error::other("output reader thread panicked"))??;
        }
        Ok(locked(&self.bytes).clone())
    }

    fn diagnostic_tail(&self, deadline: Instant) -> String {
        // Do not take the handle mutex: a successful waiter can hold it
        // while joining a reader whose inherited pipe never reaches EOF.
        while !self.complete.load(Ordering::Acquire) && pause_until(deadline) {}
        let mut complete = self.complete.load(Ordering::Acquire);
        let mut tail = match locked_until(&self.bytes, deadline) {
            Some(bytes) => output_tail(&bytes),
            None => {
                complete = false;
                "[captured output unavailable: buffer busy]".to_owned()
            }
        };
        match locked_until(&self.failure, deadline) {
            Some(failure) => {
                if let Some(error) = failure.as_ref() {
                    tail.push_str(&format!("\n[output collection failed: {error}]"));
                }
            }
            None => complete = false,
        }
        if !complete {
            tail.push_str("\n[output collection incomplete after 2 s]");
        }
        tail
    }
}

struct MonitoredProgram {
    child: Mutex<Child>,
    output: CapturedOutput,
    errors: CapturedOutput,
    program: OsString,
    pid: u32,
    diagnostic_deadline: OnceLock<Instant>,
}

/// A shareable view of a child for peers that wait on separate test threads.
/// Dropping a monitor does not stop the process; its `ProgramChild` owns it.
#[derive(Clone)]
pub struct ProgramMonitor {
    program: Arc<MonitoredProgram>,
}

impl ProgramMonitor {
    /// Stops the child and collects both output tails within one shared
    /// diagnostic budget, including when a peer only owns a monitor.
    pub(crate) fn diagnostic_block(&self, port: Option<u16>) -> String {
        let deadline = *self.program.diagnostic_deadline
            .get_or_init(|| Instant::now() + DIAGNOSTIC_LIMIT);
        let exited = self.status_until(deadline);
        let status = match exited {
            Ok(Some(status)) => format!("already exited: yes; exit status: {status}"),
            Ok(None) => "already exited: no; killed by harness".to_owned(),
            Err(error) => format!("already exited: unknown ({error}); killed by harness"),
        };
        self.stop_for_diagnostics(deadline);
        let port = port.map_or_else(|| "unknown".to_owned(), |port| port.to_string());
        format!(
            "\n----- BEGIN TEST CHILD DIAGNOSTICS -----\nprogram: {}\nPID: {}\nport: {port}\n{status}\nstderr (last {DIAGNOSTIC_TAIL_BYTES} bytes):\n{}\nstdout (last {DIAGNOSTIC_TAIL_BYTES} bytes):\n{}\n----- END TEST CHILD DIAGNOSTICS -----\n",
            self.program.program.to_string_lossy(),
            self.program.pid,
            self.program.errors.diagnostic_tail(deadline),
            self.program.output.diagnostic_tail(deadline),
        )
    }

    /// Fails with the child's stderr tail if it exited before readiness.
    /// This checks the actual child even when another server accepts this port.
    pub fn assert_running(&self, port: u16) {
        let deadline = self.program.diagnostic_deadline.get().copied()
            .unwrap_or_else(|| Instant::now() + DIAGNOSTIC_LIMIT);
        let status = self.status_until(deadline);
        match status {
            Ok(Some(status)) => {
                let deadline = *self.program.diagnostic_deadline.get_or_init(|| deadline);
                self.stop_for_diagnostics(deadline);
                panic!(
                    "firn exited before listening on {port}: {status}\nstderr (last {DIAGNOSTIC_TAIL_BYTES} bytes):\n{}",
                    self.program.errors.diagnostic_tail(deadline)
                );
            }
            Ok(None) => {}
            Err(error) => {
                let deadline = *self.program.diagnostic_deadline.get_or_init(|| deadline);
                self.stop_for_diagnostics(deadline);
                panic!("check firn while waiting for port {port}: {error}");
            }
        }
    }

    /// Probes without applying the normal program timeout or blocking on a
    /// concurrent waiter that owns the child mutex.
    fn status_until(&self, deadline: Instant) -> std::io::Result<Option<ExitStatus>> {
        locked_until(&self.program.child, deadline)
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "child status lock exceeded diagnostic deadline",
                )
            })?
            .try_wait()
    }

    /// Signals even an exited leader's group, then polls reaping within the
    /// same budget as both output readers. Never invokes a helper process or
    /// Child::wait, either of which could block indefinitely.
    fn stop_for_diagnostics(&self, deadline: Instant) {
        #[cfg(unix)]
        {
            unsafe extern "C" {
                fn kill(pid: i32, signal: i32) -> i32;
            }
            // spawn assigns each child its own group; SIGKILL is 9 on the
            // Unix hosts used by this harness. kill itself does not wait.
            unsafe {
                kill(-(self.program.pid as i32), 9);
            }
        }
        if let Some(mut child) = locked_until(&self.program.child, deadline) {
            let _ = child.kill();
        }
        loop {
            match self.status_until(deadline) {
                Ok(None) if pause_until(deadline) => {}
                _ => break,
            }
        }
    }

    // Every spawned child owns its process group. When its leader has
    // exited, only remaining live members are stopped; otherwise they can
    // hold the captured pipes open and prevent diagnostics from completing.
    fn kill_group(&self) {
        #[cfg(unix)]
        let _ = Command::new("/bin/kill")
            .args(["-KILL", "--", &format!("-{}", self.program.pid)])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

/// A test owns its process group, continuously drains both output channels,
/// and reaps it on success, timeout or panic. This bounds native test execution,
/// not any source-language proof or acceptance decision.
pub struct ProgramChild {
    monitor: ProgramMonitor,
    deadline: Instant,
    port: Option<PortLease>,
}

impl std::ops::Deref for ProgramChild {
    type Target = ProgramMonitor;

    fn deref(&self) -> &Self::Target {
        &self.monitor
    }
}

impl ProgramChild {
    pub(crate) fn spawn(command: &mut Command) -> std::io::Result<Self> {
        Self::spawn_with_limit(command, PROGRAM_DEADLINE)
    }

    pub(crate) fn spawn_with_limit(
        command: &mut Command,
        limit: Duration,
    ) -> std::io::Result<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command.spawn()?;
        let pid = child.id();
        let output = CapturedOutput::drain(child.stdout.take());
        let errors = CapturedOutput::drain(child.stderr.take());
        Ok(Self {
            monitor: ProgramMonitor {
                program: Arc::new(MonitoredProgram {
                    child: Mutex::new(child),
                    output,
                    errors,
                    program: command.get_program().to_os_string(),
                    pid,
                    diagnostic_deadline: OnceLock::new(),
                }),
            },
            deadline: Instant::now() + limit,
            port: None,
        })
    }

    /// Retains the port reservation until this child has stopped and supplies
    /// the port in panic diagnostics. The caller keeps its lease for restarts.
    pub fn with_port(mut self, port: &PortLease) -> Self {
        self.port = Some(port.clone());
        self
    }

    /// A clonable process view for a peer running on another test thread.
    pub fn monitor(&self) -> ProgramMonitor {
        self.monitor.clone()
    }

    /// The operating system's identifier of the running process.
    pub fn id(&self) -> u32 {
        self.monitor.program.pid
    }

    pub fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        let status = locked(&self.monitor.program.child).try_wait()?;
        if status.is_none() && Instant::now() >= self.deadline {
            self.terminate();
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "native test child exceeded its deadline",
            ));
        }
        Ok(status)
    }

    fn terminate(&mut self) {
        let mut child = locked(&self.monitor.program.child);
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        self.monitor.kill_group();
        let _ = child.kill();
        let _ = child.wait();
    }

    pub fn wait_with_output(mut self) -> std::io::Result<Output> {
        let status = loop {
            if let Some(status) = self.try_wait()? {
                break status;
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        Ok(Output {
            status,
            stdout: self.monitor.program.output.finish()?,
            stderr: self.monitor.program.errors.finish()?,
        })
    }

    /// Stops this child and builds the panic report within one shared two
    /// second collection budget. A later Drop never repeats blocking cleanup.
    fn diagnostic_block(&mut self) -> String {
        self.monitor.diagnostic_block(self.port.as_ref().map(PortLease::port))
    }
}

impl Drop for ProgramChild {
    fn drop(&mut self) {
        if std::thread::panicking() {
            let diagnostic = self.diagnostic_block();
            // A closed or failing stderr must not cause a second panic.
            let _ = std::io::stderr().lock().write_all(diagnostic.as_bytes());
        } else if self.monitor.program.diagnostic_deadline.get().is_none() {
            self.terminate();
        } else {
            // Reap a child that exited after the diagnostic deadline, but
            // never resume the normal blocking cleanup after diagnostics.
            let _ = self.monitor.status_until(Instant::now());
        }
    }
}

/// Runs `command` as `Command::output` does, with no standard input and both
/// outputs captured, as an owned process that is stopped once `limit` has
/// passed and then answers `TimedOut`.
pub(crate) fn output_within(command: &mut Command, limit: Duration) -> std::io::Result<Output> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    ProgramChild::spawn_with_limit(command, limit)?.wait_with_output()
}

pub(crate) fn run_command(command: &mut Command) -> Output {
    output_within(command, PROGRAM_DEADLINE)
        .unwrap_or_else(|error| panic!("run native test command: {error}"))
}

/// The redis-server `REDIS_SERVER` names, checked to be 7.2.16, the oracle
/// every case that runs Redis itself compares firn with.
pub(crate) fn redis_oracle() -> PathBuf {
    let binary = std::env::var_os("REDIS_SERVER")
        .map(PathBuf::from)
        .expect("REDIS_SERVER must name redis-server 7.2.16, the oracle");
    let output = run_command(Command::new(&binary).arg("--version"));
    assert!(output.status.success(), "Redis version command: {output:?}");
    assert!(
        String::from_utf8_lossy(&output.stdout).contains(" v=7.2.16 "),
        "{output:?}"
    );
    binary
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_lease_rejects_held_candidate_and_reuses_released_candidate() {
        let held = lease_port();
        // Hold an OS reservation for an unleased candidate so no parallel
        // lease_port call can receive it during the deterministic sequence.
        let listener = loop {
            let listener = std::net::TcpListener::bind(("127.0.0.1", 0))
                .expect("reserve deterministic candidate in the operating system");
            let port = listener.local_addr().expect("candidate address").port();
            if !locked(LEASED_PORTS.get_or_init(|| Mutex::new(HashSet::new())))
                .contains(&port)
            {
                break listener;
            }
        };
        let next_port = listener.local_addr().expect("candidate address").port();
        let mut candidates = [held.port(), next_port].into_iter();
        let mut calls = 0;
        let lease = lease_from(|| {
            calls += 1;
            candidates.next().expect("held candidate must be skipped once")
        });
        assert_eq!(calls, 2, "a held candidate must be refused");
        assert_eq!(lease.port(), next_port);
        drop(lease);
        let mut reuse_calls = 0;
        let reused = lease_from(|| {
            reuse_calls += 1;
            assert_eq!(
                reuse_calls, 1,
                "released candidate must be accepted immediately"
            );
            next_port
        });
        assert_eq!(reused.port(), next_port, "dropping a lease frees its port");
        drop(reused);
        drop(listener);
    }

    /// Spawns a firn-free witness through the same process and capture path.
    fn shell_child(script: &str, lease: &PortLease) -> ProgramChild {
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", script])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        ProgramChild::spawn(&mut command)
            .expect("spawn diagnostic witness")
            .with_port(lease)
    }

    /// Waits for a fixture's natural exit without the normal timeout cleanup.
    fn wait_for_witness_exit(child: &ProgramChild) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if child.monitor.status_until(deadline).expect("probe witness").is_some() {
                return;
            }
            assert!(pause_until(deadline), "witness did not exit promptly");
        }
    }

    #[test]
    fn panic_diagnostic_block_preserves_exited_child_and_both_outputs() {
        let lease = lease_port();
        let mut child = shell_child("echo OUT-MARK; echo ERR-MARK >&2; exit 7", &lease);
        let pid = child.id();
        wait_for_witness_exit(&child);
        let diagnostic = child.diagnostic_block();
        assert!(diagnostic.contains("program: /bin/sh"), "{diagnostic}");
        assert!(diagnostic.contains(&format!("PID: {pid}")), "{diagnostic}");
        assert!(
            diagnostic.contains(&format!("port: {}", lease.port())),
            "{diagnostic}"
        );
        assert!(diagnostic.contains("already exited: yes"), "{diagnostic}");
        assert!(diagnostic.contains("exit status: 7"), "{diagnostic}");
        assert!(diagnostic.contains("OUT-MARK"), "{diagnostic}");
        assert!(diagnostic.contains("ERR-MARK"), "{diagnostic}");
    }

    #[test]
    fn panic_diagnostic_block_stops_running_child_within_budget() {
        let lease = lease_port();
        let mut child = shell_child("sleep 30", &lease);
        assert!(child.monitor.status_until(Instant::now() + DIAGNOSTIC_LIMIT)
            .expect("probe running witness")
            .is_none());
        let started = Instant::now();
        let diagnostic = child.diagnostic_block();
        assert!(
            diagnostic.contains("already exited: no; killed by harness"),
            "{diagnostic}"
        );
        assert!(child.monitor.status_until(Instant::now())
            .expect("probe reaped witness")
            .is_some());
        // Include ordinary Drop: it must not re-enter unbounded cleanup.
        drop(child);
        assert!(started.elapsed() < Duration::from_secs(8), "{diagnostic}");
    }

    #[test]
    fn diagnostic_tail_preserves_output_collection_errors() {
        struct FailedPipe;
        impl Read for FailedPipe {
            fn read(&mut self, _buffer: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("READ-FAILURE-MARKER"))
            }
        }
        let output = CapturedOutput::drain(Some(FailedPipe));
        let diagnostic = output.diagnostic_tail(Instant::now() + DIAGNOSTIC_LIMIT);
        assert!(
            diagnostic.contains("[output collection failed: READ-FAILURE-MARKER]"),
            "{diagnostic}"
        );
    }

    #[test]
    fn panic_diagnostics_bound_output_collection_without_pipe_eof() {
        struct HeldPipe {
            release: std::sync::mpsc::Receiver<()>,
            marker: Option<&'static [u8]>,
            ready: std::sync::mpsc::Sender<()>,
        }
        impl Read for HeldPipe {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                if let Some(marker) = self.marker.take() {
                    buffer[..marker.len()].copy_from_slice(marker);
                    return Ok(marker.len());
                }
                let _ = self.ready.send(());
                // A regressed unbounded collector reaches EOF after thirty
                // seconds and fails the timing assertion instead of hanging CI.
                let _ = self.release.recv_timeout(Duration::from_secs(30));
                Ok(0)
            }
        }
        // Disconnecting on either success or panic releases both readers;
        // this witness leaves no detached descendant or blocked thread.
        let (release_output, output_receiver) = std::sync::mpsc::channel();
        let (release_errors, error_receiver) = std::sync::mpsc::channel();
        let (ready, readers_ready) = std::sync::mpsc::channel();
        let lease = lease_port();
        let mut child = shell_child("exit 7", &lease);
        wait_for_witness_exit(&child);
        let program = Arc::get_mut(&mut child.monitor.program).expect("unshared fixture");
        program.output = CapturedOutput::drain(Some(HeldPipe {
            release: output_receiver,
            marker: Some(b"OUTPUT-BEFORE-EOF"),
            ready: ready.clone(),
        }));
        program.errors = CapturedOutput::drain(Some(HeldPipe {
            release: error_receiver,
            marker: Some(b"ERROR-BEFORE-EOF"),
            ready,
        }));
        for _ in 0..2 {
            readers_ready.recv_timeout(Duration::from_secs(10))
                .expect("reader captured its marker before blocking");
        }
        let started = Instant::now();
        let diagnostic = child.diagnostic_block();
        drop(child);
        let elapsed = started.elapsed();
        drop(release_output);
        drop(release_errors);
        assert!(elapsed < Duration::from_secs(8), "{diagnostic}");
        assert!(diagnostic.contains("OUTPUT-BEFORE-EOF"), "{diagnostic}");
        assert!(diagnostic.contains("ERROR-BEFORE-EOF"), "{diagnostic}");
        assert_eq!(
            diagnostic.matches("[output collection incomplete after 2 s]").count(),
            2,
            "both tails share one diagnostic budget: {diagnostic}"
        );
    }

    #[test]
    fn concurrent_port_leases_do_not_reuse_held_ports() {
        let ready = Arc::new(std::sync::Barrier::new(8));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let ready = Arc::clone(&ready);
                std::thread::spawn(move || {
                    let leases: Vec<_> = (0..50).map(|_| lease_port()).collect();
                    ready.wait();
                    leases
                })
            })
            .collect();
        // Moving every thread's leases here holds all 400 until the check
        // ends, including after the worker that acquired them has exited.
        let leases: Vec<_> = threads
            .into_iter()
            .flat_map(|thread| thread.join().expect("port leasing thread"))
            .collect();
        let mut ports = HashSet::new();
        for lease in &leases {
            assert!(
                ports.insert(lease.port()),
                "port {} leased twice",
                lease.port()
            );
        }
        assert_eq!(ports.len(), 400);
    }

    #[test]
    fn readiness_does_not_retry_a_timeout_after_a_completed_handshake() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0))
            .expect("listen for the readiness regression");
        let address = listener.local_addr().expect("readiness listener address");
        listener.set_nonblocking(true).expect("bound regression acceptance");
        let lease = lease_port();
        let child = shell_child("exec sleep 30", &lease);
        let mut listener = Some(listener);
        let mut accepted = None;
        let mut attempts = 0;
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::connect_to_when_ready_using(address, &child, |address, timeout| {
                attempts += 1;
                // The old retry-on-any-error rule reaches this assertion on
                // attempt two, rather than waiting for the readiness deadline.
                assert_eq!(attempts, 1, "readiness reconnected after consuming its slot");
                let stream = std::net::TcpStream::connect_timeout(address, timeout)?;
                let listener = listener.take().expect("only one accepted connection");
                let accept_deadline = Instant::now() + Duration::from_secs(10);
                accepted = Some(loop {
                    match listener.accept() {
                        Ok((peer, _)) => break peer,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < accept_deadline => {
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        Err(error) => panic!("the completed handshake was not admitted: {error}"),
                    }
                });
                drop(listener);
                drop(stream);
                Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "timeout observed after a completed handshake",
                ))
            })
        }));
        let failure = outcome.expect_err("a timeout must fail readiness");
        let message = failure.downcast_ref::<String>().expect("formatted connection error");
        assert!(message.contains("timeout observed after a completed handshake"), "{message}");
        assert!(message.contains("BEGIN TEST CHILD DIAGNOSTICS"), "{message}");
        assert!(message.contains(&format!("PID: {}", child.id())), "{message}");
        assert!(message.contains(&format!("port: {}", address.port())), "{message}");
        assert!(message.contains("already exited: no; killed by harness"), "{message}");
        assert!(message.contains("stderr (last 4096 bytes):"), "{message}");
        assert!(message.contains("stdout (last 4096 bytes):"), "{message}");
        assert_eq!(attempts, 1, "an admitted connection must not be replaced");
        let mut accepted = crate::bounded_stream(accepted.expect("one connection was admitted"));
        assert_eq!(accepted.read(&mut [0_u8; 1]).expect("read abandoned connection"), 0);
    }

    #[test]
    fn readiness_retries_refusal_before_connecting() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0))
            .expect("listen for the refusal regression");
        let address = listener.local_addr().expect("refusal listener address");
        let lease = lease_port();
        let child = shell_child("exec sleep 30", &lease);
        let mut attempts = 0;
        let stream = crate::connect_to_when_ready_using(address, &child, |address, timeout| {
            attempts += 1;
            if attempts == 1 {
                Err(std::io::ErrorKind::ConnectionRefused.into())
            } else {
                std::net::TcpStream::connect_timeout(address, timeout)
            }
        });
        assert_eq!(attempts, 2, "a not-yet-listening child must get another attempt");
        drop(stream);
    }

    #[test]
    fn readiness_retry_rule_rejects_other_errors_and_an_expired_deadline() {
        let deadline = Instant::now() + Duration::from_secs(20);
        for kind in [
            std::io::ErrorKind::TimedOut,
            std::io::ErrorKind::ConnectionReset,
            std::io::ErrorKind::Interrupted,
            std::io::ErrorKind::PermissionDenied,
        ] {
            assert!(!crate::retry_refused_connection(&kind.into(), deadline), "{kind:?}");
        }
        assert!(!crate::retry_refused_connection(
            &std::io::ErrorKind::ConnectionRefused.into(), Instant::now(),
        ));
    }

    #[test]
    fn readiness_reports_child_exit_status_and_stderr() {
        let lease = lease_port();
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", "echo FIRN_READINESS_EXIT_MARKER >&2; exit 7"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let child = ProgramChild::spawn(&mut command)
            .expect("spawn readiness witness")
            .with_port(&lease);
        let started = Instant::now();
        let panic = std::panic::catch_unwind(|| crate::connect_when_ready(lease.port(), &child))
            .expect_err("an exited child cannot become ready");
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "readiness must detect child exit before the 20 second connect deadline"
        );
        let message = panic
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| panic.downcast_ref::<&str>().copied())
            .expect("string panic message");
        assert!(
            message.contains(&format!(
                "firn exited before listening on {}: exit status: 7",
                lease.port()
            )),
            "readiness panic lost exit status: {message}"
        );
        assert!(
            message.contains("FIRN_READINESS_EXIT_MARKER"),
            "readiness panic lost stderr: {message}"
        );
    }
}
