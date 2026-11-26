//! [`MemLog`]: the in-memory reference log.
//!
//! It runs the same intake ([`crate::intake`]) as [`SqliteLog`](crate::sqlite::SqliteLog)
//! and keeps the same projections, in memory, so the contract suite (T4) and
//! the property test (T5) run every rule against both and compare their
//! exports byte for byte. Nothing is persisted; dropping it loses the log.

use std::collections::HashMap;

use evidence_core::time::{self, Timestamp};
use evidence_core::verify::PrevLookup;
use evidence_log::{merkle, Digest, Genesis, Heads, SubjectKey};

use crate::api::{
    CheckpointInfo, Head, Item, LogRead, LogWrite, Receipt, StoreError, StoredAttestation,
    StoredObject,
};
use crate::clock::Clock;
use crate::intake::{self, IntakeState, NewEntry, NewItem, Plan, Tip};
use crate::signer::Signer;

/// An in-memory store/0 log with one writer.
///
/// - [`MemLog::init`] creates it at epoch 1, as `SqliteLog::init` does.
/// - [`MemLog::reopen`] models closing the writer and opening the store
///   again: the epoch goes up by one, as `SqliteLog::open` makes it.
/// - [`LogWrite::commit`] and [`LogWrite::checkpoint`] plan with
///   [`crate::intake`] against `self` (its [`IntakeState`]), then apply the
///   plan's entries in order to the fields below, which cannot fail half
///   way.
///
/// Given the same calls and clock readings, its [`LogRead`] answers and its
/// export are identical to a `SqliteLog`'s.
pub struct MemLog {
    log_id: String,
    genesis: Genesis,
    epoch: u64,
    clock: Box<dyn Clock>,
    /// Every entry, as planned, by seq − 1.
    entries: Vec<NewEntry>,
    /// Entry ref → seq.
    refs: HashMap<String, u64>,
    /// Held object id → (seq, parsed `at`).
    objects: HashMap<String, (u64, Timestamp)>,
    /// Object id → the seqs of its attestations, in seq order.
    attestations: HashMap<String, Vec<u64>>,
    /// The heads fold over every object, in seq order.
    heads: Heads,
    /// Every checkpoint, in seq order.
    checkpoints: Vec<CheckpointInfo>,
}

impl MemLog {
    /// Create a log named `name`, keyed by `signer`, at epoch 1: the genesis
    /// at seq 1 and its author attestation at seq 2, in one transaction (see
    /// [`crate::api`], "Creating a store").
    ///
    /// `E_USAGE` for an invalid name or key, before the clock is read.
    /// Otherwise reads `clock` once. `S_STORE_KEY` if the signer's author
    /// line does not check. The log keeps `clock` for every later
    /// transaction.
    pub fn init(
        name: &str,
        signer: &dyn Signer,
        clock: Box<dyn Clock>,
    ) -> Result<MemLog, StoreError> {
        let genesis = intake::new_genesis(name, signer)?;
        let now = clock.now();
        let (log_id, plan) = intake::plan_genesis(&genesis, signer, now)?;
        let mut log = MemLog {
            log_id,
            genesis,
            epoch: 1,
            clock,
            entries: Vec::new(),
            refs: HashMap::new(),
            objects: HashMap::new(),
            attestations: HashMap::new(),
            heads: Heads::default(),
            checkpoints: Vec::new(),
        };
        log.apply(plan)?;
        Ok(log)
    }

    /// Close the writer and open the log again: the epoch goes up by one.
    /// Reads no clock and writes nothing. The `reopen` operation of an
    /// `inputs.json` script.
    pub fn reopen(&mut self) {
        self.epoch += 1;
    }

    /// Apply a plan: its entries in order (each entry, its item, an object's
    /// heads change), then its checkpoint. Everything that can fail is
    /// checked first, so the log takes the whole plan or none of it.
    fn apply(&mut self, plan: Plan) -> Result<Vec<Receipt>, StoreError> {
        let mut ats = Vec::with_capacity(plan.entries.len());
        for (next, new) in (self.entries.len() as u64 + 1..).zip(&plan.entries) {
            if new.entry.seq != next {
                return Err(StoreError::Io(format!(
                    "a plan gives seq {} where seq {next} comes next",
                    new.entry.seq
                )));
            }
            ats.push(match &new.item {
                NewItem::Object(object) => Some(time::parse(&object.at).map_err(|_| {
                    StoreError::Io(format!("object {} has at {:?}", object.id, object.at))
                })?),
                NewItem::Attestation(_) => None,
            });
        }
        for (new, at) in plan.entries.into_iter().zip(ats) {
            let seq = new.entry.seq;
            self.refs.insert(new.entry.reference.clone(), seq);
            match &new.item {
                NewItem::Object(object) => {
                    if let Some(at) = at {
                        self.objects.insert(object.id.clone(), (seq, at));
                    }
                    self.heads
                        .apply(&object.id, &object.subject, &object.lane, &object.prev);
                }
                NewItem::Attestation(attestation) => {
                    self.attestations
                        .entry(attestation.object.clone())
                        .or_default()
                        .push(seq);
                }
            }
            self.entries.push(new);
        }
        if let Some(checkpoint) = plan.checkpoint {
            self.checkpoints.push(checkpoint);
        }
        Ok(plan.receipts)
    }

    /// The entry at `seq`, if there is one.
    fn entry(&self, seq: u64) -> Option<&NewEntry> {
        let index = usize::try_from(seq.checked_sub(1)?).ok()?;
        self.entries.get(index)
    }

    /// The entries with seq 1 to `upto`. `E_USAGE` above the head.
    fn up_to(&self, upto: u64) -> Result<&[NewEntry], StoreError> {
        match usize::try_from(upto) {
            Ok(n) if n <= self.entries.len() => Ok(&self.entries[..n]),
            _ => Err(StoreError::Usage(format!(
                "seq {upto} is above the head, seq {}",
                self.entries.len()
            ))),
        }
    }
}

impl LogRead for MemLog {
    fn log_id(&self) -> &str {
        &self.log_id
    }

    fn genesis(&self) -> &Genesis {
        &self.genesis
    }

    fn head(&self) -> Result<Head, StoreError> {
        let last = self
            .entries
            .last()
            .ok_or_else(|| StoreError::Io("the log has no entries".into()))?;
        Ok(Head {
            seq: last.entry.seq,
            entry_hash: last.hash.clone(),
        })
    }

    fn lines(&self, from_seq: u64, limit: usize) -> Result<Vec<(u64, String)>, StoreError> {
        let skip = usize::try_from(from_seq.saturating_sub(1)).unwrap_or(usize::MAX);
        Ok(self
            .entries
            .iter()
            .skip(skip)
            .take(limit)
            .map(|new| (new.entry.seq, new.line.clone()))
            .collect())
    }

    fn object(&self, id: &str) -> Result<Option<StoredObject>, StoreError> {
        let Some((seq, _)) = self.objects.get(id) else {
            return Ok(None);
        };
        Ok(self.entry(*seq).and_then(|new| match &new.item {
            NewItem::Object(object) => Some(StoredObject {
                id: object.id.clone(),
                seq: *seq,
                received_at: new.entry.received_at.clone(),
                bytes: object.bytes.clone(),
            }),
            NewItem::Attestation(_) => None,
        }))
    }

    fn attestations(&self, object_id: &str) -> Result<Vec<StoredAttestation>, StoreError> {
        let seqs = self.attestations.get(object_id).map(Vec::as_slice);
        Ok(seqs
            .unwrap_or_default()
            .iter()
            .filter_map(|seq| match &self.entry(*seq)?.item {
                NewItem::Attestation(a) => Some(StoredAttestation {
                    hash: a.hash.clone(),
                    seq: *seq,
                    line: a.line.clone(),
                }),
                NewItem::Object(_) => None,
            })
            .collect())
    }

    fn heads(&self, subject: &SubjectKey, lane: &str) -> Result<Vec<String>, StoreError> {
        Ok(self.heads.get(subject, lane))
    }

    fn head_rows(&self) -> Result<Vec<(SubjectKey, String, String)>, StoreError> {
        Ok(self.heads.rows())
    }

    fn checkpoints(&self) -> Result<Vec<CheckpointInfo>, StoreError> {
        Ok(self.checkpoints.clone())
    }

    fn leaf_hashes(&self, upto: u64) -> Result<Vec<Digest>, StoreError> {
        Ok(self.up_to(upto)?.iter().map(|new| new.leaf).collect())
    }
}

impl LogWrite for MemLog {
    fn epoch(&self) -> u64 {
        self.epoch
    }

    /// Reads the clock once, plans with `intake::plan_commit(self, …)`, and
    /// applies the plan.
    fn commit(&mut self, items: &[Item<'_>]) -> Result<Vec<Receipt>, StoreError> {
        let now = self.clock.now();
        let plan = intake::plan_commit(self, &self.log_id, self.epoch, now, items)?;
        self.apply(plan)
    }

    /// `intake::check_store_key`, then one clock reading,
    /// `intake::plan_checkpoint(self, …)`, and the plan applied.
    fn checkpoint(&mut self, signer: &dyn Signer) -> Result<CheckpointInfo, StoreError> {
        intake::check_store_key(&self.genesis, signer)?;
        let now = self.clock.now();
        let plan =
            intake::plan_checkpoint(self, &self.genesis, &self.log_id, self.epoch, signer, now)?;
        let checkpoint = plan
            .checkpoint
            .clone()
            .ok_or_else(|| StoreError::Io("a checkpoint plan without its checkpoint".into()))?;
        self.apply(plan)?;
        Ok(checkpoint)
    }
}

/// The planner's view of a `MemLog`: never fails, except `root` above the
/// head (`E_USAGE`).
impl IntakeState for MemLog {
    fn tip(&self) -> Result<Tip, StoreError> {
        let last = self
            .entries
            .last()
            .ok_or_else(|| StoreError::Io("the log has no entries".into()))?;
        let received_at = time::parse(&last.entry.received_at).map_err(|_| {
            StoreError::Io(format!(
                "seq {} has received_at {:?}",
                last.entry.seq, last.entry.received_at
            ))
        })?;
        Ok(Tip {
            seq: last.entry.seq,
            entry_hash: last.hash.clone(),
            received_at,
        })
    }

    fn receipt(&self, reference: &str) -> Result<Option<Receipt>, StoreError> {
        Ok(self
            .refs
            .get(reference)
            .and_then(|seq| self.entry(*seq))
            .map(NewEntry::receipt))
    }

    fn object_at(&self, id: &str) -> Result<Option<Timestamp>, StoreError> {
        Ok(self.prev_at(id))
    }

    fn heads(&self, subject: &SubjectKey, lane: &str) -> Result<Vec<String>, StoreError> {
        LogRead::heads(self, subject, lane)
    }

    fn latest_checkpoint(&self) -> Result<Option<CheckpointInfo>, StoreError> {
        Ok(self.checkpoints.last().cloned())
    }

    fn root(&self, size: u64) -> Result<Digest, StoreError> {
        Ok(merkle::root(&self.leaf_hashes(size)?))
    }
}

/// `prev_at(id)`: the `at` of the held object with this id, or None.
impl PrevLookup for MemLog {
    fn prev_at(&self, id: &str) -> Option<Timestamp> {
        self.objects.get(id).map(|(_, at)| *at)
    }
}
