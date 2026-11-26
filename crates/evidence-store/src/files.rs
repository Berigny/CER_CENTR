//! File-system steps shared by [`export`](crate::export::export),
//! [`import`](crate::import::import),
//! [`read_export_dir`](crate::export::read_export_dir) and
//! [`SqliteLog::init`](crate::sqlite::SqliteLog::init): refusing an
//! existing target, building in a partial directory, moving it into place
//! durably, and reading only regular files.
//!
//! Every failure is `S_IO` with the path in its detail, except where a
//! function says `S_EXISTS`.

use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read as _, Write as _};
use std::path::{Path, PathBuf};

use crate::api::StoreError;
use crate::export::partial_path;

/// How many names [`create_partial`] tries before it gives up.
pub(crate) const PARTIAL_TRIES: u32 = 1000;

/// `S_IO` for a file operation on `path` that failed with `e`.
pub(crate) fn io_error(path: &Path, e: io::Error) -> StoreError {
    StoreError::Io(format!("{}: {e}", path.display()))
}

/// [`io_error`] as a closure, for `map_err`.
pub(crate) fn io_at(path: &Path) -> impl FnOnce(io::Error) -> StoreError + '_ {
    move |e| io_error(path, e)
}

/// `S_EXISTS` if anything is at `path`, a file or a dangling symlink
/// included; `S_IO` if that cannot be told.
pub(crate) fn refuse_existing(path: &Path) -> Result<(), StoreError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(StoreError::Exists(path.to_path_buf())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(io_error(path, e)),
    }
}

/// `target`'s file name. `E_USAGE` if it has none (`/`, `..`): there is no
/// name to build a partial directory under.
pub(crate) fn target_name(target: &Path) -> Result<&OsStr, StoreError> {
    target
        .file_name()
        .ok_or_else(|| StoreError::Usage(format!("{} has no file name", target.display())))
}

/// Create a new, empty partial directory beside `target` and return its
/// path: the first of [`partial_path`]`(target, 1)`, `(target, 2)`, … up to
/// [`PARTIAL_TRIES`] that `create_dir` creates. A name that exists is
/// skipped and left alone: it belongs to another attempt running now, or
/// to one that was killed (maybe in an earlier process with the same id),
/// so a leftover never blocks a retry. `S_EXISTS` when every name exists;
/// `S_IO` for any other failure, `target`'s parent missing included.
pub(crate) fn create_partial(target: &Path) -> Result<PathBuf, StoreError> {
    create_partial_within(target, PARTIAL_TRIES)
}

/// [`create_partial`], trying at most `tries` names.
fn create_partial_within(target: &Path, tries: u32) -> Result<PathBuf, StoreError> {
    for n in 1..=tries {
        let partial = partial_path(target, n)?;
        match fs::create_dir(&partial) {
            Ok(()) => return Ok(partial),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(io_error(&partial, e)),
        }
    }
    Err(StoreError::Exists(partial_path(target, tries)?))
}

/// The bytes of the regular file at `path`, a symlink followed: see
/// [`read_bounded`]. The caller holds all of them in memory, as many as the
/// file's size says (a sparse file may claim any size).
pub(crate) fn read_regular(path: &Path) -> io::Result<Vec<u8>> {
    read_bounded(path, u64::MAX, true)
}

/// The bytes of the regular file at `path`, read so that nothing at the
/// path can block the read or make it endless:
///
/// - Anything but a regular file (a directory, a FIFO, a device, a socket)
///   is an `io::ErrorKind::InvalidInput` error and is not opened: opening a
///   FIFO blocks until a writer comes, and a device such as `/dev/zero`
///   never ends. On Unix the file is opened with `O_NONBLOCK` and
///   `O_NOCTTY` and its type checked again on the open file, so a path
///   swapped for a FIFO in between is refused instead of blocking the
///   open.
/// - At most as many bytes as the open file's size are read. A file that
///   holds more than its size says (a `/proc` file reports size 0 and may
///   never end) is an `io::ErrorKind::InvalidData` error, and so is one
///   whose size is above `limit`.
/// - With `follow` false, a symlink at `path` is not followed: it is not a
///   regular file (and on Unix `O_NOFOLLOW` refuses one swapped in before
///   the open).
pub(crate) fn read_bounded(path: &Path, limit: u64, follow: bool) -> io::Result<Vec<u8>> {
    let not_regular = || io::Error::new(io::ErrorKind::InvalidInput, "not a regular file");
    let first = if follow {
        fs::metadata(path)?
    } else {
        fs::symlink_metadata(path)?
    };
    if !first.is_file() {
        return Err(not_regular());
    }
    crate::hook::at("read:open", path).map_err(|e| io::Error::other(e.to_string()))?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let nofollow = if follow { 0 } else { libc::O_NOFOLLOW };
        options.custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY | nofollow);
    }
    let file = options.open(path)?;
    let opened = file.metadata()?;
    if !opened.is_file() {
        return Err(not_regular());
    }
    let size = opened.len();
    if size > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{size} bytes, more than the {limit} expected"),
        ));
    }
    let mut bytes = Vec::new();
    (&file).take(size).read_to_end(&mut bytes)?;
    if (&file).read(&mut [0u8])? != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the file holds more than its size says",
        ));
    }
    Ok(bytes)
}

/// Write a new file at `path` (`create_new`) and sync it.
pub(crate) fn write_synced(path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(io_at(path))?;
    file.write_all(bytes).map_err(io_at(path))?;
    file.sync_all().map_err(io_at(path))
}

/// Sync an existing file.
pub(crate) fn sync_file(path: &Path) -> Result<(), StoreError> {
    File::open(path)
        .and_then(|f| f.sync_all())
        .map_err(io_at(path))
}

/// Move a finished partial directory into place: sync it, refuse with
/// `S_EXISTS` if `target` exists, rename it to `target`, and sync
/// `target`'s parent directory.
///
/// On failure nothing is left at `target`: before the rename the partial
/// directory is removed, and after it (the parent could not be synced, so
/// the new name may not be durable) `target` is removed again. The standard
/// library has no rename that refuses to replace, so on Unix an empty
/// directory created at `target` between the check and the rename is
/// replaced.
pub(crate) fn place(partial: &Path, target: &Path) -> Result<(), StoreError> {
    let moved = sync_dir(partial)
        .and_then(|()| refuse_existing(target))
        .and_then(|()| {
            fs::rename(partial, target).map_err(|e| match refuse_existing(target) {
                Err(exists @ StoreError::Exists(_)) => exists,
                _ => io_error(target, e),
            })
        });
    if let Err(e) = moved {
        let _ = fs::remove_dir_all(partial);
        return Err(e);
    }
    if let Err(e) = sync_dir(parent_of(target)) {
        let _ = fs::remove_dir_all(target);
        return Err(e);
    }
    Ok(())
}

/// The directory that holds `path`: its parent, or `.` for a bare name.
pub(crate) fn parent_of(path: &Path) -> &Path {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    }
}

/// Sync a directory, so that the names in it are durable. Unix only: other
/// systems cannot open a directory as a file.
#[cfg(unix)]
pub(crate) fn sync_dir(dir: &Path) -> Result<(), StoreError> {
    crate::hook::at("sync", dir)?;
    File::open(dir)
        .and_then(|d| d.sync_all())
        .map_err(io_at(dir))
}

/// Sync a directory: nothing to do where directories cannot be opened.
#[cfg(not(unix))]
pub(crate) fn sync_dir(_dir: &Path) -> Result<(), StoreError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_targets_are_refused_dangling_symlinks_included() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("file");
        fs::write(&file, b"x").unwrap();
        assert_eq!(refuse_existing(&file).unwrap_err().code_str(), "S_EXISTS");
        assert_eq!(
            refuse_existing(tmp.path()).unwrap_err().code_str(),
            "S_EXISTS"
        );
        assert!(refuse_existing(&tmp.path().join("absent")).is_ok());
        #[cfg(unix)]
        {
            let link = tmp.path().join("link");
            std::os::unix::fs::symlink(tmp.path().join("nowhere"), &link).unwrap();
            assert_eq!(refuse_existing(&link).unwrap_err().code_str(), "S_EXISTS");
        }
    }

    #[test]
    fn a_partial_directory_is_placed_once() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("out");
        let partial = create_partial(&target).unwrap();
        assert_eq!(partial, partial_path(&target, 1).unwrap());
        write_synced(&partial.join("a"), b"bytes").unwrap();
        assert_eq!(
            write_synced(&partial.join("a"), b"again")
                .unwrap_err()
                .code_str(),
            "S_IO"
        );
        place(&partial, &target).unwrap();
        assert!(!partial.exists());
        assert_eq!(fs::read(target.join("a")).unwrap(), b"bytes");

        // A target that exists: refused, and the partial directory removed.
        let partial = create_partial(&target).unwrap();
        assert_eq!(place(&partial, &target).unwrap_err().code_str(), "S_EXISTS");
        assert!(!partial.exists());
        assert_eq!(fs::read(target.join("a")).unwrap(), b"bytes");
        assert_eq!(
            create_partial(&tmp.path().join("absent/p"))
                .unwrap_err()
                .code_str(),
            "S_IO"
        );
        assert_eq!(
            create_partial(Path::new("/")).unwrap_err().code_str(),
            "E_USAGE"
        );
    }

    /// Partial directories left by attempts that were killed, in this
    /// process or an earlier one with the same id, never block a retry and
    /// are left as they are. Every attempt gets a directory of its own.
    #[test]
    fn leftover_partial_directories_are_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("out");
        let pid = std::process::id();
        let leftovers = [
            tmp.path().join(format!("out.partial-{pid}")),
            partial_path(&target, 1).unwrap(),
            partial_path(&target, 2).unwrap(),
        ];
        for dir in &leftovers {
            fs::create_dir(dir).unwrap();
            fs::write(dir.join("keep"), b"k").unwrap();
        }
        let first = create_partial(&target).unwrap();
        assert_eq!(first, partial_path(&target, 3).unwrap());
        let second = create_partial(&target).unwrap();
        assert_eq!(second, partial_path(&target, 4).unwrap());
        for dir in &leftovers {
            assert_eq!(fs::read(dir.join("keep")).unwrap(), b"k");
        }
        place(&first, &target).unwrap();
        assert_eq!(place(&second, &target).unwrap_err().code_str(), "S_EXISTS");

        // Every name taken: S_EXISTS, naming the last one tried.
        let other = tmp.path().join("other");
        for n in 1..=3 {
            fs::create_dir(partial_path(&other, n).unwrap()).unwrap();
        }
        match create_partial_within(&other, 3) {
            Err(StoreError::Exists(path)) => assert_eq!(path, partial_path(&other, 3).unwrap()),
            e => panic!("expected S_EXISTS, got {e:?}"),
        }
        assert_eq!(
            create_partial_within(&other, 4).unwrap(),
            partial_path(&other, 4).unwrap()
        );
    }

    #[test]
    fn only_regular_files_are_read() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("file");
        fs::write(&file, b"bytes").unwrap();
        assert_eq!(read_regular(&file).unwrap(), b"bytes");
        assert_eq!(
            read_regular(tmp.path()).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(
            read_regular(&tmp.path().join("absent")).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        #[cfg(unix)]
        {
            let link = tmp.path().join("link");
            std::os::unix::fs::symlink(&file, &link).unwrap();
            assert_eq!(read_regular(&link).unwrap(), b"bytes", "followed");
            let device = tmp.path().join("zero");
            std::os::unix::fs::symlink("/dev/zero", &device).unwrap();
            if Path::new("/dev/zero").exists() {
                assert_eq!(
                    read_regular(&device).unwrap_err().kind(),
                    io::ErrorKind::InvalidInput
                );
            }
        }
    }

    /// A file that holds more than its size says is not read on: a `/proc`
    /// file reports size 0 and may never end (`/proc/self/pagemap` reads
    /// for gigabytes). Nor is a file larger than the limit asked.
    #[cfg(unix)]
    #[test]
    fn files_are_read_up_to_their_size() {
        let tmp = tempfile::tempdir().unwrap();
        let proc_file = Path::new("/proc/self/status");
        if proc_file.exists() {
            let link = tmp.path().join("status");
            std::os::unix::fs::symlink(proc_file, &link).unwrap();
            for path in [proc_file, link.as_path()] {
                let e = read_regular(path).unwrap_err();
                assert_eq!(e.kind(), io::ErrorKind::InvalidData, "{}", path.display());
            }
        }
        let file = tmp.path().join("file");
        fs::write(&file, b"0123456789").unwrap();
        assert_eq!(read_bounded(&file, 10, true).unwrap(), b"0123456789");
        let e = read_bounded(&file, 9, true).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::InvalidData);
        // Not following: a symlink is refused, its target is read.
        let link = tmp.path().join("link");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        assert!(read_bounded(&link, 10, false).is_err());
        assert_eq!(read_bounded(&link, 10, true).unwrap(), b"0123456789");
    }

    /// A regular file swapped for a FIFO between the type check and the
    /// open is refused, not waited on: the open does not block, and the
    /// type is checked again on the open file.
    #[cfg(unix)]
    #[test]
    fn a_file_swapped_for_a_fifo_does_not_block() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("manifest.json");
        fs::write(&file, b"{}").unwrap();
        let fifo = tmp.path().join("fifo");
        let c_path = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
        // SAFETY: a valid C string; mkfifo only creates a file.
        assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);
        let (tx, rx) = std::sync::mpsc::channel();
        let path = file.clone();
        std::thread::spawn(move || {
            let _hook = crate::hook::set(move |point, at| {
                if point == "read:open" {
                    fs::rename(&fifo, at).unwrap();
                }
                Ok(())
            });
            let _ = tx.send(read_regular(&path).map_err(|e| e.kind()));
        });
        let got = rx.recv_timeout(std::time::Duration::from_secs(30));
        if got.is_err() {
            // Let the blocked open return, so the test can end.
            let _ = OpenOptions::new().write(true).open(&file);
        }
        let got = got.expect("read_regular blocked on a FIFO");
        assert_eq!(got, Err(io::ErrorKind::InvalidInput));
    }

    #[test]
    fn a_bare_name_lives_in_the_current_directory() {
        assert_eq!(parent_of(Path::new("out")), Path::new("."));
        assert_eq!(parent_of(Path::new("a/out")), Path::new("a"));
        assert_eq!(parent_of(Path::new("/out")), Path::new("/"));
    }
}
