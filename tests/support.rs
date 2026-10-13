//! The process and fixture helpers used by the network cases.

use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
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
    loop {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0))
            .expect("bind loopback port for lease");
        let port = listener.local_addr().expect("leased listener address").port();
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

    /// Starts the program on one runtime route with raw invocation arguments,
    /// and hands back the running child.
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
        self.spawn_on_route_options(native_ring, settings, arguments, false)
    }

    /// Linux append-failure cases inherit ignored SIGXFSZ so RLIMIT_FSIZE
    /// makes write return EFBIG instead of terminating the owned child.
    #[cfg(target_os = "linux")]
    pub fn spawn_on_route_ignoring_xfsz(
        &self,
        native_ring: bool,
        arguments: &[&[u8]],
    ) -> ProgramChild {
        self.spawn_on_route_options(native_ring, &[], arguments, true)
    }

    fn spawn_on_route_options(
        &self,
        native_ring: bool,
        settings: &[(&str, &str)],
        arguments: &[&[u8]],
        ignore_xfsz: bool,
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
        ProgramChild::spawn(&mut command).expect("spawn compiled program")
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

fn output_tail(bytes: &[u8]) -> String {
    String::from_utf8_lossy(&bytes[bytes.len().saturating_sub(DIAGNOSTIC_TAIL_BYTES)..])
        .into_owned()
}

struct CapturedOutput {
    bytes: Arc<Mutex<Vec<u8>>>,
    reader: Mutex<Option<std::thread::JoinHandle<std::io::Result<()>>>>,
}

impl CapturedOutput {
    fn drain(reader: Option<impl Read + Send + 'static>) -> Self {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&bytes);
        let reader = reader.map(|mut reader| {
            std::thread::spawn(move || {
                let mut chunk = [0_u8; 4096];
                loop {
                    let count = reader.read(&mut chunk)?;
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
        }
    }

    fn finish(&self) -> std::io::Result<Vec<u8>> {
        // Keep the handle lock through joining so another monitor cannot
        // report a partial tail while this reader is still draining.
        let mut reader = locked(&self.reader);
        if let Some(thread) = reader.take() {
            thread
                .join()
                .map_err(|_| std::io::Error::other("output reader thread panicked"))??;
        }
        Ok(locked(&self.bytes).clone())
    }

    fn diagnostic_tail(&self) -> String {
        match self.finish() {
            Ok(bytes) => output_tail(&bytes),
            Err(error) => format!(
                "{}\n[output collection failed: {error}]",
                output_tail(&locked(&self.bytes))
            ),
        }
    }
}

struct MonitoredProgram {
    child: Mutex<Child>,
    output: CapturedOutput,
    errors: CapturedOutput,
    program: OsString,
    pid: u32,
}

/// A shareable view of a child for peers that wait on separate test threads.
/// Dropping a monitor does not stop the process; its `ProgramChild` owns it.
#[derive(Clone)]
pub struct ProgramMonitor {
    program: Arc<MonitoredProgram>,
}

impl ProgramMonitor {
    /// Fails with the child's completed stderr if it exited before readiness.
    /// This checks the actual child even when another server accepts this port.
    pub fn assert_running(&self, port: u16) {
        let status = locked(&self.program.child).try_wait();
        match status {
            Ok(Some(status)) => {
                self.kill_group();
                panic!(
                    "firn exited before listening on {port}: {status}\nstderr (last {DIAGNOSTIC_TAIL_BYTES} bytes):\n{}",
                    self.program.errors.diagnostic_tail()
                );
            }
            Ok(None) => {}
            Err(error) => panic!("check firn while waiting for port {port}: {error}"),
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

    fn diagnose_panic(&mut self) {
        let exited = locked(&self.monitor.program.child).try_wait();
        let status = match exited {
            Ok(Some(status)) => {
                self.monitor.kill_group();
                format!("already exited: yes; exit status: {status}")
            }
            Ok(None) => "already exited: no; killed by harness".to_owned(),
            Err(error) => format!("already exited: unknown ({error}); killed by harness"),
        };
        self.terminate();
        let port = self.port.as_ref().map_or_else(
            || "unknown".to_owned(),
            |lease| lease.port().to_string(),
        );
        let diagnostic = format!(
            "\n----- BEGIN TEST CHILD DIAGNOSTICS -----\nprogram: {}\nPID: {}\nport: {port}\n{status}\nstderr (last {DIAGNOSTIC_TAIL_BYTES} bytes):\n{}\nstdout (last {DIAGNOSTIC_TAIL_BYTES} bytes):\n{}\n----- END TEST CHILD DIAGNOSTICS -----\n",
            self.monitor.program.program.to_string_lossy(),
            self.id(),
            self.monitor.program.errors.diagnostic_tail(),
            self.monitor.program.output.diagnostic_tail(),
        );
        // Standard error can be closed or fail; cleanup must never panic.
        let _ = std::io::stderr().lock().write_all(diagnostic.as_bytes());
    }
}

impl Drop for ProgramChild {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.diagnose_panic();
        } else {
            self.terminate();
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

#[cfg(test)]
mod tests {
    use super::*;

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
        let panic = std::panic::catch_unwind(|| crate::connect_when_ready(lease.port(), &child))
            .expect_err("an exited child cannot become ready");
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
