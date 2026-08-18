use std::{
    fmt,
    net::SocketAddr,
    process::Stdio,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

use next_rs_core::{Error, Result};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::{Child, Command},
    sync::Mutex,
    time::timeout,
};

/// How the renderer process is started.
#[derive(Debug, Clone)]
pub struct RendererCommand {
    pub program: String,
    pub args: Vec<String>,
    pub working_directory: Option<String>,
    pub env: Vec<(String, String)>,
}

impl RendererCommand {
    /// `node <entry>`, which is what `next-rs build` generates (spec §82 step 13).
    pub fn node(entry: impl Into<String>) -> Self {
        Self {
            program: "node".to_owned(),
            args: vec![entry.into()],
            working_directory: None,
            env: Vec::new(),
        }
    }

    pub fn with_working_directory(mut self, directory: impl Into<String>) -> Self {
        self.working_directory = Some(directory.into());
        self
    }

    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }
}

#[derive(Debug, Clone)]
pub struct RendererProcessOptions {
    pub command: RendererCommand,
    /// How long to wait for the readiness line before giving up.
    pub startup_timeout: Duration,
    /// Restart after a crash, up to this many times. Zero disables restarts.
    pub max_restarts: usize,
}

impl RendererProcessOptions {
    pub fn new(command: RendererCommand) -> Self {
        Self {
            command,
            startup_timeout: Duration::from_secs(30),
            max_restarts: 3,
        }
    }

    pub fn with_startup_timeout(mut self, timeout: Duration) -> Self {
        self.startup_timeout = timeout;
        self
    }

    pub fn with_max_restarts(mut self, restarts: usize) -> Self {
        self.max_restarts = restarts;
        self
    }
}

/// A lazily started, supervised React renderer process (spec §79, §80).
///
/// Nothing is spawned until the first `.ssr()` slot asks for an address. That is
/// what makes §80 structural: a deployment whose pages are all client-only never
/// starts Node, and [`RendererProcess::spawns`] stays at zero for the life of the
/// process.
pub struct RendererProcess {
    options: RendererProcessOptions,
    state: Mutex<Option<Running>>,
    spawns: AtomicUsize,
    restarts: AtomicUsize,
}

struct Running {
    address: SocketAddr,
    child: Child,
}

impl fmt::Debug for RendererProcess {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RendererProcess")
            .field("program", &self.options.command.program)
            .field("spawns", &self.spawns())
            .finish()
    }
}

impl RendererProcess {
    pub fn new(options: RendererProcessOptions) -> Self {
        Self {
            options,
            state: Mutex::new(None),
            spawns: AtomicUsize::new(0),
            restarts: AtomicUsize::new(0),
        }
    }

    /// Times the process has been started. Zero means §80 held for this run.
    pub fn spawns(&self) -> usize {
        self.spawns.load(Ordering::Relaxed)
    }

    pub fn restarts(&self) -> usize {
        self.restarts.load(Ordering::Relaxed)
    }

    /// The address of a running renderer, starting one if needed.
    pub async fn address(&self) -> Result<SocketAddr> {
        let mut state = self.state.lock().await;

        if let Some(running) = state.as_mut() {
            // `try_wait` is the only way to notice a process that exited while
            // nothing was talking to it.
            match running.child.try_wait() {
                Ok(None) => return Ok(running.address),
                Ok(Some(status)) => {
                    if self.restarts.load(Ordering::Relaxed) >= self.options.max_restarts {
                        return Err(Error::internal(format!(
                            "the React renderer exited ({status}) and has already been restarted \
                             {} time(s)",
                            self.options.max_restarts
                        )));
                    }
                    self.restarts.fetch_add(1, Ordering::Relaxed);
                    *state = None;
                }
                Err(error) => {
                    return Err(Error::internal(format!(
                        "cannot check the React renderer process: {error}"
                    )));
                }
            }
        }

        let running = self.spawn().await?;
        let address = running.address;
        *state = Some(running);
        Ok(address)
    }

    /// Stops the process if one is running.
    pub async fn shutdown(&self) -> Result<()> {
        let mut state = self.state.lock().await;
        if let Some(mut running) = state.take() {
            let _ = running.child.start_kill();
            let _ = running.child.wait().await;
        }
        Ok(())
    }

    async fn spawn(&self) -> Result<Running> {
        let mut command = Command::new(&self.options.command.program);
        command
            .args(&self.options.command.args)
            .stdout(Stdio::piped())
            // The child's stderr is the developer's window into a failing
            // component; inheriting it keeps React's own error output visible.
            .stderr(Stdio::inherit())
            .stdin(Stdio::null())
            // Killing the child with the parent avoids leaking a renderer when a
            // dev server is interrupted.
            .kill_on_drop(true);
        if let Some(directory) = &self.options.command.working_directory {
            command.current_dir(directory);
        }
        for (key, value) in &self.options.command.env {
            command.env(key, value);
        }

        let mut child = command.spawn().map_err(|error| {
            Error::internal(format!(
                "cannot start the React renderer (`{}`): {error}",
                self.options.command.program
            ))
        })?;
        self.spawns.fetch_add(1, Ordering::Relaxed);

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::internal("the React renderer has no stdout"))?;

        // The renderer binds port 0 and announces what it got, so no port has to
        // be reserved, guessed or polled for.
        let address = match timeout(self.options.startup_timeout, read_ready_line(stdout)).await {
            Ok(Ok(address)) => address,
            Ok(Err(error)) => {
                let _ = child.start_kill();
                return Err(error);
            }
            Err(_) => {
                let _ = child.start_kill();
                return Err(Error::internal(format!(
                    "the React renderer did not become ready within {:?}",
                    self.options.startup_timeout
                )));
            }
        };

        Ok(Running { address, child })
    }
}

/// Reads the `next-rs-renderer ready <port>` handshake.
///
/// Other lines are forwarded to stdout rather than discarded: a renderer that
/// logs before it is ready is usually a renderer that is about to fail, and
/// swallowing that output makes it much harder to see why.
async fn read_ready_line(stdout: tokio::process::ChildStdout) -> Result<SocketAddr> {
    let mut lines = BufReader::new(stdout).lines();
    while let Some(line) = lines
        .next_line()
        .await
        .map_err(|error| Error::internal(format!("cannot read renderer output: {error}")))?
    {
        if let Some(port) = parse_ready_line(&line) {
            // Keep draining in the background so a chatty renderer cannot fill
            // its stdout pipe and block.
            tokio::spawn(async move {
                while let Ok(Some(line)) = lines.next_line().await {
                    println!("[next-rs renderer] {line}");
                }
            });
            return Ok(SocketAddr::from(([127, 0, 0, 1], port)));
        }
        println!("[next-rs renderer] {line}");
    }
    Err(Error::internal(
        "the React renderer exited before announcing a port",
    ))
}

/// Parses `next-rs-renderer ready <port>`.
pub(crate) fn parse_ready_line(line: &str) -> Option<u16> {
    let rest = line.trim().strip_prefix("next-rs-renderer ready ")?;
    rest.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_readiness_handshake() {
        assert_eq!(
            parse_ready_line("next-rs-renderer ready 41234"),
            Some(41234)
        );
        assert_eq!(
            parse_ready_line("  next-rs-renderer ready 8080  "),
            Some(8080)
        );
        assert_eq!(parse_ready_line("next-rs-renderer ready"), None);
        assert_eq!(parse_ready_line("listening on 3000"), None);
        assert_eq!(parse_ready_line("next-rs-renderer ready abc"), None);
        // A port that does not fit in a u16 is not a port.
        assert_eq!(parse_ready_line("next-rs-renderer ready 99999"), None);
    }

    #[test]
    fn a_process_that_is_never_used_is_never_spawned() {
        let process = RendererProcess::new(RendererProcessOptions::new(RendererCommand::node(
            "does-not-exist.mjs",
        )));
        // Spec §80: constructing the renderer must not start it.
        assert_eq!(process.spawns(), 0);
        assert!(format!("{process:?}").contains("spawns: 0"));
    }

    #[tokio::test]
    async fn a_missing_program_is_reported_clearly() {
        let process = RendererProcess::new(RendererProcessOptions::new(RendererCommand {
            program: "next-rs-no-such-program".to_owned(),
            args: vec![],
            working_directory: None,
            env: vec![],
        }));
        let error = process.address().await.unwrap_err();
        assert!(
            error.message().contains("cannot start the React renderer"),
            "{}",
            error.message()
        );
    }

    #[tokio::test]
    async fn a_process_that_never_announces_a_port_times_out() {
        // `sleep` starts, produces no output, and outlives the timeout.
        let process = RendererProcess::new(
            RendererProcessOptions::new(RendererCommand {
                program: "sleep".to_owned(),
                args: vec!["30".to_owned()],
                working_directory: None,
                env: vec![],
            })
            .with_startup_timeout(Duration::from_millis(250)),
        );
        let error = process.address().await.unwrap_err();
        assert!(error.message().contains("did not become ready"));
        assert_eq!(process.spawns(), 1);
    }

    #[tokio::test]
    async fn a_process_that_exits_without_a_port_is_reported() {
        let process = RendererProcess::new(RendererProcessOptions::new(RendererCommand {
            program: "true".to_owned(),
            args: vec![],
            working_directory: None,
            env: vec![],
        }));
        let error = process.address().await.unwrap_err();
        assert!(
            error.message().contains("exited before announcing a port"),
            "{}",
            error.message()
        );
    }

    #[tokio::test]
    async fn the_handshake_yields_a_loopback_address() {
        let process = RendererProcess::new(RendererProcessOptions::new(RendererCommand {
            program: "sh".to_owned(),
            args: vec![
                "-c".to_owned(),
                "echo warming up; echo 'next-rs-renderer ready 45999'; sleep 30".to_owned(),
            ],
            working_directory: None,
            env: vec![],
        }));

        let address = process.address().await.unwrap();
        assert_eq!(address.port(), 45999);
        assert!(address.ip().is_loopback());
        assert_eq!(process.spawns(), 1);

        // A second call reuses the running process rather than starting another.
        let again = process.address().await.unwrap();
        assert_eq!(again, address);
        assert_eq!(process.spawns(), 1);

        process.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn a_crashed_process_is_restarted_up_to_the_limit() {
        let process = RendererProcess::new(
            RendererProcessOptions::new(RendererCommand {
                program: "sh".to_owned(),
                // Announces a port, then exits immediately.
                args: vec![
                    "-c".to_owned(),
                    "echo 'next-rs-renderer ready 46001'".to_owned(),
                ],
                working_directory: None,
                env: vec![],
            })
            .with_max_restarts(1),
        );

        process.address().await.unwrap();
        // Give the child time to exit so `try_wait` observes it.
        tokio::time::sleep(Duration::from_millis(150)).await;

        process.address().await.unwrap();
        assert_eq!(process.restarts(), 1);
        assert_eq!(process.spawns(), 2);

        tokio::time::sleep(Duration::from_millis(150)).await;
        let error = process.address().await.unwrap_err();
        assert!(
            error.message().contains("already been restarted"),
            "{}",
            error.message()
        );
    }

    #[tokio::test]
    async fn shutting_down_an_unstarted_process_is_fine() {
        let process = RendererProcess::new(RendererProcessOptions::new(RendererCommand::node(
            "unused.mjs",
        )));
        process.shutdown().await.unwrap();
        assert_eq!(process.spawns(), 0);
    }

    #[test]
    fn commands_carry_working_directory_and_environment() {
        let command = RendererCommand::node(".next-rs/generated/react-renderer.mjs")
            .with_working_directory("/srv/app")
            .with_env("NODE_ENV", "production");
        assert_eq!(command.program, "node");
        assert_eq!(command.working_directory.as_deref(), Some("/srv/app"));
        assert_eq!(
            command.env,
            vec![("NODE_ENV".to_owned(), "production".to_owned())]
        );
    }
}
