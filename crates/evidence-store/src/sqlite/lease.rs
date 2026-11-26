//! The writer lease: an exclusive advisory lock on `<store>/LOCK`. See
//! `docs/m1-store-spec.md` section 7.3.
//!
//! One writer at a time holds the lease, for as long as its [`Lease`] value
//! lives; dropping it (or the process ending, however it ends) releases it.
//! Readers never take it. The lock is `std::fs::File::try_lock` (stable since
//! Rust 1.89; `flock` on Unix, `LockFileEx` on Windows). It is per open file,
//! so a second `Lease::acquire` on the same store fails even inside the
//! process that holds the first. `flock` is unreliable on NFS, so a store
//! must be on a local disk.
//!
//! The file's text, `"pid <pid>\nepoch <n>\n"`, is for diagnostics only: it
//! names the holder in `S_LOCKED` messages. Nothing reads it to decide who
//! holds the lease. (A reader of a store it cannot write compares it before
//! and after each read, to notice that a writer has opened the store; see
//! [`SqliteReader::open`](super::SqliteReader::open).)
//!
//! A failed `SqliteLog::init` removes the `LOCK` it created while it still
//! holds the lease. Whoever opened that file before and locks it after
//! holds a file that is no longer the store's `LOCK`, so on Unix `acquire`
//! checks that the name still leads to the file it locked, and otherwise
//! opens the name again.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use crate::api::StoreError;

/// The name of the lease file in a store directory.
pub const LOCK_FILE: &str = "LOCK";

/// How many times [`Lease::acquire`] opens `LOCK` before it gives up on a
/// name that keeps leading to another file than the one it locked.
const OPENS: usize = 8;

/// The writer lease on one store. Held until dropped.
#[derive(Debug)]
pub struct Lease {
    /// `<dir>/LOCK`, open for reading and writing, locked.
    file: File,
}

impl Lease {
    /// Take the lease on the store in `dir`: open `<dir>/LOCK` (creating it
    /// if absent, never truncating it) for reading and writing, then
    /// `try_lock` it. On Unix, check that `<dir>/LOCK` still names the file
    /// it locked (same device and inode); if it does not (it was removed,
    /// or removed and created again, in between), let go and start again,
    /// up to 8 times in all.
    ///
    /// - `S_LOCKED` (`StoreError::Locked`) when another holder has it
    ///   (`TryLockError::WouldBlock`); `holder` is the file's text as read
    ///   then, which may be empty while a holder is rewriting it. Also
    ///   `S_LOCKED`, with an empty `holder`, when the name never stays.
    /// - `S_IO` for any other failure.
    ///
    /// The caller checks that `dir` holds a store (or, for `init`, is to
    /// hold one) before taking the lease, so that `LOCK` is never created
    /// anywhere else.
    pub fn acquire(dir: &Path) -> Result<Lease, StoreError> {
        let path = dir.join(LOCK_FILE);
        for _ in 0..OPENS {
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&path)
                .map_err(|e| StoreError::Io(format!("cannot open {}: {e}", path.display())))?;
            match file.try_lock() {
                Ok(()) if still_named(&file, &path) => return Ok(Lease { file }),
                // Dropping the file lets go of it.
                Ok(()) => continue,
                Err(TryLockError::WouldBlock) => {
                    // Diagnostics only: whatever can be read now, maybe
                    // nothing.
                    let mut text = Vec::new();
                    let holder = match (&file).read_to_end(&mut text) {
                        Ok(_) => String::from_utf8_lossy(&text).into_owned(),
                        Err(_) => String::new(),
                    };
                    return Err(StoreError::Locked { holder });
                }
                Err(TryLockError::Error(e)) => {
                    return Err(StoreError::Io(format!(
                        "cannot lock {}: {e}",
                        path.display()
                    )))
                }
            }
        }
        Err(StoreError::Locked {
            holder: String::new(),
        })
    }

    /// Rewrite the diagnostic text as `"pid <this process id>\nepoch
    /// <epoch>\n"`: truncate, write from the start, then `sync_data`.
    /// `S_IO` on failure.
    pub fn record(&mut self, epoch: u64) -> Result<(), StoreError> {
        let text = format!("pid {}\nepoch {epoch}\n", std::process::id());
        let mut file = &self.file;
        file.set_len(0)
            .and_then(|()| file.seek(SeekFrom::Start(0)))
            .and_then(|_| file.write_all(text.as_bytes()))
            .and_then(|()| file.sync_data())
            .map_err(|e| StoreError::Io(format!("cannot write the lease file {LOCK_FILE}: {e}")))
    }

    /// The text of `<dir>/LOCK`, lossily decoded as UTF-8, without taking
    /// the lease: who holds, or last held, the store. `S_IO` if it cannot be
    /// read.
    pub fn holder(dir: &Path) -> Result<String, StoreError> {
        let path = dir.join(LOCK_FILE);
        let bytes = fs::read(&path)
            .map_err(|e| StoreError::Io(format!("cannot read {}: {e}", path.display())))?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
}

/// Whether `path` still names `file`: the same device and inode. A name
/// that is gone does not; one whose metadata cannot be read is taken to.
#[cfg(unix)]
fn still_named(file: &File, path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (file.metadata(), fs::metadata(path)) {
        (Ok(locked), Ok(named)) => locked.dev() == named.dev() && locked.ino() == named.ino(),
        (_, Err(e)) => e.kind() != std::io::ErrorKind::NotFound,
        (Err(_), Ok(_)) => true,
    }
}

/// Elsewhere a file that is open cannot simply be removed, so the name
/// stays with the file.
#[cfg(not(unix))]
fn still_named(_file: &File, _path: &Path) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_holder_at_a_time_even_in_one_process() {
        let tmp = tempfile::tempdir().unwrap();
        let mut first = Lease::acquire(tmp.path()).unwrap();
        first.record(7).unwrap();
        let text = format!("pid {}\nepoch 7\n", std::process::id());
        assert_eq!(Lease::holder(tmp.path()).unwrap(), text);
        match Lease::acquire(tmp.path()) {
            Err(StoreError::Locked { holder }) => assert_eq!(holder, text),
            other => panic!("expected S_LOCKED, got {other:?}"),
        }
        // A shorter text replaces a longer one whole.
        first.record(10).unwrap();
        first.record(8).unwrap();
        let text = format!("pid {}\nepoch 8\n", std::process::id());
        assert_eq!(Lease::holder(tmp.path()).unwrap(), text);
        drop(first);
        let mut second = Lease::acquire(tmp.path()).unwrap();
        assert_eq!(Lease::holder(tmp.path()).unwrap(), text, "never truncated");
        second.record(9).unwrap();
    }

    /// The check that lets `acquire` start again when `LOCK` was removed,
    /// or removed and created again, after it was opened.
    #[cfg(unix)]
    #[test]
    fn a_lock_file_that_was_replaced_is_not_the_lease() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(LOCK_FILE);
        let first = Lease::acquire(tmp.path()).unwrap();
        assert!(still_named(&first.file, &path));
        fs::remove_file(&path).unwrap();
        assert!(!still_named(&first.file, &path), "removed");
        // The removed file is still locked, but a new LOCK is a new lease.
        let second = Lease::acquire(tmp.path()).unwrap();
        assert!(still_named(&second.file, &path));
        assert!(!still_named(&first.file, &path), "replaced");
        assert!(matches!(
            Lease::acquire(tmp.path()),
            Err(StoreError::Locked { .. })
        ));
        drop(first);
        assert!(matches!(
            Lease::acquire(tmp.path()),
            Err(StoreError::Locked { .. })
        ));
        drop(second);
        Lease::acquire(tmp.path()).unwrap();
    }

    #[test]
    fn a_missing_directory_is_an_io_error() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("absent");
        assert_eq!(Lease::acquire(&missing).unwrap_err().code_str(), "S_IO");
        assert_eq!(Lease::holder(&missing).unwrap_err().code_str(), "S_IO");
        assert!(!missing.exists());
    }
}
