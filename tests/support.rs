//! The process and fixture helpers used by the network cases.

use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static NEXT_EXECUTION: AtomicU64 = AtomicU64::new(0);

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
        let executable = std::env::var_os("FIRN")
            .map(PathBuf::from)
            .expect("FIRN must name the absolute path to a built firn executable");
        assert!(
            executable.is_absolute() && executable.is_file(),
            "FIRN must name an existing file by absolute path, got {}",
            executable.display()
        );
        let sequence = NEXT_EXECUTION.fetch_add(1, Ordering::Relaxed);
        let directory = std::env::temp_dir().join(format!(
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
        let mut command = Command::new(&self.executable);
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

/// A test owns its process group, continuously drains both output channels,
/// and reaps it on success, timeout or panic. This bounds native test execution,
/// not any source-language proof or acceptance decision.
pub struct ProgramChild {
    child: Child,
    output: Option<std::thread::JoinHandle<std::io::Result<Vec<u8>>>>,
    errors: Option<std::thread::JoinHandle<std::io::Result<Vec<u8>>>>,
    deadline: Instant,
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
        fn drain(
            reader: impl Read + Send + 'static,
        ) -> std::thread::JoinHandle<std::io::Result<Vec<u8>>> {
            std::thread::spawn(move || {
                let mut reader = reader;
                let mut bytes = Vec::new();
                let mut chunk = [0_u8; 4096];
                loop {
                    let read = reader.read(&mut chunk)?;
                    if read == 0 {
                        return Ok(bytes);
                    }
                    bytes.extend_from_slice(&chunk[..read]);
                }
            })
        }
        Ok(Self {
            output: child.stdout.take().map(drain),
            errors: child.stderr.take().map(drain),
            child,
            deadline: Instant::now() + limit,
        })
    }

    /// The operating system's identifier of the running process.
    pub fn id(&self) -> u32 {
        self.child.id()
    }

    pub fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        let status = self.child.try_wait()?;
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
        if self.child.try_wait().ok().flatten().is_some() {
            return;
        }
        #[cfg(unix)]
        let _ = Command::new("/bin/kill")
            .args(["-KILL", "--", &format!("-{}", self.child.id())])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    pub fn wait_with_output(mut self) -> std::io::Result<Output> {
        let status = loop {
            if let Some(status) = self.try_wait()? {
                break status;
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        fn collected(
            thread: Option<std::thread::JoinHandle<std::io::Result<Vec<u8>>>>,
        ) -> std::io::Result<Vec<u8>> {
            match thread {
                None => Ok(Vec::new()),
                Some(thread) => thread.join().expect("output reader thread"),
            }
        }
        Ok(Output {
            status,
            stdout: collected(self.output.take())?,
            stderr: collected(self.errors.take())?,
        })
    }
}

impl Drop for ProgramChild {
    fn drop(&mut self) {
        self.terminate();
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
