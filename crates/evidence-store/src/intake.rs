//! The intake rules, shared by every writer. See `spec/store-0.md`, "Intake"
//! and "Checkpoint and tree", and the rules in [`crate::api`].
//!
//! A writer does not check items itself. It reads its clock once, asks a
//! planner here what the transaction would write, and then writes exactly
//! that, or nothing:
//!
//! - [`plan_commit`] for a batch of items ([`LogWrite::commit`](crate::api::LogWrite::commit));
//! - [`new_genesis`] then [`plan_genesis`] to create a store;
//! - [`check_store_key`] then [`plan_checkpoint`] for a checkpoint.
//!
//! A planner reads the log only through [`IntakeState`] and writes nothing,
//! so [`MemLog`](crate::mem::MemLog) and [`SqliteLog`](crate::sqlite::SqliteLog)
//! run the same checks and build the same bytes. `SqliteLog` implements
//! `IntakeState` over its open `BEGIN IMMEDIATE` transaction, so what a plan
//! read is still true when it is written.
//!
//! A [`Plan`] holds everything a backend writes, already computed: each new
//! entry with its canonical text, entry hash, leaf hash and segment line, and
//! the rows of its object or attestation. A backend applies the entries in
//! order:
//!
//! - insert the entry, then its object or attestation row;
//! - for an object, apply its heads change: remove each of its `prev` ids
//!   from `heads(subject, lane)` and add its id (`evidence_log::Heads::apply`;
//!   in SQL, a `DELETE` of those rows then an `INSERT`). Applied in order,
//!   this is right even when a `prev` is earlier in the same plan;
//! - then the `checkpoints` row, if [`Plan::checkpoint`] is set.

use std::collections::{hash_map, BTreeSet, HashMap};

use evidence_core::attest::{self, Attestation};
use evidence_core::canon;
use evidence_core::json::Value;
use evidence_core::object::{self, is_hash, Checked};
use evidence_core::time::{self, Timestamp};
use evidence_core::verify::{check_prev, PrevLookup};
use evidence_log::clock::{format_millis, next_received, service_at};
use evidence_log::genesis::is_store_name;
use evidence_log::merkle::{self, MAX_SIZE};
use evidence_log::{
    attestation_ref, lane_of, render_line, subject_of, Checkpoint, Digest, Entry, Genesis, Kind,
    SubjectKey, STORE_SYSTEM,
};

use crate::api::{CheckpointInfo, Item, Receipt, RejectCode, Rejected, StoreError};
use crate::signer::Signer;

/// The last entry of a log: what the next entry chains to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tip {
    pub seq: u64,
    pub entry_hash: String,
    /// Its `received_at`, parsed.
    pub received_at: Timestamp,
}

/// What a planner needs to know about the log it plans for. Every method
/// reports a storage failure as an error, never as "not held", so a failed
/// read can never turn into a rejection.
///
/// Only a created store has an `IntakeState`: [`plan_genesis`] needs none.
pub trait IntakeState {
    /// The last entry.
    fn tip(&self) -> Result<Tip, StoreError>;

    /// The receipt of the entry whose `ref` is `reference`, with `duplicate:
    /// false`, or None if no entry has that ref. Objects and attestations
    /// share one ref space (`entries.ref` is unique).
    fn receipt(&self, reference: &str) -> Result<Option<Receipt>, StoreError>;

    /// The `at` of the held object with this id, parsed, or None if it is
    /// not held.
    fn object_at(&self, id: &str) -> Result<Option<Timestamp>, StoreError>;

    /// `heads(subject, lane)`, sorted. Empty if there are none.
    fn heads(&self, subject: &SubjectKey, lane: &str) -> Result<Vec<String>, StoreError>;

    /// The checkpoint with the highest seq, or None if the log has none.
    fn latest_checkpoint(&self) -> Result<Option<CheckpointInfo>, StoreError>;

    /// The RFC 9162 tree hash over the leaves of entries 1 to `size`.
    /// `size` is at most the tip's seq.
    fn root(&self, size: u64) -> Result<Digest, StoreError>;
}

/// The row of a new object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewObject {
    pub id: String,
    /// `at` as the object writes it (not reformatted).
    pub at: String,
    pub op: String,
    pub subject: SubjectKey,
    /// `evidence_log::lane_of` the object.
    pub lane: String,
    /// The object's `prev` ids, in list order: its heads change.
    pub prev: Vec<String>,
    /// The canonical object.
    pub bytes: String,
}

/// The row of a new attestation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewAttestation {
    /// Its ref: `"sha256:"` + hex SHA-256 of `line`.
    pub hash: String,
    /// The attested object's id.
    pub object: String,
    pub purpose: String,
    pub signer: String,
    pub key: String,
    /// `at` as the attestation writes it.
    pub at: String,
    /// The canonical attestation line, without LF.
    pub line: String,
}

/// The item of a new entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NewItem {
    Object(NewObject),
    Attestation(NewAttestation),
}

/// One entry a plan appends, with everything a backend stores for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewEntry {
    pub entry: Entry,
    /// `entry.canonical()`: the `entries.entry` text and the leaf bytes.
    pub canonical: String,
    /// `entry.hash()`.
    pub hash: String,
    /// `evidence_log::merkle::leaf_hash` of `canonical`.
    pub leaf: Digest,
    /// The segment line, without LF (`evidence_log::render_line`).
    pub line: String,
    pub item: NewItem,
}

impl NewEntry {
    /// This entry's receipt, with `duplicate: false`.
    pub fn receipt(&self) -> Receipt {
        Receipt {
            kind: self.entry.kind,
            reference: self.entry.reference.clone(),
            seq: self.entry.seq,
            entry_hash: self.hash.clone(),
            received_at: self.entry.received_at.clone(),
            duplicate: false,
        }
    }
}

/// What one transaction writes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    /// For [`plan_commit`], one receipt per item, in batch order, duplicates
    /// included; for the other planners, one per new entry.
    pub receipts: Vec<Receipt>,
    /// The new entries, in seq order: consecutive seqs from the tip's + 1
    /// (from 1 for a genesis). Empty when every item is a duplicate.
    pub entries: Vec<NewEntry>,
    /// The `checkpoints` row a checkpoint plan adds.
    pub checkpoint: Option<CheckpointInfo>,
}

/// Plan one batch: run intake over `items` in order, as [`crate::api`]
/// ("Batches") and `spec/store-0.md` ("Intake") say, for a writer at `epoch`
/// whose clock read `now` for this transaction.
///
/// For an object item at index i:
///
/// 1. `evidence_core::object::check` (steps 1-5): its `E_` code on failure.
/// 2. `subject.system` = `"evidence-store"`: `S_RESERVED_SUBJECT`.
/// 3. Its id held by `state` or by an earlier item: that receipt with
///    `duplicate: true`; go to the next item.
/// 4. Step 6: each `prev`, in list order up to the first that is not held,
///    resolved by an earlier item or `state.object_at` before
///    `evidence_core::verify::check_prev` runs over what was resolved (so
///    storage errors propagate and never read as "not held"):
///    `E_PREV_MISSING` or `E_PREV_ORDER`.
/// 5. `if_heads`, if given, compared as a set with `heads(subject, lane)`
///    including earlier items: `S_STALE_HEADS` with the current heads.
///    `state.heads` is read only when some item of the batch has
///    `if_heads`.
/// 6. A new entry: `seq` = the previous entry's + 1, `prev` its entry hash,
///    `log` = `log_id`, `epoch`, and `received_at` =
///    `format_millis(next_received(now, Some(tip.received_at)))`, the same
///    for every entry of the batch. The object is stored as
///    `canon::canonicalize` of its parsed value.
///
/// For an attestation item:
///
/// 1. `evidence_core::attest::parse`: `E_ATTESTATION_SHAPE`.
/// 2. Its `object` held by `state` or by an earlier item:
///    `S_ATTESTATION_TARGET` otherwise.
/// 3. `verify_signature`: `E_SIGNATURE_INVALID`.
/// 4. Its ref (`evidence_log::attestation_ref` of the parsed value) held:
///    that receipt with `duplicate: true`.
/// 5. A new entry, as for an object, with the canonical line.
///
/// A rejection is `StoreError::Rejected` with the item's index; heads are
/// set only for `S_STALE_HEADS`. Reads `state` only; never panics on any
/// input bytes.
///
/// Errors that are not about an item: `E_USAGE` for an `epoch` outside 1 to
/// 2^53 − 1; `S_IO` for a `log_id` that is not a hash (a damaged store), a
/// clock reading that gives no valid `received_at` (outside the years 0000
/// to 9999), or a log already at seq 2^53 − 1; and whatever `state` fails
/// with.
pub fn plan_commit(
    state: &dyn IntakeState,
    log_id: &str,
    epoch: u64,
    now: Timestamp,
    items: &[Item<'_>],
) -> Result<Plan, StoreError> {
    let tip = state.tip()?;
    let received_at = millis(next_received(now, Some(tip.received_at)), "received_at")?;
    let mut batch = Batch {
        state,
        draft: Draft::new(log_id, epoch, received_at, tip.seq, tip.entry_hash)?,
        compares: items.iter().any(|item| {
            matches!(
                item,
                Item::Object {
                    if_heads: Some(_),
                    ..
                }
            )
        }),
        heads: HashMap::new(),
    };
    let mut receipts = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let receipt = match *item {
            Item::Object { bytes, if_heads } => batch.object(index, bytes, if_heads)?,
            Item::Attestation { line } => batch.attestation(index, line)?,
        };
        receipts.push(receipt);
    }
    Ok(Plan {
        receipts,
        entries: batch.draft.entries,
        checkpoint: None,
    })
}

/// The genesis of a new store named `name`, keyed by `signer`. `E_USAGE`
/// unless the name matches `^[a-z0-9][a-z0-9.-]{0,62}$` and
/// `signer.public_key()` is a store key (`evidence_log::Genesis::new`).
///
/// A writer calls this before it reads its clock, so a refused name or key
/// uses no reading.
pub fn new_genesis(name: &str, signer: &dyn Signer) -> Result<Genesis, StoreError> {
    if !is_store_name(name) {
        return Err(StoreError::Usage(format!(
            "store name {name:?} must match ^[a-z0-9][a-z0-9.-]{{0,62}}$"
        )));
    }
    let key = signer.public_key();
    Genesis::new(name, &key).map_err(|_| {
        StoreError::Usage(format!(
            "the signer's public key {key:?} is not an Ed25519 key written as \
             ed25519: and 32 bytes of unpadded base64url"
        ))
    })
}

/// Plan the two entries that create a store: the genesis object at seq 1
/// and its author attestation at seq 2, both at epoch 1 with `received_at` =
/// `format_millis(next_received(now, None))`. The genesis `at` is
/// `format_millis(service_at(now, []))`; the author line is
/// [`sign_checked`]`(signer, genesis, log id, "author", at)`.
///
/// Returns the log id (the genesis object's id, which is also seq 1's
/// `log`, `ref` and `prev`) and the plan. `S_STORE_KEY` if the signer's line
/// does not check; `S_IO` if `now` gives no valid timestamp.
pub fn plan_genesis(
    genesis: &Genesis,
    signer: &dyn Signer,
    now: Timestamp,
) -> Result<(String, Plan), StoreError> {
    let at = millis(service_at(now, &[]), "at")?;
    let received_at = millis(next_received(now, None), "received_at")?;
    let object = store_checked(genesis.object(&at))?;
    let log_id = object.id.clone();
    let author = sign_checked(signer, genesis, &log_id, "author", &at)?;
    // Seq 1 chains to the log id: its `prev` is the log id.
    let mut draft = Draft::new(&log_id, 1, received_at, 0, log_id.clone())?;
    let receipts = vec![
        draft.push_object(&object)?,
        draft.push_attestation(&author)?,
    ];
    let plan = Plan {
        receipts,
        entries: draft.entries,
        checkpoint: None,
    };
    Ok((log_id, plan))
}

/// `S_STORE_KEY` unless `signer.public_key()` equals the genesis key. A
/// writer calls this before it reads its clock for a checkpoint.
pub fn check_store_key(genesis: &Genesis, signer: &dyn Signer) -> Result<(), StoreError> {
    if signer.public_key() == genesis.key {
        Ok(())
    } else {
        Err(StoreError::StoreKey)
    }
}

/// Plan a checkpoint and its witness, as [`crate::api`] ("Checkpoints")
/// says: `size`, `head` and `root` from `state.tip()` and
/// `state.root(size)`; `prev` and the service time from
/// `state.latest_checkpoint()` and that checkpoint's `state.object_at`; the
/// witness from [`sign_checked`]`(signer, genesis, id, "witness", at)`; both
/// entries at `epoch` with one `received_at`; and [`Plan::checkpoint`] set.
///
/// The caller has already run [`check_store_key`]. `S_STORE_KEY` if the
/// witness does not check; `S_IO` if the latest checkpoint's object is not
/// held (a damaged store), and as [`plan_commit`] for the log id, the clock
/// and a full log; `E_USAGE` as [`plan_commit`] for the epoch.
pub fn plan_checkpoint(
    state: &dyn IntakeState,
    genesis: &Genesis,
    log_id: &str,
    epoch: u64,
    signer: &dyn Signer,
    now: Timestamp,
) -> Result<Plan, StoreError> {
    let tip = state.tip()?;
    let size = tip.seq;
    let root = merkle::to_text(&state.root(size)?);
    let previous = state.latest_checkpoint()?;
    let mut prev_ats = Vec::new();
    if let Some(previous) = &previous {
        let at = state.object_at(&previous.id)?.ok_or_else(|| {
            StoreError::Io(format!(
                "the latest checkpoint {} (seq {}) is not held",
                previous.id, previous.seq
            ))
        })?;
        prev_ats.push(at);
    }
    let at = millis(service_at(now, &prev_ats), "at")?;
    let received_at = millis(next_received(now, Some(tip.received_at)), "received_at")?;
    let content = Checkpoint {
        log: log_id.to_string(),
        size,
        head: tip.entry_hash.clone(),
        root: root.clone(),
    };
    let prev = previous.as_ref().map(|p| p.id.as_str());
    let object = store_checked(content.object(genesis, &at, prev))?;
    let witness = sign_checked(signer, genesis, &object.id, "witness", &at)?;
    let mut draft = Draft::new(log_id, epoch, received_at, tip.seq, tip.entry_hash.clone())?;
    let receipts = vec![
        draft.push_object(&object)?,
        draft.push_attestation(&witness)?,
    ];
    let checkpoint = CheckpointInfo {
        id: object.id,
        seq: receipts[0].seq,
        size,
        root,
        head: tip.entry_hash,
    };
    Ok(Plan {
        receipts,
        entries: draft.entries,
        checkpoint: Some(checkpoint),
    })
}

/// Ask `signer` for an attestation and check what comes back: it must parse
/// as attestation/0 and verify, and its `object`, `purpose`, `signer`, `key`
/// and `at` must be exactly `object_id`, `purpose`, the genesis signer, the
/// genesis key and `at`. `S_STORE_KEY` otherwise. The signer's own errors are
/// passed on.
///
/// The caller stores `canon::canonicalize(&attestation.value)`, whatever
/// spelling the signer returned.
pub fn sign_checked(
    signer: &dyn Signer,
    genesis: &Genesis,
    object_id: &str,
    purpose: &str,
    at: &str,
) -> Result<Attestation, StoreError> {
    let line = signer.sign(object_id, purpose, &genesis.signer, at)?;
    let attestation = attest::parse(line.as_bytes()).map_err(|_| StoreError::StoreKey)?;
    attestation
        .verify_signature()
        .map_err(|_| StoreError::StoreKey)?;
    let text = |k: &str| attestation.value.get(k).and_then(Value::as_str);
    let exact = attestation.object == object_id
        && attestation.purpose == purpose
        && attestation.signer == genesis.signer
        && text("key") == Some(genesis.key.as_str())
        && text("at") == Some(at);
    if !exact {
        return Err(StoreError::StoreKey);
    }
    Ok(attestation)
}

/// `t` as a writer writes it, `YYYY-MM-DDTHH:MM:SS.mmmZ`. `S_IO` if that is
/// not a valid timestamp, which only a clock reading outside the years 0000
/// to 9999 gives.
fn millis(t: Timestamp, what: &str) -> Result<String, StoreError> {
    let text = format_millis(t);
    if time::parse(&text).is_err() {
        return Err(StoreError::Io(format!(
            "the clock reading gives {what} {text:?}, which is not a timestamp"
        )));
    }
    Ok(text)
}

/// An object the store authors, checked as any object is (evidence/0 steps
/// 1-5). Always passes; `S_IO` rather than a panic if it ever did not.
fn store_checked(value: Value) -> Result<Checked, StoreError> {
    object::check_value(value)
        .map_err(|e| StoreError::Io(format!("the store's own object does not check: {e}")))
}

/// The rejection of item `index` with an evidence/0 result.
fn core(index: usize, e: evidence_core::Error) -> StoreError {
    Rejected::new(index, RejectCode::Core(e.code), e.detail).into()
}

/// `text` of a member of a checked object or attestation, which its shape
/// check guarantees is a string.
fn member(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// The entries a plan appends, chained from the log's tip, and the refs and
/// objects they hold.
struct Draft<'a> {
    log_id: &'a str,
    epoch: u64,
    received_at: String,
    /// Seq and entry hash of the last entry, in the log or in the draft. Seq
    /// 0, with the log id as its hash, before a genesis.
    seq: u64,
    hash: String,
    entries: Vec<NewEntry>,
    /// Ref → index in `entries`.
    refs: HashMap<String, usize>,
    /// Object id → `at`, for the objects in `entries`.
    objects: HashMap<String, Timestamp>,
}

impl<'a> Draft<'a> {
    fn new(
        log_id: &'a str,
        epoch: u64,
        received_at: String,
        seq: u64,
        hash: String,
    ) -> Result<Draft<'a>, StoreError> {
        if !(1..=MAX_SIZE).contains(&epoch) {
            return Err(StoreError::Usage(format!(
                "epoch {epoch} is not an integer from 1 to 2^53 - 1"
            )));
        }
        if !is_hash(log_id) {
            return Err(StoreError::Io(format!(
                "the log id {log_id:?} is not a hash"
            )));
        }
        Ok(Draft {
            log_id,
            epoch,
            received_at,
            seq,
            hash,
            entries: Vec::new(),
            refs: HashMap::new(),
            objects: HashMap::new(),
        })
    }

    /// The receipt of the draft entry with this ref, as a duplicate.
    fn held(&self, reference: &str) -> Option<Receipt> {
        let entry = self.entries.get(*self.refs.get(reference)?)?;
        Some(Receipt {
            duplicate: true,
            ..entry.receipt()
        })
    }

    /// Append an object that passed steps 1-5, in canonical form.
    fn push_object(&mut self, object: &Checked) -> Result<Receipt, StoreError> {
        let subject = subject_of(&object.value);
        let lane = lane_of(&object.value);
        self.push_object_in(object, subject, lane)
    }

    fn push_object_in(
        &mut self,
        object: &Checked,
        subject: SubjectKey,
        lane: String,
    ) -> Result<Receipt, StoreError> {
        let bytes = canon::canonicalize(&object.value);
        let item = NewItem::Object(NewObject {
            id: object.id.clone(),
            at: member(&object.value, "at"),
            op: member(&object.value, "op"),
            subject,
            lane,
            prev: object.prev.clone(),
            bytes: bytes.clone(),
        });
        let receipt = self.push(Kind::Object, object.id.clone(), &bytes, item)?;
        self.objects.insert(object.id.clone(), object.at);
        Ok(receipt)
    }

    /// Append an attestation that parsed, in canonical form.
    fn push_attestation(&mut self, attestation: &Attestation) -> Result<Receipt, StoreError> {
        let line = canon::canonicalize(&attestation.value);
        let reference = attestation_ref(&attestation.value);
        let item = NewItem::Attestation(NewAttestation {
            hash: reference.clone(),
            object: attestation.object.clone(),
            purpose: attestation.purpose.clone(),
            signer: attestation.signer.clone(),
            key: member(&attestation.value, "key"),
            at: member(&attestation.value, "at"),
            line: line.clone(),
        });
        self.push(Kind::Attestation, reference, &line, item)
    }

    /// The next entry: seq + 1, chained to the last entry's hash.
    fn push(
        &mut self,
        kind: Kind,
        reference: String,
        body: &str,
        item: NewItem,
    ) -> Result<Receipt, StoreError> {
        let seq = self
            .seq
            .checked_add(1)
            .filter(|seq| *seq <= MAX_SIZE)
            .ok_or_else(|| StoreError::Io("the log is full: seq is at 2^53 - 1".into()))?;
        let entry = Entry {
            log: self.log_id.to_string(),
            seq,
            kind,
            reference: reference.clone(),
            received_at: self.received_at.clone(),
            epoch: self.epoch,
            prev: self.hash.clone(),
        };
        let canonical = entry.canonical();
        let new = NewEntry {
            hash: canon::sha256_hex(canonical.as_bytes()),
            leaf: merkle::leaf_hash(canonical.as_bytes()),
            line: render_line(&canonical, kind, body),
            canonical,
            entry,
            item,
        };
        let receipt = new.receipt();
        self.seq = seq;
        self.hash = new.hash.clone();
        self.refs.insert(reference, self.entries.len());
        self.entries.push(new);
        Ok(receipt)
    }
}

/// A batch being planned: the draft, over the log `state` reads.
struct Batch<'a> {
    state: &'a dyn IntakeState,
    draft: Draft<'a>,
    /// Whether any item has `if_heads`. Only then are heads read and the
    /// batch's own heads changes followed.
    compares: bool,
    /// `heads(subject, lane)` as the batch leaves it, for each subject and
    /// lane the batch has read or changed.
    heads: HashMap<(SubjectKey, String), BTreeSet<String>>,
}

impl Batch<'_> {
    /// Intake steps 1-6 for an object item.
    fn object(
        &mut self,
        index: usize,
        bytes: &[u8],
        if_heads: Option<&[String]>,
    ) -> Result<Receipt, StoreError> {
        // 1. evidence/0 steps 1-5.
        let object = object::check(bytes).map_err(|e| core(index, e))?;
        // 2. Store subjects are the store's own.
        let subject = subject_of(&object.value);
        if subject.system == STORE_SYSTEM {
            let detail =
                format!("subject.system {STORE_SYSTEM:?} is reserved for the store's own objects");
            return Err(Rejected::new(index, RejectCode::ReservedSubject, detail).into());
        }
        // 3. Already held: that receipt, and nothing more is checked.
        if let Some(receipt) = self.held(&object.id)? {
            return Ok(receipt);
        }
        // 4. Step 6 against the log and the earlier items. check_prev stops
        // at the first prev that is not held, so resolving stops there too.
        let mut resolved = Resolved::default();
        for p in &object.prev {
            match self.object_at(p)? {
                Some(at) => {
                    resolved.0.insert(p.clone(), at);
                }
                None => break,
            }
        }
        check_prev(&object, &resolved).map_err(|e| core(index, e))?;
        // 5. Compare-and-set, as sets. An earlier item's heads change counts,
        // so when any item compares, every object's change is followed.
        let lane = lane_of(&object.value);
        if self.compares {
            self.compare_and_change(index, &object, &subject, &lane, if_heads)?;
        }
        // 6. Append.
        self.draft.push_object_in(&object, subject, lane)
    }

    /// Step 5 for an object with `if_heads` (`S_STALE_HEADS` unless the
    /// heads equal it as a set), then the object's heads change, so that a
    /// later item compares against the heads after it.
    fn compare_and_change(
        &mut self,
        index: usize,
        object: &Checked,
        subject: &SubjectKey,
        lane: &str,
        if_heads: Option<&[String]>,
    ) -> Result<(), StoreError> {
        let heads = self.heads_of(subject, lane)?;
        if let Some(expected) = if_heads {
            let expected: BTreeSet<&str> = expected.iter().map(String::as_str).collect();
            if !expected
                .iter()
                .copied()
                .eq(heads.iter().map(String::as_str))
            {
                let current: Vec<String> = heads.iter().cloned().collect();
                let detail = format!(
                    "the heads of {}/{}/{} in lane {lane} are [{}]",
                    subject.system,
                    subject.r#type,
                    subject.key,
                    current.join(", ")
                );
                return Err(Rejected {
                    heads: current,
                    ..Rejected::new(index, RejectCode::StaleHeads, detail)
                }
                .into());
            }
        }
        // The heads change, as the backend will apply it.
        for p in &object.prev {
            heads.remove(p);
        }
        heads.insert(object.id.clone());
        Ok(())
    }

    /// Intake steps 1-5 for an attestation item.
    fn attestation(&mut self, index: usize, line: &[u8]) -> Result<Receipt, StoreError> {
        // 1. attestation/0 shape.
        let attestation = attest::parse(line).map_err(|e| core(index, e))?;
        // 2. The target is held, by the log or by an earlier item.
        if self.object_at(&attestation.object)?.is_none() {
            let detail = format!("the target object {} is not held", attestation.object);
            return Err(Rejected::new(index, RejectCode::AttestationTarget, detail).into());
        }
        // 3. The signature.
        attestation.verify_signature().map_err(|e| core(index, e))?;
        // 4. Already held: that receipt.
        if let Some(receipt) = self.held(&attestation_ref(&attestation.value))? {
            return Ok(receipt);
        }
        // 5. Append.
        self.draft.push_attestation(&attestation)
    }

    /// The receipt of a held ref, as a duplicate: an earlier item's, or the
    /// log's.
    fn held(&self, reference: &str) -> Result<Option<Receipt>, StoreError> {
        if let Some(receipt) = self.draft.held(reference) {
            return Ok(Some(receipt));
        }
        Ok(self.state.receipt(reference)?.map(|receipt| Receipt {
            duplicate: true,
            ..receipt
        }))
    }

    /// `at` of a held object: an earlier item, or the log's.
    fn object_at(&self, id: &str) -> Result<Option<Timestamp>, StoreError> {
        match self.draft.objects.get(id) {
            Some(at) => Ok(Some(*at)),
            None => self.state.object_at(id),
        }
    }

    /// `heads(subject, lane)` after the earlier items, read from the log the
    /// first time.
    fn heads_of(
        &mut self,
        subject: &SubjectKey,
        lane: &str,
    ) -> Result<&mut BTreeSet<String>, StoreError> {
        match self.heads.entry((subject.clone(), lane.to_string())) {
            hash_map::Entry::Occupied(slot) => Ok(slot.into_mut()),
            hash_map::Entry::Vacant(slot) => {
                let current = self.state.heads(subject, lane)?;
                Ok(slot.insert(current.into_iter().collect()))
            }
        }
    }
}

/// Prev ids resolved before step 6 runs: id → `at`.
#[derive(Default)]
struct Resolved(HashMap<String, Timestamp>);

impl PrevLookup for Resolved {
    fn prev_at(&self, id: &str) -> Option<Timestamp> {
        self.0.get(id).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{LogRead, LogWrite};
    use crate::clock::ScriptClock;
    use crate::mem::MemLog;
    use crate::signer::SeedSigner;
    use std::path::{Path, PathBuf};

    fn vectors() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-vectors")
    }

    fn store_key() -> SeedSigner {
        SeedSigner::from_file(&vectors().join("keys/store-demo.seed")).unwrap()
    }

    fn ts(text: &str) -> Timestamp {
        time::parse(text).unwrap()
    }

    fn object_vector(case: &str) -> Vec<u8> {
        std::fs::read(vectors().join(format!("objects/{case}/object.json"))).unwrap()
    }

    /// A log with the genesis and object vector 001 (seq 3), and its clock,
    /// with no reading left.
    fn log() -> (MemLog, ScriptClock) {
        let clock = ScriptClock::new(&["2026-11-26T00:00:00Z", "2026-11-26T00:00:01Z"]);
        let mut log = MemLog::init("cer-demo", &store_key(), Box::new(clock.clone())).unwrap();
        log.append(&object_vector("001-create-inline")).unwrap();
        (log, clock)
    }

    /// A signer with the store key that returns `line` whatever it is asked.
    struct Fixed(String);

    impl Signer for Fixed {
        fn public_key(&self) -> String {
            store_key().public_key()
        }

        fn sign(&self, _: &str, _: &str, _: &str, _: &str) -> Result<String, StoreError> {
            Ok(self.0.clone())
        }
    }

    /// A signer that fails with its own error.
    struct Failing;

    impl Signer for Failing {
        fn public_key(&self) -> String {
            store_key().public_key()
        }

        fn sign(&self, _: &str, _: &str, _: &str, _: &str) -> Result<String, StoreError> {
            Err(StoreError::Io("the key service is down".into()))
        }
    }

    /// Which `IntakeState` read fails.
    #[derive(Clone, Copy, PartialEq)]
    enum Fail {
        Receipt,
        ObjectAt,
        Heads,
        /// Not an error: a latest checkpoint whose object is not held.
        Checkpoint,
    }

    /// A log whose one kind of read fails with `S_IO`.
    struct Broken<'a>(&'a MemLog, Fail);

    impl Broken<'_> {
        fn fail<T>(&self, which: Fail, ok: Result<T, StoreError>) -> Result<T, StoreError> {
            if self.1 == which {
                Err(StoreError::Io("disk I/O error".into()))
            } else {
                ok
            }
        }
    }

    impl IntakeState for Broken<'_> {
        fn tip(&self) -> Result<Tip, StoreError> {
            self.0.tip()
        }
        fn receipt(&self, reference: &str) -> Result<Option<Receipt>, StoreError> {
            self.fail(Fail::Receipt, self.0.receipt(reference))
        }
        fn object_at(&self, id: &str) -> Result<Option<Timestamp>, StoreError> {
            self.fail(Fail::ObjectAt, self.0.object_at(id))
        }
        fn heads(&self, subject: &SubjectKey, lane: &str) -> Result<Vec<String>, StoreError> {
            self.fail(Fail::Heads, IntakeState::heads(self.0, subject, lane))
        }
        fn latest_checkpoint(&self) -> Result<Option<CheckpointInfo>, StoreError> {
            if self.1 != Fail::Checkpoint {
                return self.0.latest_checkpoint();
            }
            let h = canon::sha256_hex(b"absent");
            Ok(Some(CheckpointInfo {
                id: h.clone(),
                seq: 3,
                size: 2,
                root: h.clone(),
                head: h,
            }))
        }
        fn root(&self, size: u64) -> Result<Digest, StoreError> {
            self.0.root(size)
        }
    }

    #[test]
    fn a_genesis_plan_matches_the_reference_writer() {
        // Export vector 001: name cer-demo, the store-demo key, one reading.
        let case = vectors().join("store/exports/001-genesis");
        let segment = std::fs::read(case.join("000001.jsonl")).unwrap();
        let genesis = new_genesis("cer-demo", &store_key()).unwrap();
        let (log_id, plan) =
            plan_genesis(&genesis, &store_key(), ts("2026-11-26T00:00:00.000Z")).unwrap();
        assert_eq!(
            log_id,
            "sha256:7d69e3112259c45a165118501a0552b09c9118c0a78d79482ef538d4d1f5a9f3"
        );
        let lines: String = plan
            .entries
            .iter()
            .map(|e| format!("{}\n", e.line))
            .collect();
        assert_eq!(lines.as_bytes(), segment.as_slice());
        assert_eq!(plan.receipts.len(), 2);
        for (new, receipt) in plan.entries.iter().zip(&plan.receipts) {
            assert_eq!(new.hash, new.entry.hash());
            assert_eq!(new.canonical, new.entry.canonical());
            assert_eq!(new.leaf, merkle::leaf_hash(new.canonical.as_bytes()));
            assert_eq!(&new.receipt(), receipt);
        }
        assert_eq!(plan.checkpoint, None);
    }

    #[test]
    fn names_and_keys_are_checked() {
        for name in ["", "Demo", ".demo", "demo store", &"x".repeat(64)] {
            let e = new_genesis(name, &store_key()).unwrap_err();
            assert_eq!(e.code_str(), "E_USAGE", "{name:?}");
        }
        let genesis = new_genesis("demo-1.x", &store_key()).unwrap();
        assert_eq!(genesis.signer, "service:evidence-store/demo-1.x");
        assert_eq!(genesis.key, store_key().public_key());
        check_store_key(&genesis, &store_key()).unwrap();
        let other = SeedSigner::from_file(&vectors().join("keys/assessor-01.seed")).unwrap();
        assert_eq!(check_store_key(&genesis, &other), Err(StoreError::StoreKey));
    }

    #[test]
    fn signer_output_is_checked() {
        let genesis = new_genesis("cer-demo", &store_key()).unwrap();
        let id = canon::sha256_hex(b"an object");
        let at = "2026-11-26T00:00:00.000Z";
        let good = store_key()
            .sign(&id, "witness", &genesis.signer, at)
            .unwrap();
        let checked = sign_checked(&Fixed(good.clone()), &genesis, &id, "witness", at).unwrap();
        assert_eq!(canon::canonicalize(&checked.value), good);
        // What it signs must be exactly what was asked.
        let asks = [
            (canon::sha256_hex(b"another object"), "witness", at),
            (id.clone(), "author", at),
            (id.clone(), "witness", "2026-11-26T00:00:00Z"),
        ];
        for (object_id, purpose, at) in asks {
            let r = sign_checked(&Fixed(good.clone()), &genesis, &object_id, purpose, at);
            assert_eq!(r.unwrap_err(), StoreError::StoreKey, "{purpose} {at}");
        }
        // Not attestation/0, or not verifying.
        let forged = good.replace(&id, &canon::sha256_hex(b"x"));
        for line in [String::new(), "{}".into(), forged] {
            let r = sign_checked(&Fixed(line), &genesis, &id, "witness", at);
            assert_eq!(r.unwrap_err(), StoreError::StoreKey);
        }
        // The signer's own errors are passed on.
        let r = sign_checked(&Failing, &genesis, &id, "witness", at);
        assert_eq!(r.unwrap_err().code_str(), "S_IO");
    }

    #[test]
    fn storage_errors_are_never_rejections() {
        let (log, _) = log();
        let now = ts("2026-11-26T00:00:02Z");
        let id = log.log_id().to_string();
        let v001 = object_vector("001-create-inline");
        let v002 = object_vector("002-update-with-prev");
        let author_001 = std::fs::read(
            vectors().join("objects/003-agent-assertion-approved/attestations.jsonl"),
        )
        .unwrap();
        let author_001 = author_001.split(|b| *b == b'\n').nth(2).unwrap();
        let object = |bytes| Item::Object {
            bytes,
            if_heads: None,
        };
        let cases: [(Fail, Item<'_>); 4] = [
            (Fail::Receipt, object(&v001)),
            (Fail::ObjectAt, object(&v002)),
            (Fail::ObjectAt, Item::Attestation { line: author_001 }),
            (
                Fail::Heads,
                Item::Object {
                    bytes: &v002,
                    if_heads: Some(&[]),
                },
            ),
        ];
        for (fail, item) in cases {
            let r = plan_commit(&Broken(&log, fail), &id, 1, now, &[item]);
            assert_eq!(r.unwrap_err(), StoreError::Io("disk I/O error".into()));
        }
        // Heads are read only when an item compares.
        let plan = plan_commit(&Broken(&log, Fail::Heads), &id, 1, now, &[object(&v002)]).unwrap();
        assert_eq!(plan.entries.len(), 1);
    }

    #[test]
    fn a_plan_needs_an_epoch_and_a_log_id() {
        let (log, _) = log();
        let now = ts("2026-11-26T00:00:02Z");
        let id = log.log_id().to_string();
        for epoch in [0, MAX_SIZE + 1] {
            let r = plan_commit(&log, &id, epoch, now, &[]);
            assert_eq!(r.unwrap_err().code_str(), "E_USAGE");
        }
        let r = plan_commit(&log, "not-a-hash", 1, now, &[]);
        assert_eq!(r.unwrap_err().code_str(), "S_IO");
        // A reading past the year 9999 gives no timestamp.
        let far = Timestamp {
            secs: 300_000_000_000,
            nanos: 0,
        };
        let r = plan_commit(&log, &id, 1, far, &[]);
        assert_eq!(r.unwrap_err().code_str(), "S_IO");
        // An empty batch plans nothing.
        let plan = plan_commit(&log, &id, 1, now, &[]).unwrap();
        assert_eq!(
            plan,
            Plan {
                receipts: Vec::new(),
                entries: Vec::new(),
                checkpoint: None,
            }
        );
    }

    #[test]
    fn a_checkpoint_plan_chains_to_the_latest_checkpoint() {
        let (mut log, clock) = log();
        let genesis = log.genesis().clone();
        let id = log.log_id().to_string();
        let now = ts("2026-11-26T00:00:05Z");
        let first = plan_checkpoint(&log, &genesis, &id, 1, &store_key(), now).unwrap();
        let info = first.checkpoint.clone().unwrap();
        assert_eq!((info.seq, info.size), (4, 3));
        assert_eq!(info.root, merkle::to_text(&log.root(3).unwrap()));
        assert_eq!(info.head, log.head().unwrap().entry_hash);
        assert_eq!(first.receipts.len(), 2);
        let received: Vec<&str> = first
            .entries
            .iter()
            .map(|e| e.entry.received_at.as_str())
            .collect();
        assert_eq!(received, ["2026-11-26T00:00:05.000Z"; 2]);

        // Written through the log, it is the next plan's prev; with the
        // clock behind, the next plan keeps its at.
        clock.push(now);
        assert_eq!(log.checkpoint(&store_key()).unwrap(), info);
        let behind = ts("2026-11-26T00:00:01Z");
        let second = plan_checkpoint(&log, &genesis, &id, 1, &store_key(), behind).unwrap();
        let NewItem::Object(object) = &second.entries[0].item else {
            panic!("a checkpoint plan starts with its object");
        };
        assert_eq!(object.prev, vec![info.id.clone()]);
        assert_eq!(object.at, "2026-11-26T00:00:05.000Z");
        assert_eq!(second.checkpoint.map(|c| c.size), Some(5));

        // A latest checkpoint whose object is not held: a damaged store.
        let broken = Broken(&log, Fail::Checkpoint);
        let r = plan_checkpoint(&broken, &genesis, &id, 1, &store_key(), now);
        assert_eq!(r.unwrap_err().code_str(), "S_IO");
        // A witness that does not check.
        let r = plan_checkpoint(&log, &genesis, &id, 1, &Fixed("{}".into()), now);
        assert_eq!(r.unwrap_err(), StoreError::StoreKey);
    }
}
