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
//!
//! `LOCK` must be a regular file in the store directory: a symlink there is
//! not followed (`S_IO`), so a planted link cannot make a writer overwrite
//! another file with its lease text.
//!
//! The lock belongs to the open file, so a process that starts another
//! program (`fork` then `exec`) while it holds the lease lends the child a
//! copy of that open file until the child's `exec` closes it. If the lease
//! is dropped in that moment, it stays held until the `exec`, and an
//! `acquire` right after the drop, in this process or another, can get
//! `S_LOCKED` with no other writer about. A caller that starts programs
//! from other threads while it reopens a store retries after a moment.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use crate::api::StoreError;
use crate::files::read_bounded;
use crate::hook;

/// The name of the lease file in a store directory.
pub const LOCK_FILE: &str = "LOCK";

/// The most of `LOCK` that is ever read: its text is a few dozen bytes.
pub(crate) const LOCK_TEXT_LIMIT: u64 = 4096;

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
    /// if absent, never truncating it, and on Unix with `O_NOFOLLOW` and
    /// `O_NONBLOCK`) for reading and writing, check that it is a regular
    /// file, then `try_lock` it. On Unix, check that `<dir>/LOCK` still
    /// names the file it locked (same device and inode); if it does not (it
    /// was removed, or removed and created again, in between), let go and
    /// start again, up to 8 times in all.
    ///
    /// - `S_LOCKED` (`StoreError::Locked`) when another holder has it
    ///   (`TryLockError::WouldBlock`); `holder` is the file's text as read
    ///   then (at most 4096 bytes), which may be empty while a holder is
    ///   rewriting it. Also `S_LOCKED`, with an empty `holder`, when the
    ///   name never stays.
    /// - `S_IO` for any other failure, a `LOCK` that is a symlink or not a
    ///   regular file included.
    ///
    /// The caller checks that `dir` holds a store (or, for `init`, is to
    /// hold one) before taking the lease, so that `LOCK` is never created
    /// anywhere else.
    pub fn acquire(dir: &Path) -> Result<Lease, StoreError> {
        let path = dir.join(LOCK_FILE);
        let cannot_open = |e: &dyn std::fmt::Display| {
            StoreError::Io(format!("cannot open {}: {e}", path.display()))
        };
        for _ in 0..OPENS {
            hook::at("lease:open", &path)?;
            let mut options = OpenOptions::new();
            options.read(true).write(true).create(true).truncate(false);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_NOCTTY);
            }
            let file = options.open(&path).map_err(|e| cannot_open(&e))?;
            if !file.metadata().map_err(|e| cannot_open(&e))?.is_file() {
                return Err(cannot_open(&"not a regular file"));
            }
            hook::at("lease:opened", &path)?;
            match file.try_lock() {
                Ok(()) if still_named(&file, &path) => return Ok(Lease { file }),
                // Dropping the file lets go of it.
                Ok(()) => continue,
                Err(TryLockError::WouldBlock) => {
                    // Diagnostics only: whatever can be read now, maybe
                    // nothing.
                    let mut text = Vec::new();
                    let holder = match (&file).take(LOCK_TEXT_LIMIT).read_to_end(&mut text) {
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
    /// read, is not a regular file (a symlink is not followed) or holds more
    /// than 4096 bytes.
    pub fn holder(dir: &Path) -> Result<String, StoreError> {
        let path = dir.join(LOCK_FILE);
        let bytes = read_bounded(&path, LOCK_TEXT_LIMIT, false)
            .map_err(|e| StoreError::Io(format!("cannot read {}: {e}", path.display())))?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
}

/// Whether `path` still names `file`: the same device and inode. A name
/// that is gone does not; one whose metadata cannot be read is taken to.
#[cfg(unix)]
fn still_named(file: &File, path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (file.metadata(), std::fs::symlink_metadata(path)) {
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
    use std::fs;

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

    /// `acquire` itself starts again when `LOCK` is removed, or removed and
    /// created again, between its open and its lock: the lease it returns
    /// is on the file the name leads to now.
    #[cfg(unix)]
    #[test]
    fn acquire_starts_again_when_lock_is_replaced_before_it_locks() {
        for recreate in [false, true] {
            let tmp = tempfile::tempdir().unwrap();
            let path = tmp.path().join(LOCK_FILE);
            let mut opens = 0;
            let _hook = hook::set(move |point, at| {
                if point == "lease:opened" {
                    opens += 1;
                    if opens == 1 {
                        fs::remove_file(at).unwrap();
                        if recreate {
                            fs::write(at, b"pid 1\nepoch 1\n").unwrap();
                        }
                    }
                }
                Ok(())
            });
            let lease = Lease::acquire(tmp.path()).unwrap();
            assert!(still_named(&lease.file, &path), "recreate {recreate}");
            assert!(matches!(
                Lease::acquire(tmp.path()),
                Err(StoreError::Locked { .. })
            ));
        }
    }

    /// A `LOCK` that is a symlink is not followed, so the file it leads to
    /// is never written; one that is not a regular file is refused too.
    #[cfg(unix)]
    #[test]
    fn a_lock_that_is_not_a_regular_file_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let victim = tmp.path().join("victim.txt");
        fs::write(&victim, b"keep me\n").unwrap();
        let dir = tmp.path().join("store");
        fs::create_dir(&dir).unwrap();
        std::os::unix::fs::symlink(&victim, dir.join(LOCK_FILE)).unwrap();
        assert_eq!(Lease::acquire(&dir).unwrap_err().code_str(), "S_IO");
        assert_eq!(Lease::holder(&dir).unwrap_err().code_str(), "S_IO");
        assert_eq!(fs::read(&victim).unwrap(), b"keep me\n");
        fs::remove_file(dir.join(LOCK_FILE)).unwrap();
        // A dangling symlink is not followed to create its target.
        std::os::unix::fs::symlink(tmp.path().join("made"), dir.join(LOCK_FILE)).unwrap();
        assert_eq!(Lease::acquire(&dir).unwrap_err().code_str(), "S_IO");
        assert!(!tmp.path().join("made").exists());
        fs::remove_file(dir.join(LOCK_FILE)).unwrap();
        fs::create_dir(dir.join(LOCK_FILE)).unwrap();
        assert_eq!(Lease::acquire(&dir).unwrap_err().code_str(), "S_IO");
        fs::remove_dir(dir.join(LOCK_FILE)).unwrap();
        // A LOCK too long to be lease text is not read whole.
        fs::write(dir.join(LOCK_FILE), vec![b'x'; 5000]).unwrap();
        assert_eq!(Lease::holder(&dir).unwrap_err().code_str(), "S_IO");
        let held = Lease::acquire(&dir).unwrap();
        match Lease::acquire(&dir) {
            Err(StoreError::Locked { holder }) => assert_eq!(holder.len(), 4096),
            other => panic!("expected S_LOCKED, got {other:?}"),
        }
        drop(held);
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
