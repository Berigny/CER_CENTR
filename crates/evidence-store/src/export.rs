//! Export: the portable form of a log (segments/0). See `spec/store-0.md`,
//! "Export", and `docs/m1-store-spec.md` section 7.
//!
//! An export is a directory holding `manifest.json` and segment files
//! `000001.jsonl`, `000002.jsonl`, …. Its bytes depend only on the log and
//! `segment_size`, so a `MemLog`, a `SqliteLog`, a `SqliteReader` and the
//! JavaScript reference writer export the same log to the same bytes:
//!
//! - segment i (from 1) holds the lines of seqs (i − 1) × `segment_size` + 1
//!   to the smaller of i × `segment_size` and `size`, each followed by one
//!   LF;
//! - `manifest.json` is `Manifest::canonical()`, with no trailing LF, and its
//!   `checkpoint` member names the latest checkpoint the export holds,
//!   omitted when it holds none.
//!
//! [`read_export_dir`] is the host side of reading an export, shared by
//! [`import`](crate::import::import) and
//! [`verify_export_dir`](crate::verify::verify_export_dir).
//!
//! [`export`] and `import` never leave a half-written target: they build in
//! a partial directory beside it, [`partial_path`]`(target, n)` for the
//! first `n` from 1 whose name is free, and rename that into place, with
//! the files and directories synced first.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use evidence_core::canon;
use evidence_log::manifest::MANIFEST_FILE;
use evidence_log::merkle::MAX_SIZE;
use evidence_log::{Manifest, SegmentRef};

use crate::api::{damaged, LogRead, StoreError};
use crate::files::{
    create_partial, io_at, place, read_regular, refuse_existing, target_name, write_synced,
};

/// Lines per segment file when the caller does not choose.
pub const DEFAULT_SEGMENT_SIZE: u64 = 10_000;

/// How many lines an export asks a log for at a time.
const CHUNK: usize = 1024;

/// A whole export, in memory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportFiles {
    pub manifest: Manifest,
    /// (file name, bytes): the segment files in order, then `manifest.json`.
    /// The same files, with the same bytes, as [`export`] writes.
    pub files: Vec<(String, Vec<u8>)>,
}

/// The export of `log` with `segment_size` lines per segment, in memory: for
/// tests that compare exports byte for byte (T2, T5) without writing
/// directories. Holds the whole export, so it is for small logs; [`export`]
/// holds one segment at a time.
///
/// `size` and `head` are read once, first ([`LogRead::head`]); only lines 1
/// to `size` are read, and the manifest's `checkpoint` is the last of
/// [`LogRead::checkpoints`] with seq ≤ `size`, so an append that lands
/// meanwhile changes nothing.
///
/// - `E_USAGE` unless `segment_size` is from 1 to 2^53 − 1.
/// - `S_IO` unless the lines read are exactly seqs 1 to `size`, each without
///   an LF in it (a damaged store: verify it).
pub fn export_files(log: &dyn LogRead, segment_size: u64) -> Result<ExportFiles, StoreError> {
    check_segment_size(segment_size)?;
    let fixed = Fixed::read(log)?;
    let mut files = Vec::new();
    let manifest = build(log, fixed, segment_size, |file, bytes| {
        files.push((file.to_string(), bytes));
        Ok(())
    })?;
    files.push((MANIFEST_FILE.to_string(), manifest.canonical().into_bytes()));
    Ok(ExportFiles { manifest, files })
}

/// Write the export of `log` to the directory `out`, which must not exist,
/// and return its manifest. The files and bytes are those of
/// [`export_files`], with the same `size` rules and errors, plus:
///
/// 1. `E_USAGE` if `out` has no file name; `S_EXISTS` if `out` exists (a
///    file or a dangling symlink included). Both are checked before the log
///    is read.
/// 2. Write into a new partial directory, [`partial_path`]`(out, n)` for
///    the first `n` from 1 that `create_dir` creates (its parent must
///    exist). A name that exists, left by an attempt that was killed, is
///    skipped and left alone; `S_EXISTS` only if the first 1000 all exist.
///    One segment at a time, each file `sync_all`ed after it is written,
///    then `manifest.json`, also synced.
/// 3. Sync the partial directory, rename it to `out`, and sync `out`'s
///    parent directory (directory syncs on Unix only). `S_EXISTS` if `out`
///    appeared meanwhile. (The standard library has no rename that refuses
///    to replace: on Unix an empty directory created at `out` between that
///    check and the rename is replaced.)
///
/// On any failure after step 2 begins, the partial directory is removed and
/// nothing is left at `out`: if the parent cannot be synced after the
/// rename, `out` is removed again.
pub fn export(log: &dyn LogRead, out: &Path, segment_size: u64) -> Result<Manifest, StoreError> {
    check_segment_size(segment_size)?;
    target_name(out)?;
    refuse_existing(out)?;
    let fixed = Fixed::read(log)?;
    let partial = create_partial(out)?;
    let written = build(log, fixed, segment_size, |file, bytes| {
        write_synced(&partial.join(file), &bytes)
    })
    .and_then(|manifest| {
        let path = partial.join(MANIFEST_FILE);
        write_synced(&path, manifest.canonical().as_bytes())?;
        Ok(manifest)
    });
    match written {
        Ok(manifest) => {
            place(&partial, out)?;
            Ok(manifest)
        }
        Err(e) => {
            let _ = fs::remove_dir_all(&partial);
            Err(e)
        }
    }
}

/// `<target>.partial-<pid>-<n>`: `target`'s file name with `.partial-`,
/// this process's id, `-` and `n` appended, in the same parent directory.
/// `export` and `import` build in the first of these, from `n` = 1, that
/// they can create, and rename it into place, so a crash never leaves a
/// half-written `target`; a partial directory that a killed attempt left
/// never blocks a later one. `E_USAGE` if `target` has no file name (`/`,
/// `..`).
pub fn partial_path(target: &Path, n: u32) -> Result<PathBuf, StoreError> {
    let mut partial = target_name(target)?.to_os_string();
    partial.push(format!(".partial-{}-{n}", std::process::id()));
    Ok(target.with_file_name(partial))
}

/// The bytes of an export directory, as a verifier takes them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportBytes {
    /// The bytes of `manifest.json`.
    pub manifest: Vec<u8>,
    /// One byte string per segment file, in manifest order (see
    /// [`read_export_dir`]).
    pub segments: Vec<Vec<u8>>,
}

impl ExportBytes {
    /// The segments as slices, for `evidence_log::verify_export`.
    pub fn segment_slices(&self) -> Vec<&[u8]> {
        self.segments.iter().map(Vec::as_slice).collect()
    }
}

/// Read an export directory the way `spec/store-0.md` ("Export only",
/// reading the segments) asks a host to:
///
/// - `S_NOT_A_STORE` if `dir` holds no `manifest.json` (`dir` absent, or
///   not a directory, included); `S_IO` if it exists but cannot be read or
///   is not a regular file.
/// - If `Manifest::parse` accepts it: one byte string per file the manifest
///   lists, in its order, each the bytes of that file in `dir`, or empty
///   bytes when it is absent, cannot be read or is not a regular file (so
///   the verifier reports `S_SEGMENT_HASH` for it). Other files are
///   ignored.
/// - Otherwise: the files of `dir` whose names are six ASCII digits then
///   `.jsonl`, in name order, read the same way. `S_IO` if `dir` cannot be
///   listed.
///
/// Only regular files are read (a symlink is followed to one), and each
/// only up to the size it reports: an export may come from anywhere, and a
/// FIFO would block the read forever, while a device such as `/dev/zero`,
/// or a `/proc` file that reports size 0, would never end. A file that
/// holds more than its size says reads as unreadable, as a file that is not
/// regular does. Each file is read whole into memory, so the memory used is
/// the sum of the sizes the export's files report.
pub fn read_export_dir(dir: &Path) -> Result<ExportBytes, StoreError> {
    let path = dir.join(MANIFEST_FILE);
    let manifest = match read_regular(&path) {
        Ok(bytes) => bytes,
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
            ) =>
        {
            return Err(StoreError::NotAStore(dir.to_path_buf()));
        }
        Err(e) => return Err(io_at(&path)(e)),
    };
    let names = match Manifest::parse(&manifest) {
        Ok(listed) => listed.segments.into_iter().map(|s| s.file).collect(),
        Err(_) => segment_files(dir)?,
    };
    let segments = names
        .iter()
        .map(|name| read_regular(&dir.join(name)).unwrap_or_default())
        .collect();
    Ok(ExportBytes { manifest, segments })
}

/// The names in `dir` that are six ASCII digits then `.jsonl`, in name
/// order.
fn segment_files(dir: &Path) -> Result<Vec<String>, StoreError> {
    let mut names = Vec::new();
    for entry in fs::read_dir(dir).map_err(io_at(dir))? {
        let entry = entry.map_err(io_at(dir))?;
        if let Some(name) = entry.file_name().to_str() {
            if is_segment_name(name) {
                names.push(name.to_string());
            }
        }
    }
    names.sort();
    Ok(names)
}

/// Six ASCII digits then `.jsonl`.
fn is_segment_name(name: &str) -> bool {
    name.strip_suffix(".jsonl")
        .is_some_and(|digits| digits.len() == 6 && digits.bytes().all(|b| b.is_ascii_digit()))
}

/// `E_USAGE` unless `segment_size` is from 1 to 2^53 − 1.
fn check_segment_size(segment_size: u64) -> Result<(), StoreError> {
    if (1..=MAX_SIZE).contains(&segment_size) {
        Ok(())
    } else {
        Err(StoreError::Usage(format!(
            "segment_size must be from 1 to 2^53 - 1, not {segment_size}"
        )))
    }
}

/// What an export holds, read before any line.
struct Fixed {
    log: String,
    /// The head's seq: the export holds seqs 1 to `size`.
    size: u64,
    /// The head's entry hash.
    head: String,
    /// (id, size) of the last checkpoint with seq ≤ `size`.
    checkpoint: Option<(String, u64)>,
}

impl Fixed {
    fn read(log: &dyn LogRead) -> Result<Fixed, StoreError> {
        let head = log.head()?;
        if head.seq == 0 || head.seq > MAX_SIZE {
            return Err(damaged(format!("the log's head is seq {}", head.seq)));
        }
        let checkpoint = log
            .checkpoints()?
            .into_iter()
            .rev()
            .find(|c| c.seq <= head.seq)
            .map(|c| (c.id, c.size));
        Ok(Fixed {
            log: log.log_id().to_string(),
            size: head.seq,
            head: head.entry_hash,
            checkpoint,
        })
    }
}

/// Build the segments of the export one at a time, hand each to `put` as
/// (file name, bytes), and return the manifest.
fn build(
    log: &dyn LogRead,
    fixed: Fixed,
    segment_size: u64,
    mut put: impl FnMut(&str, Vec<u8>) -> Result<(), StoreError>,
) -> Result<Manifest, StoreError> {
    let mut segments = Vec::new();
    for index in 1..=Manifest::segment_count(fixed.size, segment_size) {
        let (first, last) = Manifest::segment_range(index, fixed.size, segment_size)
            .ok_or_else(|| StoreError::Io(format!("segment {index} has no seqs")))?;
        let file = usize::try_from(index)
            .map(Manifest::file_name)
            .map_err(|_| StoreError::Io(format!("segment {index} is beyond this platform")))?;
        let bytes = segment(log, first, last)?;
        segments.push(SegmentRef {
            file: file.clone(),
            first,
            last,
            hash: canon::sha256_hex(&bytes),
        });
        put(&file, bytes)?;
    }
    Ok(Manifest {
        log: fixed.log,
        size: fixed.size,
        head: fixed.head,
        segment_size,
        segments,
        checkpoint: fixed.checkpoint,
    })
}

/// The bytes of the segment holding seqs `first` to `last`: each line and
/// an LF. `S_IO` unless the log gives exactly those seqs, in order, and no
/// line holds an LF.
fn segment(log: &dyn LogRead, first: u64, last: u64) -> Result<Vec<u8>, StoreError> {
    let mut bytes = Vec::new();
    let mut next = first;
    while next <= last {
        let want = usize::try_from(last - next + 1).map_or(CHUNK, |n| n.min(CHUNK));
        let chunk = log.lines(next, want)?;
        if chunk.is_empty() {
            return Err(damaged(format!("the log has no seq {next}")));
        }
        for (seq, line) in chunk {
            if next > last || seq != next {
                return Err(damaged(format!(
                    "the export needs seq {next} and the log gave seq {seq}"
                )));
            }
            if line.contains('\n') {
                return Err(damaged(format!("the line of seq {seq} holds a line feed")));
            }
            bytes.extend_from_slice(line.as_bytes());
            bytes.push(b'\n');
            // next ≤ last ≤ 2^53 − 1: no overflow.
            next += 1;
        }
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{CheckpointInfo, Head, LogWrite, StoredAttestation, StoredObject};
    use crate::clock::ScriptClock;
    use crate::mem::MemLog;
    use crate::signer::SeedSigner;
    use evidence_log::{verify_export, Digest, Genesis, SubjectKey, VerifyOptions};

    fn vectors() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-vectors")
    }

    fn store_key() -> SeedSigner {
        SeedSigner::from_file(&vectors().join("keys/store-demo.seed")).unwrap()
    }

    fn ts(text: &str) -> evidence_core::time::Timestamp {
        evidence_core::time::parse(text).unwrap()
    }

    /// Genesis, object vectors 001 and 002, a checkpoint, then object
    /// vector 004: seqs 1 to 7, the checkpoint at seq 5.
    fn log() -> MemLog {
        let clock = ScriptClock::new(&["2026-11-26T00:00:00Z"]);
        let mut log = MemLog::init("cer-demo", &store_key(), Box::new(clock.clone())).unwrap();
        for case in ["001-create-inline", "002-update-with-prev"] {
            let bytes = fs::read(vectors().join("objects").join(case).join("object.json")).unwrap();
            clock.push(ts("2026-11-26T00:00:01Z"));
            log.append(&bytes).unwrap();
        }
        clock.push(ts("2026-11-26T00:00:02Z"));
        log.checkpoint(&store_key()).unwrap();
        let bytes = fs::read(vectors().join("objects/004-external-content/object.json")).unwrap();
        clock.push(ts("2026-11-26T00:00:03Z"));
        log.append(&bytes).unwrap();
        assert_eq!(clock.remaining(), 0);
        log
    }

    /// A log that answers as the `MemLog` does, except for what is changed
    /// here.
    #[derive(Default)]
    struct Doctored {
        /// The head is this seq (as if read before later appends).
        head: Option<u64>,
        /// Leave this seq out of `lines`.
        drop_seq: Option<u64>,
        /// Give one line more than asked.
        extra: bool,
        /// Put an LF into the line of this seq.
        lf_seq: Option<u64>,
    }

    struct View<'a>(&'a MemLog, Doctored);

    impl LogRead for View<'_> {
        fn log_id(&self) -> &str {
            self.0.log_id()
        }
        fn genesis(&self) -> &Genesis {
            self.0.genesis()
        }
        fn head(&self) -> Result<Head, StoreError> {
            match self.1.head {
                None => self.0.head(),
                Some(seq) => {
                    let lines = self.0.lines(seq, 1)?;
                    let line = evidence_log::parse_line(lines[0].1.as_bytes()).unwrap();
                    Ok(Head {
                        seq,
                        entry_hash: line.entry.hash(),
                    })
                }
            }
        }
        fn lines(&self, from_seq: u64, limit: usize) -> Result<Vec<(u64, String)>, StoreError> {
            let limit = if self.1.extra { limit + 1 } else { limit };
            let mut lines = self.0.lines(from_seq, limit)?;
            lines.retain(|(seq, _)| Some(*seq) != self.1.drop_seq);
            for (seq, line) in &mut lines {
                if Some(*seq) == self.1.lf_seq {
                    line.insert(1, '\n');
                }
            }
            Ok(lines)
        }
        fn object(&self, id: &str) -> Result<Option<StoredObject>, StoreError> {
            self.0.object(id)
        }
        fn attestations(&self, object_id: &str) -> Result<Vec<StoredAttestation>, StoreError> {
            self.0.attestations(object_id)
        }
        fn heads(&self, subject: &SubjectKey, lane: &str) -> Result<Vec<String>, StoreError> {
            self.0.heads(subject, lane)
        }
        fn head_rows(&self) -> Result<Vec<(SubjectKey, String, String)>, StoreError> {
            self.0.head_rows()
        }
        fn checkpoints(&self) -> Result<Vec<CheckpointInfo>, StoreError> {
            self.0.checkpoints()
        }
        fn leaf_hashes(&self, upto: u64) -> Result<Vec<Digest>, StoreError> {
            self.0.leaf_hashes(upto)
        }
    }

    fn report_of(files: &ExportFiles) -> evidence_log::StoreReport {
        let (manifest, segments) = files.files.split_last().unwrap();
        assert_eq!(manifest.0, MANIFEST_FILE);
        let segments: Vec<&[u8]> = segments.iter().map(|(_, b)| b.as_slice()).collect();
        verify_export(&manifest.1, &segments, VerifyOptions::default())
    }

    #[test]
    fn segments_follow_the_manifest_layout() {
        let log = log();
        let lines = log.lines(1, 100).unwrap();
        assert_eq!(lines.len(), 7);
        let checkpoint = log.checkpoints().unwrap()[0].clone();
        for segment_size in [1, 2, 3, 6, 7, 8, DEFAULT_SEGMENT_SIZE, MAX_SIZE] {
            let files = export_files(&log, segment_size).unwrap();
            let m = &files.manifest;
            assert_eq!(m.log, log.log_id());
            assert_eq!((m.size, m.head.clone()), {
                let head = log.head().unwrap();
                (head.seq, head.entry_hash)
            });
            assert_eq!(m.segment_size, segment_size);
            assert_eq!(m.checkpoint, Some((checkpoint.id.clone(), checkpoint.size)));
            assert_eq!(files.files.len(), m.segments.len() + 1);
            assert_eq!(
                m.segments.len() as u64,
                Manifest::segment_count(7, segment_size)
            );
            for (seg, (name, bytes)) in m.segments.iter().zip(&files.files) {
                assert_eq!(&seg.file, name);
                assert_eq!(seg.hash, canon::sha256_hex(bytes));
                let want: String = lines[seg.first as usize - 1..seg.last as usize]
                    .iter()
                    .map(|(_, l)| format!("{l}\n"))
                    .collect();
                assert_eq!(bytes, want.as_bytes(), "{name}");
            }
            let (name, bytes) = files.files.last().unwrap();
            assert_eq!(name, MANIFEST_FILE);
            assert_eq!(bytes, m.canonical().as_bytes());
            assert_eq!(Manifest::parse(bytes).as_ref(), Ok(m));
            let report = report_of(&files);
            assert!(report.ok(), "{segment_size}: {:?}", report.findings);
        }
    }

    #[test]
    fn segment_size_is_a_position() {
        let log = log();
        for bad in [0, MAX_SIZE + 1, u64::MAX] {
            assert_eq!(export_files(&log, bad).unwrap_err().code_str(), "E_USAGE");
            let dir = tempfile::tempdir().unwrap();
            let out = dir.path().join("out");
            assert_eq!(export(&log, &out, bad).unwrap_err().code_str(), "E_USAGE");
            assert!(fs::read_dir(dir.path()).unwrap().next().is_none());
        }
    }

    #[test]
    fn only_the_lines_up_to_the_head_read_first() {
        let log = log();
        let checkpoint = log.checkpoints().unwrap()[0].clone();
        // A head read before the checkpoint landed: no checkpoint, 4 lines.
        let before = View(
            &log,
            Doctored {
                head: Some(4),
                ..Doctored::default()
            },
        );
        let files = export_files(&before, 3).unwrap();
        assert_eq!(files.manifest.size, 4);
        assert_eq!(files.manifest.checkpoint, None);
        assert!(report_of(&files).ok());
        // A head read after the checkpoint and its witness, before seq 7.
        let after = View(
            &log,
            Doctored {
                head: Some(6),
                ..Doctored::default()
            },
        );
        let files = export_files(&after, 3).unwrap();
        assert_eq!(files.manifest.size, 6);
        assert_eq!(
            files.manifest.checkpoint,
            Some((checkpoint.id, checkpoint.size))
        );
        assert!(report_of(&files).ok());
    }

    #[test]
    fn a_log_without_checkpoints_exports_none() {
        let clock = ScriptClock::new(&["2026-11-26T00:00:00Z"]);
        let log = MemLog::init("cer-demo", &store_key(), Box::new(clock)).unwrap();
        let files = export_files(&log, 1).unwrap();
        assert_eq!(files.manifest.size, 2);
        assert_eq!(files.manifest.checkpoint, None);
        assert_eq!(files.files.len(), 3);
        assert!(report_of(&files).ok());
    }

    #[test]
    fn damaged_lines_are_s_io() {
        let log = log();
        let cases = [
            Doctored {
                drop_seq: Some(1),
                ..Doctored::default()
            },
            Doctored {
                drop_seq: Some(4),
                ..Doctored::default()
            },
            Doctored {
                drop_seq: Some(7),
                ..Doctored::default()
            },
            Doctored {
                extra: true,
                head: Some(6),
                ..Doctored::default()
            },
            Doctored {
                lf_seq: Some(3),
                ..Doctored::default()
            },
        ];
        for doctored in cases {
            let view = View(&log, doctored);
            for segment_size in [2, 3, DEFAULT_SEGMENT_SIZE] {
                let e = export_files(&view, segment_size).unwrap_err();
                assert_eq!(e.code_str(), "S_IO", "{e}");
                let dir = tempfile::tempdir().unwrap();
                let out = dir.path().join("out");
                let e = export(&view, &out, segment_size).unwrap_err();
                assert_eq!(e.code_str(), "S_IO", "{e}");
                // Nothing is left: neither the target nor the partial
                // directory.
                assert!(fs::read_dir(dir.path()).unwrap().next().is_none());
            }
        }
        // A log that would give one line more than asked, read in one
        // segment up to its true head: there is no line past the head to
        // give, so the export is whole.
        let view = View(
            &log,
            Doctored {
                extra: true,
                ..Doctored::default()
            },
        );
        assert!(export_files(&view, DEFAULT_SEGMENT_SIZE).is_ok());
    }

    #[test]
    fn partial_paths() {
        let pid = std::process::id();
        assert_eq!(
            partial_path(Path::new("a/b"), 1).unwrap(),
            PathBuf::from(format!("a/b.partial-{pid}-1"))
        );
        assert_eq!(
            partial_path(Path::new("a/b/"), 2).unwrap(),
            PathBuf::from(format!("a/b.partial-{pid}-2"))
        );
        assert_eq!(
            partial_path(Path::new("out"), 1000).unwrap(),
            PathBuf::from(format!("out.partial-{pid}-1000"))
        );
        for bad in ["/", "..", "a/..", "."] {
            let e = partial_path(Path::new(bad), 1).unwrap_err();
            assert_eq!(e.code_str(), "E_USAGE", "{bad}");
        }
    }

    /// The names and bytes of every file in `dir`, sorted by name.
    fn files_in(dir: &Path) -> Vec<(String, Vec<u8>)> {
        let mut files: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|e| {
                let e = e.unwrap();
                let name = e.file_name().into_string().unwrap();
                (name, fs::read(e.path()).unwrap())
            })
            .collect();
        files.sort();
        files
    }

    #[test]
    fn export_writes_the_same_bytes_and_nothing_else() {
        let log = log();
        for segment_size in [2, DEFAULT_SEGMENT_SIZE] {
            let dir = tempfile::tempdir().unwrap();
            let out = dir.path().join("export");
            let manifest = export(&log, &out, segment_size).unwrap();
            let mut want = export_files(&log, segment_size).unwrap();
            assert_eq!(manifest, want.manifest);
            want.files.sort();
            assert_eq!(files_in(&out), want.files);
            // Only the export is left beside it: no partial directory.
            let names: Vec<_> = fs::read_dir(dir.path())
                .unwrap()
                .map(|e| e.unwrap().file_name())
                .collect();
            assert_eq!(names, vec![std::ffi::OsString::from("export")]);
            // Read back as a host reads it, it verifies.
            let bytes = read_export_dir(&out).unwrap();
            let report = verify_export(
                &bytes.manifest,
                &bytes.segment_slices(),
                VerifyOptions::default(),
            );
            assert!(report.ok(), "{:?}", report.findings);
        }
    }

    #[test]
    fn export_refuses_an_existing_target() {
        let log = log();
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out");

        fs::create_dir(&out).unwrap();
        assert_eq!(export(&log, &out, 4).unwrap_err().code_str(), "S_EXISTS");
        fs::remove_dir(&out).unwrap();

        fs::write(&out, b"x").unwrap();
        assert_eq!(export(&log, &out, 4).unwrap_err().code_str(), "S_EXISTS");
        assert_eq!(fs::read(&out).unwrap(), b"x");
        fs::remove_file(&out).unwrap();

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dir.path().join("nowhere"), &out).unwrap();
            assert_eq!(export(&log, &out, 4).unwrap_err().code_str(), "S_EXISTS");
            fs::remove_file(&out).unwrap();
        }

        // Partial directories left by attempts that were killed, in this
        // process or an earlier one with the same id.
        let pid = std::process::id();
        let leftovers = [
            dir.path().join(format!("out.partial-{pid}")),
            partial_path(&out, 1).unwrap(),
            partial_path(&out, 2).unwrap(),
        ];
        for partial in &leftovers {
            fs::create_dir(partial).unwrap();
            fs::write(partial.join("keep"), b"k").unwrap();
        }

        // No parent: S_IO, nothing created.
        let orphan = dir.path().join("missing").join("out");
        assert_eq!(export(&log, &orphan, 4).unwrap_err().code_str(), "S_IO");
        assert!(!dir.path().join("missing").exists());

        // No file name.
        assert_eq!(
            export(&log, &dir.path().join(".."), 4)
                .unwrap_err()
                .code_str(),
            "E_USAGE"
        );

        // And then it works, past the leftovers, which never block an
        // export and are left as they were; no other partial directory is
        // left.
        export(&log, &out, 4).unwrap();
        assert!(out.join(MANIFEST_FILE).is_file());
        assert_eq!(export(&log, &out, 4).unwrap_err().code_str(), "S_EXISTS");
        for partial in &leftovers {
            assert_eq!(fs::read(partial.join("keep")).unwrap(), b"k");
        }
        assert_eq!(
            fs::read_dir(dir.path()).unwrap().count(),
            leftovers.len() + 1
        );
    }

    #[test]
    fn reading_an_export_directory() {
        let log = log();
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("export");
        let manifest = export(&log, &out, 3).unwrap();
        let files = export_files(&log, 3).unwrap();
        let segment = |i: usize| files.files[i].1.clone();
        assert_eq!(manifest.segments.len(), 3);

        // As listed.
        let bytes = read_export_dir(&out).unwrap();
        assert_eq!(bytes.manifest, manifest.canonical().into_bytes());
        assert_eq!(bytes.segments, vec![segment(0), segment(1), segment(2)]);

        // A listed file that is absent, or cannot be read, reads as empty;
        // files the manifest does not list are ignored.
        fs::remove_file(out.join("000002.jsonl")).unwrap();
        fs::create_dir(out.join("000002.jsonl")).unwrap();
        fs::write(out.join("000004.jsonl"), b"extra\n").unwrap();
        let bytes = read_export_dir(&out).unwrap();
        assert_eq!(bytes.segments, vec![segment(0), Vec::new(), segment(2)]);
        fs::remove_dir(out.join("000002.jsonl")).unwrap();
        let bytes = read_export_dir(&out).unwrap();
        assert_eq!(bytes.segments, vec![segment(0), Vec::new(), segment(2)]);
        let report = verify_export(
            &bytes.manifest,
            &bytes.segment_slices(),
            VerifyOptions::default(),
        );
        assert!(report
            .findings
            .iter()
            .any(|f| f.code == evidence_log::FindingCode::SegmentHash));

        // A manifest that does not parse: the six-digit .jsonl files, in
        // name order, and nothing else.
        fs::write(out.join(MANIFEST_FILE), b"{}").unwrap();
        for other in ["1.jsonl", "0000001.jsonl", "00000a.jsonl", "000005.json"] {
            fs::write(out.join(other), b"ignored\n").unwrap();
        }
        fs::create_dir(out.join("000000.jsonl")).unwrap();
        let bytes = read_export_dir(&out).unwrap();
        assert_eq!(bytes.manifest, b"{}");
        assert_eq!(
            bytes.segments,
            vec![Vec::new(), segment(0), segment(2), b"extra\n".to_vec()]
        );

        // No manifest.json: not an export.
        fs::remove_file(out.join(MANIFEST_FILE)).unwrap();
        assert_eq!(
            read_export_dir(&out).unwrap_err().code_str(),
            "S_NOT_A_STORE"
        );
        assert_eq!(
            read_export_dir(&dir.path().join("absent"))
                .unwrap_err()
                .code_str(),
            "S_NOT_A_STORE"
        );
        let file = dir.path().join("file");
        fs::write(&file, b"x").unwrap();
        assert_eq!(
            read_export_dir(&file).unwrap_err().code_str(),
            "S_NOT_A_STORE"
        );

        // A manifest.json that cannot be read.
        fs::create_dir(out.join(MANIFEST_FILE)).unwrap();
        assert_eq!(read_export_dir(&out).unwrap_err().code_str(), "S_IO");
    }

    /// `read_export_dir` run on another thread, given up after a while: a
    /// FIFO read with `fs::read` would block forever.
    #[cfg(unix)]
    fn read_with_deadline(dir: &Path) -> Result<ExportBytes, StoreError> {
        let dir = dir.to_path_buf();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(read_export_dir(&dir));
        });
        rx.recv_timeout(std::time::Duration::from_secs(30))
            .expect("read_export_dir did not return")
    }

    /// A FIFO, a device or a `/proc` file in an export, under a listed name
    /// or as `manifest.json`, is not read: a listed file reads as absent
    /// (empty bytes, `S_SEGMENT_HASH`) and the manifest as `S_IO`. Nothing
    /// blocks and nothing reads without end.
    #[cfg(unix)]
    #[test]
    fn files_that_are_not_regular_are_not_read() {
        let log = log();
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("export");
        export(&log, &out, 3).unwrap();
        let files = export_files(&log, 3).unwrap();
        let segment = |i: usize| files.files[i].1.clone();
        // Through libc rather than the mkfifo program: a process started
        // here would hold a copy of every open file, the lease of a test
        // running beside this one included, until it runs its program.
        let mkfifo = |path: &Path| {
            let c_path = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
            // SAFETY: a valid C string; mkfifo only creates a file.
            unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) == 0 }
        };

        fs::remove_file(out.join("000002.jsonl")).unwrap();
        let fifo = mkfifo(&out.join("000002.jsonl"));
        if !fifo {
            std::os::unix::fs::symlink("/dev/null", out.join("000002.jsonl")).unwrap();
        }
        let bytes = read_with_deadline(&out).unwrap();
        assert_eq!(bytes.segments, vec![segment(0), Vec::new(), segment(2)]);
        let report = crate::verify::verify_export_dir(&out, VerifyOptions::default()).unwrap();
        assert!(report
            .findings
            .iter()
            .any(|f| f.code == evidence_log::FindingCode::SegmentHash));
        let into = dir.path().join("store");
        assert!(!crate::import::import(&out, &into).unwrap().ok());
        assert!(!into.exists());

        // A device, through a symlink: not read at all.
        fs::remove_file(out.join("000002.jsonl")).unwrap();
        std::os::unix::fs::symlink("/dev/zero", out.join("000002.jsonl")).unwrap();
        let bytes = read_with_deadline(&out).unwrap();
        assert_eq!(bytes.segments, vec![segment(0), Vec::new(), segment(2)]);
        // A /proc file says it is a regular file of size 0, then reads on
        // (/proc/self/pagemap for gigabytes): read as absent too.
        let proc_file = Path::new("/proc/self/status");
        if proc_file.exists() {
            fs::remove_file(out.join("000002.jsonl")).unwrap();
            std::os::unix::fs::symlink(proc_file, out.join("000002.jsonl")).unwrap();
            let bytes = read_with_deadline(&out).unwrap();
            assert_eq!(bytes.segments, vec![segment(0), Vec::new(), segment(2)]);
        }
        fs::remove_file(out.join("000002.jsonl")).unwrap();
        fs::write(out.join("000002.jsonl"), segment(1)).unwrap();
        assert!(read_with_deadline(&out).is_ok());

        // manifest.json that is not a regular file: S_IO.
        let manifest = fs::read(out.join(MANIFEST_FILE)).unwrap();
        fs::remove_file(out.join(MANIFEST_FILE)).unwrap();
        if fifo {
            assert!(mkfifo(&out.join(MANIFEST_FILE)));
            assert_eq!(read_with_deadline(&out).unwrap_err().code_str(), "S_IO");
            fs::remove_file(out.join(MANIFEST_FILE)).unwrap();
        }
        std::os::unix::fs::symlink("/dev/zero", out.join(MANIFEST_FILE)).unwrap();
        assert_eq!(read_with_deadline(&out).unwrap_err().code_str(), "S_IO");
        fs::remove_file(out.join(MANIFEST_FILE)).unwrap();
        if proc_file.exists() {
            std::os::unix::fs::symlink(proc_file, out.join(MANIFEST_FILE)).unwrap();
            assert_eq!(read_with_deadline(&out).unwrap_err().code_str(), "S_IO");
            fs::remove_file(out.join(MANIFEST_FILE)).unwrap();
        }
        // A symlink to a regular file is followed.
        let elsewhere = dir.path().join("manifest-copy.json");
        fs::write(&elsewhere, &manifest).unwrap();
        std::os::unix::fs::symlink(&elsewhere, out.join(MANIFEST_FILE)).unwrap();
        assert_eq!(read_with_deadline(&out).unwrap().manifest, manifest);
    }
}
