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
//!   directory of another user), or whose `log.db` its user does not own,
//!   it reads as an immutable file when it can; see [`SqliteReader::open`].
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
//! 4. Its tables, indexes and views are exactly those [`SCHEMA_SQL`]
//!    creates: each with the same type, name, table and `sql` as in a fresh
//!    in-memory database built from `SCHEMA_SQL`, and no others (no view,
//!    no virtual table); otherwise `S_NOT_A_STORE`. Nothing reads a table
//!    before this, so a `log.db` whose `entries` is a view that never ends
//!    is refused instead of read. Triggers are not part of this check:
//!    `verify_store` reports them (`S_TRIGGERS_MISSING`), and the writer
//!    refuses triggers of its own (see [`SqliteLog::open`]).
//! 5. `meta` has a `format` row, which is [`FORMAT`] (`S_FORMAT` if it is
//!    something else), and rows `log_id`, `name`, `store_key`,
//!    `store_signer` and `writer_epoch`, with `Genesis::new(name, store_key)`
//!    valid, `store_signer` its signer, and `writer_epoch` a decimal integer
//!    from 1 to 2^53 − 1. Any missing row, or any other value, is
//!    `S_NOT_A_STORE`. `log_id` is taken as it is: `verify_store` checks it
//!    against the lines (`S_META_MISMATCH`), and the writer against the
//!    genesis line (see [`SqliteLog::open`]).
//!
//! The log id and genesis these give are cached for [`LogRead::log_id`] and
//! [`LogRead::genesis`]. A reader never reads the lines to open; the writer
//! reads two (see [`SqliteLog::open`]).
//!
//! Every path handed to SQLite is a plain file name: a relative one is
//! given as `./<path>`, because the bundled SQLite takes any name that
//! starts with `file:` as a URI (it is built with `SQLITE_USE_URI`), so a
//! store directory named `file:x` would otherwise open another file.

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
    damaged, CheckpointInfo, Head, Item, LogRead, LogWrite, Receipt, StoreError, StoredAttestation,
    StoredObject,
};
use crate::clock::Clock;
use crate::files::{io_error, parent_of, read_bounded, refuse_existing, sync_dir, sync_file};
use crate::hook;
use crate::intake;
use crate::signer::Signer;
use db::Writer;
use evidence_core::json::Value;
use evidence_log::{parse_line, Body};
use lease::{Lease, LOCK_FILE, LOCK_TEXT_LIMIT};

/// The log file in a store directory.
pub const DB_FILE: &str = "log.db";

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
    ///    not exist it is created, with its parents, and the parent of
    ///    every directory created is synced, so that their names are
    ///    durable. A parent that cannot be synced (one this user can write
    ///    but not read, say) is `S_IO` here, before anything is built.
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
    ///    to replace), then sync `dir`, so that the store's name is durable
    ///    before anything is returned.
    /// 7. Open `log.db` as the writer, run [`WRITER_PRAGMAS`], and write the
    ///    lease text with epoch 1.
    ///
    /// `log.db` only ever names a complete store, so an init killed at any
    /// point leaves either a store or a directory that the next `init`
    /// takes.
    ///
    /// If anything fails after the lease is taken and before step 6 renames
    /// the store, what this call created is removed while it still holds
    /// the lease: [`INIT_FILE`] and its files, and `LOCK` if this call
    /// created it; then the lease is released. Once `log.db` is in place it
    /// is a complete store, which another process may already have opened,
    /// so a failure after the rename (a sync, or opening the writer)
    /// removes nothing: the error is returned and the store stays, as a
    /// killed init would leave it, for [`SqliteLog::open`]. After any
    /// failure, the directories step 2 created are removed if they are
    /// empty, nearest first. Nothing is removed after `S_LOCKED`, after a
    /// lease that could not be taken for another reason, or after the
    /// `S_EXISTS` of step 3: the `LOCK` there may be another init's.
    ///
    /// Another `init` racing this one on the same path gets `S_LOCKED` or
    /// `S_EXISTS`, or, if this one created the directory and removes it as
    /// it fails, `S_IO` (the directory is gone); a retry then creates the
    /// store.
    pub fn init(
        dir: &Path,
        name: &str,
        signer: &dyn Signer,
        clock: Box<dyn Clock>,
    ) -> Result<SqliteLog, StoreError> {
        let genesis = intake::new_genesis(name, signer)?;
        let created = claim_dir(dir)?;
        let result = created
            .iter()
            .try_for_each(|made| sync_dir(parent_of(made)))
            .and_then(|()| create(dir, genesis, signer, clock));
        if result.is_err() {
            // Only if empty: an init that started meanwhile keeps its files.
            for made in &created {
                if fs::remove_dir(made).is_err() {
                    break;
                }
            }
        }
        result
    }

    /// Open the store in `dir` for writing:
    ///
    /// 1. The checks under [Opening a store](self#opening-a-store).
    /// 2. Every trigger in `log.db` is one of [`TRIGGERS`] with the
    ///    `tbl_name` and `sql` that [`SCHEMA_SQL`] gives it (one that is
    ///    missing is allowed, and `verify_store` reports it); otherwise
    ///    `S_IO`. A trigger of another definition could drop or change the
    ///    rows this writer inserts while it reports success.
    /// 3. Line 1 holds a genesis whose name, signer and key are those in
    ///    `meta`, and its entry's `ref` and `log` are `meta.log_id`;
    ///    otherwise `S_IO`. The writer signs checkpoints with the key in
    ///    `meta`, and puts the log id in `meta` into every entry and
    ///    checkpoint it writes, so both must be the log's.
    /// 4. Take the lease (`S_LOCKED` if another writer has it).
    /// 5. Check that `<dir>/log.db` is still the file this connection
    ///    opened (on Unix, the same device and inode): `S_NOT_A_STORE` if
    ///    it is gone, `S_IO` if it is another file. Nothing that follows
    ///    the rules removes or replaces a `log.db`, but a writer must never
    ///    write to a file that no longer has the store's name.
    /// 6. In one transaction, add one to `meta.writer_epoch` (`S_IO` if it
    ///    is below the epoch of the last entry, which only damage gives),
    ///    then rewrite the lease text. The new value is this writer's
    ///    epoch.
    ///
    /// Reads no clock. Steps 1 to 3 take no lease and create no `LOCK`.
    pub fn open(dir: &Path, clock: Box<dyn Clock>) -> Result<SqliteLog, StoreError> {
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let (mut conn, opened) = connect(dir, flags)?;
        let meta = check_store(&conn, dir)?;
        check_writer_triggers(&conn)?;
        check_genesis_line(&conn, &meta)?;
        hook::at("open:checked", dir)?;
        let mut lease = Lease::acquire(dir)?;
        let path = dir.join(DB_FILE);
        match fs::metadata(&path) {
            Ok(now) if same_file(&opened, &now) => {}
            Ok(_) => {
                return Err(StoreError::Io(format!(
                    "{} was replaced while it was opened; open the store again",
                    path.display()
                )))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Err(StoreError::NotAStore(dir.to_path_buf()))
            }
            Err(e) => return Err(io_error(&path, e)),
        }
        conn.execute_batch(WRITER_PRAGMAS)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        // Read again under the lease: a writer may have opened and closed
        // since the checks.
        let current = db::meta(&tx, "writer_epoch")?
            .as_deref()
            .and_then(parse_epoch)
            .ok_or_else(|| StoreError::NotAStore(dir.to_path_buf()))?;
        if let Some(last) = db::last_epoch(&tx)? {
            if current < last {
                return Err(damaged(format!(
                    "meta.writer_epoch is {current}, below the epoch of the last entry, {last}"
                )));
            }
        }
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
    /// store](self#opening-a-store). Takes no lease, changes nothing in the
    /// store, and leaves no file in it that a writer could not remove.
    ///
    /// SQLite reads a database in WAL mode through `log.db-wal` and
    /// `log.db-shm` beside it (beside the file a symlinked `log.db` leads
    /// to), and creates them when they are absent. A file a process creates
    /// belongs to its user and primary group, and a writer that cannot open
    /// them cannot open the store until someone removes them: a member of
    /// the store's group can write a group-writable `log.db`, but the files
    /// it creates (in a directory without the setgid bit) are of its own
    /// group; root's are handed to `log.db`'s owner only after they are
    /// created, so a root reader killed in between leaves files of root.
    /// So a reader lets SQLite create them only when this process can
    /// write `log.db` and its effective user owns it. It first asks SQLite
    /// whether this process can write `log.db`, compares the owner, then:
    ///
    /// - It can and owns it: an ordinary read-only connection. When its
    ///   first read fails for want of write access to `dir`
    ///   (`SQLITE_CANTOPEN` or `SQLITE_READONLY`: a read-only file system,
    ///   or a directory this user cannot write) and the store has no WAL
    ///   content, it reads `log.db` as an immutable file instead (below);
    ///   with WAL content, `S_IO`, as SQLite reports it.
    /// - Otherwise, and the store has no WAL content: `log.db` as an
    ///   immutable file.
    /// - Otherwise, the store has WAL content and `log.db-shm` exists (a
    ///   writer is open, or one did not close): an ordinary read-only
    ///   connection, which reads the WAL, its latest frames included,
    ///   through the files the writer made (SQLite's read-only WAL
    ///   support), and creates nothing, unless that writer closes and
    ///   removes them in the moment between this check and the read. Now
    ///   and then such a read is `S_IO` (`SQLITE_READONLY_CANTINIT`), and a
    ///   retry reads. Without `log.db-shm`: `S_IO`.
    ///
    /// "No WAL content" is `log.db-wal` absent or an empty regular file,
    /// SQLite's own rule: then `log.db` alone holds the whole log and no
    /// writer has it open. Read as an immutable file (URI parameters
    /// `mode=ro&immutable=1`), it creates nothing. A `log.db-wal` or
    /// `log.db-shm` that is there and is not a regular file (a FIFO, say,
    /// whose open would block) is `S_IO` before anything is opened, for the
    /// writer too.
    ///
    /// SQLite does not watch an immutable file, so such a reader watches
    /// for a writer itself. When it opens it notes, by absolute path, the
    /// text and file status of `LOCK` (or its absence) and the file status
    /// of `log.db` (device, inode, size, modification and change times);
    /// after every read transaction it checks that the WAL still holds
    /// nothing and both are unchanged; otherwise that read is `S_IO` (open
    /// a new reader), since what it read may be torn. A writer that opens
    /// the store creates or rewrites `LOCK` before it changes `log.db`, has
    /// `log.db-wal` while open, and changes `log.db` when it closes, so a
    /// writer is noticed even if `LOCK` is then removed, and the paths stay
    /// those of this store if the process changes its working directory.
    pub fn open(dir: &Path) -> Result<SqliteReader, StoreError> {
        let (conn, frozen) = connect_reader(dir)?;
        let checked = check_store(&conn, dir);
        if let Some(frozen) = &frozen {
            frozen.check()?;
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
    /// may not let SQLite create files in the store, or cannot (see
    /// [`open`](Self::open)).
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

    /// True when the triggers in `log.db` are exactly [`TRIGGERS`], each
    /// with the `tbl_name` and `sql` that [`SCHEMA_SQL`] gives it (compared
    /// with a fresh in-memory database built from `SCHEMA_SQL`): none
    /// missing, none changed, and no other trigger, since another trigger
    /// could drop or change what a writer inserts.
    pub fn has_append_only_triggers(&self) -> Result<bool, StoreError> {
        self.read(|conn| Ok(db::schema_rows(conn, true)? == expected_schema()?.triggers))
    }

    /// What `verify_store` checks in the tables beside the lines, in this
    /// reader's snapshot: see [`db::derived`].
    pub(crate) fn derived(&self) -> Result<db::Derived, StoreError> {
        self.read(db::derived)
    }

    /// The genesis as `meta` stores it now (inside a snapshot, as of that
    /// snapshot), or None if a row is missing or does not make a genesis
    /// whose signer is `meta.store_signer`, and `meta.writer_epoch` as
    /// stored, or None.
    pub(crate) fn stored_genesis_and_epoch(
        &self,
    ) -> Result<(Option<Genesis>, Option<u64>), StoreError> {
        self.read(|conn| {
            let name = db::meta(conn, "name")?;
            let key = db::meta(conn, "store_key")?;
            let signer = db::meta(conn, "store_signer")?;
            let genesis = match (name, key, signer) {
                (Some(name), Some(key), Some(signer)) => Genesis::new(&name, &key)
                    .ok()
                    .filter(|genesis| genesis.signer == signer),
                _ => None,
            };
            let epoch = db::meta(conn, "writer_epoch")?
                .as_deref()
                .and_then(parse_epoch);
            Ok((genesis, epoch))
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
            frozen.check()?;
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

/// `path` as a plain file name for SQLite: `./<path>` if it is relative,
/// so that a name starting with `file:` is not read as a URI (see [Opening a
/// store](self#opening-a-store)).
pub(crate) fn plain(path: &Path) -> PathBuf {
    if path.is_relative() {
        Path::new(".").join(path)
    } else {
        path.to_path_buf()
    }
}

/// Open `<dir>/log.db` with `flags`, which never include
/// `SQLITE_OPEN_CREATE`: `S_NOT_A_STORE` if it is not there or not a file,
/// `S_IO` if `log.db-wal` or `log.db-shm` is there and not a regular file
/// (see [`check_wal_files`]). Also returns the file status `log.db` had
/// just before the open.
fn connect(dir: &Path, flags: OpenFlags) -> Result<(Connection, fs::Metadata), StoreError> {
    let path = dir.join(DB_FILE);
    let seen = match fs::metadata(&path) {
        Ok(m) if m.is_file() => m,
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
    };
    check_wal_files(&path)?;
    let conn = Connection::open_with_flags(plain(&path), flags).map_err(|e| engine(e, dir))?;
    // Set before the checks read anything; it writes nothing.
    conn.busy_timeout(BUSY_TIMEOUT)?;
    Ok((conn, seen))
}

/// `S_IO` unless `log.db-wal` and `log.db-shm` beside `db` (beside the file
/// it leads to) are absent or regular files, symlinks followed. SQLite
/// opens them at the first read: a FIFO there would block that open for
/// ever (SQLite opens a file it cannot write read-only, and a FIFO opened
/// read-only waits for a writer), and nothing but a regular file is a WAL.
fn check_wal_files(db: &Path) -> Result<(), StoreError> {
    for suffix in ["-wal", "-shm"] {
        let path = beside(db, suffix).map_err(|e| io_error(db, e))?;
        match fs::metadata(&path) {
            Ok(m) if m.is_file() => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Ok(_) => {
                return Err(StoreError::Io(format!(
                    "{}: not a regular file",
                    path.display()
                )))
            }
            Err(e) => return Err(io_error(&path, e)),
        }
    }
    Ok(())
}

/// Whether this process's effective user owns the file with status `m`.
/// Off Unix there are no owners to tell apart, so it does.
fn owns(m: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // SAFETY: geteuid has no preconditions and cannot fail.
        m.uid() == unsafe { libc::geteuid() }
    }
    #[cfg(not(unix))]
    {
        let _ = m;
        true
    }
}

/// Whether two file statuses are of one file: the same device and inode on
/// Unix. Elsewhere a file that is open cannot be removed, so it is taken
/// to be.
fn same_file(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        a.dev() == b.dev() && a.ino() == b.ino()
    }
    #[cfg(not(unix))]
    {
        let _ = (a, b);
        true
    }
}

/// A reader's connection to `<dir>/log.db`, with [`READER_PRAGMAS`] run:
/// read-only, or immutable when this process may not let SQLite create
/// files in the store (its user does not own `log.db`, or cannot write it
/// or `dir`) and the store has no WAL content (see [`SqliteReader::open`]).
fn connect_reader(dir: &Path) -> Result<(Connection, Option<Frozen>), StoreError> {
    // Asked for read-write, SQLite opens a file this process cannot write
    // read-only, and says so. Opening reads nothing, and creates nothing.
    let probe = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let (conn, seen) = connect(dir, probe)?;
    let may_create = !conn.is_readonly(rusqlite::MAIN_DB)? && owns(&seen);
    drop(conn);
    let db = dir.join(DB_FILE);
    if !may_create {
        let wal = beside(&db, "-wal").map_err(|e| io_error(&db, e))?;
        if !wal_has_content(&wal) {
            return frozen(dir).ok_or_else(|| {
                StoreError::Io(format!(
                    "{}: cannot be read without write access",
                    db.display()
                ))
            });
        }
        let shm = beside(&db, "-shm").map_err(|e| io_error(&db, e))?;
        if fs::symlink_metadata(&shm).is_err() {
            return Err(StoreError::Io(format!(
                "{}: the store has WAL content and no {}, which this process may not create",
                db.display(),
                shm.display()
            )));
        }
    }
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let (conn, _) = connect(dir, flags)?;
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
    frozen(dir).ok_or_else(|| engine(e, dir))
}

/// `log.db` in `dir` as an immutable file, with what [`Frozen`] watches;
/// None if the store has WAL content, or what is to be watched cannot be
/// read.
fn frozen(dir: &Path) -> Option<(Connection, Option<Frozen>)> {
    let frozen = Frozen::new(dir).ok().flatten()?;
    let uri = immutable_uri(&frozen.db)?;
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
        | OpenFlags::SQLITE_OPEN_URI
        | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(uri, flags).ok()?;
    conn.busy_timeout(BUSY_TIMEOUT).ok()?;
    conn.execute_batch(READER_PRAGMAS).ok()?;
    Some((conn, Some(frozen)))
}

/// What a reader of `log.db` as an immutable file checks after each read
/// (see [`SqliteReader::open`]), every path absolute, resolved when the
/// reader opened.
struct Frozen {
    /// `<dir>/LOCK`.
    lock_path: PathBuf,
    /// Its file status and text from before the reader opened; None if
    /// there was no `LOCK`.
    lock: Option<(Stamp, Vec<u8>)>,
    /// The WAL SQLite would use: beside the file `log.db` leads to.
    wal: PathBuf,
    /// `<dir>/log.db`, and its file status from before the reader opened.
    db: PathBuf,
    db_stamp: Stamp,
}

impl Frozen {
    /// What to watch in `dir`, read now, in this order: `LOCK`, then
    /// whether the WAL holds anything (None if it does), then `log.db`. A
    /// writer that opens after `LOCK` is read changes it, or has WAL content,
    /// before it changes `log.db`, and its WAL stays until `log.db` holds
    /// all of it. An error if `LOCK` or `log.db` cannot be read, or the
    /// paths cannot be resolved.
    fn new(dir: &Path) -> io::Result<Option<Frozen>> {
        let dir = std::path::absolute(dir)?;
        let lock_path = dir.join(LOCK_FILE);
        let db = dir.join(DB_FILE);
        let lock = lock_state(&lock_path)?;
        let wal = beside(&db, "-wal")?;
        if wal_has_content(&wal) {
            return Ok(None);
        }
        let db_stamp = Stamp::of(&fs::metadata(&db)?);
        Ok(Some(Frozen {
            lock_path,
            lock,
            wal,
            db,
            db_stamp,
        }))
    }

    /// `S_IO` unless the WAL holds nothing and `LOCK` and `log.db` are as
    /// they were: no writer has opened the store since the reader did.
    fn check(&self) -> Result<(), StoreError> {
        let unchanged = !wal_has_content(&self.wal)
            && lock_state(&self.lock_path).is_ok_and(|lock| lock == self.lock)
            && fs::metadata(&self.db).is_ok_and(|m| Stamp::of(&m) == self.db_stamp);
        if unchanged {
            Ok(())
        } else {
            let dir = parent_of(&self.db);
            Err(StoreError::Io(format!(
                "{}: a writer opened the store while it was read without write access; \
                 open a new reader",
                dir.display()
            )))
        }
    }
}

/// What tells one version of a file from another without reading it:
/// size and modification time, and on Unix the device, inode and change
/// time too.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Stamp {
    len: u64,
    modified: Option<std::time::SystemTime>,
    #[cfg(unix)]
    unix: (u64, u64, i64, i64),
}

impl Stamp {
    fn of(m: &fs::Metadata) -> Stamp {
        Stamp {
            len: m.len(),
            modified: m.modified().ok(),
            #[cfg(unix)]
            unix: {
                use std::os::unix::fs::MetadataExt;
                (m.dev(), m.ino(), m.ctime(), m.ctime_nsec())
            },
        }
    }
}

/// The file status and text of the `LOCK` at `path`, or None if there is
/// none. An error if it cannot be read, is not a regular file (a symlink is
/// not followed) or is longer than any lease text.
fn lock_state(path: &Path) -> io::Result<Option<(Stamp, Vec<u8>)>> {
    let stamp = match fs::symlink_metadata(path) {
        Ok(m) => Stamp::of(&m),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    Ok(Some((stamp, read_bounded(path, LOCK_TEXT_LIMIT, false)?)))
}

/// The file SQLite keeps beside `db` with this suffix (`-wal`, `-shm`):
/// beside the file `db` leads to, symlinks resolved, as SQLite resolves
/// them.
fn beside(db: &Path, suffix: &str) -> io::Result<PathBuf> {
    let mut name = fs::canonicalize(db)?.into_os_string();
    name.push(suffix);
    Ok(PathBuf::from(name))
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
    // Before anything reads a table: a table may be a view that never ends.
    if db::schema_rows(conn, false).map_err(|e| engine(e, dir))? != expected_schema()?.tables {
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

/// The `sqlite_master` rows a store has, as [`SCHEMA_SQL`] creates them in
/// a fresh in-memory database: its tables, indexes and views (step 4 of
/// [Opening a store](self#opening-a-store)), and its triggers. Built once.
pub(crate) struct Schema {
    pub(crate) tables: Vec<db::SchemaRow>,
    pub(crate) triggers: Vec<db::SchemaRow>,
}

/// The [`Schema`] of [`SCHEMA_SQL`].
fn expected_schema() -> Result<&'static Schema, StoreError> {
    static SCHEMA: std::sync::OnceLock<Schema> = std::sync::OnceLock::new();
    if let Some(schema) = SCHEMA.get() {
        return Ok(schema);
    }
    let fresh = Connection::open_in_memory()?;
    fresh.execute_batch(SCHEMA_SQL)?;
    let schema = Schema {
        tables: db::schema_rows(&fresh, false)?,
        triggers: db::schema_rows(&fresh, true)?,
    };
    Ok(SCHEMA.get_or_init(|| schema))
}

/// Step 2 of [`SqliteLog::open`]: `S_IO` unless every trigger in `log.db`
/// is one that [`SCHEMA_SQL`] creates, as it creates it.
fn check_writer_triggers(conn: &Connection) -> Result<(), StoreError> {
    let expected = &expected_schema()?.triggers;
    for row in db::schema_rows(conn, true)? {
        if !expected.contains(&row) {
            return Err(damaged(format!(
                "log.db has a trigger that schema.sql does not create, {}",
                row.name
            )));
        }
    }
    Ok(())
}

/// Step 3 of [`SqliteLog::open`]: `S_IO` unless line 1 holds a genesis
/// object whose content names `meta`'s name, signer and key, and whose
/// entry's `ref` (the object's id) and `log` are `meta`'s log id.
fn check_genesis_line(conn: &Connection, meta: &Meta) -> Result<(), StoreError> {
    let lines = db::lines(conn, 1, 1)?;
    let found = match lines.as_slice() {
        [(1, line)] => parse_line(line.as_bytes()).ok().and_then(|line| {
            let Body::Object(object) = line.body else {
                return None;
            };
            let inline = object.get("content")?.get("inline")?;
            let field = |key: &str| inline.get(key).and_then(Value::as_str).map(String::from);
            Some((
                line.entry.reference,
                line.entry.log,
                field("name")?,
                field("signer")?,
                field("key")?,
            ))
        }),
        _ => None,
    };
    let genesis = &meta.genesis;
    let stored = (
        meta.log_id.clone(),
        meta.log_id.clone(),
        genesis.name.clone(),
        genesis.signer.clone(),
        genesis.key.clone(),
    );
    if found.as_ref() != Some(&stored) {
        return Err(damaged(
            "meta's log_id, name, store_signer and store_key are not those of the genesis \
             at seq 1",
        ));
    }
    Ok(())
}

/// One `meta` value for the open checks, or None when the row is missing or
/// does not hold text. `S_NOT_A_STORE` when `meta` has no `k` or `v`
/// column. (The schema check before it has already required `meta` as
/// `schema.sql` creates it; these cases stay refused if it is ever
/// loosened.)
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
/// before `log.db` is in place removes what this call created before the
/// lease is released (dropped when this returns): `LOCK` last, so that no
/// other init takes the lease while these files are still being removed.
/// A failure after that removes nothing.
fn create(
    dir: &Path,
    genesis: Genesis,
    signer: &dyn Signer,
    clock: Box<dyn Clock>,
) -> Result<SqliteLog, StoreError> {
    let lock = dir.join(LOCK_FILE);
    let made_lock = fs::symlink_metadata(&lock).is_err_and(|e| e.kind() == io::ErrorKind::NotFound);
    // Without the lease nothing here is ours: another init may have created
    // LOCK since it was looked at, and may hold it.
    let mut lease = Lease::acquire(dir)?;
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
        // log.db is a complete store, and may be open elsewhere already.
        Err(e) if placed => Err(e),
        Err(e) => {
            // Still under the lease: no other init builds here meanwhile.
            for name in &INIT_LEFTOVERS[1..] {
                let _ = fs::remove_file(dir.join(name));
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
    let mut conn = Connection::open_with_flags(plain(&init), flags)?;
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
    hook::at("init:placed", dir)?;
    sync_dir(dir)?;

    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(plain(&path), flags)?;
    conn.busy_timeout(BUSY_TIMEOUT)?;
    conn.execute_batch(WRITER_PRAGMAS)?;
    lease.record(1)?;
    Ok((conn, log_id))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;
    use crate::clock::ScriptClock;
    use crate::signer::SeedSigner;

    fn store_key() -> SeedSigner {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-vectors/keys/store-demo.seed");
        SeedSigner::from_file(&path).unwrap()
    }

    /// A clock with one reading, as `init` uses.
    fn one_reading() -> Box<dyn Clock> {
        Box::new(ScriptClock::new(&["2026-11-26T00:00:00Z"]))
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// `init` syncs the parent of every directory it created before it
    /// takes the lease or builds anything, and the store directory once
    /// `log.db` is in place.
    #[test]
    fn init_syncs_before_it_builds() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("a/b/store");
        let synced = Rc::new(RefCell::new(Vec::new()));
        let _hook = hook::set({
            let (synced, lock, db) = (synced.clone(), dir.join(LOCK_FILE), dir.join(DB_FILE));
            move |point, path| {
                if point == "sync" {
                    let seen = (path.to_path_buf(), lock.exists(), db.exists());
                    synced.borrow_mut().push(seen);
                }
                Ok(())
            }
        });
        drop(SqliteLog::init(&dir, "cer-demo", &store_key(), one_reading()).unwrap());
        let parent = |p: &str| (tmp.path().join(p), false, false);
        assert_eq!(
            *synced.borrow(),
            [
                parent("a/b"),
                parent("a"),
                parent(""),
                (dir.clone(), true, true)
            ]
        );
    }

    /// A parent that cannot be synced (in a directory this user can write
    /// but not read, say) fails `init` before the lease is taken or
    /// anything built, and every directory it created is removed again: a
    /// retry fails the same way, instead of succeeding once a directory is
    /// left from the first try.
    #[test]
    fn init_fails_before_building_when_a_parent_cannot_be_synced() {
        let tmp = tempfile::tempdir().unwrap();
        let top = tmp.path().to_path_buf();
        let dir = top.join("x/store");
        let _hook = hook::set({
            let top = top.clone();
            move |point, path| {
                if point == "sync" && path == top {
                    return Err(StoreError::Io("injected: permission denied".into()));
                }
                Ok(())
            }
        });
        for _ in 0..2 {
            let e = SqliteLog::init(&dir, "cer-demo", &store_key(), one_reading());
            assert_eq!(e.err().unwrap().code_str(), "S_IO");
            assert!(names(&top).is_empty());
        }
    }

    /// Once `log.db` is in place it is a complete store, which another
    /// process may already have open: a failure after that removes nothing,
    /// so what that process reads and writes stays the store's.
    #[test]
    fn a_failure_after_the_store_is_placed_leaves_the_store() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("store");
        let opened = Rc::new(RefCell::new(None));
        let _hook = hook::set({
            let opened = opened.clone();
            move |point, path| {
                if point == "init:placed" {
                    *opened.borrow_mut() = Some(SqliteReader::open(path)?);
                    return Err(StoreError::Io("injected: the sync failed".into()));
                }
                Ok(())
            }
        });
        let e = SqliteLog::init(&dir, "cer-demo", &store_key(), one_reading());
        assert_eq!(e.err().unwrap().code_str(), "S_IO");
        assert!(names(&dir).contains(&DB_FILE.to_string()));
        let reader = opened.borrow_mut().take().unwrap();
        assert_eq!(reader.head().unwrap().seq, 2);
        assert!(crate::verify_store(&dir, Default::default()).unwrap().ok());
        let log = SqliteLog::open(&dir, Box::new(ScriptClock::default())).unwrap();
        assert_eq!(log.epoch(), 2);
        assert_eq!(log.head().unwrap(), reader.head().unwrap());
    }

    /// A lease that cannot be taken for a reason other than another holder
    /// (too many open files, say) removes nothing: the `LOCK` there may be
    /// one that a racing init created after this one looked, and holds.
    #[test]
    fn an_init_that_cannot_take_the_lease_removes_no_lock() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("store");
        fs::create_dir(&dir).unwrap();
        let racing = Rc::new(RefCell::new(None));
        let hook = hook::set({
            let (racing, dir) = (racing.clone(), dir.clone());
            move |point, _| {
                if point == "lease:open" && racing.borrow().is_none() {
                    *racing.borrow_mut() = Some(Lease::acquire(&dir)?);
                    return Err(StoreError::Io("injected: too many open files".into()));
                }
                Ok(())
            }
        });
        let e = SqliteLog::init(&dir, "cer-demo", &store_key(), one_reading());
        assert_eq!(e.err().unwrap().code_str(), "S_IO");
        drop(hook);
        assert_eq!(names(&dir), [LOCK_FILE]);
        let e = Lease::acquire(&dir).unwrap_err();
        assert_eq!(e.code_str(), "S_LOCKED", "the racing init still holds LOCK");
        drop(racing.borrow_mut().take());
        drop(SqliteLog::init(&dir, "cer-demo", &store_key(), one_reading()).unwrap());
    }

    /// The writer writes only to the file the store's name leads to once it
    /// holds the lease: a `log.db` replaced, or removed, after the open
    /// checks is refused.
    #[cfg(unix)]
    #[test]
    fn open_refuses_a_log_db_replaced_before_the_lease() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("store");
        drop(SqliteLog::init(&dir, "cer-demo", &store_key(), one_reading()).unwrap());
        let db = dir.join(DB_FILE);
        let old = dir.join("log.db.old");
        for (epoch, replace, code) in [(2, true, "S_IO"), (3, false, "S_NOT_A_STORE")] {
            let hook = hook::set({
                let (db, old) = (db.clone(), old.clone());
                move |point, _| {
                    if point == "open:checked" {
                        fs::rename(&db, &old).unwrap();
                        if replace {
                            fs::copy(&old, &db).unwrap();
                        }
                    }
                    Ok(())
                }
            });
            let e = SqliteLog::open(&dir, Box::new(ScriptClock::default()));
            assert_eq!(e.err().unwrap().code_str(), code);
            drop(hook);
            if replace {
                fs::remove_file(&old).unwrap();
            } else {
                fs::rename(&old, &db).unwrap();
            }
            let log = SqliteLog::open(&dir, Box::new(ScriptClock::default())).unwrap();
            assert_eq!(log.epoch(), epoch, "the refused open wrote no epoch");
        }
    }

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
        let wal = tmp.path().join("log.db-wal");
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

    /// A `log.db-wal` or `log.db-shm` that is a FIFO is `S_IO` for every
    /// reader and the writer, at once: SQLite opens it read-only when it
    /// cannot write it, which for a FIFO waits for a writer that never
    /// comes. As root (who can write the FIFO) it is shown with a `log.db`
    /// of another user too, which root reads without creating anything.
    #[cfg(unix)]
    #[test]
    fn wal_files_that_are_not_regular_files_are_refused() {
        use std::os::unix::ffi::OsStrExt;
        use std::time::Duration;
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("store");
        drop(SqliteLog::init(&dir, "cer-demo", &store_key(), one_reading()).unwrap());
        let db = dir.join(DB_FILE);
        let mkfifo = |path: &Path| {
            let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
            // SAFETY: a valid NUL-terminated path.
            assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o644) }, 0);
        };
        // Each open on a thread of its own, given 30 seconds: one that
        // blocks fails the test instead of hanging it.
        let code = |open: fn(&Path) -> Result<(), StoreError>| {
            let (tx, rx) = std::sync::mpsc::channel();
            let dir = dir.clone();
            std::thread::spawn(move || {
                let _ = tx.send(open(&dir).err().map(|e| e.code_str()));
            });
            rx.recv_timeout(Duration::from_secs(30))
                .expect("the open blocked")
        };
        let reader: fn(&Path) -> Result<(), StoreError> = |dir| SqliteReader::open(dir).map(drop);
        let writer: fn(&Path) -> Result<(), StoreError> =
            |dir| SqliteLog::open(dir, Box::new(ScriptClock::default())).map(drop);
        let mut given_away = false;
        for other_owner in [false, true] {
            if other_owner {
                // Only root can give a file to another user.
                use std::os::unix::fs::MetadataExt;
                let other = if fs::metadata(&db).unwrap().uid() == 65534 {
                    65533
                } else {
                    65534
                };
                given_away = std::os::unix::fs::chown(&db, Some(other), Some(other)).is_ok();
                if !given_away {
                    break;
                }
            }
            for (fifo, regular) in [("-wal", None), ("-shm", Some(("-wal", vec![0x5a; 32])))] {
                let fifo = dir.join(format!("{DB_FILE}{fifo}"));
                mkfifo(&fifo);
                if let Some((suffix, bytes)) = &regular {
                    fs::write(dir.join(format!("{DB_FILE}{suffix}")), bytes).unwrap();
                }
                let case = format!("{} (another owner: {other_owner})", fifo.display());
                assert_eq!(code(reader), Some("S_IO"), "reader: {case}");
                let found = crate::verify_store(&dir, Default::default()).err();
                assert_eq!(found.map(|e| e.code_str()), Some("S_IO"), "{case}");
                assert_eq!(code(writer), Some("S_IO"), "writer: {case}");
                fs::remove_file(&fifo).unwrap();
                if let Some((suffix, _)) = &regular {
                    fs::remove_file(dir.join(format!("{DB_FILE}{suffix}"))).unwrap();
                }
                assert_eq!(names(&dir), [LOCK_FILE, DB_FILE], "{case}");
            }
            assert_eq!(code(reader), None);
            // Settle what a reader that owns log.db may leave.
            assert_eq!(code(writer), None);
        }
        // Without a FIFO, a reader that owns log.db reads as usual, and
        // one that does not reads an immutable file and creates nothing.
        let reader = SqliteReader::open(&dir).unwrap();
        assert_eq!(reader.is_immutable(), given_away);
        assert_eq!(reader.head().unwrap().seq, 2);
        drop(reader);
        if given_away {
            assert_eq!(names(&dir), [LOCK_FILE, DB_FILE]);
        }
    }

    /// `claim_dir` lists the directories it creates, whose parents `init`
    /// syncs (see `init_syncs_before_it_builds`).
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
        let triggers = db::schema_rows(&conn, true).unwrap();
        assert_eq!(triggers, expected_schema().unwrap().triggers);
        let mut names: Vec<&str> = triggers.iter().map(|t| t.name.as_str()).collect();
        names.sort_unstable();
        let mut want = TRIGGERS.to_vec();
        want.sort_unstable();
        assert_eq!(names, want);
        for t in &triggers {
            assert_eq!(t.kind, "trigger");
            assert!(
                t.name.starts_with(&format!("{}_no_", t.table)),
                "{}",
                t.name
            );
            let sql = t.sql.as_deref().unwrap();
            assert!(sql.contains("RAISE(ABORT, 'S_APPEND_ONLY')"));
        }
        // Tables and indexes, with the autoindexes of their keys; no views.
        let tables = &expected_schema().unwrap().tables;
        assert_eq!(tables, &db::schema_rows(&conn, false).unwrap());
        assert!(tables
            .iter()
            .all(|t| t.kind == "table" || t.kind == "index"));
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
