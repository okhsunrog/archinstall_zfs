use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use color_eyre::eyre::{Context, Result, bail};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone)]
pub struct CmdOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

impl CmdOutput {
    pub fn success(&self) -> bool {
        self.exit_code == 0
    }
}

pub trait CommandRunner: Send + Sync {
    fn run(&self, program: &str, args: &[&str]) -> Result<CmdOutput>;

    fn run_with_stdin(&self, program: &str, args: &[&str], stdin: &[u8]) -> Result<CmdOutput>;

    /// Like `run`, but stops the command when `cancel` fires instead of
    /// waiting for it. Runners that cannot stop anything fall back to `run`.
    fn run_cancellable(
        &self,
        program: &str,
        args: &[&str],
        cancel: &CancellationToken,
    ) -> Result<CmdOutput> {
        let _ = cancel;
        self.run(program, args)
    }

    fn run_with_stdin_cancellable(
        &self,
        program: &str,
        args: &[&str],
        stdin: &[u8],
        cancel: &CancellationToken,
    ) -> Result<CmdOutput> {
        let _ = cancel;
        self.run_with_stdin(program, args, stdin)
    }
}

pub struct RealRunner;

impl CommandRunner for RealRunner {
    fn run(&self, program: &str, args: &[&str]) -> Result<CmdOutput> {
        tracing::debug!(program, ?args, "running command");
        let output = Command::new(program)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .wrap_err_with(|| format!("failed to execute: {program}"))?;

        let result = CmdOutput {
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            exit_code: output.status.code().unwrap_or(-1),
        };
        tracing::debug!(exit_code = result.exit_code, "command finished");
        log_output(program, &result);
        Ok(result)
    }

    fn run_with_stdin(&self, program: &str, args: &[&str], stdin_data: &[u8]) -> Result<CmdOutput> {
        tracing::debug!(program, ?args, "running command with stdin");
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .wrap_err_with(|| format!("failed to spawn: {program}"))?;

        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write;
            stdin
                .write_all(stdin_data)
                .wrap_err("failed to write to stdin")?;
        }

        let output = child
            .wait_with_output()
            .wrap_err("failed to wait on child")?;

        let result = CmdOutput {
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            exit_code: output.status.code().unwrap_or(-1),
        };
        log_output(program, &result);
        Ok(result)
    }

    fn run_cancellable(
        &self,
        program: &str,
        args: &[&str],
        cancel: &CancellationToken,
    ) -> Result<CmdOutput> {
        run_supervised(program, args, None, cancel)
    }

    fn run_with_stdin_cancellable(
        &self,
        program: &str,
        args: &[&str],
        stdin: &[u8],
        cancel: &CancellationToken,
    ) -> Result<CmdOutput> {
        run_supervised(program, args, Some(stdin), cancel)
    }
}

fn log_output(program: &str, result: &CmdOutput) {
    for line in result.stdout.lines() {
        if !line.trim().is_empty() {
            tracing::trace!("[{program}] {line}");
        }
    }
    for line in result.stderr.lines() {
        if !line.trim().is_empty() {
            tracing::trace!("[{program} stderr] {line}");
        }
    }
}

/// How long a stopped command's process group gets to exit on SIGTERM
/// before it is killed. dracut, dkms and makepkg all clean up on SIGTERM;
/// nothing here needs longer.
const TERMINATE_GRACE: Duration = Duration::from_secs(5);
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Run a command in its own process group, stopping the whole group as soon
/// as `cancel` fires. A build under arch-chroot forks compilers and
/// compressors; killing only the direct child would leave those holding the
/// target's mounts busy, so the cleanup could not unmount them.
fn run_supervised(
    program: &str,
    args: &[&str],
    stdin_data: Option<&[u8]>,
    cancel: &CancellationToken,
) -> Result<CmdOutput> {
    use std::os::unix::process::CommandExt;

    if cancel.is_cancelled() {
        bail!("{program} not started: installation cancelled");
    }
    tracing::debug!(program, ?args, "running command");
    let mut child = Command::new(program)
        .args(args)
        .stdin(if stdin_data.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .wrap_err_with(|| format!("failed to spawn: {program}"))?;

    if let (Some(data), Some(mut stdin)) = (stdin_data, child.stdin.take()) {
        use std::io::Write;
        // The command may exit before reading it all; that shows up in its
        // exit status, not here.
        let _ = stdin.write_all(data);
    }

    // Drain both pipes on their own threads so a chatty command cannot fill
    // one and block while this thread watches the token.
    let stdout = child.stdout.take().map(drain);
    let stderr = child.stderr.take().map(drain);

    let status = loop {
        if let Some(status) = child.try_wait().wrap_err("failed to wait on child")? {
            break status;
        }
        if cancel.is_cancelled() {
            tracing::info!(program, "stopping the running command");
            stop_group(&mut child);
            bail!("{program} stopped: installation cancelled");
        }
        std::thread::sleep(POLL_INTERVAL);
    };

    let collect = |handle: Option<std::thread::JoinHandle<Vec<u8>>>| {
        handle
            .and_then(|h| h.join().ok())
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default()
    };
    let result = CmdOutput {
        stdout: collect(stdout),
        stderr: collect(stderr),
        exit_code: status.code().unwrap_or(-1),
    };
    tracing::debug!(exit_code = result.exit_code, "command finished");
    log_output(program, &result);
    Ok(result)
}

fn drain<R: Read + Send + 'static>(mut reader: R) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = reader.read_to_end(&mut bytes);
        bytes
    })
}

/// SIGTERM the child's process group, then SIGKILL whatever is still there
/// after the grace period. The child was spawned as its own group leader,
/// so its pid is the group id.
fn stop_group(child: &mut Child) {
    use nix::sys::signal::{Signal, killpg};
    use nix::unistd::Pid;

    let group = Pid::from_raw(child.id() as i32);
    let _ = killpg(group, Signal::SIGTERM);
    let deadline = Instant::now() + TERMINATE_GRACE;
    while Instant::now() < deadline {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    let _ = killpg(group, Signal::SIGKILL);
    let _ = child.wait();
}

/// A runner whose every command stops when the token fires, for the
/// install pipeline; the cleanup that follows keeps the plain runner so it
/// can still unmount and export after a cancel.
pub struct CancellableRunner {
    inner: Arc<dyn CommandRunner>,
    cancel: CancellationToken,
}

impl CancellableRunner {
    pub fn new(inner: Arc<dyn CommandRunner>, cancel: CancellationToken) -> Self {
        Self { inner, cancel }
    }
}

impl CommandRunner for CancellableRunner {
    fn run(&self, program: &str, args: &[&str]) -> Result<CmdOutput> {
        self.inner.run_cancellable(program, args, &self.cancel)
    }

    fn run_with_stdin(&self, program: &str, args: &[&str], stdin: &[u8]) -> Result<CmdOutput> {
        self.inner
            .run_with_stdin_cancellable(program, args, stdin, &self.cancel)
    }

    fn run_cancellable(
        &self,
        program: &str,
        args: &[&str],
        cancel: &CancellationToken,
    ) -> Result<CmdOutput> {
        self.inner.run_cancellable(program, args, cancel)
    }

    fn run_with_stdin_cancellable(
        &self,
        program: &str,
        args: &[&str],
        stdin: &[u8],
        cancel: &CancellationToken,
    ) -> Result<CmdOutput> {
        self.inner
            .run_with_stdin_cancellable(program, args, stdin, cancel)
    }
}

pub fn chroot(runner: &dyn CommandRunner, target: &Path, cmd: &str) -> Result<CmdOutput> {
    let target_str = target.to_string_lossy();
    runner.run("arch-chroot", &[&*target_str, "bash", "-c", cmd])
}

/// Run a command inside an arch-chroot without bash interpretation.
/// Arguments are passed directly to the program, avoiding shell injection.
pub fn chroot_cmd(
    runner: &dyn CommandRunner,
    target: &Path,
    program: &str,
    args: &[&str],
) -> Result<CmdOutput> {
    let target_str = target.to_string_lossy();
    let mut full_args = vec![&*target_str, program];
    full_args.extend_from_slice(args);
    runner.run("arch-chroot", &full_args)
}

/// Run a program inside the chroot and fail, naming `context`, when it exits
/// non-zero. For the common case where nothing but the exit status matters.
pub fn chroot_checked(
    runner: &dyn CommandRunner,
    target: &Path,
    program: &str,
    args: &[&str],
    context: &str,
) -> Result<()> {
    let output = chroot_cmd(runner, target, program, args)?;
    check_exit(&output, context)
}

/// Shell-quote a string for safe interpolation into bash commands.
/// Returns the string wrapped in single quotes with internal single quotes escaped.
pub fn shell_quote(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }
    // If string only contains safe characters, return as-is
    if s.chars().all(|c| {
        c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | ':' | '@' | '+' | ',')
    }) {
        return s.to_string();
    }
    // Wrap in single quotes, escaping any embedded single quotes
    format!("'{}'", s.replace('\'', "'\\''"))
}

pub fn check_exit(output: &CmdOutput, context: &str) -> Result<()> {
    if output.success() {
        Ok(())
    } else {
        bail!(
            "{context}: exit code {}, stderr: {}",
            output.exit_code,
            output.stderr.trim()
        );
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    #[derive(Debug, Clone, Default)]
    pub struct CannedResponse {
        pub stdout: String,
        pub stderr: String,
        pub exit_code: i32,
    }

    #[derive(Debug, Clone)]
    pub struct RecordedCall {
        pub program: String,
        pub args: Vec<String>,
    }

    pub struct RecordingRunner {
        pub calls: Mutex<Vec<RecordedCall>>,
        pub responses: Mutex<VecDeque<CannedResponse>>,
    }

    impl RecordingRunner {
        pub fn new(responses: Vec<CannedResponse>) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                responses: Mutex::new(VecDeque::from(responses)),
            }
        }

        pub fn calls(&self) -> Vec<RecordedCall> {
            self.calls.lock().unwrap().clone()
        }

        fn next_response(&self) -> CannedResponse {
            let mut responses = self.responses.lock().unwrap();
            responses.pop_front().unwrap_or_default()
        }

        fn record(&self, program: &str, args: &[&str]) {
            self.calls.lock().unwrap().push(RecordedCall {
                program: program.to_string(),
                args: args.iter().map(|s| s.to_string()).collect(),
            });
        }
    }

    impl CommandRunner for RecordingRunner {
        fn run(&self, program: &str, args: &[&str]) -> Result<CmdOutput> {
            self.record(program, args);
            let resp = self.next_response();
            Ok(CmdOutput {
                stdout: resp.stdout,
                stderr: resp.stderr,
                exit_code: resp.exit_code,
            })
        }

        fn run_with_stdin(&self, program: &str, args: &[&str], _stdin: &[u8]) -> Result<CmdOutput> {
            self.run(program, args)
        }
    }

    #[test]
    fn test_cmd_output_success() {
        let output = CmdOutput {
            stdout: "ok".into(),
            stderr: String::new(),
            exit_code: 0,
        };
        assert!(output.success());
    }

    #[test]
    fn test_cmd_output_failure() {
        let output = CmdOutput {
            stdout: String::new(),
            stderr: "error".into(),
            exit_code: 1,
        };
        assert!(!output.success());
    }

    #[test]
    fn test_recording_runner() {
        let runner = RecordingRunner::new(vec![CannedResponse {
            stdout: "hello\n".into(),
            stderr: String::new(),
            exit_code: 0,
        }]);

        let result = runner.run("echo", &["hello"]).unwrap();
        assert_eq!(result.stdout, "hello\n");
        assert!(result.success());

        let calls = runner.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].program, "echo");
        assert_eq!(calls[0].args, vec!["hello"]);
    }

    #[test]
    fn chroot_checked_passes_the_argv_through_and_reports_failures_by_context() {
        let runner = RecordingRunner::new(vec![
            CannedResponse::default(),
            CannedResponse {
                exit_code: 1,
                stderr: "no such preset".into(),
                ..Default::default()
            },
        ]);

        chroot_checked(&runner, Path::new("/mnt"), "locale-gen", &[], "locale-gen").unwrap();
        let err = chroot_checked(
            &runner,
            Path::new("/mnt"),
            "mkinitcpio",
            &["-p", "linux"],
            "mkinitcpio -p linux",
        )
        .unwrap_err();

        let calls = runner.calls();
        assert_eq!(calls[0].program, "arch-chroot");
        assert_eq!(calls[0].args, ["/mnt", "locale-gen"]);
        assert_eq!(calls[1].args, ["/mnt", "mkinitcpio", "-p", "linux"]);
        assert!(err.to_string().contains("mkinitcpio -p linux"), "{err}");
        assert!(err.to_string().contains("no such preset"), "{err}");
    }

    #[test]
    fn test_check_exit_ok() {
        let output = CmdOutput {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: 0,
        };
        assert!(check_exit(&output, "test").is_ok());
    }

    #[test]
    fn test_check_exit_fail() {
        let output = CmdOutput {
            stdout: String::new(),
            stderr: "bad thing".into(),
            exit_code: 1,
        };
        let err = check_exit(&output, "test command").unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("test command"));
        assert!(msg.contains("bad thing"));
    }
}

#[cfg(test)]
mod supervised_tests {
    use super::*;

    #[test]
    fn a_cancelled_command_and_its_children_stop_promptly() {
        let cancel = CancellationToken::new();
        let stopper = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            stopper.cancel();
        });
        let started = Instant::now();
        // The shell forks sleep; only a group-wide signal reaches it.
        let error = RealRunner
            .run_cancellable("sh", &["-c", "sleep 30 & wait"], &cancel)
            .expect_err("a stopped command is an error");
        assert!(error.to_string().contains("cancelled"), "{error}");
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn a_finished_command_reports_its_output() {
        let cancel = CancellationToken::new();
        let output = RealRunner
            .run_with_stdin_cancellable("cat", &[], b"hello", &cancel)
            .unwrap();
        assert!(output.success());
        assert_eq!(output.stdout, "hello");
    }

    #[test]
    fn a_cancelled_token_refuses_to_start_anything() {
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(RealRunner.run_cancellable("true", &[], &cancel).is_err());
    }
}
