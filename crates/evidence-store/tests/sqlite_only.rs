//! What only the SQLite store has (`docs/m1-store-spec.md` section 11): the
//! lease and epochs (T7, library part), the append-only triggers and the
//! live-store findings (T8), opening checks (`S_NOT_A_STORE`, `S_FORMAT`),
//! reading a store the reader cannot write, `init` into an occupied path
//! (`S_EXISTS`) and after an init that was killed, and the export vectors
//! read as directories and imported (T10, library part).
//!
//! The rules every writer shares, and the export, verify and import round
//! trip of a live log, run against `SqliteLog` and `MemLog` in the contract
//! suite (`tests/contract.rs`). Data: the object, attestation, key and export
//! vectors in `test-vectors/`. Nothing is real.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use evidence_core::canon;
use evidence_core::json::Value;
use evidence_core::verify::PrevLookup;
use evidence_log::{parse_line, Source, StoreReport, VerifyOptions};
use evidence_store::sqlite::lease::{Lease, LOCK_FILE};
use evidence_store::sqlite::{DB_FILE, INIT_FILE, TRIGGERS};
use evidence_store::{
    import, verify_export_dir, verify_store, LogRead, LogWrite, ScriptClock, Signer, SqliteLog,
    SqliteReader, StoreError,
};
use rusqlite::Connection;

mod common;
use common::{
    case_json, case_name, error_code, export_cases, names, nth_line, read_vector, store_key, ts,
    vectors, ALL,
};

// ---------------------------------------------------------------------------
// Helpers

/// The bytes of an object vector.
fn object(case: &str) -> Vec<u8> {
    read_vector(&format!("objects/{case}/object.json"))
}

/// Line `n` (from 1) of an attestations file.
fn attestation(case: &str, n: usize) -> Vec<u8> {
    nth_line(
        &read_vector(&format!("objects/{case}/attestations.jsonl")),
        n,
    )
}

/// A clock with these readings, and a clone of it for a writer.
fn clock(readings: &[&str]) -> ScriptClock {
    ScriptClock::new(readings)
}

/// A new store in `dir`: the genesis, object vectors 001 and 002, the
/// author line of 001, and a checkpoint (seqs 1 to 7).
fn sample(dir: &Path) -> SqliteLog {
    let c = clock(&[
        "2026-11-26T00:00:00Z",
        "2026-11-26T00:00:01Z",
        "2026-11-26T00:00:02Z",
        "2026-11-26T00:00:03Z",
        "2026-11-26T00:00:04Z",
    ]);
    let mut log = SqliteLog::init(dir, "cer-demo", &store_key(), Box::new(c.clone())).unwrap();
    log.append(&object("001-create-inline")).unwrap();
    log.append(&object("002-update-with-prev")).unwrap();
    log.attest(&attestation("003-agent-assertion-approved", 3))
        .unwrap();
    log.checkpoint(&store_key()).unwrap();
    assert_eq!(c.remaining(), 0);
    assert_eq!(log.head().unwrap().seq, 7);
    log
}

/// A writable connection straight to `log.db`, as a tamperer would open it.
fn raw(dir: &Path) -> Connection {
    Connection::open(dir.join(DB_FILE)).unwrap()
}

fn codes(report: &StoreReport) -> Vec<(&'static str, Option<u64>)> {
    let findings = report.findings.iter();
    findings.map(|f| (f.code.as_str(), f.seq)).collect()
}

fn verify(dir: &Path) -> StoreReport {
    verify_store(dir, VerifyOptions::default()).unwrap()
}

fn lock_text(epoch: u64) -> String {
    format!("pid {}\nepoch {epoch}\n", std::process::id())
}

/// A signer with the store key whose lines are not attestations.
struct Garbage;

impl Signer for Garbage {
    fn public_key(&self) -> String {
        store_key().public_key()
    }

    fn sign(&self, _: &str, _: &str, _: &str, _: &str) -> Result<String, StoreError> {
        Ok("not an attestation".into())
    }
}

// ---------------------------------------------------------------------------
// T7: the lease and epochs

/// A second writer is refused with `S_LOCKED`, naming the holder, without
/// reading a clock or changing the store; it gets in once the first closes.
#[test]
fn t7_a_second_writer_is_locked_out() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("store");
    let first = sample(&dir);
    assert_eq!(Lease::holder(&dir).unwrap(), lock_text(1));
    assert_eq!(
        names(&dir),
        [LOCK_FILE, DB_FILE, "log.db-shm", "log.db-wal"]
    );

    // No readings: a refused open must not read the clock.
    let none = ScriptClock::default();
    match SqliteLog::open(&dir, Box::new(none.clone())) {
        Err(StoreError::Locked { holder }) => assert_eq!(holder, lock_text(1)),
        other => panic!("expected S_LOCKED, got {:?}", other.err()),
    }
    assert!(matches!(
        Lease::acquire(&dir),
        Err(StoreError::Locked { .. })
    ));
    assert_eq!(Lease::holder(&dir).unwrap(), lock_text(1), "untouched");

    drop(first);
    let second = SqliteLog::open(&dir, Box::new(none.clone())).unwrap();
    assert_eq!(second.epoch(), 2, "the refused open did not count");
    assert_eq!(Lease::holder(&dir).unwrap(), lock_text(2));
    let e = SqliteLog::open(&dir, Box::new(none)).err().unwrap();
    assert_eq!(e.code_str(), "S_LOCKED");
    assert_eq!(
        e.to_string(),
        format!(
            "S_LOCKED: another writer holds the store (pid {} epoch 2)",
            std::process::id()
        )
    );
    assert_eq!(second.dir(), dir.as_path());
}

/// Readers open, read and verify while a writer holds the lease; each call
/// sees the latest commit, and a snapshot sees one moment throughout.
#[test]
fn t7_readers_work_while_a_writer_holds_the_lease() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("store");
    let c = clock(&["2026-11-26T00:00:00Z", "2026-11-26T00:00:01Z"]);
    let mut writer = SqliteLog::init(&dir, "cer-demo", &store_key(), Box::new(c.clone())).unwrap();
    writer.append(&object("001-create-inline")).unwrap();

    let reader = SqliteReader::open(&dir).unwrap();
    assert_eq!(reader.dir(), dir.as_path());
    assert_eq!(reader.log_id(), writer.log_id());
    assert_eq!(reader.genesis(), writer.genesis());
    assert_eq!(reader.head().unwrap(), writer.head().unwrap());
    assert!(reader.has_append_only_triggers().unwrap());
    assert!(verify(&dir).ok());

    // Each call is its own read transaction: a commit shows at once.
    c.push(ts("2026-11-26T00:00:02Z"));
    writer.append(&object("002-update-with-prev")).unwrap();
    assert_eq!(reader.head().unwrap().seq, 4);

    // A snapshot does not see a commit made while it is open.
    c.push(ts("2026-11-26T00:00:03Z"));
    let (before, inside) = reader
        .snapshot(|r| {
            let before = r.head()?;
            writer.checkpoint(&store_key())?;
            Ok((before, r.lines(1, 100)?))
        })
        .unwrap();
    assert_eq!(before.seq, 4);
    assert_eq!(inside.len(), 4, "the snapshot is of one moment");
    assert_eq!(reader.head().unwrap().seq, 6);
    assert_eq!(reader.lines(1, 100).unwrap(), writer.lines(1, 100).unwrap());
    assert_eq!(reader.checkpoints().unwrap(), writer.checkpoints().unwrap());
    assert_eq!(reader.head_rows().unwrap(), writer.head_rows().unwrap());
    assert_eq!(
        reader.leaf_hashes(6).unwrap(),
        writer.leaf_hashes(6).unwrap()
    );
    assert_eq!(error_code(reader.leaf_hashes(7)), "E_USAGE");

    // A snapshot ends with an error from inside it, and the reader goes on.
    let r: Result<(), StoreError> = reader.snapshot(|_| Err(StoreError::Io("stop".into())));
    assert_eq!(r.unwrap_err(), StoreError::Io("stop".into()));
    assert_eq!(reader.head().unwrap().seq, 6);

    // Prev lookups: the at of a held object, or None.
    let id_001 = writer.lines(3, 1).unwrap()[0].1.clone();
    let line = parse_line(id_001.as_bytes()).unwrap();
    let held = Some(ts("2026-11-20T05:12:44Z"));
    assert_eq!(reader.prev_at(&line.entry.reference), held);
    assert_eq!(writer.prev_at(&line.entry.reference), held);
    let absent = canon::sha256_hex(b"absent");
    assert_eq!(reader.prev_at(&absent), None);
    assert_eq!(writer.prev_at(&absent), None);
    assert_eq!(reader.object(&absent).unwrap(), None);
    assert_eq!(
        reader.object(&line.entry.reference).unwrap(),
        writer.object(&line.entry.reference).unwrap()
    );
    let attestations = reader.attestations(writer.log_id()).unwrap();
    assert_eq!(attestations, writer.attestations(writer.log_id()).unwrap());
    assert_eq!(attestations.len(), 1);
    assert!(verify(&dir).ok());
    assert_eq!(c.remaining(), 0);
}

/// Every open adds one to the epoch, which every entry it appends carries,
/// and which `meta.writer_epoch` and the lease text record.
#[test]
fn t7_the_epoch_increments_on_each_open() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("store");
    let c = clock(&["2026-11-26T00:00:00Z", "2026-11-26T00:00:01Z"]);
    let mut log = SqliteLog::init(&dir, "cer-demo", &store_key(), Box::new(c.clone())).unwrap();
    log.append(&object("001-create-inline")).unwrap();
    let appends = [
        ("002-update-with-prev", "2026-11-26T00:00:02Z"),
        ("004-external-content", "2026-11-26T00:00:03Z"),
        ("005-profile-extension", "2026-11-26T00:00:04Z"),
    ];
    for (n, (case, at)) in (2..).zip(appends) {
        drop(log);
        log = SqliteLog::open(&dir, Box::new(c.clone())).unwrap();
        assert_eq!(log.epoch(), n);
        assert_eq!(Lease::holder(&dir).unwrap(), lock_text(n));
        c.push(ts(at));
        log.append(&object(case)).unwrap();
    }
    let epochs: Vec<u64> = log
        .lines(1, 100)
        .unwrap()
        .iter()
        .map(|(_, l)| parse_line(l.as_bytes()).unwrap().entry.epoch)
        .collect();
    assert_eq!(epochs, [1, 1, 1, 2, 3, 4]);
    drop(log);
    let stored: String = raw(&dir)
        .query_row("SELECT v FROM meta WHERE k = 'writer_epoch'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(stored, "4");
    assert!(verify(&dir).ok());
    assert_eq!(c.remaining(), 0);
}

// ---------------------------------------------------------------------------
// T8: append-only, and what verify_store finds

/// The message of an SQLite error.
fn message(e: rusqlite::Error) -> String {
    match e {
        rusqlite::Error::SqliteFailure(_, Some(message)) => message,
        other => panic!("expected an SQLite failure with a message, got {other:?}"),
    }
}

/// Raw UPDATE and DELETE on the log tables abort with `S_APPEND_ONLY`.
#[test]
fn t8_the_log_tables_are_append_only() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("store");
    drop(sample(&dir));
    let conn = raw(&dir);
    for sql in [
        "UPDATE entries SET received_at = received_at WHERE seq = 3",
        "UPDATE entries SET epoch = 2",
        "DELETE FROM entries WHERE seq = 7",
        "DELETE FROM entries",
        "UPDATE objects SET bytes = bytes",
        "DELETE FROM objects WHERE seq = 4",
        "UPDATE attestations SET line = line",
        "DELETE FROM attestations",
    ] {
        let e = conn.execute(sql, []).unwrap_err();
        assert_eq!(message(e), "S_APPEND_ONLY", "{sql}");
    }
    drop(conn);
    let report = verify(&dir);
    assert!(report.ok(), "{:?}", report.findings);
    assert_eq!(report.checked.entries, 7);
}

/// With a trigger dropped, an edit to one object's bytes is found at its seq
/// (`S_OBJECT_INVALID`, core `E_ID_MISMATCH`), and so is the missing
/// trigger. The edited object is no longer held, so the heads table, which
/// still names it, drifts from the fold too.
#[test]
fn t8_an_edit_without_triggers_is_found() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("store");
    drop(sample(&dir));
    let conn = raw(&dir);
    conn.execute_batch("DROP TRIGGER objects_no_update")
        .unwrap();
    let changed = conn
        .execute(
            "UPDATE objects SET bytes = replace(bytes, '\"at\":\"2026-11-21T01:00:00Z\"', \
             '\"at\":\"2026-11-21T01:00:01Z\"') WHERE seq = 4",
            [],
        )
        .unwrap();
    assert_eq!(changed, 1);
    drop(conn);
    assert!(!SqliteReader::open(&dir)
        .unwrap()
        .has_append_only_triggers()
        .unwrap());
    let report = verify(&dir);
    assert_eq!(
        codes(&report),
        [
            ("S_PROJECTION_DRIFT", None),
            ("S_TRIGGERS_MISSING", None),
            ("S_OBJECT_INVALID", Some(4)),
        ]
    );
    let finding = &report.findings[2];
    assert_eq!(finding.core.map(|c| c.as_str()), Some("E_ID_MISMATCH"));
    assert_eq!(report.source, Source::Store);
}

/// A trigger recreated with other SQL, or on another table, is missing too;
/// an extra trigger does not matter.
#[test]
fn t8_triggers_must_be_exactly_the_schemas() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("store");
    drop(sample(&dir));
    let conn = raw(&dir);
    conn.execute_batch("CREATE TRIGGER heads_extra BEFORE DELETE ON meta BEGIN SELECT 1; END;")
        .unwrap();
    assert!(verify(&dir).ok(), "an extra trigger");
    let cases = [
        "CREATE TRIGGER entries_no_delete BEFORE DELETE ON entries BEGIN SELECT RAISE(ABORT, 'no'); END",
        "CREATE TRIGGER entries_no_delete BEFORE DELETE ON heads BEGIN SELECT RAISE(ABORT, 'S_APPEND_ONLY'); END",
    ];
    for sql in cases {
        conn.execute_batch("DROP TRIGGER entries_no_delete")
            .unwrap();
        conn.execute_batch(sql).unwrap();
        assert_eq!(
            codes(&verify(&dir)),
            [("S_TRIGGERS_MISSING", None)],
            "{sql}"
        );
    }
    // Every one of the six counts.
    let schema = "CREATE TRIGGER entries_no_delete BEFORE DELETE ON entries BEGIN SELECT RAISE(ABORT, 'S_APPEND_ONLY'); END";
    conn.execute_batch("DROP TRIGGER entries_no_delete")
        .unwrap();
    conn.execute_batch(schema).unwrap();
    assert!(verify(&dir).ok(), "restored");
    for name in TRIGGERS {
        let sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'trigger' AND name = ?1",
                [name],
                |r| r.get(0),
            )
            .unwrap();
        conn.execute_batch(&format!("DROP TRIGGER {name}")).unwrap();
        assert_eq!(
            codes(&verify(&dir)),
            [("S_TRIGGERS_MISSING", None)],
            "{name}"
        );
        conn.execute_batch(&sql).unwrap();
    }
    assert!(verify(&dir).ok());
}

/// The projections and `meta.log_id` are checked against the lines, each
/// with at most one finding.
#[test]
fn t8_projection_drift_and_meta_mismatch() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("store");
    drop(sample(&dir));
    let conn = raw(&dir);
    let drift = [("S_PROJECTION_DRIFT", None)];

    // A heads row deleted, then restored.
    let row: (String, String, String, String, String) = conn
        .query_row(
            "SELECT system, type, key, lane, id FROM heads WHERE system = 'rec-registry'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap();
    conn.execute("DELETE FROM heads WHERE id = ?1", [&row.4])
        .unwrap();
    assert_eq!(codes(&verify(&dir)), drift);
    conn.execute(
        "INSERT INTO heads VALUES (?1, ?2, ?3, ?4, ?5)",
        [&row.0, &row.1, &row.2, &row.3, &row.4],
    )
    .unwrap();
    assert!(verify(&dir).ok());

    // A heads row too many: the prev that 002 replaced.
    let id_001: String = conn
        .query_row("SELECT id FROM objects WHERE seq = 3", [], |r| r.get(0))
        .unwrap();
    conn.execute(
        "INSERT INTO heads VALUES (?1, ?2, ?3, ?4, ?5)",
        [&row.0, &row.1, &row.2, &row.3, &id_001],
    )
    .unwrap();
    assert_eq!(codes(&verify(&dir)), drift);
    conn.execute("DELETE FROM heads WHERE id = ?1", [&id_001])
        .unwrap();

    // A checkpoints row changed, then deleted: one finding each time.
    conn.execute("UPDATE checkpoints SET size = size + 1", [])
        .unwrap();
    assert_eq!(codes(&verify(&dir)), drift);
    conn.execute("DELETE FROM checkpoints", []).unwrap();
    assert_eq!(codes(&verify(&dir)), drift);
    // Both tables at once: still one finding.
    conn.execute("DELETE FROM heads", []).unwrap();
    assert_eq!(codes(&verify(&dir)), drift);
    drop(conn);

    // meta.log_id: another hash opens (it is taken as stored) and is found.
    let tmp2 = tempfile::tempdir().unwrap();
    let dir2 = tmp2.path().join("store");
    drop(sample(&dir2));
    let other = canon::sha256_hex(b"another log");
    raw(&dir2)
        .execute("UPDATE meta SET v = ?1 WHERE k = 'log_id'", [&other])
        .unwrap();
    assert_eq!(SqliteReader::open(&dir2).unwrap().log_id(), other);
    assert_eq!(codes(&verify(&dir2)), [("S_META_MISMATCH", None)]);
}

/// Damage the triggers would have stopped: a deleted entry is a gap in the
/// lines, and an entry whose object row is gone is an invalid line. Both are
/// findings, never errors.
#[test]
fn t8_missing_rows_are_findings() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("store");
    drop(sample(&dir));
    let conn = raw(&dir);
    // Foreign keys off, as a tamperer's connection might have them.
    conn.execute_batch("PRAGMA foreign_keys = OFF; DROP TRIGGER objects_no_delete")
        .unwrap();
    conn.execute("DELETE FROM objects WHERE seq = 4", [])
        .unwrap();
    let reader = SqliteReader::open(&dir).unwrap();
    let entry: String = conn
        .query_row("SELECT entry FROM entries WHERE seq = 4", [], |r| r.get(0))
        .unwrap();
    let line = reader.lines(4, 1).unwrap();
    assert_eq!(line, [(4, format!("{{\"entry\":{entry}}}"))]);
    let report = verify(&dir);
    let found = codes(&report);
    assert!(found.contains(&("S_LINE_INVALID", None)), "{found:?}");
    assert!(found.contains(&("S_TRIGGERS_MISSING", None)), "{found:?}");

    conn.execute_batch("DROP TRIGGER entries_no_delete")
        .unwrap();
    conn.execute("DELETE FROM entries WHERE seq = 5", [])
        .unwrap();
    let seqs: Vec<u64> = reader.lines(1, 100).unwrap().iter().map(|l| l.0).collect();
    assert_eq!(seqs, [1, 2, 3, 4, 6, 7]);
    let found = codes(&verify(&dir));
    assert!(found.contains(&("S_SEQ_GAP", Some(6))), "{found:?}");
    // What a proof or an export needs is an error, not a finding.
    assert_eq!(error_code(reader.leaf_hashes(7)), "S_IO");
    assert_eq!(reader.leaf_hashes(4).unwrap().len(), 4);
}

// ---------------------------------------------------------------------------
// Opening

/// Whether `dir` holds a lease file.
fn has_lock(dir: &Path) -> bool {
    dir.join(LOCK_FILE).exists()
}

/// Open `dir` as a writer (with no clock readings) and as a reader; both
/// must fail with `code`, and neither may create `LOCK`.
#[track_caller]
fn refused(dir: &Path, code: &str) {
    let had_lock = has_lock(dir);
    let writer = SqliteLog::open(dir, Box::new(ScriptClock::default()));
    assert_eq!(error_code(writer), code, "writer");
    assert_eq!(error_code(SqliteReader::open(dir)), code, "reader");
    assert_eq!(
        error_code(verify_store(dir, VerifyOptions::default())),
        code
    );
    assert_eq!(has_lock(dir), had_lock, "no lease file is created");
}

/// No `log.db`, or one that is not a store/0 database.
#[test]
fn not_a_store() {
    let tmp = tempfile::tempdir().unwrap();
    let absent = tmp.path().join("absent");
    refused(&absent, "S_NOT_A_STORE");
    assert!(!absent.exists());
    let empty = tmp.path().join("empty");
    fs::create_dir(&empty).unwrap();
    refused(&empty, "S_NOT_A_STORE");
    assert!(names(&empty).is_empty(), "nothing is created");
    let file = tmp.path().join("file");
    fs::write(&file, b"x").unwrap();
    refused(&file, "S_NOT_A_STORE");

    // log.db is a directory, not a database, or another application's.
    let dir = tmp.path().join("other");
    fs::create_dir_all(dir.join(DB_FILE)).unwrap();
    refused(&dir, "S_NOT_A_STORE");
    fs::remove_dir(dir.join(DB_FILE)).unwrap();
    fs::write(dir.join(DB_FILE), vec![b'x'; 4096]).unwrap();
    refused(&dir, "S_NOT_A_STORE");
    fs::write(dir.join(DB_FILE), b"").unwrap();
    refused(&dir, "S_NOT_A_STORE");
    fs::remove_file(dir.join(DB_FILE)).unwrap();
    raw(&dir)
        .execute_batch("CREATE TABLE meta (k TEXT PRIMARY KEY, v TEXT NOT NULL);")
        .unwrap();
    refused(&dir, "S_NOT_A_STORE");
    // The right header, and a meta table of another shape or with values
    // that are not text.
    raw(&dir)
        .execute_batch(
            "PRAGMA application_id = 1163281456; PRAGMA user_version = 1; \
             DROP TABLE meta; CREATE TABLE meta (key TEXT, value TEXT);",
        )
        .unwrap();
    refused(&dir, "S_NOT_A_STORE");
    raw(&dir)
        .execute_batch(
            "DROP TABLE meta; CREATE TABLE meta (k, v); \
             INSERT INTO meta VALUES ('format', 0);",
        )
        .unwrap();
    refused(&dir, "S_NOT_A_STORE");
    // The right header but no meta table.
    fs::remove_file(dir.join(DB_FILE)).unwrap();
    raw(&dir)
        .execute_batch(
            "PRAGMA application_id = 1163281456; PRAGMA user_version = 1; CREATE TABLE t (x);",
        )
        .unwrap();
    refused(&dir, "S_NOT_A_STORE");
    // Another application id with another version: the id is checked first.
    raw(&dir)
        .execute_batch("PRAGMA application_id = 7; PRAGMA user_version = 2;")
        .unwrap();
    refused(&dir, "S_NOT_A_STORE");
}

/// A `meta` row that is missing or invalid is `S_NOT_A_STORE`; one that
/// names another format or schema version is `S_FORMAT`.
#[test]
fn meta_is_checked_on_open() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("store");
    drop(sample(&dir));
    let original: Vec<(String, String)> = {
        let conn = raw(&dir);
        let mut stmt = conn.prepare("SELECT k, v FROM meta ORDER BY k").unwrap();
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        rows.map(Result::unwrap).collect()
    };
    let keys: Vec<&str> = original.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(
        keys,
        [
            "format",
            "log_id",
            "name",
            "store_key",
            "store_signer",
            "writer_epoch"
        ]
    );
    let set = |key: &str, value: Option<&str>| {
        let conn = raw(&dir);
        conn.execute("DELETE FROM meta WHERE k = ?1", [key])
            .unwrap();
        if let Some(value) = value {
            conn.execute("INSERT INTO meta VALUES (?1, ?2)", [key, value])
                .unwrap();
        }
    };
    let restore = |key: &str| {
        let value = &original.iter().find(|(k, _)| k == key).unwrap().1;
        set(key, Some(value));
    };
    let other_key = common::seed("assessor-01").public_key();
    let cases: [(&str, Option<&str>, &str); 15] = [
        ("format", None, "S_NOT_A_STORE"),
        ("format", Some("store/1"), "S_FORMAT"),
        ("log_id", None, "S_NOT_A_STORE"),
        ("name", None, "S_NOT_A_STORE"),
        ("name", Some("Cer Demo"), "S_NOT_A_STORE"),
        ("name", Some("other"), "S_NOT_A_STORE"),
        ("store_key", None, "S_NOT_A_STORE"),
        ("store_key", Some("ed25519:not-a-key"), "S_NOT_A_STORE"),
        ("store_signer", None, "S_NOT_A_STORE"),
        (
            "store_signer",
            Some("service:evidence-store/other"),
            "S_NOT_A_STORE",
        ),
        ("writer_epoch", None, "S_NOT_A_STORE"),
        ("writer_epoch", Some("0"), "S_NOT_A_STORE"),
        ("writer_epoch", Some("+1"), "S_NOT_A_STORE"),
        ("writer_epoch", Some("one"), "S_NOT_A_STORE"),
        ("writer_epoch", Some("9007199254740992"), "S_NOT_A_STORE"),
    ];
    for (key, value, code) in cases {
        set(key, value);
        refused(&dir, code);
        restore(key);
        assert!(verify(&dir).ok(), "{key} restored");
    }
    // Another key, consistent with its signer and name: it opens (only the
    // genesis line would tell), and a checkpoint with the real key is
    // refused before the clock is read.
    set("store_key", Some(&other_key));
    let mut log = SqliteLog::open(&dir, Box::new(ScriptClock::default())).unwrap();
    assert_eq!(log.genesis().key, other_key);
    assert_eq!(error_code(log.checkpoint(&store_key())), "S_STORE_KEY");
    drop(log);
    restore("store_key");

    // The schema version.
    raw(&dir).execute_batch("PRAGMA user_version = 2").unwrap();
    refused(&dir, "S_FORMAT");
    raw(&dir).execute_batch("PRAGMA user_version = 1").unwrap();
    raw(&dir).execute_batch("DROP TABLE meta").unwrap();
    refused(&dir, "S_NOT_A_STORE");
}

// ---------------------------------------------------------------------------
// Reading a store the reader cannot write

/// Everything a reader is used for, against `dir`: the head, a line, an
/// object, proofs, `verify_store` and an export into `out`, which must
/// equal `want`. Returns the reader.
#[cfg(unix)]
#[track_caller]
fn read_everything(dir: &Path, out: &Path, want: &evidence_store::ExportFiles) -> SqliteReader {
    let reader = SqliteReader::open(dir).unwrap();
    assert_eq!(reader.head().unwrap().seq, 7);
    assert_eq!(reader.lines(3, 1).unwrap().len(), 1);
    let id = parse_line(reader.lines(3, 1).unwrap()[0].1.as_bytes())
        .unwrap()
        .entry
        .reference;
    assert!(reader.object(&id).unwrap().is_some());
    assert!(reader.prev_at(&id).is_some());
    assert!(evidence_store::inclusion(&reader, 3, 7).unwrap().verify());
    assert!(reader.has_append_only_triggers().unwrap());
    assert_eq!(&evidence_store::export_files(&reader, 3).unwrap(), want);
    evidence_store::export(&reader, out, 3).unwrap();
    assert!(verify_export_dir(out, VerifyOptions::default())
        .unwrap()
        .ok());
    assert!(verify(dir).ok());
    reader
}

/// A store that SQLite cannot open the WAL of, for want of write access
/// (`SQLITE_CANTOPEN` at the first read), is read as an immutable file when
/// it has no WAL content, and creates nothing. Here `log.db-wal` is a
/// symlink into a directory that does not exist, which SQLite cannot create
/// even for root; the directory permission case follows, where the user is
/// not root.
#[cfg(unix)]
#[test]
fn a_store_that_cannot_be_written_is_read_as_an_immutable_file() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("store");
    drop(sample(&dir));
    let want = evidence_store::export_files(&SqliteReader::open(&dir).unwrap(), 3).unwrap();
    // Settle what that reader left, as the next writer does.
    drop(SqliteLog::open(&dir, Box::new(ScriptClock::default())).unwrap());
    assert_eq!(names(&dir), [LOCK_FILE, DB_FILE]);

    let wal = dir.join("log.db-wal");
    std::os::unix::fs::symlink(tmp.path().join("absent/wal"), &wal).unwrap();
    let before = names(&dir);
    let reader = read_everything(&dir, &tmp.path().join("e1"), &want);
    assert!(reader.is_immutable());
    assert_eq!(names(&dir), before, "nothing created");
    // A writer cannot open it either.
    let writer = SqliteLog::open(&dir, Box::new(ScriptClock::default()));
    assert_eq!(error_code(writer), "S_IO");

    // A writer that opens the store rewrites LOCK before it changes
    // log.db: every read after that is S_IO, inside a snapshot too, and a
    // new reader reads again.
    let lock = fs::read(dir.join(LOCK_FILE)).unwrap();
    fs::write(dir.join(LOCK_FILE), lock_text(99)).unwrap();
    assert_eq!(error_code(reader.head()), "S_IO");
    assert_eq!(error_code(reader.snapshot(|r| r.head())), "S_IO");
    assert!(reader.prev_at("sha256:0").is_none());
    let reader = SqliteReader::open(&dir).unwrap();
    assert!(reader.is_immutable());
    assert_eq!(reader.head().unwrap().seq, 7);
    fs::remove_file(dir.join(LOCK_FILE)).unwrap();
    assert_eq!(error_code(reader.head()), "S_IO", "LOCK removed");

    // A store with no LOCK (an import no writer has opened yet): a LOCK
    // that appears is a writer.
    let reader = SqliteReader::open(&dir).unwrap();
    assert!(reader.is_immutable());
    assert_eq!(reader.head().unwrap().seq, 7);
    fs::write(dir.join(LOCK_FILE), b"").unwrap();
    assert_eq!(error_code(reader.head()), "S_IO", "LOCK created");
    fs::write(dir.join(LOCK_FILE), &lock).unwrap();

    // WAL content (a writer open, or one that did not close): not read as
    // a file, since log.db alone may not hold the log. Here SQLite opens
    // the WAL but cannot create log.db-shm.
    fs::remove_file(&wal).unwrap();
    let shm = dir.join("log.db-shm");
    std::os::unix::fs::symlink(tmp.path().join("absent/shm"), &shm).unwrap();
    fs::write(&wal, [0x5a; 32]).unwrap();
    assert_eq!(error_code(SqliteReader::open(&dir)), "S_IO");
    assert_eq!(
        error_code(verify_store(&dir, VerifyOptions::default())),
        "S_IO"
    );
    // An empty WAL holds nothing: read as a file.
    fs::write(&wal, b"").unwrap();
    let reader = read_everything(&dir, &tmp.path().join("e2"), &want);
    assert!(reader.is_immutable());
    // A WAL with content that appears later is a writer.
    fs::write(&wal, [0x5a; 32]).unwrap();
    assert_eq!(error_code(reader.head()), "S_IO", "WAL content");
    fs::remove_file(&wal).unwrap();
    fs::remove_file(&shm).unwrap();
    assert!(!SqliteReader::open(&dir).unwrap().is_immutable());
}

/// The usual case: a store directory this user cannot write. As root the
/// permission does not hold, and the reader reads as usual.
#[cfg(unix)]
#[test]
fn a_store_in_a_directory_this_user_cannot_write_is_read() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("store");
    drop(sample(&dir));
    let want = evidence_store::export_files(&SqliteReader::open(&dir).unwrap(), 3).unwrap();
    drop(SqliteLog::open(&dir, Box::new(ScriptClock::default())).unwrap());
    let before = names(&dir);
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).unwrap();
    let writable = fs::write(dir.join("probe"), b"").is_ok();
    let _ = fs::remove_file(dir.join("probe"));
    let reader = read_everything(&dir, &tmp.path().join("export"), &want);
    if !writable {
        assert!(reader.is_immutable());
        assert_eq!(names(&dir), before, "nothing created");
    }
    drop(reader);
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
}

// ---------------------------------------------------------------------------
// Creating

/// `init` takes an absent path (creating its parents) or an empty
/// directory, and refuses anything else with `S_EXISTS`, reading no clock
/// and leaving what is there as it was.
#[test]
fn init_needs_an_absent_path_or_an_empty_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let none = || Box::new(ScriptClock::default());
    let one = || Box::new(clock(&["2026-11-26T00:00:00Z"]));

    let nested = tmp.path().join("a/b/store");
    drop(SqliteLog::init(&nested, "cer-demo", &store_key(), one()).unwrap());
    assert_eq!(
        names(&nested),
        [LOCK_FILE, DB_FILE],
        "no WAL files once closed"
    );
    assert_eq!(Lease::holder(&nested).unwrap(), lock_text(1));
    let e = SqliteLog::init(&nested, "cer-demo", &store_key(), none());
    assert_eq!(error_code(e), "S_EXISTS", "an existing store");

    let empty = tmp.path().join("empty");
    fs::create_dir(&empty).unwrap();
    drop(SqliteLog::init(&empty, "cer-demo", &store_key(), one()).unwrap());
    assert!(verify(&empty).ok());

    let full = tmp.path().join("full");
    fs::create_dir(&full).unwrap();
    fs::write(full.join("notes.txt"), b"keep").unwrap();
    let e = SqliteLog::init(&full, "cer-demo", &store_key(), none());
    assert_eq!(error_code(e), "S_EXISTS");
    assert_eq!(names(&full), ["notes.txt"]);

    let file = tmp.path().join("file");
    fs::write(&file, b"keep").unwrap();
    let e = SqliteLog::init(&file, "cer-demo", &store_key(), none());
    assert_eq!(error_code(e), "S_EXISTS");
    assert_eq!(fs::read(&file).unwrap(), b"keep");

    // E_USAGE comes first, and creates nothing.
    let unused = tmp.path().join("unused");
    let e = SqliteLog::init(&unused, "Not A Name", &store_key(), none());
    assert_eq!(error_code(e), "E_USAGE");
    assert!(!unused.exists());
    let e = SqliteLog::init(&file, "Not A Name", &store_key(), none());
    assert_eq!(error_code(e), "E_USAGE");
}

/// A failed `init` removes what it created: the directory if it made it,
/// otherwise the files it put in the empty directory it was given.
#[test]
fn a_failed_init_leaves_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let absent = tmp.path().join("a/store");
    let c = clock(&["2026-11-26T00:00:00Z"]);
    let e = SqliteLog::init(&absent, "cer-demo", &Garbage, Box::new(c.clone()));
    assert_eq!(error_code(e), "S_STORE_KEY");
    assert_eq!(c.remaining(), 0, "one reading");
    assert!(!absent.exists());
    assert_eq!(names(&tmp.path().join("a")), Vec::<String>::new());

    let empty = tmp.path().join("empty");
    fs::create_dir(&empty).unwrap();
    c.push(ts("2026-11-26T00:00:00Z"));
    let e = SqliteLog::init(&empty, "cer-demo", &Garbage, Box::new(c.clone()));
    assert_eq!(error_code(e), "S_STORE_KEY");
    assert!(names(&empty).is_empty());

    // The directory can be used again.
    c.push(ts("2026-11-26T00:00:00Z"));
    drop(SqliteLog::init(&empty, "cer-demo", &store_key(), Box::new(c)).unwrap());
    assert!(verify(&empty).ok());
}

/// What an init that was killed leaves (`LOCK`, and `log.db.init` with
/// SQLite's files beside it, whatever they hold) is not a store, and the
/// next `init` takes the directory: it removes the leftovers under the
/// lease and creates the store.
#[test]
fn init_takes_what_a_killed_init_left() {
    let tmp = tempfile::tempdir().unwrap();
    let garbage = vec![0x5a; 4096];
    let leftovers: [&[(&str, &[u8])]; 5] = [
        &[(LOCK_FILE, b"")],
        &[(LOCK_FILE, b"pid 1\n")],
        &[
            (LOCK_FILE, b""),
            (INIT_FILE, &garbage),
            ("log.db.init-journal", &garbage),
        ],
        &[
            (LOCK_FILE, b""),
            (INIT_FILE, b""),
            ("log.db.init-wal", &garbage),
            ("log.db.init-shm", &garbage),
        ],
        &[(INIT_FILE, &garbage)],
    ];
    for (i, files) in leftovers.iter().enumerate() {
        let dir = tmp.path().join(format!("left-{i}"));
        fs::create_dir(&dir).unwrap();
        for (name, bytes) in *files {
            fs::write(dir.join(name), bytes).unwrap();
        }
        refused(&dir, "S_NOT_A_STORE");
        let c = clock(&["2026-11-26T00:00:00Z"]);
        let log = SqliteLog::init(&dir, "cer-demo", &store_key(), Box::new(c.clone())).unwrap();
        assert_eq!(c.remaining(), 0);
        assert_eq!(log.head().unwrap().seq, 2);
        drop(log);
        assert_eq!(names(&dir), [LOCK_FILE, DB_FILE], "{i}");
        assert_eq!(Lease::holder(&dir).unwrap(), lock_text(1));
        assert!(verify(&dir).ok(), "{i}");
    }

    // A failed init removes the leftovers it took over, but not a LOCK it
    // did not create.
    let dir = tmp.path().join("failed");
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join(LOCK_FILE), b"").unwrap();
    fs::write(dir.join(INIT_FILE), &garbage).unwrap();
    let c = clock(&["2026-11-26T00:00:00Z"]);
    assert_eq!(
        error_code(SqliteLog::init(&dir, "cer-demo", &Garbage, Box::new(c))),
        "S_STORE_KEY"
    );
    assert_eq!(names(&dir), [LOCK_FILE]);

    // While another init holds the lease: S_LOCKED, no clock reading, and
    // its files are left as they are.
    let lease = Lease::acquire(&dir).unwrap();
    fs::write(dir.join(INIT_FILE), &garbage).unwrap();
    let e = SqliteLog::init(
        &dir,
        "cer-demo",
        &store_key(),
        Box::new(ScriptClock::default()),
    );
    assert_eq!(error_code(e), "S_LOCKED");
    assert_eq!(names(&dir), [LOCK_FILE, INIT_FILE]);
    assert_eq!(fs::read(dir.join(INIT_FILE)).unwrap(), garbage);
    drop(lease);

    // Leftovers beside anything else: S_EXISTS, and nothing is touched.
    fs::write(dir.join("notes.txt"), b"keep").unwrap();
    let e = SqliteLog::init(
        &dir,
        "cer-demo",
        &store_key(),
        Box::new(ScriptClock::default()),
    );
    assert_eq!(error_code(e), "S_EXISTS");
    assert_eq!(names(&dir), [LOCK_FILE, INIT_FILE, "notes.txt"]);
}

/// The variable that makes [`killed_init_child`] create a store.
const INIT_CHILD: &str = "EVIDENCE_STORE_TEST_INIT_CHILD";

/// Not a test of its own: [`init_killed_at_any_moment`] runs this test
/// binary with only this test, and `INIT_CHILD` set to a directory, and
/// kills it while it creates a store there. Without the variable it does
/// nothing.
#[test]
fn killed_init_child() {
    let Some(dir) = std::env::var_os(INIT_CHILD) else {
        return;
    };
    let c = clock(&["2026-11-26T00:00:00Z"]);
    drop(SqliteLog::init(Path::new(&dir), "cer-demo", &store_key(), Box::new(c)).unwrap());
}

/// An init killed at any moment (here, from 0 to 20 ms after its process
/// starts) leaves either a complete store, which verifies and opens, or
/// nothing but what the next `init` takes: never a directory that is
/// neither.
#[test]
fn init_killed_at_any_moment() {
    let tmp = tempfile::tempdir().unwrap();
    let exe = std::env::current_exe().unwrap();
    let leftovers = [
        LOCK_FILE,
        INIT_FILE,
        "log.db.init-journal",
        "log.db.init-wal",
        "log.db.init-shm",
    ];
    let (mut stores, mut taken) = (0, 0);
    for run in 0..40_u64 {
        let dir = tmp.path().join(format!("run-{run}")).join("store");
        let mut child = Command::new(&exe)
            .args(["killed_init_child", "--exact", "--test-threads=1", "-q"])
            .env(INIT_CHILD, &dir)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        std::thread::sleep(Duration::from_micros(run * 500));
        let _ = child.kill();
        child.wait().unwrap();
        if dir.join(DB_FILE).exists() {
            assert!(verify(&dir).ok(), "run {run}: {:?}", names(&dir));
            drop(SqliteLog::open(&dir, Box::new(ScriptClock::default())).unwrap());
            stores += 1;
            continue;
        }
        if dir.exists() {
            let left = names(&dir);
            assert!(
                left.iter().all(|name| leftovers.contains(&name.as_str())),
                "run {run}: {left:?}"
            );
        }
        let c = clock(&["2026-11-26T00:00:00Z"]);
        drop(SqliteLog::init(&dir, "cer-demo", &store_key(), Box::new(c)).unwrap());
        assert!(verify(&dir).ok(), "run {run}");
        taken += 1;
    }
    assert_eq!(stores + taken, 40);
}

/// Two writers on one store cannot both create it: the second `init` into
/// a directory that the first has just made a store is `S_EXISTS`, and the
/// first's files stay.
#[test]
fn init_never_removes_another_store() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("store");
    let log = sample(&dir);
    let before = names(&dir);
    let e = SqliteLog::init(
        &dir,
        "cer-demo",
        &store_key(),
        Box::new(ScriptClock::default()),
    );
    assert_eq!(error_code(e), "S_EXISTS");
    assert_eq!(names(&dir), before);
    drop(log);
    assert!(verify(&dir).ok());
}

// ---------------------------------------------------------------------------
// Export directories and import (T10, library part)

/// `verify_export_dir` with the default options, as `store verify
/// <export>` runs it: ok for every valid case and not ok for every tampered
/// one. (The unit tests of `verify.rs` compare every report with the case's
/// golden report.)
#[test]
fn t10_only_the_valid_exports_verify() {
    for valid in [true, false] {
        let cases = export_cases(valid);
        assert_eq!(cases.len(), if valid { 7 } else { 35 });
        for case in cases {
            let report = verify_export_dir(&case, VerifyOptions::default()).unwrap();
            assert_eq!(report.ok(), valid, "{}", case_name(&case));
            assert_eq!(report.source, Source::Export);
        }
    }
}

/// The lines of an export's segment files, in order, without their LF.
fn export_lines(case: &Path) -> Vec<String> {
    let mut segments: Vec<PathBuf> = fs::read_dir(case)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
        .collect();
    segments.sort();
    segments
        .iter()
        .flat_map(|p| {
            let text = fs::read_to_string(p).unwrap();
            text.lines().map(String::from).collect::<Vec<_>>()
        })
        .collect()
}

/// Every valid export imports to a store that verifies with the export's
/// own report (but for its source), holds the same lines, and opens at the
/// next epoch for more appends.
#[test]
fn import_rebuilds_every_valid_export() {
    let cases = export_cases(true);
    assert_eq!(cases.len(), 7);
    for case in cases {
        let tmp = tempfile::tempdir().unwrap();
        let into = tmp.path().join("store");
        let report = import(&case, &into).unwrap();
        let name = case_name(&case);
        assert!(report.ok(), "{name}: {:?}", report.findings);
        assert_eq!(report.source, Source::Store, "{name}");
        // verify_store's report is the export's, byte for byte, but for
        // its source.
        let expect = case_json(&case).get("expect").unwrap().clone();
        let as_export = report
            .to_value()
            .with("source", Value::String("export".into()));
        assert_eq!(
            canon::canonicalize(&as_export),
            canon::canonicalize(&expect),
            "{name}"
        );
        assert_eq!(names(tmp.path()), ["store"], "{name}: no partial directory");
        assert_eq!(
            names(&into),
            [DB_FILE],
            "{name}: no LOCK until a writer opens"
        );

        let lines = export_lines(&case);
        let reader = SqliteReader::open(&into).unwrap();
        let held: Vec<String> = reader
            .lines(1, ALL)
            .unwrap()
            .into_iter()
            .map(|l| l.1)
            .collect();
        assert_eq!(held, lines, "{name}: the lines, byte for byte");
        let max_epoch = lines
            .iter()
            .map(|l| parse_line(l.as_bytes()).unwrap().entry.epoch)
            .max()
            .unwrap();
        let head = reader.head().unwrap();
        assert_eq!(
            Some((head.seq, head.entry_hash.clone())),
            report.head,
            "{name}"
        );
        drop(reader);

        // The next writer appends at the highest epoch + 1: a checkpoint
        // over the imported log, with the store key of every export vector.
        let c = clock(&["2026-12-01T00:00:00Z"]);
        let mut log = SqliteLog::open(&into, Box::new(c.clone())).unwrap();
        assert_eq!(log.epoch(), max_epoch + 1, "{name}");
        let checkpoint = log.checkpoint(&store_key()).unwrap();
        assert_eq!(checkpoint.size, head.seq, "{name}");
        let added = log.lines(head.seq + 1, 10).unwrap();
        assert_eq!(added.len(), 2, "{name}");
        for (_, line) in added {
            assert_eq!(
                parse_line(line.as_bytes()).unwrap().entry.epoch,
                max_epoch + 1
            );
        }
        assert_eq!(c.remaining(), 0);
        drop(log);
        assert!(verify(&into).ok(), "{name} after a checkpoint");
    }
}

/// No tampered export imports: the export's report comes back, not ok, and
/// nothing is left at the destination or beside it.
#[test]
fn import_refuses_every_tampered_export() {
    let cases = export_cases(false);
    assert_eq!(cases.len(), 35);
    for case in cases {
        let tmp = tempfile::tempdir().unwrap();
        let into = tmp.path().join("store");
        let report = import(&case, &into).unwrap();
        let name = case_name(&case);
        assert!(!report.ok(), "{name}");
        assert_eq!(report.source, Source::Export, "{name}");
        let spec = case_json(&case);
        if spec.get("max_findings").is_none() {
            let expect = spec.get("expect").unwrap();
            assert_eq!(
                canon::canonicalize(&report.to_value()),
                canon::canonicalize(expect),
                "{name}"
            );
        }
        assert!(names(tmp.path()).is_empty(), "{name}: nothing written");
    }
}

/// The destination is checked before the export is read. Partial
/// directories that killed attempts left (in this process or an earlier one
/// with the same id) are not ours to remove, and do not block the import.
#[test]
fn import_checks_its_destination_first() {
    let tmp = tempfile::tempdir().unwrap();
    let valid = vectors().join("store/exports/002-records-and-checkpoint");
    let absent = tmp.path().join("no-export");

    let taken = tmp.path().join("taken");
    fs::create_dir(&taken).unwrap();
    assert_eq!(error_code(import(&absent, &taken)), "S_EXISTS");
    assert_eq!(error_code(import(&valid, Path::new("/"))), "E_USAGE");
    assert_eq!(error_code(import(&valid, Path::new(".."))), "E_USAGE");

    let into = tmp.path().join("store");
    assert_eq!(error_code(import(&absent, &into)), "S_NOT_A_STORE");
    let no_manifest = tmp.path().join("no-manifest");
    fs::create_dir(&no_manifest).unwrap();
    assert_eq!(error_code(import(&no_manifest, &into)), "S_NOT_A_STORE");

    let pid = std::process::id();
    let leftovers = [
        tmp.path().join(format!("store.partial-{pid}")),
        evidence_store::export::partial_path(&into, 1).unwrap(),
        evidence_store::export::partial_path(&into, 2).unwrap(),
    ];
    for partial in &leftovers {
        fs::create_dir(partial).unwrap();
        fs::write(partial.join("mine"), b"keep").unwrap();
    }
    let before = names(tmp.path());

    import(&valid, &into).unwrap();
    assert_eq!(error_code(import(&valid, &into)), "S_EXISTS");
    assert!(verify(&into).ok());
    for partial in &leftovers {
        assert_eq!(names(partial), ["mine"]);
    }
    let mut after = before;
    after.push("store".into());
    after.sort();
    assert_eq!(
        names(tmp.path()),
        after,
        "no partial directory of its own left"
    );
}

/// An export whose files were listed but are missing is reported, not an
/// error, and imports nothing.
#[test]
fn import_reads_missing_segments_as_empty() {
    let tmp = tempfile::tempdir().unwrap();
    let export = tmp.path().join("export");
    fs::create_dir(&export).unwrap();
    let source = vectors().join("store/exports/004-multi-segment");
    for name in [
        "manifest.json",
        "000001.jsonl",
        "000003.jsonl",
        "000004.jsonl",
    ] {
        fs::copy(source.join(name), export.join(name)).unwrap();
    }
    let into = tmp.path().join("store");
    let report = import(&export, &into).unwrap();
    assert!(!report.ok());
    let found = codes(&report);
    assert!(found.contains(&("S_SEGMENT_HASH", None)), "{found:?}");
    assert!(!into.exists());
}
