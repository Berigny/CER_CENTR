#![cfg(unix)]

//! Store paths relative to the working directory name the store they say,
//! whatever they look like and wherever the process goes after it opens
//! one.
//!
//! The test changes the process's working directory, so it is the only test
//! in this binary. Data: the object and key vectors in `test-vectors/`.
//! Nothing is real.

use std::fs;
use std::path::Path;

use evidence_log::VerifyOptions;
use evidence_store::sqlite::lease::LOCK_FILE;
use evidence_store::{
    export, import, verify_store, LogRead, LogWrite, ScriptClock, SqliteLog, SqliteReader,
};

mod common;
use common::{error_code, names, read_vector, store_key};

fn head(dir: &Path) -> u64 {
    SqliteReader::open(dir).unwrap().head().unwrap().seq
}

/// A clock with one reading per second from midnight, `n` of them.
fn clock(n: usize) -> ScriptClock {
    let readings: Vec<String> = (0..n)
        .map(|i| format!("2026-11-26T00:00:{i:02}Z"))
        .collect();
    let readings: Vec<&str> = readings.iter().map(String::as_str).collect();
    ScriptClock::new(&readings)
}

/// - A directory named `file:a`: the bundled SQLite takes any file name that
///   starts with `file:` as a URI, so `file:a/log.db` would open
///   `a/log.db`. Every operation must use the store in `file:a`: reading,
///   verifying, writing under its own lease, exporting, importing, and
///   creating one beside a directory `z`.
/// - A reader of an immutable file opened by a relative path watches the
///   store it opened after the process changes its working directory: no
///   false `S_IO`, and a writer is still noticed.
#[test]
fn relative_store_paths_name_the_stores_they_say() {
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_current_dir(tmp.path()).unwrap();
    let object = read_vector("objects/001-create-inline/object.json");

    let uri_like = Path::new("file:a");
    let plain = Path::new("a");
    let c = clock(2);
    let mut log = SqliteLog::init(uri_like, "cer-demo", &store_key(), Box::new(c)).unwrap();
    log.append(&object).unwrap();
    drop(log);
    drop(SqliteLog::init(plain, "cer-demo", &store_key(), Box::new(clock(1))).unwrap());
    assert_eq!((head(uri_like), head(plain)), (3, 2));
    let report = verify_store(uri_like, VerifyOptions::default()).unwrap();
    assert_eq!(report.head.map(|h| h.0), Some(3));

    // Each writer holds its own store's lease and writes its own log.
    let plain_writer = SqliteLog::open(plain, Box::new(ScriptClock::default())).unwrap();
    let e = SqliteLog::open(plain, Box::new(ScriptClock::default()));
    assert_eq!(error_code(e), "S_LOCKED");
    let mut writer = SqliteLog::open(uri_like, Box::new(clock(1))).unwrap();
    writer.checkpoint(&store_key()).unwrap();
    drop(writer);
    drop(plain_writer);
    assert_eq!((head(uri_like), head(plain)), (5, 2));
    assert!(verify_store(plain, VerifyOptions::default()).unwrap().ok());

    // Export and import, to and from names that look like URIs.
    let reader = SqliteReader::open(uri_like).unwrap();
    export(&reader, Path::new("file:e"), 2).unwrap();
    assert!(import(Path::new("file:e"), Path::new("file:i"))
        .unwrap()
        .ok());
    assert_eq!(head(Path::new("file:i")), 5);

    // init beside a directory z, which it must leave alone.
    fs::create_dir("z").unwrap();
    drop(
        SqliteLog::init(
            Path::new("file:z"),
            "cer-demo",
            &store_key(),
            Box::new(clock(1)),
        )
        .unwrap(),
    );
    assert!(names(Path::new("z")).is_empty());
    assert_eq!(head(Path::new("file:z")), 2);

    // An immutable reader, opened by a relative path, then the working
    // directory changes. log.db-wal is a symlink into a directory that does
    // not exist, which SQLite cannot create even for root.
    let store = tmp.path().join("s");
    drop(SqliteLog::init(&store, "cer-demo", &store_key(), Box::new(clock(1))).unwrap());
    let wal = store.join("log.db-wal");
    let block_wal = || std::os::unix::fs::symlink(tmp.path().join("absent/wal"), &wal).unwrap();
    block_wal();
    let reader = SqliteReader::open(Path::new("s")).unwrap();
    assert!(reader.is_immutable());
    fs::create_dir("elsewhere").unwrap();
    std::env::set_current_dir("elsewhere").unwrap();
    assert_eq!(reader.head().unwrap().seq, 2, "no false S_IO");
    // A writer opens the store and closes it, and its LOCK is removed.
    fs::remove_file(&wal).unwrap();
    let mut writer = SqliteLog::open(&store, Box::new(clock(1))).unwrap();
    writer.append(&object).unwrap();
    drop(writer);
    fs::remove_file(store.join(LOCK_FILE)).unwrap();
    block_wal();
    assert_eq!(error_code(reader.head()), "S_IO", "a writer is noticed");
    std::env::set_current_dir(std::env::temp_dir()).unwrap();
}
