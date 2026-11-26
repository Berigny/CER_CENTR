//! Import: rebuild a live store from an export. See `docs/m1-store-spec.md`
//! section 7.
//!
//! An import writes nothing unless the export verifies, and leaves nothing at
//! the destination unless the rebuilt store verifies too. It builds in a
//! partial directory and renames it into place, so a crash at any point
//! leaves no destination directory (T6).

use std::fs;
use std::path::Path;

use evidence_core::canon;
use evidence_core::json::Value;
use evidence_log::{
    lane_of, merkle, parse_line, subject_of, verify_export, Body, Genesis, Source, StoreReport,
    Verifier, VerifyOptions,
};
use rusqlite::{Connection, OpenFlags, TransactionBehavior};

use crate::api::{CheckpointInfo, StoreError};
use crate::export::read_export_dir;
use crate::files::{create_partial, place, refuse_existing, sync_file, target_name};
use crate::intake::{NewAttestation, NewEntry, NewItem, NewObject};
use crate::sqlite::db::{self, Writer};
use crate::sqlite::{DB_FILE, FORMAT, SCHEMA_SQL, WRITER_PRAGMAS};
use crate::verify::verify_store;

/// Rebuild the store in `export_dir` as a new store directory `into`.
///
/// 1. `E_USAGE` if `into` has no file name; `S_EXISTS` if `into` exists.
///    Checked before anything is read.
/// 2. Read the export with [`read_export_dir`] and run
///    `evidence_log::verify_export` with the default options. If the report
///    is not ok, return it (`Ok`, source `export`): nothing is written.
/// 3. Create a new partial directory beside `into`, as
///    [`export`](crate::export::export) does (the first free
///    [`partial_path`](crate::export::partial_path)`(into, n)`; leftovers of
///    killed attempts are skipped and left alone), create `log.db` in it,
///    run `WRITER_PRAGMAS`, and in one transaction:
///    - run `SCHEMA_SQL`;
///    - insert every line's rows exactly as they are in the export: the
///      entry's canonical text, `entry_hash`, `received_at` and epoch; the
///      object's canonical bytes with its `at`, op, subject and lane, or the
///      attestation's canonical line with its fields;
///    - insert the projections recomputed from the lines: `heads` = the
///      heads fold (`Verifier::heads().rows()`), `checkpoints` = every
///      checkpoint with no `S_CHECKPOINT_INVALID` (`Verifier::checkpoints()`);
///    - insert `meta`: `format` = `store/0`, `log_id` = the report's log,
///      `name`, `store_signer` and `store_key` from the genesis line, and
///      `writer_epoch` = the highest `epoch` of any entry, so the next
///      writer appends at that + 1.
///
///    No `LOCK` file is written; the first `SqliteLog::open` creates it.
/// 4. Close the connection and run `verify_store` on the partial directory.
///    If that report is not ok, remove the partial directory and return the
///    report.
/// 5. Sync `log.db` and the partial directory, rename it to `into` (`S_EXISTS`
///    if `into` appeared meanwhile), sync `into`'s parent (directory syncs on
///    Unix only), and return the `verify_store` report (source `store`).
///
/// On any error after step 3 begins, the partial directory is removed and
/// nothing is left at `into`. The caller checks `report.ok()`: `Ok` with a
/// report that is not ok means nothing was written.
///
/// The export is read whole into memory. `S_NOT_A_STORE` if `export_dir`
/// holds no `manifest.json`; `S_IO` if it cannot be read.
pub fn import(export_dir: &Path, into: &Path) -> Result<StoreReport, StoreError> {
    target_name(into)?;
    refuse_existing(into)?;
    let export = read_export_dir(export_dir)?;
    let segments = export.segment_slices();
    let report = verify_export(&export.manifest, &segments, VerifyOptions::default());
    if !report.ok() {
        return Ok(report);
    }
    let log_id = report
        .log
        .clone()
        .ok_or_else(|| StoreError::Io("a verified export with no log id".into()))?;

    let partial = create_partial(into)?;
    let result = build(&partial, &log_id, &segments)
        .and_then(|()| verify_store(&partial, VerifyOptions::default()))
        .and_then(|report| {
            if report.ok() {
                settle_and_place(&partial, into)?;
            }
            Ok(report)
        });
    match result {
        Ok(report) if report.ok() => Ok(report),
        other => {
            let _ = fs::remove_dir_all(&partial);
            other
        }
    }
}

/// Step 3: `log.db` in `dir`, holding the lines of a verified export, its
/// recomputed projections and its `meta`, written in one transaction. The
/// connection is closed when this returns.
fn build(dir: &Path, log_id: &str, segments: &[&[u8]]) -> Result<(), StoreError> {
    // The projections, as the verifier folds them over the lines.
    let mut fold = Verifier::new(VerifyOptions {
        source: Source::Store,
        ..VerifyOptions::default()
    });
    for segment in segments {
        fold.push_segment(segment);
    }

    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
        | OpenFlags::SQLITE_OPEN_CREATE
        | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let mut conn = Connection::open_with_flags(dir.join(DB_FILE), flags)?;
    conn.execute_batch(WRITER_PRAGMAS)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute_batch(SCHEMA_SQL)?;
    let mut genesis = None;
    let mut writer_epoch = 0;
    {
        let mut writer = Writer::new(&tx)?;
        for line in segments.iter().flat_map(|s| lines(s)) {
            let new = new_entry(line)?;
            if new.entry.seq == 1 {
                genesis = Some(genesis_of(&new)?);
            }
            writer_epoch = writer_epoch.max(new.entry.epoch);
            writer.insert(&new)?;
        }
        for (subject, lane, id) in fold.heads().rows() {
            writer.insert_head(&subject, &lane, &id)?;
        }
        for (seq, id, checkpoint) in fold.checkpoints() {
            writer.insert_checkpoint(&CheckpointInfo {
                id: id.clone(),
                seq: *seq,
                size: checkpoint.size,
                root: checkpoint.root.clone(),
                head: checkpoint.head.clone(),
            })?;
        }
    }
    let genesis =
        genesis.ok_or_else(|| StoreError::Io("a verified export with no seq 1".into()))?;
    let writer_epoch = writer_epoch.to_string();
    let meta = [
        ("format", FORMAT),
        ("log_id", log_id),
        ("name", &genesis.name),
        ("store_signer", &genesis.signer),
        ("store_key", &genesis.key),
        ("writer_epoch", &writer_epoch),
    ];
    for (key, value) in meta {
        db::insert_meta(&tx, key, value)?;
    }
    tx.commit()?;
    conn.close().map_err(|(_, e)| StoreError::from(e))
}

/// The lines of a segment file that verified: every one ends in LF.
fn lines(segment: &[u8]) -> impl Iterator<Item = &[u8]> {
    segment
        .strip_suffix(b"\n")
        .unwrap_or(segment)
        .split(|b| *b == b'\n')
        .filter(move |_| !segment.is_empty())
}

/// The rows of one verified line, as a writer would have planned them.
fn new_entry(bytes: &[u8]) -> Result<NewEntry, StoreError> {
    let damaged = |what: &str| StoreError::Io(format!("a verified export has {what}"));
    let line = parse_line(bytes).map_err(|code| damaged(&format!("a line with {code}")))?;
    let text = String::from_utf8(bytes.to_vec()).map_err(|_| damaged("a line not in UTF-8"))?;
    let member = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).map(String::from);
    let item = match &line.body {
        Body::Object(v) => NewItem::Object(NewObject {
            id: line.entry.reference.clone(),
            at: member(v, "at").ok_or_else(|| damaged("an object with no at"))?,
            op: member(v, "op").ok_or_else(|| damaged("an object with no op"))?,
            subject: subject_of(v),
            lane: lane_of(v),
            prev: v
                .get("prev")
                .and_then(Value::as_array)
                .unwrap_or_default()
                .iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect(),
            bytes: canon::canonicalize(v),
        }),
        Body::Attestation(v) => {
            let field = |key: &str| {
                member(v, key).ok_or_else(|| damaged(&format!("an attestation with no {key}")))
            };
            NewItem::Attestation(NewAttestation {
                hash: line.entry.reference.clone(),
                object: field("object")?,
                purpose: field("purpose")?,
                signer: field("signer")?,
                key: field("key")?,
                at: field("at")?,
                line: canon::canonicalize(v),
            })
        }
    };
    let canonical = line.entry_canonical;
    Ok(NewEntry {
        hash: canon::sha256_hex(canonical.as_bytes()),
        leaf: merkle::leaf_hash(canonical.as_bytes()),
        canonical,
        entry: line.entry,
        line: text,
        item,
    })
}

/// The store's name, signer and key, from the genesis at seq 1.
fn genesis_of(new: &NewEntry) -> Result<Genesis, StoreError> {
    let not_genesis = || StoreError::Io("a verified export whose seq 1 is not a genesis".into());
    let NewItem::Object(object) = &new.item else {
        return Err(not_genesis());
    };
    let value = evidence_core::json::parse(object.bytes.as_bytes()).map_err(|_| not_genesis())?;
    let inline = value
        .get("content")
        .and_then(|c| c.get("inline"))
        .ok_or_else(not_genesis)?;
    let field = |key: &str| {
        inline
            .get(key)
            .and_then(Value::as_str)
            .ok_or_else(not_genesis)
    };
    Genesis::new(field("name")?, field("key")?).map_err(|_| not_genesis())
}

/// Step 5: make `log.db` durable, then [`place`] the partial directory at
/// `into` (sync it, `S_EXISTS` if `into` appeared meanwhile, rename, sync
/// the parent, and remove `into` again if that last sync fails).
///
/// `verify_store`'s read-only connection leaves `log.db-wal` and
/// `log.db-shm` behind (only a connection that can write removes them), so
/// one read through a writable connection, closed again, settles the store
/// to `log.db` alone first.
fn settle_and_place(partial: &Path, into: &Path) -> Result<(), StoreError> {
    let db = partial.join(DB_FILE);
    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(&db, flags)?;
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| {
        r.get::<_, i64>(0)
    })?;
    conn.close().map_err(|(_, e)| StoreError::from(e))?;
    sync_file(&db)?;
    place(partial, into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_split_at_each_lf() {
        let split = |s: &'static [u8]| lines(s).collect::<Vec<_>>();
        assert_eq!(split(b""), Vec::<&[u8]>::new());
        assert_eq!(split(b"a\n"), vec![&b"a"[..]]);
        assert_eq!(split(b"a\nb\n"), vec![&b"a"[..], &b"b"[..]]);
    }
}
