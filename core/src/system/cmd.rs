use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;

use color_eyre::eyre::{Context, Result, bail};

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

    fn run_streaming(&self, program: &str, args: &[&str], tx: &Sender<String>)
    -> Result<CmdOutput>;
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
        Ok(result)
    }

    fn run_streaming(
        &self,
        program: &str,
        args: &[&str],
        tx: &Sender<String>,
    ) -> Result<CmdOutput> {
        tracing::debug!(program, ?args, "running command (streaming)");
        let mut child = Command::new(program)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .wrap_err_with(|| format!("failed to spawn: {program}"))?;

        let stdout = child.stdout.take().expect("stdout was piped");
        let reader = BufReader::new(stdout);
        let mut all_stdout = String::new();

        for line in reader.lines() {
            let line = line.wrap_err("failed to read stdout line")?;
            let _ = tx.send(line.clone());
            all_stdout.push_str(&line);
            all_stdout.push('\n');
        }

        let output = child
            .wait_with_output()
            .wrap_err("failed to wait on child")?;

        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        if !stderr.is_empty() {
            let _ = tx.send(format!("[stderr] {stderr}"));
        }

        Ok(CmdOutput {
            stdout: all_stdout,
            stderr,
            exit_code: output.status.code().unwrap_or(-1),
        })
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

pub fn chroot_streaming(
    runner: &dyn CommandRunner,
    target: &Path,
    cmd: &str,
    tx: &Sender<String>,
) -> Result<CmdOutput> {
    let target_str = target.to_string_lossy();
    runner.run_streaming("arch-chroot", &[&*target_str, "bash", "-c", cmd], tx)
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
    use std::sync::Mutex;

    #[derive(Debug, Clone)]
    pub struct CannedResponse {
        pub stdout: String,
        pub stderr: String,
        pub exit_code: i32,
    }

    impl Default for CannedResponse {
        fn default() -> Self {
            Self {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: 0,
            }
        }
    }

    #[derive(Debug, Clone)]
    pub struct RecordedCall {
        pub program: String,
        pub args: Vec<String>,
    }

    pub struct RecordingRunner {
        pub calls: Mutex<Vec<RecordedCall>>,
        pub responses: Mutex<Vec<CannedResponse>>,
    }

    impl RecordingRunner {
        pub fn new(responses: Vec<CannedResponse>) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                responses: Mutex::new(responses),
            }
        }

        pub fn calls(&self) -> Vec<RecordedCall> {
            self.calls.lock().unwrap().clone()
        }

        fn next_response(&self) -> CannedResponse {
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                CannedResponse::default()
            } else {
                responses.remove(0)
            }
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

        fn run_streaming(
            &self,
            program: &str,
            args: &[&str],
            tx: &Sender<String>,
        ) -> Result<CmdOutput> {
            self.record(program, args);
            let resp = self.next_response();
            for line in resp.stdout.lines() {
                let _ = tx.send(line.to_string());
            }
            Ok(CmdOutput {
                stdout: resp.stdout,
                stderr: resp.stderr,
                exit_code: resp.exit_code,
            })
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
