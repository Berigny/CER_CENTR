//! Helpers shared by the integration tests: the test vectors and keys,
//! synthetic objects, and the backends that every generic suite runs
//! against (`MemLog` and `SqliteLog`).
//!
//! Each test binary includes this module with `mod common;` and uses part of
//! it. Data: the vectors in `test-vectors/` and synthetic objects built
//! here. Nothing is real.

#![allow(dead_code)]

use std::fmt::Debug;
use std::fs;
use std::path::{Path, PathBuf};

use evidence_core::canon;
use evidence_core::json::{self, Value};
use evidence_core::object;
use evidence_core::time::{self, Timestamp};
use evidence_log::{Source, VerifyOptions};
use evidence_store::{
    export_files, verify_store, LogRead, LogWrite, MemLog, Rejected, ScriptClock, SeedSigner,
    Signer, SqliteLog, SqliteReader, StoreError,
};

// ---------------------------------------------------------------------------
// Backends

/// A storage backend under test. A suite is a set of functions generic over
/// `Backend`, turned into one test per function and backend by a macro.
pub trait Backend: Sized {
    /// Create a log named `name`, keyed by `signer`, whose writer reads a
    /// clone of `clock` (clones share one queue of readings).
    fn init(name: &str, signer: &dyn Signer, clock: &ScriptClock) -> Result<Self, StoreError>;

    /// The writer.
    fn log(&self) -> &dyn LogWrite;

    /// The writer, to write with.
    fn log_mut(&mut self) -> &mut dyn LogWrite;

    /// Close the writer and open the store again, handing the new writer a
    /// clone of `clock`. The epoch goes up by one.
    fn reopen(&mut self, clock: &ScriptClock);

    /// What only this backend can check about the log as it stands. Nothing
    /// for `MemLog`; see [`Sqlite::check`].
    fn check(&self) {}
}

/// [`MemLog`], the in-memory reference log.
pub struct Mem(pub MemLog);

impl Backend for Mem {
    fn init(name: &str, signer: &dyn Signer, clock: &ScriptClock) -> Result<Self, StoreError> {
        MemLog::init(name, signer, Box::new(clock.clone())).map(Mem)
    }

    fn log(&self) -> &dyn LogWrite {
        &self.0
    }

    fn log_mut(&mut self) -> &mut dyn LogWrite {
        &mut self.0
    }

    fn reopen(&mut self, _clock: &ScriptClock) {
        self.0.reopen();
    }
}

/// [`SqliteLog`], in a store directory of its own inside a temporary
/// directory that lives as long as it does. A reopen drops the writer,
/// which releases the lease, and opens the store again.
pub struct Sqlite {
    /// Removed when the backend drops, after `log`.
    tmp: tempfile::TempDir,
    log: Option<SqliteLog>,
}

impl Sqlite {
    /// The store directory.
    pub fn dir(&self) -> PathBuf {
        self.tmp.path().join("store")
    }

    pub fn writer(&self) -> &SqliteLog {
        self.log.as_ref().expect("the store is open")
    }
}

impl Backend for Sqlite {
    /// A failed init must leave no store directory behind.
    fn init(name: &str, signer: &dyn Signer, clock: &ScriptClock) -> Result<Self, StoreError> {
        let tmp = tempfile::tempdir().expect("a temporary directory");
        let dir = tmp.path().join("store");
        let log = SqliteLog::init(&dir, name, signer, Box::new(clock.clone()));
        if log.is_err() {
            assert!(!dir.exists(), "a failed init leaves no store directory");
        }
        Ok(Sqlite {
            tmp,
            log: Some(log?),
        })
    }

    fn log(&self) -> &dyn LogWrite {
        self.writer()
    }

    fn log_mut(&mut self) -> &mut dyn LogWrite {
        self.log.as_mut().expect("the store is open")
    }

    fn reopen(&mut self, clock: &ScriptClock) {
        self.log = None;
        let dir = self.dir();
        let log = SqliteLog::open(&dir, Box::new(clock.clone()))
            .unwrap_or_else(|e| panic!("reopen {}: {e}", dir.display()));
        self.log = Some(log);
    }

    /// A separate `SqliteReader` sees what the writer holds (lines, log id,
    /// genesis, heads and checkpoints), and `verify_store`, which adds the
    /// triggers and `meta.log_id`, is ok with the writer's head.
    fn check(&self) {
        let dir = self.dir();
        let writer = self.writer();
        let reader = SqliteReader::open(&dir).unwrap();
        assert_eq!(reader.log_id(), writer.log_id());
        assert_eq!(reader.genesis(), writer.genesis());
        assert_eq!(reader.lines(1, ALL), writer.lines(1, ALL));
        assert_eq!(reader.head_rows(), writer.head_rows());
        assert_eq!(reader.checkpoints(), writer.checkpoints());
        let report = verify_store(&dir, VerifyOptions::default()).unwrap();
        assert!(report.ok(), "verify_store: {:?}", report.findings);
        assert_eq!(report.source, Source::Store);
        let head = writer.head().unwrap();
        assert_eq!(report.head, Some((head.seq, head.entry_hash)));
    }
}

/// Two logs that hold the same log: the same log id, genesis, head, leaf
/// hashes, heads and checkpoints, and byte-identical exports.
#[track_caller]
pub fn same_log(a: &dyn LogRead, b: &dyn LogRead, segment_size: u64) {
    assert_eq!(a.log_id(), b.log_id());
    assert_eq!(a.genesis(), b.genesis());
    let head = a.head().unwrap();
    assert_eq!(head, b.head().unwrap());
    assert_eq!(
        a.leaf_hashes(head.seq).unwrap(),
        b.leaf_hashes(head.seq).unwrap()
    );
    assert_eq!(a.head_rows().unwrap(), b.head_rows().unwrap());
    assert_eq!(a.checkpoints().unwrap(), b.checkpoints().unwrap());
    let (a, b) = (export_files(a, segment_size), export_files(b, segment_size));
    assert!(a.unwrap() == b.unwrap(), "the exports differ");
}

// ---------------------------------------------------------------------------
// Results

/// The rejection a write failed with.
#[track_caller]
pub fn rejected<T: Debug>(result: Result<T, StoreError>) -> Rejected {
    match result {
        Err(StoreError::Rejected(r)) => r,
        other => panic!("expected a rejection, got {other:?}"),
    }
}

/// The code of the error a call failed with.
#[track_caller]
pub fn error_code<T>(result: Result<T, StoreError>) -> &'static str {
    match result {
        Err(e) => e.code_str(),
        Ok(_) => panic!("expected an error, got a success"),
    }
}

// ---------------------------------------------------------------------------
// Files and vectors

/// More lines than any test log holds.
pub const ALL: usize = 100_000;

pub fn vectors() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-vectors")
}

/// The bytes of a file.
#[track_caller]
pub fn read(path: &Path) -> Vec<u8> {
    fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The bytes of `test-vectors/<path>`.
#[track_caller]
pub fn read_vector(path: &str) -> Vec<u8> {
    read(&vectors().join(path))
}

/// A JSON file, parsed.
#[track_caller]
pub fn load(path: &Path) -> Value {
    json::parse(&read(path)).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Line `n`, counting from 1, of JSON Lines bytes, without its LF.
#[track_caller]
pub fn nth_line(bytes: &[u8], n: usize) -> Vec<u8> {
    let mut lines: Vec<&[u8]> = bytes.split(|b| *b == b'\n').collect();
    if bytes.ends_with(b"\n") {
        lines.pop();
    }
    assert!(n >= 1 && n <= lines.len(), "no line {n}");
    lines[n - 1].to_vec()
}

/// The names in a directory, sorted.
pub fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Every file in a directory with its bytes, sorted by name.
pub fn files_in(dir: &Path) -> Vec<(String, Vec<u8>)> {
    names(dir)
        .into_iter()
        .map(|name| {
            let bytes = read(&dir.join(&name));
            (name, bytes)
        })
        .collect()
}

/// The export vector cases, `test-vectors/store/exports/<case>`, in name
/// order: the valid ones (0xx) or the tampered ones (1xx).
pub fn export_cases(valid: bool) -> Vec<PathBuf> {
    let root = vectors().join("store/exports");
    let mut cases: Vec<PathBuf> = fs::read_dir(&root)
        .unwrap_or_else(|e| panic!("{}: {e}", root.display()))
        .map(|e| e.unwrap().path())
        .filter(|p| case_name(p).starts_with('0') == valid)
        .collect();
    cases.sort();
    assert!(!cases.is_empty(), "no export cases in {}", root.display());
    cases
}

/// The case's directory name.
pub fn case_name(case: &Path) -> String {
    case.file_name().unwrap().to_string_lossy().into_owned()
}

/// A case's `case.json`.
pub fn case_json(case: &Path) -> Value {
    load(&case.join("case.json"))
}

pub fn ts(text: &str) -> Timestamp {
    time::parse(text).unwrap_or_else(|_| panic!("{text:?} is not a timestamp"))
}

/// A signer for `test-vectors/keys/<name>.seed`.
pub fn seed(name: &str) -> SeedSigner {
    SeedSigner::from_file(&vectors().join(format!("keys/{name}.seed")))
        .unwrap_or_else(|e| panic!("seed {name}: {e}"))
}

/// The test store key, `store-demo`, which every export vector uses.
pub fn store_key() -> SeedSigner {
    seed("store-demo")
}

// ---------------------------------------------------------------------------
// Synthetic objects

/// An object as given to a store, and its id.
#[derive(Clone, Debug)]
pub struct Obj {
    pub id: String,
    pub bytes: Vec<u8>,
}

/// A sealed synthetic evidence/0 object in canonical form: `inline` JSON
/// content, the actor `service:<actor>`, and `schema.hash` the SHA-256 of
/// the schema ref.
pub fn sealed(
    op: &str,
    schema: &str,
    subject: [&str; 3],
    inline: Value,
    actor: &str,
    at: &str,
    prev: &[String],
) -> Obj {
    let s = |t: &str| Value::String(t.to_string());
    let unsealed = Value::Object(vec![
        ("v".into(), s("evidence/0")),
        ("op".into(), s(op)),
        (
            "schema".into(),
            Value::Object(vec![
                ("ref".into(), s(schema)),
                ("hash".into(), s(&canon::sha256_hex(schema.as_bytes()))),
            ]),
        ),
        (
            "subject".into(),
            Value::Object(vec![
                ("system".into(), s(subject[0])),
                ("type".into(), s(subject[1])),
                ("key".into(), s(subject[2])),
            ]),
        ),
        (
            "content".into(),
            Value::Object(vec![
                ("media_type".into(), s("application/json")),
                ("hash".into(), s(&canon::digest(&inline))),
                ("inline".into(), inline),
            ]),
        ),
        (
            "actor".into(),
            Value::Object(vec![
                ("id".into(), s(&format!("service:{actor}"))),
                ("kind".into(), s("service")),
            ]),
        ),
        ("at".into(), s(at)),
        (
            "prev".into(),
            Value::Array(prev.iter().map(|p| s(p)).collect()),
        ),
    ]);
    let sealed = object::seal(&unsealed);
    Obj {
        id: sealed
            .get("id")
            .and_then(Value::as_str)
            .expect("a sealed object has an id")
            .to_string(),
        bytes: canon::canonicalize(&sealed).into_bytes(),
    }
}
