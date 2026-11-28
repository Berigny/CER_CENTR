//! The store's programming interface: the read and write traits every log
//! implements, the items a writer takes, the receipts it gives back, and its
//! errors.
//!
//! The normative rules for any writer are in `spec/store-0.md` ("Intake",
//! "Checkpoint and tree", "Two clocks"). This page adds the rules of this
//! crate's writers, [`MemLog`](crate::mem::MemLog) and
//! [`SqliteLog`](crate::sqlite::SqliteLog). Given the same calls and the same
//! clock readings, the two write byte-identical logs, and both match the
//! JavaScript reference writer (`tools/reference/log.mjs`, `Writer`) on every
//! `inputs.json` script. Both run their checks through [`crate::intake`], so
//! the rules live in one place.
//!
//! # Transactions and the clock
//!
//! - `init`, [`LogWrite::commit`] and [`LogWrite::checkpoint`] are each one
//!   transaction: everything lands or nothing does.
//! - Each reads the clock **exactly once**.
//!   - `commit` reads it first, before any intake check. A batch that is
//!     rejected, that holds only duplicates, or that is empty still uses one
//!     reading. The reference writer also reads its clock before it checks an
//!     item.
//!   - `init` and `checkpoint` read it once their argument checks pass. A
//!     refusal with `E_USAGE` or `S_EXISTS` from `init`, or with
//!     `S_STORE_KEY` because the signer's public key is not the genesis key,
//!     reads nothing.
//!   - Opening a store, [`MemLog::reopen`](crate::mem::MemLog::reopen),
//!     export, import, verification and every [`LogRead`] method read no
//!     clock.
//! - That one reading `now` gives every new entry of the transaction the same
//!   `received_at`: `format_millis(next_received(now, last))`, where `last`
//!   is the `received_at` of the log's last entry before the transaction
//!   (none for `init`). It also gives store-authored objects their `at`,
//!   through `service_at` (below).
//!
//! # Batches
//!
//! [`LogWrite::commit`] checks its items in batch order, each with the
//! intake steps of `spec/store-0.md` against the log as it would be with the
//! batch's earlier accepted items appended:
//!
//! - a `prev`, or an attestation's `object`, may name an object earlier in
//!   the batch;
//! - `if_heads` is compared with `heads(subject, lane)` after the heads
//!   changes of the earlier items. It is compared as a set: order and
//!   repeated ids do not matter, and `Some(&[])` means the subject and lane
//!   must have no heads. `None` means no comparison;
//! - an item whose ref is already held, by the log or by an earlier item of
//!   the batch, gets that entry's receipt with `duplicate: true`. It adds no
//!   entry and does not fail the batch, and its `if_heads` is not evaluated;
//! - the first item that is rejected fails the whole batch:
//!   [`StoreError::Rejected`], with [`Rejected::index`] its position in the
//!   batch, counting from 0. Nothing is written and later items are not
//!   checked. [`Rejected::heads`] holds the current heads, sorted, for
//!   `S_STALE_HEADS` only; for every other code it is empty;
//! - otherwise the new entries take consecutive seqs in batch order, and the
//!   receipts come back one per item, in batch order.
//!
//! A batch of one item is exactly one `append`, `attest` or `append_if_heads`
//! call, and exactly one operation of an `inputs.json` script.
//!
//! # Creating a store
//!
//! `MemLog::init` and `SqliteLog::init`, through
//! [`intake::plan_genesis`](crate::intake::plan_genesis):
//!
//! 1. `E_USAGE` unless `name` is a store name and `signer.public_key()` a
//!    store key (`evidence_log::Genesis::new`). `SqliteLog::init` then gives
//!    `S_EXISTS` unless its directory is absent, empty, or holds only what
//!    an init that was killed leaves (see `SqliteLog::init`).
//! 2. Read the clock once. `at` = `format_millis(service_at(now, []))`, and
//!    `received_at` = `format_millis(next_received(now, None))`, the same
//!    text.
//! 3. The genesis is `Genesis::new(name, key).object(at)`; its id is the log
//!    id. The author attestation is `signer.sign(log id, "author", genesis
//!    signer, at)`, checked as below.
//! 4. One transaction, at epoch 1: seq 1 the genesis (its entry's `log`,
//!    `ref` and `prev` are all the log id), seq 2 the author attestation
//!    (`prev` the entry hash of seq 1), and the genesis's heads row (lane
//!    `assert:evidence-store/genesis`).
//!
//! # Checkpoints
//!
//! [`LogWrite::checkpoint`], through
//! [`intake::plan_checkpoint`](crate::intake::plan_checkpoint):
//!
//! 1. `S_STORE_KEY` if `signer.public_key()` is not the genesis key. Nothing
//!    is read or written, the clock included.
//! 2. Read the clock once.
//! 3. `size` = the seq of the log's last entry, `head` = its entry hash, and
//!    `root` = the tree hash over leaves 1 to `size`: every entry before the
//!    checkpoint's own.
//! 4. The previous checkpoint is the one with the highest seq (the last of
//!    [`LogRead::checkpoints`]), if any. `at` =
//!    `format_millis(service_at(now, [previous at]))`, or
//!    `service_at(now, [])` for the first checkpoint. `prev` = `[previous
//!    id]`, or `[]`.
//! 5. The object is `Checkpoint { log, size, head, root }.object(genesis, at,
//!    prev)`. The witness is `signer.sign(id, "witness", genesis signer,
//!    at)`, checked as below.
//! 6. One transaction: seq `size` + 1 the checkpoint (its heads change is in
//!    lane `assert:evidence-store/checkpoint`), seq `size` + 2 the witness,
//!    both with the same `received_at`, and the `checkpoints` row (seq
//!    `size` + 1, id, size, root, head).
//!
//! A checkpoint may be written at any time, directly after another included:
//! the previous checkpoint's witness is always a newer entry, so `size` then
//! grows by 2. The reference writer does the same.
//!
//! **What a signer returns is checked.** The line must parse as
//! attestation/0, its signature must verify, and it must attest exactly the
//! object, purpose, signer and `at` asked for, with the genesis key.
//! Otherwise the request fails with `S_STORE_KEY` and nothing is written.
//! The line is stored in canonical form, whatever form the signer returned.
//!
//! # Epochs and reopening
//!
//! - A new store's writer has epoch 1.
//! - [`SqliteLog::open`](crate::sqlite::SqliteLog::open) takes the lease,
//!   then adds one to `meta.writer_epoch` in its own transaction. The result
//!   is the epoch of every entry that writer appends.
//! - [`MemLog::reopen`](crate::mem::MemLog::reopen) models closing the
//!   writer and opening it again: the epoch goes up by one. It reads no clock
//!   and writes nothing.
//! - [`import`](crate::import::import) sets `writer_epoch` to the highest
//!   epoch in the export, so the next writer appends at that epoch + 1.
//! - A [`ScriptClock`](crate::clock::ScriptClock) survives a reopen: its
//!   clones share one queue of readings. A replay hands each writer a clone,
//!   and keeps one to check that every reading was used.
//!
//! # Prev lookups
//!
//! `MemLog`, `SqliteLog` and `SqliteReader` implement
//! `evidence_core::verify::PrevLookup`, for `evidence verify --store` and
//! `store get`: `prev_at(id)` is the `at` of the held object with that id,
//! parsed, or `None` when no object with that id is held. `PrevLookup` has no
//! error channel, so in the SQLite types a storage error also reads as `None`
//! (not held). A caller that must tell the two apart asks
//! [`LogRead::object`] first.
//!
//! Intake does not use these impls: it resolves every `prev` through
//! [`intake::IntakeState`](crate::intake::IntakeState), which does report
//! storage errors.
//!
//! # Errors
//!
//! Every [`StoreError`] has a stable code, [`StoreError::code_str`]:
//!
//! | Error | Code | Returned when |
//! |---|---|---|
//! | [`Rejected`](StoreError::Rejected) | the item's code ([`Rejected::code_str`]) | Intake refused an item of a batch. |
//! | [`StoreKey`](StoreError::StoreKey) | `S_STORE_KEY` | The signer's public key is not the genesis key, or what it signed does not verify as the genesis key (see above). |
//! | [`Locked`](StoreError::Locked) | `S_LOCKED` | Another writer holds `<dir>/LOCK`. |
//! | [`NotAStore`](StoreError::NotAStore) | `S_NOT_A_STORE` | Opening a directory with no `log.db`; a `log.db` that is not an SQLite database or whose `application_id` is not `0x45564430`; tables, indexes or views that are not exactly the schema's (a view, an extra table, a table changed in place); a `meta` row that is missing, a stored name or key that is not valid, a `store_signer` that is not the genesis signer, or a `writer_epoch` that is not a decimal integer from 1 to 2^53 − 1. A `log.db` gone by the time the writer holds the lease. An export directory with no `manifest.json`. |
//! | [`Exists`](StoreError::Exists) | `S_EXISTS` | `init` into a path that exists and is neither an empty directory nor one holding only what a killed init leaves; `export` to, or `import` into, a path that exists; 1000 `<path>.partial-<pid>-<n>` directories that all exist. |
//! | [`Format`](StoreError::Format) | `S_FORMAT` | `user_version` is not 1, or `meta.format` is not `store/0`. |
//! | [`Usage`](StoreError::Usage) | `E_USAGE` | An invalid argument: a store name, a seed, a signer argument, a `segment_size` outside 1 to 2^53 − 1, a proof position outside the log, `leaf_hashes` beyond the head, a path with no file name. As in the bindings, this is evidence/0's existing `E_USAGE`. |
//! | [`Sqlite`](StoreError::Sqlite) | `S_IO` | The storage engine failed. |
//! | [`Io`](StoreError::Io) | `S_IO` | A file operation failed (an export's `manifest.json` that is not a regular file, or holds more than its size says, included; a `LOCK` that is a symlink, a hard link or not a regular file; a `log.db-wal` or `log.db-shm` that is not a regular file), or stored content an operation must read is damaged (for example a line a proof or an export needs, a missing seq below the head in `leaf_hashes`, a checkpoint of a size a proof covers whose root is not the computed one, or whose row is not what its checkpoint object signed, or a stored `log_id` that is not a hash). `verify_store` reports such damage as findings instead. The writer refuses damage it would carry into the log: a trigger the schema does not create, `meta` (log id, name, signer or key) that disagrees with the genesis line, a `writer_epoch` below the last entry's epoch, a last entry whose columns are not those of its entry text (its `epoch` included), and a `log.db` replaced before it took the lease. Also: a log already at seq 2^53 − 1, an open that would take `writer_epoch` past 2^53 − 1, a clock reading outside the years 0000 to 9999, and a read by a `SqliteReader` that reads a store it cannot write as an immutable file after a writer opened that store. |

use std::fmt;
use std::path::PathBuf;

use evidence_core::json::Value;
use evidence_core::Code;
use evidence_log::{Digest, Genesis, Kind, SubjectKey};

use crate::signer::Signer;

/// The last entry of a log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Head {
    pub seq: u64,
    pub entry_hash: String,
}

impl Head {
    /// `{"seq":…,"entry_hash":"sha256:…"}`, as in a report's `head`.
    pub fn to_value(&self) -> Value {
        Value::Object(vec![
            ("seq".into(), Value::Number(self.seq as f64)),
            ("entry_hash".into(), Value::String(self.entry_hash.clone())),
        ])
    }
}

/// What a writer gives back for an item it accepted.
///
/// For a duplicate, every field but `duplicate` is that of the entry that
/// already holds the item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Receipt {
    pub kind: Kind,
    /// The entry's `ref`: the object id, or the attestation's hash.
    pub reference: String,
    pub seq: u64,
    pub entry_hash: String,
    pub received_at: String,
    /// True when the item was already held and nothing was appended.
    pub duplicate: bool,
}

impl Receipt {
    /// `{"kind","ref","seq","entry_hash","received_at","duplicate"}`.
    pub fn to_value(&self) -> Value {
        Value::Object(vec![
            ("kind".into(), Value::String(self.kind.as_str().into())),
            ("ref".into(), Value::String(self.reference.clone())),
            ("seq".into(), Value::Number(self.seq as f64)),
            ("entry_hash".into(), Value::String(self.entry_hash.clone())),
            (
                "received_at".into(),
                Value::String(self.received_at.clone()),
            ),
            ("duplicate".into(), Value::Bool(self.duplicate)),
        ])
    }
}

/// A held object, as stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredObject {
    pub id: String,
    pub seq: u64,
    /// Its entry's `received_at`.
    pub received_at: String,
    /// The canonical object.
    pub bytes: String,
}

/// A held attestation, as stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredAttestation {
    /// Its `ref`: `"sha256:"` + hex SHA-256 of the canonical line.
    pub hash: String,
    pub seq: u64,
    /// The canonical attestation/0 line, without LF.
    pub line: String,
}

/// One checkpoint: a row of the `checkpoints` projection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckpointInfo {
    /// The checkpoint object's id.
    pub id: String,
    /// The seq of the checkpoint object (its witness is at seq + 1).
    pub seq: u64,
    /// The number of entries it covers.
    pub size: u64,
    /// The tree hash over entries 1 to `size`, `"sha256:"` + hex.
    pub root: String,
    /// The entry hash of seq = `size`.
    pub head: String,
}

impl CheckpointInfo {
    /// `{"checkpoint":"<id>","seq":…,"size":…,"root":"…","head":"…"}`, the
    /// output of `evidence store checkpoint`.
    pub fn to_value(&self) -> Value {
        Value::Object(vec![
            ("checkpoint".into(), Value::String(self.id.clone())),
            ("seq".into(), Value::Number(self.seq as f64)),
            ("size".into(), Value::Number(self.size as f64)),
            ("root".into(), Value::String(self.root.clone())),
            ("head".into(), Value::String(self.head.clone())),
        ])
    }
}

/// One item of a batch.
#[derive(Clone, Copy, Debug)]
pub enum Item<'a> {
    /// An evidence/0 object, as bytes (any valid JSON spelling; it is stored
    /// in canonical form).
    Object {
        bytes: &'a [u8],
        /// When given, `heads(subject, lane)` must equal this set exactly
        /// (empty: no heads), or the batch fails with `S_STALE_HEADS`.
        if_heads: Option<&'a [String]>,
    },
    /// One attestation/0 line, without its LF (any valid JSON spelling; it
    /// is stored in canonical form, and its ref is the hash of that form).
    Attestation { line: &'a [u8] },
}

/// Why intake refused an item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RejectCode {
    /// An evidence/0 result: `E_PARSE`, `E_NUMBER_RANGE`, `E_DUPLICATE_KEY`,
    /// `E_SHAPE`, `E_TIMESTAMP`, `E_ID_MISMATCH`, `E_CONTENT_HASH_MISMATCH`,
    /// `E_PREV_MISSING`, `E_PREV_ORDER`, `E_ATTESTATION_SHAPE` or
    /// `E_SIGNATURE_INVALID`, returned unchanged.
    Core(Code),
    /// `S_STALE_HEADS`: `if_heads` differs from the current heads.
    StaleHeads,
    /// `S_RESERVED_SUBJECT`: an object whose `subject.system` is
    /// `"evidence-store"`.
    ReservedSubject,
    /// `S_ATTESTATION_TARGET`: the attestation's object is not held.
    AttestationTarget,
}

impl RejectCode {
    /// The code as written: the `E_` code, or the `S_` code.
    pub fn as_str(self) -> &'static str {
        match self {
            RejectCode::Core(code) => code.as_str(),
            RejectCode::StaleHeads => "S_STALE_HEADS",
            RejectCode::ReservedSubject => "S_RESERVED_SUBJECT",
            RejectCode::AttestationTarget => "S_ATTESTATION_TARGET",
        }
    }
}

/// An item intake refused, which failed its whole batch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rejected {
    /// The item's position in the batch, counting from 0.
    pub index: usize,
    pub code: RejectCode,
    /// Human-readable detail, for standard error. Never part of any output
    /// that implementations must agree on.
    pub detail: String,
    /// For `S_STALE_HEADS`, the current heads of the item's subject and lane,
    /// sorted. Empty for every other code.
    pub heads: Vec<String>,
}

impl Rejected {
    /// A rejection with no heads.
    pub fn new(index: usize, code: RejectCode, detail: impl Into<String>) -> Rejected {
        Rejected {
            index,
            code,
            detail: detail.into(),
            heads: Vec::new(),
        }
    }

    /// The code as written: the `E_` code, or `S_STALE_HEADS`,
    /// `S_RESERVED_SUBJECT` or `S_ATTESTATION_TARGET`.
    pub fn code_str(&self) -> &'static str {
        self.code.as_str()
    }
}

/// Everything a store operation can fail with. See the [module
/// docs](self#errors) for when each is returned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoreError {
    /// Intake refused an item; nothing in its batch was written.
    Rejected(Rejected),
    /// `S_STORE_KEY`: the signer is not the store key.
    StoreKey,
    /// `S_LOCKED`: another writer holds the lease. `holder` is the text of
    /// `<dir>/LOCK` (`"pid <pid>\nepoch <n>\n"`), for diagnostics only.
    Locked { holder: String },
    /// `S_NOT_A_STORE`: this path holds no store (or no export).
    NotAStore(PathBuf),
    /// `S_EXISTS`: this path already exists.
    Exists(PathBuf),
    /// `S_FORMAT`: a format version this implementation does not support.
    Format(String),
    /// `E_USAGE`: an invalid argument.
    Usage(String),
    /// `S_IO`: the storage engine failed.
    Sqlite(String),
    /// `S_IO`: a file operation failed, or stored content is damaged.
    Io(String),
}

impl StoreError {
    /// The stable code: the rejected item's code, `S_STORE_KEY`, `S_LOCKED`,
    /// `S_NOT_A_STORE`, `S_EXISTS`, `S_FORMAT`, `E_USAGE` or `S_IO`.
    pub fn code_str(&self) -> &'static str {
        match self {
            StoreError::Rejected(r) => r.code_str(),
            StoreError::StoreKey => "S_STORE_KEY",
            StoreError::Locked { .. } => "S_LOCKED",
            StoreError::NotAStore(_) => "S_NOT_A_STORE",
            StoreError::Exists(_) => "S_EXISTS",
            StoreError::Format(_) => "S_FORMAT",
            StoreError::Usage(_) => "E_USAGE",
            StoreError::Sqlite(_) | StoreError::Io(_) => "S_IO",
        }
    }
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let code = self.code_str();
        match self {
            StoreError::Rejected(r) => write!(f, "{code}: item {}: {}", r.index, r.detail),
            StoreError::StoreKey => write!(f, "{code}: the signer is not the store key"),
            StoreError::Locked { holder } => {
                let holder = holder.split_whitespace().collect::<Vec<_>>().join(" ");
                write!(f, "{code}: another writer holds the store ({holder})")
            }
            StoreError::NotAStore(path) => write!(f, "{code}: {} is not a store", path.display()),
            StoreError::Exists(path) => write!(f, "{code}: {} already exists", path.display()),
            StoreError::Format(detail) | StoreError::Usage(detail) | StoreError::Io(detail) => {
                write!(f, "{code}: {detail}")
            }
            StoreError::Sqlite(detail) => write!(f, "{code}: storage engine: {detail}"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<Rejected> for StoreError {
    fn from(r: Rejected) -> StoreError {
        StoreError::Rejected(r)
    }
}

impl From<std::io::Error> for StoreError {
    /// `S_IO`. Prefer adding the path with `map_err` where one is known.
    fn from(e: std::io::Error) -> StoreError {
        StoreError::Io(e.to_string())
    }
}

impl From<rusqlite::Error> for StoreError {
    /// `S_IO`.
    fn from(e: rusqlite::Error) -> StoreError {
        StoreError::Sqlite(e.to_string())
    }
}

/// `S_IO` for stored content that an operation must read and finds
/// damaged (a missing seq, a line that does not parse): `"<detail>: verify
/// the store"`. Export and proofs give this; `verify_store` reports the same
/// damage as findings.
pub(crate) fn damaged(detail: impl fmt::Display) -> StoreError {
    StoreError::Io(format!("{detail}: verify the store"))
}

/// Reading a log. Implemented by [`MemLog`](crate::mem::MemLog),
/// [`SqliteLog`](crate::sqlite::SqliteLog) and
/// [`SqliteReader`](crate::sqlite::SqliteReader).
///
/// Every method reads what the log holds now. A log only grows, so anything
/// read about seqs up to a [`head`](Self::head) read earlier stays true.
/// `SqliteReader` runs each call in its own read transaction;
/// [`SqliteReader::snapshot`](crate::sqlite::SqliteReader::snapshot) runs
/// several in one.
pub trait LogRead {
    /// The log id. For the SQLite types this is `meta.log_id`, which
    /// `verify_store` checks against the lines (`S_META_MISMATCH`).
    fn log_id(&self) -> &str;

    /// The store's name, signer and key. For the SQLite types these come
    /// from `meta` (`name`, `store_key`), not from the genesis line.
    fn genesis(&self) -> &Genesis;

    /// The last entry: the highest seq and its entry hash.
    fn head(&self) -> Result<Head, StoreError>;

    /// `(seq, line)` for each entry with seq ≥ `from_seq`, in seq order, at
    /// most `limit` of them (none past the head, none for `limit` 0). A line
    /// is the canonical segment line without its LF: `{"entry":E,"object":O}`
    /// or `{"attestation":A,"entry":E}`, built from the stored canonical
    /// texts with `evidence_log::render_line`.
    ///
    /// In an undamaged log the seqs are consecutive. A SQLite store whose
    /// append-only triggers were removed may have gaps, and the seqs show
    /// where. An entry whose object or attestation row is missing is given as
    /// the line `{"entry":E}`, which verification reports as `S_LINE_INVALID`.
    fn lines(&self, from_seq: u64, limit: usize) -> Result<Vec<(u64, String)>, StoreError>;

    /// The held object with this id, or None.
    fn object(&self, id: &str) -> Result<Option<StoredObject>, StoreError>;

    /// The held attestations whose `object` is `object_id`, in seq order.
    fn attestations(&self, object_id: &str) -> Result<Vec<StoredAttestation>, StoreError>;

    /// `heads(subject, lane)`, sorted (byte order of the ids). Empty if the
    /// subject and lane have no heads.
    fn heads(&self, subject: &SubjectKey, lane: &str) -> Result<Vec<String>, StoreError>;

    /// Every head as (subject key, lane, id), sorted by all three in byte
    /// order: exactly what `evidence_log::Heads::rows` gives for the fold
    /// over the lines. The projection-drift check compares the two.
    fn head_rows(&self) -> Result<Vec<(SubjectKey, String, String)>, StoreError>;

    /// Every checkpoint, in seq order. The last is the latest.
    fn checkpoints(&self) -> Result<Vec<CheckpointInfo>, StoreError>;

    /// The leaf hashes, SHA-256(0x00 ‖ canonical entry), of the entries with
    /// seq 1 to `upto`, in seq order. `E_USAGE` if `upto` is above the
    /// head's seq; `upto` 0 gives none. `S_IO` if a seq up to `upto` is
    /// missing (only a damaged SQLite store can lack one).
    fn leaf_hashes(&self, upto: u64) -> Result<Vec<Digest>, StoreError>;
}

/// Writing a log: one writer, one transaction per call. See the [module
/// docs](self) for the rules.
pub trait LogWrite: LogRead {
    /// The writer epoch stamped into every entry this writer appends.
    fn epoch(&self) -> u64;

    /// Check `items` in order and append them in one transaction: every item
    /// lands, or none does. Reads the clock exactly once, first. Returns one
    /// receipt per item, in batch order. See [Batches](self#batches).
    fn commit(&mut self, items: &[Item<'_>]) -> Result<Vec<Receipt>, StoreError>;

    /// Append one object: a batch of one.
    fn append(&mut self, object: &[u8]) -> Result<Receipt, StoreError> {
        let receipts = self.commit(&[Item::Object {
            bytes: object,
            if_heads: None,
        }])?;
        single(receipts)
    }

    /// Append one object if `heads(subject, lane)` equals `expected` as a
    /// set; an empty `expected` means the subject and lane must have no
    /// heads. `S_STALE_HEADS`, with the current heads, otherwise. A batch of
    /// one.
    fn append_if_heads(
        &mut self,
        object: &[u8],
        expected: &[String],
    ) -> Result<Receipt, StoreError> {
        let receipts = self.commit(&[Item::Object {
            bytes: object,
            if_heads: Some(expected),
        }])?;
        single(receipts)
    }

    /// Append one attestation line: a batch of one.
    fn attest(&mut self, line: &[u8]) -> Result<Receipt, StoreError> {
        let receipts = self.commit(&[Item::Attestation { line }])?;
        single(receipts)
    }

    /// Write a checkpoint over every entry so far, and its witness, in one
    /// transaction. `S_STORE_KEY`, with nothing read or written, if
    /// `signer.public_key()` is not the genesis key. See
    /// [Checkpoints](self#checkpoints).
    fn checkpoint(&mut self, signer: &dyn Signer) -> Result<CheckpointInfo, StoreError>;
}

/// The one receipt of a batch of one.
fn single(receipts: Vec<Receipt>) -> Result<Receipt, StoreError> {
    let mut receipts = receipts.into_iter();
    match (receipts.next(), receipts.next()) {
        (Some(receipt), None) => Ok(receipt),
        _ => Err(StoreError::Io(
            "a batch of one item did not give exactly one receipt".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_error_has_its_code() {
        let rejected = |code| StoreError::Rejected(Rejected::new(0, code, "x"));
        let cases = [
            (
                rejected(RejectCode::Core(Code::IdMismatch)),
                "E_ID_MISMATCH",
            ),
            (rejected(RejectCode::Core(Code::PrevOrder)), "E_PREV_ORDER"),
            (rejected(RejectCode::StaleHeads), "S_STALE_HEADS"),
            (rejected(RejectCode::ReservedSubject), "S_RESERVED_SUBJECT"),
            (
                rejected(RejectCode::AttestationTarget),
                "S_ATTESTATION_TARGET",
            ),
            (StoreError::StoreKey, "S_STORE_KEY"),
            (
                StoreError::Locked {
                    holder: "pid 1\nepoch 2\n".into(),
                },
                "S_LOCKED",
            ),
            (StoreError::NotAStore(PathBuf::from("d")), "S_NOT_A_STORE"),
            (StoreError::Exists(PathBuf::from("d")), "S_EXISTS"),
            (StoreError::Format("user_version 2".into()), "S_FORMAT"),
            (StoreError::Usage("segment_size 0".into()), "E_USAGE"),
            (StoreError::Sqlite("disk I/O error".into()), "S_IO"),
            (StoreError::Io("permission denied".into()), "S_IO"),
        ];
        for (error, code) in cases {
            assert_eq!(error.code_str(), code);
            assert!(error.to_string().starts_with(code), "{error}");
        }
    }

    #[test]
    fn display_keeps_diagnostics_on_one_line() {
        let e = StoreError::Locked {
            holder: "pid 41\nepoch 3\n".into(),
        };
        assert_eq!(
            e.to_string(),
            "S_LOCKED: another writer holds the store (pid 41 epoch 3)"
        );
    }

    #[test]
    fn receipts_and_checkpoints_render_their_cli_shapes() {
        let h = "sha256:1111111111111111111111111111111111111111111111111111111111111111";
        let receipt = Receipt {
            kind: Kind::Attestation,
            reference: h.into(),
            seq: 7,
            entry_hash: h.into(),
            received_at: "2026-11-26T00:00:01.000Z".into(),
            duplicate: true,
        };
        assert_eq!(
            evidence_core::canon::canonicalize(&receipt.to_value()),
            format!(
                "{{\"duplicate\":true,\"entry_hash\":\"{h}\",\"kind\":\"attestation\",\
                 \"received_at\":\"2026-11-26T00:00:01.000Z\",\"ref\":\"{h}\",\"seq\":7}}"
            )
        );
        let checkpoint = CheckpointInfo {
            id: h.into(),
            seq: 11,
            size: 10,
            root: h.into(),
            head: h.into(),
        };
        assert_eq!(
            evidence_core::canon::canonicalize(&checkpoint.to_value()),
            format!(
                "{{\"checkpoint\":\"{h}\",\"head\":\"{h}\",\"root\":\"{h}\",\"seq\":11,\"size\":10}}"
            )
        );
        let head = Head {
            seq: 2,
            entry_hash: h.into(),
        };
        assert_eq!(
            evidence_core::canon::canonicalize(&head.to_value()),
            format!("{{\"entry_hash\":\"{h}\",\"seq\":2}}")
        );
    }

    #[test]
    fn a_batch_of_one_needs_exactly_one_receipt() {
        assert_eq!(single(Vec::new()).unwrap_err().code_str(), "S_IO");
    }
}
