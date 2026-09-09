//! A small parent process restores the VT even if the KMS child aborts or is killed.
//! Slint isolates keyboard input while running; the parent only owns recovery.

use std::fs::{File, OpenOptions};
use std::io;
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
        };
        // SAFETY: both GET ioctls write an int to valid aligned storage.
        check(unsafe {
            libc::ioctl(console.file.as_raw_fd(), 0x4b44, &mut console.keyboard_mode)
        })?;
        check(unsafe { libc::ioctl(console.file.as_raw_fd(), 0x4b3b, &mut console.display_mode) })?;
        Ok(Some(console))
    }

    fn restore(&self) -> io::Result<()> {
        // SAFETY: the descriptor is open; these are valid terminal operations.
        // Flush before reenabling input so a waiting shell receives no GUI text.
        check(unsafe { libc::tcflush(self.file.as_raw_fd(), libc::TCIFLUSH) })?;
        check(unsafe { libc::ioctl(self.file.as_raw_fd(), 0x4b45, self.keyboard_mode) })?;
        check(unsafe { libc::ioctl(self.file.as_raw_fd(), 0x4b3a, self.display_mode) })
    }
}

/// Returns the GUI exit status in the supervisor, or None in the GUI child.
/// Call before starting threads, installing logging, or creating a Slint window.
pub fn supervise() -> io::Result<Option<ExitStatus>> {
    if std::env::var_os(CHILD_ENV).is_some() {
        // SAFETY: startup is single threaded. Do not propagate the marker further.
        unsafe {
            std::env::remove_var(CHILD_ENV);
        }
        return Ok(None);
    }
    let Some(console) = Console::open()? else {
        return Ok(None);
    };
    let mut command = Command::new(std::env::current_exe()?);
    command
        .args(std::env::args_os().skip(1))
        .env(CHILD_ENV, "1");
    // Restore default signal handling in the child before exec. The closure only
    // uses async-signal-safe libc calls, without allocation or locks.
    unsafe {
        command.pre_exec(|| {
            for signal in SIGNALS {
                libc::signal(signal, libc::SIG_DFL);
            }
            Ok(())
        });
    }
    let _signals = SignalForwarder::new();
    let mut child = command.spawn()?;
    CHILD_PID.store(child.id() as i32, Ordering::Relaxed);
    let pending = PENDING_SIGNAL.swap(0, Ordering::Relaxed);
    if pending != 0 {
        forward_signal(pending);
    }
    let status = match child.wait() {
        Ok(status) => status,
        Err(error) => {
            // Do not restore console input while the GUI could still be alive.
            child.kill()?;
            child.wait()?;
            CHILD_PID.store(0, Ordering::Relaxed);
            console.restore()?;
            return Err(error);
        }
    };
    CHILD_PID.store(0, Ordering::Relaxed);
    let restored = console.restore();
    restored?;
    Ok(Some(status))
}

pub fn exit_code(status: ExitStatus) -> i32 {
    status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(1))
}
