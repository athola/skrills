//! Handles signals for graceful shutdown and child process management.
//!
//! It provides utilities for managing system signals, ensuring
//! proper application behavior, especially on Unix-like systems.

use anyhow::Result;

#[cfg(unix)]
/// Installs a `SIGCHLD` handler to reap child processes, preventing zombies.
///
/// # Safety
/// - Alter process-wide signal disposition; call only during single-threaded startup.
/// - Discard child exit status; downstream waiters must not expect to reap children.
#[allow(unsafe_code)]
pub fn ignore_sigchld() -> Result<()> {
    use std::ptr;
    // SAFETY: This code configures the process-wide SIGCHLD handler.
    // - We use SIG_IGN with SA_NOCLDWAIT to automatically reap zombies.
    // - This is safe because: (1) libc::sigaction is the standard POSIX API
    //   for signal handling, (2) we zero-initialize the sigaction struct,
    //   (3) we check the return code and propagate errors, and (4) we call
    //   sigemptyset to properly initialize the signal mask.
    // - Caller invariants: Must be called during single-threaded startup
    //   before spawning threads that might wait on child processes.
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_flags = libc::SA_NOCLDWAIT | libc::SA_RESTART;
        sa.sa_sigaction = libc::SIG_IGN;
        libc::sigemptyset(&mut sa.sa_mask);
        let rc = libc::sigaction(libc::SIGCHLD, &sa, ptr::null_mut());
        if rc != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    Ok(())
}

#[cfg(not(unix))]
/// Provides a stub for `ignore_sigchld` on non-Unix platforms.
pub fn ignore_sigchld() -> Result<()> {
    Ok(())
}
