//! Verifying a live log and an export directory. See `spec/store-0.md`,
//! "Verification", "Live store only" and "Export only".
//!
//! One algorithm, `evidence_log::Verifier`, checks every source; this module
//! feeds it:
//!
//! | | lines | projections (`S_PROJECTION_DRIFT`) | triggers (`S_TRIGGERS_MISSING`) | `meta.log_id` (`S_META_MISMATCH`) | manifest |
//! |---|---|---|---|---|---|
//! | [`verify_log`], any [`LogRead`] | yes | yes | no | no | no |
//! | [`verify_store`], a store directory | yes | yes | yes | yes | no |
//! | [`verify_export_dir`], an export directory | yes | no | no | no | yes |
//!
//! A log's own [`LogRead::log_id`] is not compared with the lines by
//! `verify_log`: for `MemLog` it is the genesis id by construction, and for
//! the SQLite types it is `meta.log_id`, which only `verify_store` checks.
//! Triggers exist only in the SQLite store.

use std::path::Path;

use evidence_log::{
    verify_export, Finding, FindingCode, Source, StoreReport, Verifier, VerifyOptions,
};

use crate::api::{CheckpointInfo, LogRead, StoreError};
use crate::export::read_export_dir;
use crate::sqlite::SqliteReader;

/// How many lines [`verify_log`] asks a log for at a time.
const CHUNK: usize = 1024;

/// Verify any log: its lines, then its projections.
///
/// 1. A `Verifier` with `opts.max_findings` and source `store` (whatever
///    `opts.source` says).
/// 2. Push every line with `push_line`, in seq order: [`LogRead::lines`]
///    from seq 1, in chunks, each next chunk starting after the last seq
///    returned, until a chunk comes back empty. Gaps in the seqs are
///    `S_SEQ_GAP` findings, not errors.
/// 3. Projection drift: one `S_PROJECTION_DRIFT` finding (no seq) if
///    [`LogRead::head_rows`] differs in any row from
///    `Verifier::heads().rows()`, or [`LogRead::checkpoints`] differs in any
///    row from `Verifier::checkpoints()` (seq, id, size, root, head).
/// 4. `finish()`.
///
/// An error from the log (`S_IO`) is returned as an error, not a finding:
/// a report is only given for a log that could be read.
pub fn verify_log(log: &dyn LogRead, opts: VerifyOptions) -> Result<StoreReport, StoreError> {
    Ok(check_log(log, opts)?.finish())
}

/// Steps 1 to 3 of [`verify_log`]: the lines and the projection drift
/// check, with the verifier given back before `finish()`, so that
/// [`verify_store`] can add the live-store findings in the same snapshot.
pub(crate) fn check_log(log: &dyn LogRead, opts: VerifyOptions) -> Result<Verifier, StoreError> {
    let mut verifier = Verifier::new(VerifyOptions {
        max_findings: opts.max_findings,
        source: Source::Store,
    });
    let mut from = 1;
    loop {
        let chunk = log.lines(from, CHUNK)?;
        let Some(last) = chunk.last().map(|(seq, _)| *seq) else {
            break;
        };
        for (_, line) in &chunk {
            verifier.push_line(line.as_bytes());
        }
        // A log gives seqs from `from` on; one that went back would never
        // end this loop.
        if last < from {
            return Err(StoreError::Io(format!(
                "asked for lines from seq {from}, the log gave seq {last}"
            )));
        }
        match last.checked_add(1) {
            Some(next) => from = next,
            None => break,
        }
    }
    if projections_drift(log, &verifier)? {
        verifier.add_finding(Finding::new(FindingCode::ProjectionDrift));
    }
    Ok(verifier)
}

/// Whether the log's `heads` or `checkpoints` differ in any row from the
/// fold over its lines.
fn projections_drift(log: &dyn LogRead, verifier: &Verifier) -> Result<bool, StoreError> {
    if log.head_rows()? != verifier.heads().rows() {
        return Ok(true);
    }
    let recomputed: Vec<CheckpointInfo> = verifier
        .checkpoints()
        .iter()
        .map(|(seq, id, checkpoint)| CheckpointInfo {
            id: id.clone(),
            seq: *seq,
            size: checkpoint.size,
            root: checkpoint.root.clone(),
            head: checkpoint.head.clone(),
        })
        .collect();
    Ok(log.checkpoints()? != recomputed)
}

/// Verify the store in `dir`: [`verify_log`]'s steps, plus the live-store
/// checks, all inside one [`SqliteReader::snapshot`](crate::sqlite::SqliteReader::snapshot)
/// so that the lines and the tables are of one moment, while a writer may be
/// appending.
///
/// 1. `SqliteReader::open(dir)`: `S_NOT_A_STORE` or `S_FORMAT` as it says.
/// 2. In one snapshot: the lines and the projection drift check, as
///    [`verify_log`]; then `S_TRIGGERS_MISSING` (no seq) unless
///    `has_append_only_triggers()`; then `S_META_MISMATCH` (no seq) unless
///    the lines have a log id and it equals `meta.log_id`.
/// 3. `finish()`. The report's source is `store`.
///
/// The stored log id is read inside the snapshot too; a `meta` row that is
/// gone by then gives `S_META_MISMATCH`, not an error. Damaged content in
/// the tables (an edited line, a missing seq, a row with no item) is a
/// finding; only a store that cannot be read at all is an error.
pub fn verify_store(dir: &Path, opts: VerifyOptions) -> Result<StoreReport, StoreError> {
    let reader = SqliteReader::open(dir)?;
    reader.snapshot(|r| {
        let mut verifier = check_log(r, opts)?;
        if !r.has_append_only_triggers()? {
            verifier.add_finding(Finding::new(FindingCode::TriggersMissing));
        }
        let stored = r.stored_log_id()?;
        if stored.is_none() || verifier.log_id() != stored.as_deref() {
            verifier.add_finding(Finding::new(FindingCode::MetaMismatch));
        }
        Ok(verifier.finish())
    })
}

/// Verify the export in `dir`: [`read_export_dir`], then
/// `evidence_log::verify_export` with `opts.max_findings`. The report's
/// source is `export`. `S_NOT_A_STORE` if `dir` holds no `manifest.json`.
pub fn verify_export_dir(dir: &Path, opts: VerifyOptions) -> Result<StoreReport, StoreError> {
    let export = read_export_dir(dir)?;
    Ok(verify_export(
        &export.manifest,
        &export.segment_slices(),
        opts,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{Head, LogWrite, StoredAttestation, StoredObject};
    use crate::clock::ScriptClock;
    use crate::mem::MemLog;
    use crate::signer::SeedSigner;
    use evidence_log::{Digest, Genesis, SubjectKey};

    fn store_key() -> SeedSigner {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-vectors/keys/store-demo.seed");
        SeedSigner::from_file(&path).unwrap()
    }

    /// Genesis, object vectors 001 and 002, and a checkpoint: seqs 1 to 6.
    fn log() -> MemLog {
        let clock = ScriptClock::new(&[
            "2026-11-26T00:00:00Z",
            "2026-11-26T00:00:01Z",
            "2026-11-26T00:00:02Z",
        ]);
        let mut log = MemLog::init("cer-demo", &store_key(), Box::new(clock.clone())).unwrap();
        let objects = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-vectors/objects");
        for case in ["001-create-inline", "002-update-with-prev"] {
            let bytes = std::fs::read(objects.join(case).join("object.json")).unwrap();
            log.append(&bytes).unwrap();
        }
        clock.push(evidence_core::time::parse("2026-11-26T00:00:03Z").unwrap());
        log.checkpoint(&store_key()).unwrap();
        log
    }

    /// A log that answers as `log` does, except for what is changed here.
    #[derive(Default)]
    struct Doctored {
        /// Leave this seq out of `lines`.
        drop_seq: Option<u64>,
        /// Give at most this many lines per call.
        per_call: Option<usize>,
        /// Give the lines from seq 1 whatever `from_seq` is asked.
        restart: bool,
        head_rows: Option<Vec<(SubjectKey, String, String)>>,
        checkpoints: Option<Vec<CheckpointInfo>>,
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
            self.0.head()
        }
        fn lines(&self, from_seq: u64, limit: usize) -> Result<Vec<(u64, String)>, StoreError> {
            let from = if self.1.restart { 1 } else { from_seq };
            let limit = self.1.per_call.map_or(limit, |n| n.min(limit));
            let mut lines = self.0.lines(from, limit)?;
            lines.retain(|(seq, _)| Some(*seq) != self.1.drop_seq);
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
            match &self.1.head_rows {
                Some(rows) => Ok(rows.clone()),
                None => self.0.head_rows(),
            }
        }
        fn checkpoints(&self) -> Result<Vec<CheckpointInfo>, StoreError> {
            match &self.1.checkpoints {
                Some(rows) => Ok(rows.clone()),
                None => self.0.checkpoints(),
            }
        }
        fn leaf_hashes(&self, upto: u64) -> Result<Vec<Digest>, StoreError> {
            self.0.leaf_hashes(upto)
        }
    }

    fn codes(report: &StoreReport) -> Vec<(&'static str, Option<u64>)> {
        let findings = report.findings.iter();
        findings.map(|f| (f.code.as_str(), f.seq)).collect()
    }

    #[test]
    fn an_undamaged_log_verifies_as_a_store() {
        let log = log();
        let opts = VerifyOptions {
            max_findings: 10,
            source: Source::Export,
        };
        let report = verify_log(&log, opts).unwrap();
        assert!(report.ok(), "{:?}", report.findings);
        assert_eq!(report.source, Source::Store, "whatever opts.source says");
        assert_eq!(report.log.as_deref(), Some(log.log_id()));
        let head = log.head().unwrap();
        assert_eq!(report.head, Some((head.seq, head.entry_hash)));
        assert_eq!(report.checked.entries, 6);
        assert_eq!(report.checked.checkpoints, 1);
        let checkpoint = log.checkpoints().unwrap().remove(0);
        assert_eq!(report.covered, Some((checkpoint.size, checkpoint.id)));
    }

    #[test]
    fn lines_are_read_in_chunks_up_to_the_head() {
        let log = log();
        let one_at_a_time = Doctored {
            per_call: Some(1),
            ..Doctored::default()
        };
        let report = verify_log(&View(&log, one_at_a_time), VerifyOptions::default()).unwrap();
        assert_eq!(report, verify_log(&log, VerifyOptions::default()).unwrap());
    }

    #[test]
    fn a_missing_line_is_a_finding_not_an_error() {
        let log = log();
        let gap = Doctored {
            drop_seq: Some(4),
            ..Doctored::default()
        };
        let report = verify_log(&View(&log, gap), VerifyOptions::default()).unwrap();
        let codes = codes(&report);
        assert!(codes.contains(&("S_SEQ_GAP", Some(5))), "{codes:?}");
        assert!(codes.contains(&("S_CHAIN_BREAK", Some(5))), "{codes:?}");
        assert!(
            codes.contains(&("S_CHECKPOINT_MISMATCH", Some(5))),
            "{codes:?}"
        );
    }

    #[test]
    fn projection_drift_is_one_finding() {
        let log = log();
        let mut rows = log.head_rows().unwrap();
        rows.pop();
        let mut checkpoints = log.checkpoints().unwrap();
        checkpoints[0].root = checkpoints[0].head.clone();
        let cases = [
            Doctored {
                head_rows: Some(rows.clone()),
                ..Doctored::default()
            },
            Doctored {
                checkpoints: Some(checkpoints.clone()),
                ..Doctored::default()
            },
            Doctored {
                checkpoints: Some(Vec::new()),
                ..Doctored::default()
            },
            Doctored {
                head_rows: Some(rows),
                checkpoints: Some(checkpoints),
                ..Doctored::default()
            },
        ];
        for doctored in cases {
            let report = verify_log(&View(&log, doctored), VerifyOptions::default()).unwrap();
            assert_eq!(codes(&report), vec![("S_PROJECTION_DRIFT", None)]);
        }
    }

    #[test]
    fn a_log_that_goes_back_is_an_error() {
        let log = log();
        let looping = Doctored {
            per_call: Some(2),
            restart: true,
            ..Doctored::default()
        };
        let r = verify_log(&View(&log, looping), VerifyOptions::default());
        assert_eq!(r.unwrap_err().code_str(), "S_IO");
    }

    #[test]
    fn findings_are_limited() {
        let log = log();
        let gap = Doctored {
            drop_seq: Some(4),
            ..Doctored::default()
        };
        let opts = VerifyOptions {
            max_findings: 1,
            source: Source::Store,
        };
        let report = verify_log(&View(&log, gap), opts).unwrap();
        assert_eq!(report.findings.len(), 1);
        assert!(report.truncated);
    }

    #[test]
    fn export_dirs_verify_as_their_vectors() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-vectors/store/exports");
        let mut cases: Vec<_> = std::fs::read_dir(&root)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        cases.sort();
        assert_eq!(cases.len(), 42);
        for case in cases {
            let spec = std::fs::read(case.join("case.json")).unwrap();
            let spec = evidence_core::json::parse(&spec).unwrap();
            let max_findings = spec.get("max_findings").map_or(1000, |n| match n {
                evidence_core::json::Value::Number(n) => *n as usize,
                other => panic!("max_findings {other:?}"),
            });
            let opts = VerifyOptions {
                max_findings,
                source: Source::Store,
            };
            let report = verify_export_dir(&case, opts).unwrap();
            assert_eq!(report.source, Source::Export, "whatever opts.source says");
            assert_eq!(
                evidence_core::canon::canonicalize(&report.to_value()),
                evidence_core::canon::canonicalize(spec.get("expect").unwrap()),
                "{}",
                case.display()
            );
        }
    }

    #[test]
    fn a_directory_without_a_manifest_is_not_an_export() {
        let tmp = tempfile::tempdir().unwrap();
        let r = verify_export_dir(tmp.path(), VerifyOptions::default());
        assert_eq!(r.unwrap_err().code_str(), "S_NOT_A_STORE");
        let r = verify_store(tmp.path(), VerifyOptions::default());
        assert_eq!(r.unwrap_err().code_str(), "S_NOT_A_STORE");
    }
}
