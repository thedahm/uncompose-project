//! The mutating-command sidecar lock (ADR-0011).
//!
//! Every command that rewrites the manifest (`init`, `add`, `import`) serializes
//! its read-modify-write behind an exclusive advisory `flock` on
//! `<root>/.uncompose.project.lock`. The lock file is created on first use and
//! never deleted; the kernel drops the advisory lock when the holder's file
//! closes (including on a crash), so a dead holder never wedges the next command
//! and there is no manual cleanup or recovery step.
//!
//! Readers (`show`, `verify`) take no lock — the canonical temp+rename write
//! (ADR-0002) hands every reader a consistent snapshot. v0.1 is Linux-only, which
//! is what makes `flock` a safe, universal choice (per the roadmap).

use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::io::AsRawFd;
use std::path::Path;

use crate::LOCK_FILENAME;

/// The stderr notice shown once when the project lock is contended, before the
/// blocking wait. Exposed so the CLI's tests assert the exact wording.
pub const LOCK_WAIT_NOTICE: &str = "waiting for project lock…";

/// A held exclusive lock on the project. Released when dropped: the file closes
/// and the kernel releases the advisory `flock`.
#[derive(Debug)]
pub struct ProjectLock {
    // Held only for its `Drop` — dropping the `File` closes the fd, which releases
    // the advisory lock. Named with a leading underscore because it is never read.
    _file: File,
}

impl ProjectLock {
    /// Acquire the exclusive project lock at `<root>/.uncompose.project.lock`,
    /// creating the lock file if absent. Tries once without blocking; if another
    /// holder has it, prints [`LOCK_WAIT_NOTICE`] to stderr once and then blocks
    /// until the lock is free — a wait, never a failure, so a contended command
    /// queues behind the holder rather than erroring.
    pub fn acquire(root: &Path) -> io::Result<ProjectLock> {
        let path = root.join(LOCK_FILENAME);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            // The lock file is a pure rendezvous point — its contents are never
            // read or written, only `flock`ed — so never truncate it.
            .truncate(false)
            .open(&path)?;
        match flock(&file, libc::LOCK_EX | libc::LOCK_NB) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                eprintln!("{LOCK_WAIT_NOTICE}");
                flock(&file, libc::LOCK_EX)?;
            }
            Err(e) => return Err(e),
        }
        Ok(ProjectLock { _file: file })
    }
}

/// Apply `flock(2)` to `file`, mapping a non-zero return to the last OS error. A
/// `LOCK_NB` request on an already-held lock surfaces as [`io::ErrorKind::WouldBlock`].
fn flock(file: &File, operation: libc::c_int) -> io::Result<()> {
    // SAFETY: `file` owns a valid, open fd for the whole call; `flock` only
    // operates on that descriptor and does not retain it.
    let rc = unsafe { libc::flock(file.as_raw_fd(), operation) };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
