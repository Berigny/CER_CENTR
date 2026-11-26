//! The reference store: one SQLite file, `log.db`, is the authoritative log
//! (D12 in `docs/decisions.md`). See `docs/m1-store-spec.md` section 7.
//!
//! ```text
//! <store>/
//!   log.db       the log (WAL); log.db-wal and log.db-shm beside it while open
//!   LOCK         the writer lease (see lease); "pid <pid>\nepoch <n>\n"
//!   log.db.init  only while SqliteLog::init builds the store (or after an
//!                init that was killed: the next init removes it)
//! ```
//!
//! - [`SqliteLog`] is the one writer. It holds the lease for as long as it
//!   lives, and every write is one `BEGIN IMMEDIATE … COMMIT`.
//! - [`SqliteReader`] reads without the lease, through a read-only connection
//!   (`SQLITE_OPEN_READ_ONLY` and `PRAGMA query_only = ON`), while a writer
//!   appends. A store it cannot write (a read-only file system, a
//!   directory of another user) it reads as an immutable file when it can;
//!   see [`SqliteReader::open`].
//! - Both implement [`LogRead`] and `PrevLookup` with the same queries.
//!
//! # Opening a store
//!
//! `SqliteLog::open` and `SqliteReader::open` check, in this order, before
//! the writer takes the lease:
//!
//! 1. `<dir>/log.db` exists; otherwise `S_NOT_A_STORE`. Nothing is created
//!    in a directory that does not hold one.
//! 2. It opens as an SQLite database (without `SQLITE_OPEN_CREATE`) whose
//!    `application_id` is [`APPLICATION_ID`]; otherwise `S_NOT_A_STORE`
//!    (SQLite's "file is not a database" included).
//! 3. Its `user_version` is [`USER_VERSION`]; otherwise `S_FORMAT`.
//! 4. `meta` has a `format` row, which is [`FORMAT`] (`S_FORMAT` if it is
//!    something else), and rows `log_id`, `name`, `store_key`,
//!    `store_signer` and `writer_epoch`, with `Genesis::new(name, store_key)`
//!    valid, `store_signer` its signer, and `writer_epoch` a decimal integer
//!    from 1 to 2^53 − 1. Any missing table or row, or any other value, is
//!    `S_NOT_A_STORE`. `log_id` is taken as it is: `verify_store` checks it
//!    against the lines (`S_META_MISMATCH`).
//!
//! The log id and genesis these give are cached for [`LogRead::log_id`] and
//! [`LogRead::genesis`]. Opening never reads the lines.

pub(crate) mod db;
pub mod lease;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use evidence_core::time::Timestamp;
use evidence_core::verify::PrevLookup;
use evidence_log::merkle::MAX_SIZE;
use evidence_log::{Digest, Genesis, SubjectKey};
use rusqlite::types::ValueRef;
use rusqlite::{Connection, ErrorCode, OpenFlags, OptionalExtension, TransactionBehavior};

use crate::api::{
    CheckpointInfo, Head, Item, LogRead, LogWrite, Receipt, StoreError, StoredAttestation,
    StoredObject,
};
use crate::clock::Clock;
use crate::files::{io_error, parent_of, read_regular, refuse_existing, sync_dir, sync_file};
use crate::intake;
use crate::signer::Signer;
use db::Writer;
use lease::{Lease, LOCK_FILE};

/// The log file in a store directory.
pub const DB_FILE: &str = "log.db";

/// `log.db`'s write-ahead log, beside it while a connection has it open.
const WAL_FILE: &str = "log.db-wal";

/// The name [`SqliteLog::init`] builds the new log under, in the store
/// directory, before it renames it to [`DB_FILE`], so that `log.db` only
/// ever names a complete store. Only an init that holds the lease writes
/// it; one that a killed init left is removed by the next.
pub const INIT_FILE: &str = "log.db.init";

/// What an init that was killed can leave in a store directory: `LOCK`,
/// then [`INIT_FILE`] and SQLite's files beside it. [`SqliteLog::init`]
/// takes a directory that holds nothing else.
const INIT_LEFTOVERS: [&str; 5] = [
    LOCK_FILE,
    INIT_FILE,
    "log.db.init-journal",
    "log.db.init-wal",
    "log.db.init-shm",
];

/// How long a connection waits on a lock another connection holds.
const BUSY_TIMEOUT: Duration = Duration::from_millis(5000);

/// `PRAGMA application_id` of every `log.db`: `0x45564430`, ASCII `EVD0`.
pub const APPLICATION_ID: i64 = 0x4556_4430;

/// `PRAGMA user_version` of the schema this crate writes and reads. Any
/// other value is `S_FORMAT`; migrations arrive with schema 2 (M2).
pub const USER_VERSION: i64 = 1;

/// `meta.format` of every store this crate writes and reads.
pub const FORMAT: &str = evidence_log::STORE_SPEC;

/// The keys of `meta`, all required: `format`, `log_id`, `name`,
/// `store_signer`, `store_key` and `writer_epoch` (decimal text).
pub const META_KEYS: [&str; 6] = [
    "format",
    "log_id",
    "name",
    "store_signer",
    "store_key",
    "writer_epoch",
];

/// The schema, run once inside the transaction that creates a store (init)
/// or rebuilds one (import). It sets `application_id` and `user_version` and
/// creates the tables, indexes and triggers; `meta` rows are written by code.
pub const SCHEMA_SQL: &str = include_str!("schema.sql");

/// Run by a writer's connection when it opens, outside any transaction
/// (`journal_mode` cannot change inside one, and `foreign_keys` does nothing
/// there).
pub const WRITER_PRAGMAS: &str =
    "PRAGMA journal_mode = WAL; PRAGMA synchronous = FULL; PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;";

/// Run by a reader's connection when it opens. The connection itself is
/// opened with `SQLITE_OPEN_READ_ONLY` (and, for a store the reader cannot
/// write, as an immutable file: see [`SqliteReader::open`]).
pub const READER_PRAGMAS: &str =
    "PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000; PRAGMA query_only = ON;";

/// The append-only triggers [`SCHEMA_SQL`] creates. `verify_store` gives
/// `S_TRIGGERS_MISSING` unless each is in `sqlite_master` with the same
/// `tbl_name` and `sql` as in a fresh in-memory database built from
/// `SCHEMA_SQL`.
pub const TRIGGERS: [&str; 6] = [
    "entries_no_update",
    "entries_no_delete",
    "objects_no_update",
    "objects_no_delete",
    "attestations_no_update",
    "attestations_no_delete",
];

/// The writer of a store on disk. Holds the lease until dropped.
///
/// [`LogWrite::commit`] and [`LogWrite::checkpoint`] read the clock once,
/// open `BEGIN IMMEDIATE`, plan with [`crate::intake`] against an
/// [`IntakeState`](crate::intake::IntakeState) over that transaction, insert
/// the plan's rows (entries, objects, attestations, heads changes,
/// checkpoints) and commit. On any error the transaction rolls back, so
/// nothing of the batch is written. Its [`LogRead`] answers and its export
/// are identical to a [`MemLog`](crate::mem::MemLog)'s given the same calls
/// and clock readings.
///
/// Dropping it closes the connection first, then releases the lease (the
/// fields drop in this order).
pub struct SqliteLog {
    conn: Connection,
    /// Held for its lifetime; released on drop.
    _lease: Lease,
    dir: PathBuf,
    log_id: String,
    genesis: Genesis,
    epoch: u64,
    clock: Box<dyn Clock>,
}

impl SqliteLog {
    /// Create a store in `dir`, at epoch 1, and return its writer, which
    /// holds the lease. See [`crate::api`], "Creating a store".
    ///
    /// 1. `E_USAGE` for an invalid name or key (`intake::new_genesis`).
    /// 2. `S_EXISTS` unless `dir` is absent or a directory holding nothing
    ///    but what an init that was killed leaves: `LOCK`, [`INIT_FILE`],
    ///    and `log.db.init-journal`, `-wal` and `-shm`. An empty directory
    ///    is the usual case; a file at `dir` is `S_EXISTS`. If `dir` does
    ///    not exist it is created, with its parents.
    /// 3. Take the lease (creating `LOCK`): `S_LOCKED` if another `init` or
    ///    writer holds it. Under the lease, check step 2 again (`S_EXISTS`
    ///    if `log.db`, or anything else, appeared meanwhile) and remove what
    ///    a killed init left besides `LOCK`: no other init can be building
    ///    here now.
    /// 4. Read the clock once and plan the genesis (`intake::plan_genesis`).
    /// 5. Build the store in [`INIT_FILE`]: run [`WRITER_PRAGMAS`], then one
    ///    transaction: [`SCHEMA_SQL`], the `meta` rows (`format` =
    ///    `store/0`, `log_id`, `name`, `store_signer`, `store_key`,
    ///    `writer_epoch` = `1`), and the plan's rows. Close it, which
    ///    checkpoints its WAL into it and removes the WAL (`S_IO` if a WAL
    ///    with content is left), and sync it.
    /// 6. Rename it to `log.db` (`S_EXISTS` if `log.db` appeared meanwhile;
    ///    as for exports, the standard library has no rename that refuses
    ///    to replace), then sync `dir` and the parent of every directory
    ///    step 2 created, so that the store's names are durable before
    ///    anything is returned.
    /// 7. Open `log.db` as the writer, run [`WRITER_PRAGMAS`], and write the
    ///    lease text with epoch 1.
    ///
    /// `log.db` only ever names a complete store, so an init killed at any
    /// point leaves either a store or a directory that the next `init`
    /// takes. If anything fails after the lease is taken, what this call
    /// created is removed while it still holds the lease: [`INIT_FILE`] and
    /// its files, `log.db` and its WAL files if step 6 renamed it, and
    /// `LOCK` if this call created it; then the lease is released, and
    /// `dir` is removed if this call created it and it is empty. Nothing is
    /// removed after `S_LOCKED` or the `S_EXISTS` of step 3: those files
    /// are another writer's.
    pub fn init(
        dir: &Path,
        name: &str,
        signer: &dyn Signer,
        clock: Box<dyn Clock>,
    ) -> Result<SqliteLog, StoreError> {
        let genesis = intake::new_genesis(name, signer)?;
        let created = claim_dir(dir)?;
        let result = create(dir, &created, genesis, signer, clock);
        if result.is_err() && !created.is_empty() {
            // Only if empty: an init that started meanwhile keeps its files.
            let _ = fs::remove_dir(dir);
        }
        result
    }

    /// Open the store in `dir` for writing: the checks under [Opening a
    /// store](self#opening-a-store), then take the lease (`S_LOCKED` if
    /// another writer has it), then, in one transaction, add one to
    /// `meta.writer_epoch`, then rewrite the lease text. The new value is
    /// this writer's epoch. Reads no clock.
    pub fn open(dir: &Path, clock: Box<dyn Clock>) -> Result<SqliteLog, StoreError> {
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let mut conn = connect(dir, flags)?;
        let meta = check_store(&conn, dir)?;
        let mut lease = Lease::acquire(dir)?;
        conn.execute_batch(WRITER_PRAGMAS)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        // Read again under the lease: a writer may have opened and closed
        // since the checks.
        let current = db::meta(&tx, "writer_epoch")?
            .as_deref()
            .and_then(parse_epoch)
            .ok_or_else(|| StoreError::NotAStore(dir.to_path_buf()))?;
        let epoch = current
            .checked_add(1)
            .filter(|epoch| *epoch <= MAX_SIZE)
            .ok_or_else(|| StoreError::Io("meta.writer_epoch is at 2^53 - 1".into()))?;
        tx.execute(
            "UPDATE meta SET v = ?1 WHERE k = 'writer_epoch'",
            [epoch.to_string()],
        )?;
        tx.commit()?;
        lease.record(epoch)?;
        Ok(SqliteLog {
            conn,
            _lease: lease,
            dir: dir.to_path_buf(),
            log_id: meta.log_id,
            genesis: meta.genesis,
            epoch,
            clock,
        })
    }

    /// The store directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// One write transaction: `BEGIN IMMEDIATE`, a plan made by `plan`
    /// against the transaction, its rows, `COMMIT`. Nothing is written if
    /// any step fails.
    fn write(
        &mut self,
        plan: impl FnOnce(&db::State<'_>, &str, &Genesis, u64) -> Result<intake::Plan, StoreError>,
    ) -> Result<intake::Plan, StoreError> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let plan = plan(&db::State(&tx), &self.log_id, &self.genesis, self.epoch)?;
        Writer::new(&tx)?.apply(&plan)?;
        tx.commit()?;
        Ok(plan)
    }
}

impl LogRead for SqliteLog {
    fn log_id(&self) -> &str {
        &self.log_id
    }

    fn genesis(&self) -> &Genesis {
        &self.genesis
    }

    fn head(&self) -> Result<Head, StoreError> {
        db::head(&self.conn)
    }

    fn lines(&self, from_seq: u64, limit: usize) -> Result<Vec<(u64, String)>, StoreError> {
        db::lines(&self.conn, from_seq, limit)
    }

    fn object(&self, id: &str) -> Result<Option<StoredObject>, StoreError> {
        db::object(&self.conn, id)
    }

    fn attestations(&self, object_id: &str) -> Result<Vec<StoredAttestation>, StoreError> {
        db::attestations(&self.conn, object_id)
    }

    fn heads(&self, subject: &SubjectKey, lane: &str) -> Result<Vec<String>, StoreError> {
        db::heads(&self.conn, subject, lane)
    }

    fn head_rows(&self) -> Result<Vec<(SubjectKey, String, String)>, StoreError> {
        db::head_rows(&self.conn)
    }

    fn checkpoints(&self) -> Result<Vec<CheckpointInfo>, StoreError> {
        db::checkpoints(&self.conn)
    }

    fn leaf_hashes(&self, upto: u64) -> Result<Vec<Digest>, StoreError> {
        db::leaf_hashes(&self.conn, upto)
    }
}

impl LogWrite for SqliteLog {
    fn epoch(&self) -> u64 {
        self.epoch
    }

    /// Reads the clock once, then plans with `intake::plan_commit` and
    /// writes the plan, in one `BEGIN IMMEDIATE` transaction.
    fn commit(&mut self, items: &[Item<'_>]) -> Result<Vec<Receipt>, StoreError> {
        let now = self.clock.now();
        let plan = self.write(|state, log_id, _, epoch| {
            intake::plan_commit(state, log_id, epoch, now, items)
        })?;
        Ok(plan.receipts)
    }

    /// `intake::check_store_key`, then one clock reading, then
    /// `intake::plan_checkpoint` and its rows in one `BEGIN IMMEDIATE`
    /// transaction.
    fn checkpoint(&mut self, signer: &dyn Signer) -> Result<CheckpointInfo, StoreError> {
        intake::check_store_key(&self.genesis, signer)?;
        let now = self.clock.now();
        let plan = self.write(|state, log_id, genesis, epoch| {
            let plan = intake::plan_checkpoint(state, genesis, log_id, epoch, signer, now)?;
            if plan.checkpoint.is_none() {
                return Err(StoreError::Io(
                    "a checkpoint plan without its checkpoint".into(),
                ));
            }
            Ok(plan)
        })?;
        plan.checkpoint
            .ok_or_else(|| StoreError::Io("a checkpoint plan without its checkpoint".into()))
    }
}

/// `prev_at(id)`: `SELECT at FROM objects WHERE id = ?`, parsed. None when
/// the object is not held, and also when the query fails.
impl PrevLookup for SqliteLog {
    fn prev_at(&self, id: &str) -> Option<Timestamp> {
        db::object_at(&self.conn, id).ok().flatten()
    }
}

/// A read-only view of a store on disk. Takes no lease, so it works while a
/// writer holds one.
///
/// Each [`LogRead`] call runs in its own read transaction, so what one call
/// returns is of one moment; [`snapshot`](Self::snapshot) runs several calls
/// in one.
pub struct SqliteReader {
    conn: Connection,
    dir: PathBuf,
    log_id: String,
    genesis: Genesis,
    /// Set when `log.db` is read as an immutable file (see
    /// [`open`](Self::open)).
    frozen: Option<Frozen>,
}

impl SqliteReader {
    /// Open the store in `dir` for reading: a `SQLITE_OPEN_READ_ONLY`
    /// connection with [`READER_PRAGMAS`], and the checks under [Opening a
    /// store](self#opening-a-store). Takes no lease and changes nothing in
    /// the store.
    ///
    /// SQLite reads a database in WAL mode through `log.db-wal` and
    /// `log.db-shm` beside it, and creates them when they are absent, which
    /// needs write access to `dir`. When the first read fails for want of
    /// it (`SQLITE_CANTOPEN` or `SQLITE_READONLY`: a read-only file system,
    /// or a directory this user cannot write) and the store has no WAL
    /// content (`log.db-wal` absent or empty, as SQLite judges it), then
    /// `log.db` alone holds the whole log and no writer has it open, so the
    /// reader opens `log.db` as an immutable file instead (URI parameters
    /// `mode=ro&immutable=1`) and creates nothing.
    ///
    /// SQLite does not watch an immutable file, so such a reader watches
    /// for a writer itself. It keeps the text of `LOCK` (or its absence)
    /// from before it opened, and after every read transaction it checks
    /// that `log.db-wal` still holds nothing and `LOCK` is unchanged;
    /// otherwise that read is `S_IO` (open a new reader), since what it
    /// read may be torn. A writer that opens the store creates or rewrites
    /// `LOCK` before it changes `log.db`, and has `log.db-wal` while open.
    ///
    /// A store with WAL content (its last writer is open, or did not close)
    /// is not read without write access to `dir`: `S_IO`, as SQLite reports
    /// it.
    pub fn open(dir: &Path) -> Result<SqliteReader, StoreError> {
        let (conn, frozen) = connect_reader(dir)?;
        let checked = check_store(&conn, dir);
        if let Some(frozen) = &frozen {
            frozen.check(dir)?;
        }
        let meta = checked?;
        Ok(SqliteReader {
            conn,
            dir: dir.to_path_buf(),
            log_id: meta.log_id,
            genesis: meta.genesis,
            frozen,
        })
    }

    /// Whether this reader reads `log.db` as an immutable file, because it
    /// cannot write the store directory (see [`open`](Self::open)).
    pub fn is_immutable(&self) -> bool {
        self.frozen.is_some()
    }

    /// The store directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Run `f` inside one read transaction (`BEGIN DEFERRED … COMMIT` on this
    /// connection), so that every [`LogRead`] call it makes sees one
    /// snapshot of the store, whatever a writer appends meanwhile.
    /// `verify_store` runs in one, so its drift check compares the lines with
    /// the projections of the same moment. The transaction ends whether `f`
    /// succeeds or fails; `f`'s result is returned. A snapshot inside a
    /// snapshot runs in the outer one.
    pub fn snapshot<T>(
        &self,
        f: impl FnOnce(&SqliteReader) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        self.read(|_| f(self))
    }

    /// True when every trigger in [`TRIGGERS`] is present with the
    /// `tbl_name` and `sql` that [`SCHEMA_SQL`] gives it (compared with a
    /// fresh in-memory database built from `SCHEMA_SQL`). Other triggers do
    /// not matter.
    pub fn has_append_only_triggers(&self) -> Result<bool, StoreError> {
        let fresh = Connection::open_in_memory()?;
        fresh.execute_batch(SCHEMA_SQL)?;
        self.read(|conn| {
            for name in TRIGGERS {
                let expected = db::trigger(&fresh, name)?;
                if expected.is_none() || db::trigger(conn, name)? != expected {
                    return Ok(false);
                }
            }
            Ok(true)
        })
    }

    /// `meta.log_id` as stored now (inside a snapshot, as of that snapshot),
    /// or None if the row is gone. [`LogRead::log_id`] is the value read
    /// when the store was opened.
    pub(crate) fn stored_log_id(&self) -> Result<Option<String>, StoreError> {
        self.read(|conn| db::meta(conn, "log_id"))
    }

    /// Run `f` in a read transaction: a new one, or the snapshot this
    /// connection is already in.
    fn read<T>(
        &self,
        f: impl FnOnce(&Connection) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        if !self.conn.is_autocommit() {
            return f(&self.conn);
        }
        // BEGIN DEFERRED: the snapshot starts at the first read. Dropped
        // without commit (when f fails), it rolls back, which for a read
        // transaction only ends it.
        let tx = self.conn.unchecked_transaction()?;
        let out = f(&tx).and_then(|out| {
            tx.commit()?;
            Ok(out)
        });
        // An immutable file is only what it seems while no writer has
        // opened the store; an answer read after one did may be torn.
        if let Some(frozen) = &self.frozen {
            frozen.check(&self.dir)?;
        }
        out
    }
}

impl LogRead for SqliteReader {
    fn log_id(&self) -> &str {
        &self.log_id
    }

    fn genesis(&self) -> &Genesis {
        &self.genesis
    }

    fn head(&self) -> Result<Head, StoreError> {
        self.read(db::head)
    }

    fn lines(&self, from_seq: u64, limit: usize) -> Result<Vec<(u64, String)>, StoreError> {
        self.read(|conn| db::lines(conn, from_seq, limit))
    }

    fn object(&self, id: &str) -> Result<Option<StoredObject>, StoreError> {
        self.read(|conn| db::object(conn, id))
    }

    fn attestations(&self, object_id: &str) -> Result<Vec<StoredAttestation>, StoreError> {
        self.read(|conn| db::attestations(conn, object_id))
    }

    fn heads(&self, subject: &SubjectKey, lane: &str) -> Result<Vec<String>, StoreError> {
        self.read(|conn| db::heads(conn, subject, lane))
    }

    fn head_rows(&self) -> Result<Vec<(SubjectKey, String, String)>, StoreError> {
        self.read(db::head_rows)
    }

    fn checkpoints(&self) -> Result<Vec<CheckpointInfo>, StoreError> {
        self.read(db::checkpoints)
    }

    fn leaf_hashes(&self, upto: u64) -> Result<Vec<Digest>, StoreError> {
        self.read(|conn| db::leaf_hashes(conn, upto))
    }
}

/// As for [`SqliteLog`].
impl PrevLookup for SqliteReader {
    fn prev_at(&self, id: &str) -> Option<Timestamp> {
        self.read(|conn| db::object_at(conn, id)).ok().flatten()
    }
}

// ---------------------------------------------------------------------------
// Opening and creating

/// What the open checks read from `meta`.
struct Meta {
    log_id: String,
    genesis: Genesis,
}

/// Open `<dir>/log.db` with `flags`, which never include
/// `SQLITE_OPEN_CREATE`: `S_NOT_A_STORE` if it is not there or not a file.
fn connect(dir: &Path, flags: OpenFlags) -> Result<Connection, StoreError> {
    let path = dir.join(DB_FILE);
    match fs::metadata(&path) {
        Ok(m) if m.is_file() => {}
        Ok(_) => return Err(StoreError::NotAStore(dir.to_path_buf())),
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
            ) =>
        {
            return Err(StoreError::NotAStore(dir.to_path_buf()));
        }
        Err(e) => return Err(io_error(&path, e)),
    }
    let conn = Connection::open_with_flags(&path, flags).map_err(|e| engine(e, dir))?;
    // Set before the checks read anything; it writes nothing.
    conn.busy_timeout(BUSY_TIMEOUT)?;
    Ok(conn)
}

/// A reader's connection to `<dir>/log.db`, with [`READER_PRAGMAS`] run:
/// read-only, or immutable when a read-only connection cannot read for
/// want of write access to `dir` and the store has no WAL content (see
/// [`SqliteReader::open`]).
fn connect_reader(dir: &Path) -> Result<(Connection, Option<Frozen>), StoreError> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = connect(dir, flags)?;
    // The first read opens the WAL, creating log.db-wal and log.db-shm when
    // they are absent.
    let first_read = conn
        .execute_batch(READER_PRAGMAS)
        .and_then(|()| conn.query_row("SELECT count(*) FROM sqlite_master", [], |_| Ok(())));
    let e = match first_read {
        Ok(()) => return Ok((conn, None)),
        Err(e) => e,
    };
    let wants_write_access = matches!(
        e.sqlite_error_code(),
        Some(ErrorCode::CannotOpen | ErrorCode::ReadOnly)
    );
    drop(conn);
    if !wants_write_access {
        return Err(engine(e, dir));
    }
    // LOCK before the WAL: a writer that opens after LOCK is read changes
    // it, or has log.db-wal, before it changes log.db.
    let (Ok(lock), false, Some(uri)) = (
        lock_text(dir),
        wal_has_content(&dir.join(WAL_FILE)),
        immutable_uri(&dir.join(DB_FILE)),
    ) else {
        return Err(engine(e, dir));
    };
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
        | OpenFlags::SQLITE_OPEN_URI
        | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(uri, flags).map_err(|e| engine(e, dir))?;
    conn.busy_timeout(BUSY_TIMEOUT)?;
    conn.execute_batch(READER_PRAGMAS)
        .map_err(|e| engine(e, dir))?;
    Ok((conn, Some(Frozen { lock })))
}

/// What a reader of `log.db` as an immutable file checks after each read
/// (see [`SqliteReader::open`]): the text of `LOCK` from before it opened,
/// None if there was no `LOCK`.
struct Frozen {
    lock: Option<Vec<u8>>,
}

impl Frozen {
    /// `S_IO` unless `log.db-wal` holds nothing and `LOCK` is as it was: no
    /// writer has opened the store since the reader did.
    fn check(&self, dir: &Path) -> Result<(), StoreError> {
        let unchanged = !wal_has_content(&dir.join(WAL_FILE))
            && lock_text(dir).is_ok_and(|lock| lock == self.lock);
        if unchanged {
            Ok(())
        } else {
            Err(StoreError::Io(format!(
                "{}: a writer opened the store while it was read without write access; \
                 open a new reader",
                dir.display()
            )))
        }
    }
}

/// The bytes of `<dir>/LOCK`, or None if there is none. An error if it
/// cannot be read or is not a regular file.
fn lock_text(dir: &Path) -> io::Result<Option<Vec<u8>>> {
    match read_regular(&dir.join(LOCK_FILE)) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Whether the WAL file at `path` may hold frames, judged as SQLite judges
/// whether a WAL is present: something is there (a symlink followed) and
/// it is not an empty regular file. Anything that cannot be told counts as
/// content.
fn wal_has_content(path: &Path) -> bool {
    match fs::metadata(path) {
        Ok(m) => !m.is_file() || m.len() > 0,
        Err(e) => e.kind() != io::ErrorKind::NotFound,
    }
}

/// The URI that opens `path` read-only as an immutable file: `file://`,
/// the absolute path with every byte other than an ASCII letter, a digit
/// and `-._~/` percent-encoded (after a `/` if it does not start with one,
/// as a Windows path does not), then `?mode=ro&immutable=1`. None if the
/// path cannot be made absolute or, off Unix, is not UTF-8.
fn immutable_uri(path: &Path) -> Option<String> {
    let absolute = std::path::absolute(path).ok()?;
    #[cfg(unix)]
    let bytes = std::os::unix::ffi::OsStrExt::as_bytes(absolute.as_os_str()).to_vec();
    #[cfg(not(unix))]
    let bytes = absolute.to_str()?.replace('\\', "/").into_bytes();
    let mut uri = String::from("file://");
    if bytes.first() != Some(&b'/') {
        uri.push('/');
    }
    for byte in bytes {
        if byte.is_ascii_alphanumeric() || b"-._~/".contains(&byte) {
            uri.push(char::from(byte));
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri.push_str("?mode=ro&immutable=1");
    Some(uri)
}

/// The checks under [Opening a store](self#opening-a-store), in order.
fn check_store(conn: &Connection, dir: &Path) -> Result<Meta, StoreError> {
    let not_a_store = || StoreError::NotAStore(dir.to_path_buf());
    let pragma = |name: &str| -> Result<i64, StoreError> {
        conn.pragma_query_value(None, name, |r| r.get(0))
            .map_err(|e| engine(e, dir))
    };
    if pragma("application_id")? != APPLICATION_ID {
        return Err(not_a_store());
    }
    let user_version = pragma("user_version")?;
    if user_version != USER_VERSION {
        return Err(StoreError::Format(format!(
            "{} has schema version (user_version) {user_version}; this implementation reads \
             {USER_VERSION}",
            dir.join(DB_FILE).display()
        )));
    }
    let has_meta: bool = conn.query_row(
        "SELECT count(*) > 0 FROM sqlite_master WHERE type = 'table' AND name = 'meta'",
        [],
        |r| r.get(0),
    )?;
    if !has_meta {
        return Err(not_a_store());
    }
    let meta = |key: &str| meta_text(conn, dir, key);
    let format = meta("format")?.ok_or_else(not_a_store)?;
    if format != FORMAT {
        return Err(StoreError::Format(format!(
            "{} holds format {format:?}; this implementation reads {FORMAT:?}",
            dir.display()
        )));
    }
    let required = |key: &str| meta(key)?.ok_or_else(not_a_store);
    let log_id = required("log_id")?;
    let genesis =
        Genesis::new(&required("name")?, &required("store_key")?).map_err(|_| not_a_store())?;
    if required("store_signer")? != genesis.signer {
        return Err(not_a_store());
    }
    parse_epoch(&required("writer_epoch")?).ok_or_else(not_a_store)?;
    Ok(Meta { log_id, genesis })
}

/// One `meta` value for the open checks, or None when the row is missing or
/// does not hold text (a `meta` table of another application need not be
/// `STRICT`). `S_NOT_A_STORE` when `meta` has no `k` or `v` column.
fn meta_text(conn: &Connection, dir: &Path, key: &str) -> Result<Option<String>, StoreError> {
    let mut stmt = conn
        .prepare("SELECT v FROM meta WHERE k = ?1")
        .map_err(|e| {
            // SQLITE_ERROR: the SQL does not fit the table (a missing column).
            let sql_error = match &e {
                rusqlite::Error::SqliteFailure(f, _)
                | rusqlite::Error::SqlInputError { error: f, .. } => {
                    f.extended_code == rusqlite::ffi::SQLITE_ERROR
                }
                _ => false,
            };
            if sql_error {
                StoreError::NotAStore(dir.to_path_buf())
            } else {
                engine(e, dir)
            }
        })?;
    let value = stmt
        .query_row([key], |r| match r.get_ref(0)? {
            ValueRef::Text(bytes) => Ok(Some(String::from_utf8_lossy(bytes).into_owned())),
            _ => Ok(None),
        })
        .optional()
        .map_err(|e| engine(e, dir))?;
    Ok(value.flatten())
}

/// `writer_epoch` as stored: decimal digits only, from 1 to 2^53 − 1.
fn parse_epoch(text: &str) -> Option<u64> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse()
        .ok()
        .filter(|epoch| (1..=MAX_SIZE).contains(epoch))
}

/// A storage-engine error while opening: `S_NOT_A_STORE` when the file is
/// not an SQLite database, `S_IO` otherwise.
fn engine(e: rusqlite::Error, dir: &Path) -> StoreError {
    if e.sqlite_error_code() == Some(ErrorCode::NotADatabase) {
        StoreError::NotAStore(dir.to_path_buf())
    } else {
        StoreError::from(e)
    }
}

/// Step 2 of [`SqliteLog::init`]: `S_EXISTS` unless `dir` is absent or a
/// directory holding nothing but [`INIT_LEFTOVERS`]. Creates it, with its
/// parents, if absent, and returns the directories it created: `dir`, then
/// each parent it created, nearest first. Empty when `dir` existed.
fn claim_dir(dir: &Path) -> Result<Vec<PathBuf>, StoreError> {
    match fs::symlink_metadata(dir) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let missing = missing_dirs(dir);
            fs::create_dir_all(dir).map_err(|e| io_error(dir, e))?;
            Ok(missing)
        }
        Err(e) => Err(io_error(dir, e)),
        Ok(_) if holds_only_leftovers(dir)? => Ok(Vec::new()),
        Ok(_) => Err(StoreError::Exists(dir.to_path_buf())),
    }
}

/// `dir` and each of its ancestors that does not exist, nearest first: what
/// `create_dir_all(dir)` creates.
fn missing_dirs(dir: &Path) -> Vec<PathBuf> {
    let absent = |path: &&Path| {
        !path.as_os_str().is_empty()
            && fs::symlink_metadata(path).is_err_and(|e| e.kind() == io::ErrorKind::NotFound)
    };
    dir.ancestors()
        .take_while(absent)
        .map(Path::to_path_buf)
        .collect()
}

/// Whether `dir` is a directory (a symlink followed) holding nothing but
/// [`INIT_LEFTOVERS`]. `S_IO` if it cannot be listed.
fn holds_only_leftovers(dir: &Path) -> Result<bool, StoreError> {
    if !fs::metadata(dir).is_ok_and(|m| m.is_dir()) {
        return Ok(false);
    }
    for entry in fs::read_dir(dir).map_err(|e| io_error(dir, e))? {
        let name = entry.map_err(|e| io_error(dir, e))?.file_name();
        if !INIT_LEFTOVERS.iter().any(|leftover| name == *leftover) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Remove the file at `path` if there is one; `S_IO` if that fails.
fn remove_if_present(path: &Path) -> Result<(), StoreError> {
    match fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(io_error(path, e)),
        _ => Ok(()),
    }
}

/// Steps 3 to 7 of [`SqliteLog::init`]. Once it holds the lease, a failure
/// removes what this call created before the lease is released (dropped
/// when this returns): `LOCK` last, so that no other init takes the lease
/// while these files are still being removed.
fn create(
    dir: &Path,
    created: &[PathBuf],
    genesis: Genesis,
    signer: &dyn Signer,
    clock: Box<dyn Clock>,
) -> Result<SqliteLog, StoreError> {
    let lock = dir.join(LOCK_FILE);
    let made_lock = fs::symlink_metadata(&lock).is_err_and(|e| e.kind() == io::ErrorKind::NotFound);
    let mut lease = match Lease::acquire(dir) {
        Ok(lease) => lease,
        // Another init or writer holds it: nothing here is ours.
        Err(e @ StoreError::Locked { .. }) => return Err(e),
        Err(e) => {
            if made_lock {
                let _ = fs::remove_file(&lock);
            }
            return Err(e);
        }
    };
    match holds_only_leftovers(dir) {
        Ok(true) => {}
        // A store, or something else, appeared since step 2: not ours.
        Ok(false) => return Err(StoreError::Exists(dir.to_path_buf())),
        Err(e) => {
            if made_lock {
                let _ = fs::remove_file(&lock);
            }
            return Err(e);
        }
    }
    let mut placed = false;
    let built = build(
        dir,
        created,
        &mut lease,
        &genesis,
        signer,
        clock.as_ref(),
        &mut placed,
    );
    match built {
        Ok((conn, log_id)) => Ok(SqliteLog {
            conn,
            _lease: lease,
            dir: dir.to_path_buf(),
            log_id,
            genesis,
            epoch: 1,
            clock,
        }),
        Err(e) => {
            // Still under the lease: no other init builds here meanwhile.
            for name in &INIT_LEFTOVERS[1..] {
                let _ = fs::remove_file(dir.join(name));
            }
            if placed {
                for suffix in ["", "-wal", "-shm"] {
                    let _ = fs::remove_file(dir.join(format!("{DB_FILE}{suffix}")));
                }
            }
            if made_lock {
                let _ = fs::remove_file(&lock);
            }
            Err(e)
        }
    }
}

/// Steps 3 (what is left to remove) to 7 of [`SqliteLog::init`], under the
/// lease: the writer's connection to the new `log.db`, and the log id.
/// Sets `placed` once `log.db` is renamed into place. Every connection it
/// opens is closed when it fails.
fn build(
    dir: &Path,
    created: &[PathBuf],
    lease: &mut Lease,
    genesis: &Genesis,
    signer: &dyn Signer,
    clock: &dyn Clock,
    placed: &mut bool,
) -> Result<(Connection, String), StoreError> {
    for name in &INIT_LEFTOVERS[1..] {
        remove_if_present(&dir.join(name))?;
    }
    let now = clock.now();
    let (log_id, plan) = intake::plan_genesis(genesis, signer, now)?;

    let init = dir.join(INIT_FILE);
    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
        | OpenFlags::SQLITE_OPEN_CREATE
        | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let mut conn = Connection::open_with_flags(&init, flags)?;
    conn.execute_batch(WRITER_PRAGMAS)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute_batch(SCHEMA_SQL)?;
    let meta = [
        ("format", FORMAT),
        ("log_id", &log_id),
        ("name", &genesis.name),
        ("store_signer", &genesis.signer),
        ("store_key", &genesis.key),
        ("writer_epoch", "1"),
    ];
    for (key, value) in meta {
        db::insert_meta(&tx, key, value)?;
    }
    Writer::new(&tx)?.apply(&plan)?;
    tx.commit()?;
    // The last connection to close checkpoints the WAL into the file and
    // removes it.
    conn.close().map_err(|(_, e)| StoreError::from(e))?;
    let init_wal = dir.join(format!("{INIT_FILE}-wal"));
    if wal_has_content(&init_wal) {
        return Err(StoreError::Io(format!(
            "{}: the new store's WAL was not checkpointed",
            init_wal.display()
        )));
    }
    for name in &INIT_LEFTOVERS[2..] {
        remove_if_present(&dir.join(name))?;
    }
    sync_file(&init)?;

    let path = dir.join(DB_FILE);
    refuse_existing(&path).map_err(|e| match e {
        StoreError::Exists(_) => StoreError::Exists(dir.to_path_buf()),
        e => e,
    })?;
    fs::rename(&init, &path).map_err(|e| io_error(&path, e))?;
    *placed = true;
    sync_dir(dir)?;
    for made in created {
        sync_dir(parent_of(made))?;
    }

    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(&path, flags)?;
    conn.busy_timeout(BUSY_TIMEOUT)?;
    conn.execute_batch(WRITER_PRAGMAS)?;
    lease.record(1)?;
    Ok((conn, log_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writer_epochs_are_plain_decimal_from_1_to_2_53_minus_1() {
        assert_eq!(parse_epoch("1"), Some(1));
        assert_eq!(parse_epoch("0042"), Some(42));
        assert_eq!(parse_epoch("9007199254740991"), Some(MAX_SIZE));
        for text in [
            "",
            "0",
            "-1",
            "+1",
            " 1",
            "1 ",
            "1.0",
            "1e3",
            "9007199254740992",
        ] {
            assert_eq!(parse_epoch(text), None, "{text:?}");
        }
        assert_eq!(parse_epoch(&"9".repeat(40)), None);
    }

    #[cfg(unix)]
    #[test]
    fn immutable_uris_encode_the_path() {
        assert_eq!(
            immutable_uri(Path::new("/tmp/a b/x?y#z%/log.db")).unwrap(),
            "file:///tmp/a%20b/x%3Fy%23z%25/log.db?mode=ro&immutable=1"
        );
        assert_eq!(
            immutable_uri(Path::new("//srv/log.db")).unwrap(),
            "file:////srv/log.db?mode=ro&immutable=1"
        );
        let relative = immutable_uri(Path::new("store/log.db")).unwrap();
        let cwd = std::env::current_dir().unwrap();
        assert!(relative.starts_with("file:///"), "{relative}");
        assert!(relative.ends_with("/store/log.db?mode=ro&immutable=1"));
        if cwd.to_str().is_some_and(|c| {
            c.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-._~/".contains(&b))
        }) {
            assert_eq!(
                relative,
                format!("file://{}/store/log.db?mode=ro&immutable=1", cwd.display())
            );
        }
        // Non-UTF-8 bytes are encoded as they are.
        use std::os::unix::ffi::OsStrExt;
        let odd = Path::new(std::ffi::OsStr::from_bytes(b"/x\xff/log.db"));
        assert_eq!(
            immutable_uri(odd).unwrap(),
            "file:///x%FF/log.db?mode=ro&immutable=1"
        );
    }

    /// The rule SQLite uses to decide whether a WAL is present.
    #[test]
    fn a_wal_has_content_unless_absent_or_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let wal = tmp.path().join(WAL_FILE);
        assert!(!wal_has_content(&wal));
        fs::write(&wal, b"").unwrap();
        assert!(!wal_has_content(&wal));
        fs::write(&wal, b"x").unwrap();
        assert!(wal_has_content(&wal));
        fs::remove_file(&wal).unwrap();
        fs::create_dir(&wal).unwrap();
        assert!(wal_has_content(&wal), "not a regular file");
        fs::remove_dir(&wal).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(tmp.path().join("absent/wal"), &wal).unwrap();
            assert!(!wal_has_content(&wal), "a symlink to nothing");
        }
    }

    /// `init` syncs the parent of every directory it created (`claim_dir`'s
    /// list), so that the store's own name is durable.
    #[test]
    fn init_lists_the_directories_it_creates() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("a/b/store");
        let expected = vec![dir.clone(), tmp.path().join("a/b"), tmp.path().join("a")];
        assert_eq!(missing_dirs(&dir), expected);
        assert_eq!(claim_dir(&dir).unwrap(), expected);
        assert!(dir.is_dir());
        assert!(missing_dirs(&dir).is_empty());
        assert!(claim_dir(&dir).unwrap().is_empty(), "an empty directory");
        let sibling = tmp.path().join("a/c");
        assert_eq!(claim_dir(&sibling).unwrap(), vec![sibling.clone()]);
        assert_eq!(parent_of(&sibling), tmp.path().join("a"));
        // A bare name lives in the current directory.
        assert_eq!(
            missing_dirs(Path::new("no-such-directory-here")),
            vec![PathBuf::from("no-such-directory-here")]
        );
        // Leftovers of a killed init, and nothing else.
        fs::write(dir.join(LOCK_FILE), b"").unwrap();
        fs::write(dir.join(INIT_FILE), b"x").unwrap();
        assert!(claim_dir(&dir).unwrap().is_empty());
        fs::write(dir.join(DB_FILE), b"").unwrap();
        assert_eq!(claim_dir(&dir).unwrap_err().code_str(), "S_EXISTS");
    }

    #[test]
    fn the_schema_holds_every_trigger_once() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA_SQL).unwrap();
        for name in TRIGGERS {
            let (table, sql) = db::trigger(&conn, name).unwrap().unwrap();
            assert!(name.starts_with(&format!("{table}_no_")), "{name}");
            assert!(sql.unwrap().contains("RAISE(ABORT, 'S_APPEND_ONLY')"));
        }
        let count: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type = 'trigger'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, TRIGGERS.len() as i64);
        let id: i64 = conn
            .pragma_query_value(None, "application_id", |r| r.get(0))
            .unwrap();
        assert_eq!(id, APPLICATION_ID);
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, USER_VERSION);
        assert_eq!(FORMAT, "store/0");
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(
            tables,
            [
                "attestations",
                "checkpoints",
                "entries",
                "heads",
                "meta",
                "objects"
            ]
        );
        assert_eq!(META_KEYS.len(), 6);
    }
}
