//! The parent owns VT recovery and the GUI/chroot/GUI process lifecycle.
//! Slint isolates keyboard input while the graphical child is running.

use crate::completion::Completion;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::FromRawFd;
use std::os::unix::net::UnixStream;
use std::sync::{Mutex, OnceLock};

static CHANNEL: OnceLock<Mutex<UnixStream>> = OnceLock::new();
const CHANNEL_ENV: &str = "AZFS_SESSION_FD";

pub fn connected() -> bool {
    CHANNEL.get().is_some()
}

fn send<T: serde::Serialize>(stream: &mut UnixStream, value: &T) -> io::Result<()> {
    let data = serde_json::to_vec(value)?;
    if data.len() > 16384 {
        return Err(io::Error::other("Oversized session message"));
    }
    stream.write_all(&(data.len() as u32).to_be_bytes())?;
    stream.write_all(&data)
}

fn receive<T: serde::de::DeserializeOwned>(stream: &mut UnixStream) -> io::Result<T> {
    let mut length = [0; 4];
    stream.read_exact(&mut length)?;
    let length = u32::from_be_bytes(length) as usize;
    if length > 16384 {
        return Err(io::Error::other("Oversized session message"));
    }
    let mut data = vec![0; length];
    stream.read_exact(&mut data)?;
    Ok(serde_json::from_slice(&data)?)
}

pub fn request_shell(completion: &Completion) -> io::Result<()> {
    let channel = CHANNEL
        .get()
        .ok_or_else(|| io::Error::other("No console supervisor"))?;
    send(&mut channel.lock().unwrap(), completion)
}

use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Command, ExitStatus};
use std::sync::atomic::{AtomicI32, Ordering};

use nix::libc;

const CHILD_ENV: &str = "AZFS_KMS_CHILD";
const SIGNALS: [i32; 4] = [libc::SIGINT, libc::SIGTERM, libc::SIGHUP, libc::SIGQUIT];
static CHILD_PID: AtomicI32 = AtomicI32::new(0);
static PENDING_SIGNAL: AtomicI32 = AtomicI32::new(0);

extern "C" fn forward_signal(signal: i32) {
    let pid = CHILD_PID.load(Ordering::Relaxed);
    if pid > 0 {
        // SAFETY: kill is async-signal-safe; the child is not reaped yet.
        unsafe {
            libc::kill(pid, signal);
        }
    } else {
        PENDING_SIGNAL.store(signal, Ordering::Relaxed);
    }
}

struct SignalForwarder([libc::sighandler_t; 4]);

impl SignalForwarder {
    fn new() -> Self {
        Self(SIGNALS.map(|signal| {
            // SAFETY: the handler only accesses lock-free atomics and calls kill.
            unsafe { libc::signal(signal, forward_signal as *const () as libc::sighandler_t) }
        }))
    }
}

impl Drop for SignalForwarder {
    fn drop(&mut self) {
        CHILD_PID.store(0, Ordering::Relaxed);
        for (signal, handler) in SIGNALS.into_iter().zip(self.0) {
            // SAFETY: restore the handler returned by the matching signal call.
            unsafe {
                libc::signal(signal, handler);
            }
        }
    }
}

struct Console {
    file: File,
    keyboard_mode: libc::c_int,
    display_mode: libc::c_int,
    termios: libc::termios,
}

fn check(result: libc::c_int) -> io::Result<()> {
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

impl Console {
    fn open() -> io::Result<Option<Self>> {
        let options = || {
            let mut options = OpenOptions::new();
            options
                .read(true)
                .write(true)
                .custom_flags(libc::O_NOCTTY | libc::O_CLOEXEC);
            options
        };
        let control = match options().open("/dev/tty0") {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        // Linux struct vt_stat consists of three unsigned shorts.
        let mut state = [0u16; 3];
        // SAFETY: the ioctl writes exactly three aligned unsigned shorts.
        check(unsafe { libc::ioctl(control.as_raw_fd(), 0x5603, state.as_mut_ptr()) })?;
        let file = options().open(format!("/dev/tty{}", state[0]))?;
        let mut console = Self {
            file,
            keyboard_mode: 0,
            display_mode: 0,
            // SAFETY: termios is a plain C structure initialized by tcgetattr below.
            termios: unsafe { std::mem::zeroed() },
        };
        // SAFETY: both GET ioctls write an int to valid aligned storage.
        check(unsafe {
            libc::ioctl(console.file.as_raw_fd(), 0x4b44, &mut console.keyboard_mode)
        })?;
        check(unsafe { libc::ioctl(console.file.as_raw_fd(), 0x4b3b, &mut console.display_mode) })?;
        // SAFETY: descriptor and termios pointer are valid.
        check(unsafe { libc::tcgetattr(console.file.as_raw_fd(), &mut console.termios) })?;
        Ok(Some(console))
    }

    fn restore(&self) -> io::Result<()> {
        // SAFETY: the descriptor is open; these are valid terminal operations.
        // Flush before reenabling input so a waiting shell receives no GUI text.
        check(unsafe { libc::tcsetattr(self.file.as_raw_fd(), libc::TCSAFLUSH, &self.termios) })?;
        check(unsafe { libc::tcflush(self.file.as_raw_fd(), libc::TCIFLUSH) })?;
        check(unsafe { libc::ioctl(self.file.as_raw_fd(), 0x4b45, self.keyboard_mode) })?;
        check(unsafe { libc::ioctl(self.file.as_raw_fd(), 0x4b3a, self.display_mode) })
    }
}

impl Drop for Console {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

/// The parent never enters Slint or a Tokio runtime. Each GUI generation gets
/// a fresh close-on-exec socket, so chroot children cannot impersonate the GUI.
/// In a GUI child returns the completion screen to restore, if any.
/// In the parent runs the entire lifecycle and exits only on final Quit.
pub fn supervise() -> io::Result<Option<Completion>> {
    if std::env::var_os(CHILD_ENV).is_some() {
        let fd: i32 = std::env::var(CHANNEL_ENV)
            .map_err(io::Error::other)?
            .parse()
            .map_err(io::Error::other)?;
        if fd < 3 {
            return Err(io::Error::other("Invalid session descriptor"));
        }
        // SAFETY: only used at single-threaded startup; fd was passed by our parent.
        let mut channel = unsafe {
            std::env::remove_var(CHILD_ENV);
            std::env::remove_var(CHANNEL_ENV);
            check(libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC))?;
            UnixStream::from_raw_fd(fd)
        };
        let completion = receive(&mut channel)?;
        CHANNEL
            .set(Mutex::new(channel))
            .map_err(|_| io::Error::other("Duplicate session channel"))?;
        return Ok(completion);
    }
    let Some(console) = Console::open()? else {
        return Ok(None);
    };
    supervise_console(
        console,
        std::env::current_exe()?,
        std::env::args_os().skip(1).collect(),
        None,
    )
}

fn supervise_console(
    console: Console,
    executable: std::path::PathBuf,
    arguments: Vec<std::ffi::OsString>,
    mut completion: Option<Completion>,
) -> io::Result<Option<Completion>> {
    let _signals = SignalForwarder::new();
    loop {
        let (mut parent, child_socket) = UnixStream::pair()?;
        let fd = child_socket.as_raw_fd();
        let mut command = Command::new(&executable);
        // A restored GUI must not re-read secrets/config or start an installation.
        if completion.is_none() {
            command.args(&arguments);
        } else {
            for (index, argument) in arguments.iter().enumerate() {
                if argument == "--ui-scale" {
                    if let Some(value) = arguments.get(index + 1) {
                        command.arg(argument).arg(value);
                    }
                } else if argument.to_string_lossy().starts_with("--ui-scale=") {
                    command.arg(argument);
                }
            }
        }
        command.env(CHILD_ENV, "1").env(CHANNEL_ENV, fd.to_string());
        // SAFETY: pre_exec only performs async-signal-safe syscalls.
        unsafe { command.pre_exec(move || check(libc::fcntl(fd, libc::F_SETFD, 0))) };
        let mut child = spawn_command(&mut command)?;
        drop(child_socket);
        if let Err(error) = send(&mut parent, &completion) {
            let _ = child.kill();
            let _ = child.wait();
            CHILD_PID.store(0, Ordering::Release);
            console.restore()?;
            return Err(error);
        }
        let status = wait_child(&mut child);
        console.restore()?;
        let status = status?;
        if !status.success() {
            std::process::exit(exit_code(status));
        }
        // GUI exits without a message for Quit/Reboot. A successful Shell request
        // is small enough to queue before GUI exit; never wait for an inherited fd.
        parent.set_read_timeout(Some(std::time::Duration::from_secs(2)))?;
        let requested: Completion = match receive(&mut parent) {
            Ok(request) => request,
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => std::process::exit(0),
            Err(error) => return Err(error),
        };
        let Some(target) = &requested.target else {
            return Err(io::Error::other("Missing installed system"));
        };
        let notice = crate::installed_shell::run(target).map_err(io::Error::other)?;
        console.restore()?;
        completion = Some(Completion {
            target: requested.target,
            notice,
        });
    }
}

fn spawn_command(command: &mut Command) -> io::Result<std::process::Child> {
    // SAFETY: only signal handlers are reset between fork and exec.
    unsafe {
        command.pre_exec(|| {
            for signal in SIGNALS {
                libc::signal(signal, libc::SIG_DFL);
            }
            Ok(())
        });
    }
    let child = command.spawn()?;
    CHILD_PID.store(child.id() as i32, Ordering::Release);
    let pending = PENDING_SIGNAL.swap(0, Ordering::AcqRel);
    if pending != 0 {
        forward_signal(pending);
    }
    Ok(child)
}

fn wait_child(child: &mut std::process::Child) -> io::Result<ExitStatus> {
    let result = child.wait();
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    CHILD_PID.store(0, Ordering::Release);
    result
}

pub fn wait_command(command: &mut Command) -> io::Result<ExitStatus> {
    let console = Console::open()?;
    let status = wait_child(&mut spawn_command(command)?);
    if let Some(console) = console {
        console.restore()?;
    }
    status
}

pub fn exit_code(status: ExitStatus) -> i32 {
    status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_round_trip_and_message_limit() {
        let (mut a, mut b) = UnixStream::pair().unwrap();
        let state = Completion {
            target: None,
            notice: "Restored".into(),
        };
        send(&mut a, &Some(state)).unwrap();
        let received: Option<Completion> = receive(&mut b).unwrap();
        assert_eq!(received.unwrap().notice, "Restored");
        a.write_all(&20000_u32.to_be_bytes()).unwrap();
        assert!(receive::<Completion>(&mut b).is_err());
    }

    /// Only for a disposable KMS VM. The production binary has no resume-file entry point.
    #[test]
    #[ignore = "requires disposable VM, active VT and prepared ZFS target"]
    fn vm_supervisor_round_trip() {
        let target = serde_json::from_slice(
            &std::fs::read(std::env::var("AZFS_TEST_TARGET").unwrap()).unwrap(),
        )
        .unwrap();
        let console = Console::open().unwrap().expect("active VT");
        supervise_console(
            console,
            std::env::var_os("AZFS_TEST_BINARY").unwrap().into(),
            vec!["--ui-scale".into(), "1.5".into()],
            Some(Completion {
                target: Some(target),
                notice: "VM fixture installation complete".into(),
            }),
        )
        .unwrap();
    }
}
