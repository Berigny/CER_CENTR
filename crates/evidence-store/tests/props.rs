//! T5, the property test (`docs/m1-store-spec.md` section 11).
//!
//! Random scripts of up to 200 operations over 5 subjects: creates, updates
//! with the current heads or stale ones, deletes, asserts in 2 schemas,
//! merges, attestations signed with the test keys (and some whose target is
//! not held), repeated items, batches (one that lands and one that fails at
//! its last item), checkpoints, reopens, and store clock steps backwards
//! and below a millisecond.
//!
//! Each operation's outcome is checked against a model as it runs: a new
//! entry's seq and `received_at`, a duplicate's receipt, a rejection's code,
//! index and heads, that nothing of a rejected batch lands, and that every
//! transaction reads the clock exactly once. At the end of a script these
//! must hold ([`check`]):
//!
//! - `verify_log` is ok, and every line has the epoch and `received_at` the
//!   model expects;
//! - the heads projection equals the fold over the lines, and the model;
//! - every checkpoint's root and head recompute from the leaves;
//! - the inclusion proof of every seq verifies at the latest checkpoint's
//!   size and at the head (at earlier checkpoints' sizes, those of the first
//!   and last seq);
//! - consecutive checkpoints, and the latest and the head, have consistency
//!   proofs that verify;
//! - the export holds exactly the lines, the head and the latest checkpoint.
//!
//! Every script runs against `MemLog` and against `SqliteLog`, each checked
//! against its own model as it runs. Then ([`check_sqlite`]) the two must
//! hold the same log and give byte-identical exports, `verify_store` must be
//! ok, and import(export) must give a store that verifies with the same head
//! entry hash and holds the same log. The fixed script of
//! [`every_operation_runs`] takes every kind of operation.
//!
//! The cases default to proptest's 256; `PROPTEST_CASES` sets another
//! number (the nightly run uses 4,096). Signing and verifying are most of
//! the cost: every attestation and checkpoint is verified at intake and
//! again by each verification (`verify_log`, `verify_store`, and the
//! `verify_export` and `verify_store` of an import). The workspace builds
//! the Ed25519 and SHA-256 crates optimised even in test builds for this.
//!
//! Data: synthetic objects built here and the test keys in
//! `test-vectors/keys`. Nothing is real.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Mutex, OnceLock};

use evidence_core::canon;
use evidence_core::json::Value;
use evidence_core::time::{self, Timestamp};
use evidence_core::Code;
use evidence_log::clock::{format_millis, next_received};
use evidence_log::merkle;
use evidence_log::{
    attestation_ref, lane_of, parse_line, subject_of, Body, Heads, Kind, Manifest, Source,
    StoreReport, SubjectKey, VerifyOptions, RECORD,
};
use evidence_store::{
    consistency, export, export_files, import, inclusion, verify_log, verify_store, CheckpointInfo,
    Item, LogRead, LogWrite, Receipt, RejectCode, ScriptClock, SeedSigner, Signer, SqliteReader,
    StoreError,
};
use proptest::prelude::*;

mod common;
use common::{rejected, same_log, sealed, seed, ts, Backend, Mem, Obj, Sqlite, ALL};

// ---------------------------------------------------------------------------
// Data

/// The store's name.
const NAME: &str = "props";
/// The subjects are `props-test/thing/S0` to `S4`.
const SUBJECTS: usize = 5;
const SYSTEM: &str = "props-test";
const TYPE: &str = "thing";
const RECORD_SCHEMA: &str = "props-test/thing@1";
/// Two assert schemas, so two assert lanes per subject.
const ASSERT_SCHEMAS: [&str; 2] = ["props-test/check@1", "props-test/review@2"];
/// The test keys attestations are signed with.
const KEYS: [&str; 3] = ["assessor-01", "photo-check-agent", "rec-registry-cdc"];
/// The first clock reading, and the origin of every object's `at`.
const BASE: &str = "2026-11-26T00:00:00Z";
/// An id that nothing holds, used as a prev and in `if_heads`.
const NOBODY: &str = "sha256:0000000000000000000000000000000000000000000000000000000000000000";
/// An id that nothing holds, used as an attestation's target.
const UNHELD: &str = "sha256:1111111111111111111111111111111111111111111111111111111111111111";

/// The store key, signing through the cache of [`sign_once`].
struct StoreKey;

impl Signer for StoreKey {
    fn public_key(&self) -> String {
        keyring()[STORE].public_key()
    }

    fn sign(
        &self,
        object_id: &str,
        purpose: &str,
        signer: &str,
        at: &str,
    ) -> Result<String, StoreError> {
        sign_once(STORE, object_id, purpose, signer, at)
    }
}

/// [`KEYS`], then the store key at [`STORE`].
fn keyring() -> &'static [SeedSigner] {
    static KEYRING: OnceLock<Vec<SeedSigner>> = OnceLock::new();
    KEYRING.get_or_init(|| {
        KEYS.iter()
            .map(|k| seed(k))
            .chain([seed("store-demo")])
            .collect()
    })
}

/// The index of the store key in [`keyring`].
const STORE: usize = KEYS.len();

/// `keyring()[key].sign(…)`, each line signed once per process.
///
/// Signing is slow in an unoptimised build. Every script's genesis is the
/// same, so its author line is signed once, and a script that runs against
/// two backends, or again while proptest shrinks it, reuses its lines. The
/// store still checks every line it is given.
fn sign_once(
    key: usize,
    id: &str,
    purpose: &str,
    signer: &str,
    at: &str,
) -> Result<String, StoreError> {
    static LINES: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    let lines = LINES.get_or_init(Mutex::default);
    let name = format!("{key} {id} {purpose} {signer} {at}");
    if let Some(line) = lines.lock().unwrap().get(&name) {
        return Ok(line.clone());
    }
    let line = keyring()[key].sign(id, purpose, signer, at)?;
    lines.lock().unwrap().insert(name, line.clone());
    Ok(line)
}

/// The canonical attestation line over `id` with this purpose and `at`,
/// signed with test key `key` as signer `service:<key name>`.
fn signed(id: &str, purpose: &str, key: usize, at: &str) -> Vec<u8> {
    let signer = format!("service:{}", KEYS[key]);
    sign_once(key, id, purpose, &signer, at)
        .unwrap()
        .into_bytes()
}

fn subject(s: usize) -> SubjectKey {
    SubjectKey {
        system: SYSTEM.into(),
        r#type: TYPE.into(),
        key: format!("S{s}"),
    }
}

/// `"assert:"` + the schema's name.
fn assert_lane(schema: &str) -> String {
    format!("assert:{}", schema.split('@').next().unwrap())
}

/// A sealed synthetic object about subject `s`: inline content `{"n":…}`,
/// the actor `service:props-test`.
fn build(op: &str, schema: &str, s: usize, n: u64, at: &str, prev: &[String]) -> Obj {
    let inline = Value::Object(vec![("n".into(), Value::Number(n as f64))]);
    let key = format!("S{s}");
    sealed(
        op,
        schema,
        [SYSTEM, TYPE, &key],
        inline,
        "props-test",
        at,
        prev,
    )
}

// ---------------------------------------------------------------------------
// Scripts

/// One operation of a script. A `target` picks, modulo their number, among
/// the items the model holds when the operation runs.
#[derive(Clone, Debug)]
enum Op {
    /// A create, with no prev: a fork if the subject has heads.
    Create(usize),
    /// An update over the current record heads, given as `if_heads`; or,
    /// when `stale`, with `if_heads` an older or made-up set, which must be
    /// refused with `S_STALE_HEADS` and the current heads.
    Update {
        subject: usize,
        stale: bool,
    },
    /// A delete over the current record heads.
    Delete(usize),
    /// An assert in one of two schemas, over that lane's heads.
    Assert {
        subject: usize,
        schema: usize,
    },
    /// An update listing every record head, with `if_heads` in another
    /// order and with a repeat; a create if there are fewer than two.
    Merge(usize),
    /// An attestation of a held object, signed with a test key.
    Attest {
        target: usize,
        key: usize,
        approve: bool,
    },
    /// An attestation, signed with a test key, of an object that is not
    /// held: `S_ATTESTATION_TARGET`.
    AttestMissing(usize),
    /// An object or attestation line given again: a duplicate, with the
    /// first receipt. With `stale`, an object is given a wrong `if_heads`,
    /// which is not evaluated.
    Repeat {
        target: usize,
        stale: bool,
    },
    /// One batch: an object, its attestation, and the object again.
    Batch {
        subject: usize,
        key: usize,
    },
    /// One batch that fails at its last item: two creates, then an update
    /// whose prev is not held. Nothing lands.
    BadBatch(usize),
    Checkpoint,
    Reopen,
    /// The store clock steps back this many milliseconds.
    ClockBack(u32),
}

/// An operation and how far the store clock moves on before it, in
/// nanoseconds (often less than a millisecond).
#[derive(Clone, Debug)]
struct Step {
    tick: u64,
    op: Op,
}

fn op() -> impl Strategy<Value = Op> {
    let s = || 0..SUBJECTS;
    prop_oneof![
        4 => s().prop_map(Op::Create),
        6 => (s(), prop::bool::weighted(0.25))
            .prop_map(|(subject, stale)| Op::Update { subject, stale }),
        1 => s().prop_map(Op::Delete),
        3 => (s(), 0..ASSERT_SCHEMAS.len())
            .prop_map(|(subject, schema)| Op::Assert { subject, schema }),
        2 => s().prop_map(Op::Merge),
        1 => (any::<usize>(), 0..KEYS.len(), any::<bool>())
            .prop_map(|(target, key, approve)| Op::Attest { target, key, approve }),
        1 => (0..KEYS.len()).prop_map(Op::AttestMissing),
        1 => (any::<usize>(), any::<bool>())
            .prop_map(|(target, stale)| Op::Repeat { target, stale }),
        1 => (s(), 0..KEYS.len()).prop_map(|(subject, key)| Op::Batch { subject, key }),
        1 => s().prop_map(Op::BadBatch),
        1 => Just(Op::Checkpoint),
        1 => Just(Op::Reopen),
        1 => (1u32..=3_600_000).prop_map(Op::ClockBack),
    ]
}

fn step() -> impl Strategy<Value = Step> {
    let tick = prop_oneof![0u64..1_000_000, 1_000_000u64..3_000_000_000];
    (tick, op()).prop_map(|(tick, op)| Step { tick, op })
}

/// Up to 200 operations: seven scripts in eight have at most 40, which
/// keeps 256 cases quick enough in an unoptimised build (signing and
/// verifying are the slow part), and one in eight up to 200.
fn script() -> impl Strategy<Value = Vec<Step>> {
    prop_oneof![
        7 => prop::collection::vec(step(), 0..=40),
        1 => prop::collection::vec(step(), 0..=200),
    ]
}

// ---------------------------------------------------------------------------
// Running a script

/// What the log must hold, kept beside it as the script runs.
struct Model {
    clock: ScriptClock,
    /// The next clock reading.
    now: Timestamp,
    /// Objects and attestations built so far: numbers their content and
    /// their `at` (seconds after [`BASE`]).
    built: u64,
    epoch: u64,
    /// Per seq − 1: the entry's epoch and `received_at`.
    epochs: Vec<u64>,
    received: Vec<String>,
    /// The heads fold over the objects of this test's subjects.
    heads: Heads,
    /// Per subject: every set of record heads it has had, oldest first.
    history: Vec<Vec<Vec<String>>>,
    /// Held objects and attestation lines with their first receipts.
    objects: Vec<(Obj, Receipt)>,
    attestations: Vec<(Vec<u8>, Receipt)>,
    checkpoints: Vec<CheckpointInfo>,
}

impl Model {
    fn size(&self) -> u64 {
        self.received.len() as u64
    }

    /// The next `at` of an object or attestation this test builds.
    fn claim(&mut self) -> (u64, String) {
        self.built += 1;
        let at = time::format_secs(ts(BASE).secs + self.built as i64);
        (self.built, at)
    }

    fn record(&self, s: usize) -> Vec<String> {
        self.heads.get(&subject(s), RECORD)
    }

    /// An object about subject `s`.
    fn object(&mut self, op: &str, schema: &str, s: usize, prev: &[String]) -> Obj {
        let (n, at) = self.claim();
        build(op, schema, s, n, &at, prev)
    }

    /// An attestation line over `id`, signed with test key `key`.
    fn line(&mut self, id: &str, purpose: &str, key: usize) -> Vec<u8> {
        let (_, at) = self.claim();
        signed(id, purpose, key, &at)
    }

    /// The `received_at` of the entries of a transaction read at `now`.
    fn received_at(&self, now: Timestamp) -> String {
        let last = self.received.last().map(|r| ts(r));
        format_millis(next_received(now, last))
    }

    /// Record `n` new entries of one transaction.
    fn landed(&mut self, n: usize, received_at: &str) {
        for _ in 0..n {
            self.epochs.push(self.epoch);
            self.received.push(received_at.to_string());
        }
    }

    /// Fold in an object that landed.
    fn fold(&mut self, value: &Value) {
        let id = value.get("id").and_then(Value::as_str).unwrap();
        let prev: Vec<String> = value
            .get("prev")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .map(|p| p.as_str().unwrap().to_string())
            .collect();
        let key = subject_of(value);
        self.heads.apply(id, &key, &lane_of(value), &prev);
        let s: usize = key.key[1..].parse().unwrap();
        let record = self.record(s);
        if self.history[s].last() != Some(&record) {
            self.history[s].push(record);
        }
    }
}

/// A script running against one backend.
struct Run<B> {
    backend: B,
    model: Model,
}

/// The new receipt of an item that landed at `seq`.
#[track_caller]
fn assert_new(receipt: &Receipt, kind: Kind, reference: &str, seq: u64, received_at: &str) {
    assert_eq!(receipt.kind, kind);
    assert_eq!(receipt.reference, reference);
    assert_eq!(receipt.seq, seq);
    assert_eq!(receipt.received_at, received_at);
    assert!(!receipt.duplicate);
}

impl<B: Backend> Run<B> {
    fn init() -> Run<B> {
        let clock = ScriptClock::default();
        let now = ts(BASE);
        clock.push(now);
        let backend = B::init(NAME, &StoreKey, &clock).unwrap();
        assert_eq!(clock.remaining(), 0, "init reads the clock once");
        let mut model = Model {
            clock,
            now,
            built: 0,
            epoch: 1,
            epochs: Vec::new(),
            received: Vec::new(),
            heads: Heads::default(),
            history: vec![vec![Vec::new()]; SUBJECTS],
            objects: Vec::new(),
            attestations: Vec::new(),
            checkpoints: Vec::new(),
        };
        let received_at = model.received_at(now);
        model.landed(2, &received_at);
        Run { backend, model }
    }

    fn log(&self) -> &dyn LogWrite {
        self.backend.log()
    }

    /// One transaction, given the clock reading `now`, which it must use.
    /// Returns its result and the `received_at` of any entry it adds.
    #[track_caller]
    fn write<T>(&mut self, op: impl FnOnce(&mut dyn LogWrite) -> T) -> (T, String) {
        let now = self.model.now;
        let received_at = self.model.received_at(now);
        self.model.clock.push(now);
        let out = op(self.backend.log_mut());
        assert_eq!(
            self.model.clock.remaining(),
            0,
            "a transaction reads the clock exactly once"
        );
        (out, received_at)
    }

    /// Append `obj`, with `if_heads` if given, and expect a new entry.
    #[track_caller]
    fn append_new(&mut self, obj: Obj, if_heads: Option<&[String]>) {
        let seq = self.model.size() + 1;
        let (receipt, received_at) = self.write(|log| match if_heads {
            Some(expected) => log.append_if_heads(&obj.bytes, expected),
            None => log.append(&obj.bytes),
        });
        let receipt = receipt.unwrap();
        assert_new(&receipt, Kind::Object, &obj.id, seq, &received_at);
        self.model.landed(1, &received_at);
        self.model
            .fold(&evidence_core::json::parse(&obj.bytes).unwrap());
        self.model.objects.push((obj, receipt));
    }

    fn step(&mut self, step: &Step) {
        self.model.now = advance(self.model.now, step.tick);
        match step.op.clone() {
            Op::Create(s) => {
                let obj = self.model.object("create", RECORD_SCHEMA, s, &[]);
                self.append_new(obj, None);
            }
            Op::Update { subject: s, stale } => {
                let current = self.model.record(s);
                let obj = self.model.object("update", RECORD_SCHEMA, s, &current);
                if !stale {
                    self.append_new(obj, Some(&current));
                    return;
                }
                let older = self.model.history[s]
                    .iter()
                    .rev()
                    .find(|h| !same_set(h, &current))
                    .cloned();
                let expected = older.unwrap_or_else(|| match current.is_empty() {
                    true => vec![NOBODY.to_string()],
                    false => Vec::new(),
                });
                let (result, _) = self.write(|log| log.append_if_heads(&obj.bytes, &expected));
                let r = rejected(result);
                assert_eq!(r.code, RejectCode::StaleHeads);
                assert_eq!((r.index, r.heads), (0, current));
            }
            Op::Delete(s) => {
                let current = self.model.record(s);
                let obj = self.model.object("delete", RECORD_SCHEMA, s, &current);
                self.append_new(obj, None);
            }
            Op::Assert { subject: s, schema } => {
                let schema = ASSERT_SCHEMAS[schema];
                let current = self.model.heads.get(&subject(s), &assert_lane(schema));
                let obj = self.model.object("assert", schema, s, &current);
                self.append_new(obj, None);
            }
            Op::Merge(s) => {
                let current = self.model.record(s);
                if current.len() < 2 {
                    let obj = self.model.object("create", RECORD_SCHEMA, s, &[]);
                    self.append_new(obj, None);
                    return;
                }
                let obj = self.model.object("update", RECORD_SCHEMA, s, &current);
                // The same set, in another order, with a repeat.
                let mut expected: Vec<String> = current.iter().rev().cloned().collect();
                expected.push(current[0].clone());
                self.append_new(obj, Some(&expected));
                assert_eq!(self.model.record(s).len(), 1, "a merge leaves one head");
            }
            Op::Attest {
                target,
                key,
                approve,
            } => {
                if self.model.objects.is_empty() {
                    return;
                }
                let id = self.model.objects[target % self.model.objects.len()]
                    .0
                    .id
                    .clone();
                let purpose = if approve { "approve" } else { "author" };
                let line = self.model.line(&id, purpose, key);
                self.attest_new(line);
            }
            Op::AttestMissing(key) => {
                let line = signed(UNHELD, "author", key, BASE);
                let (result, _) = self.write(|log| log.attest(&line));
                let r = rejected(result);
                assert_eq!((r.index, r.code), (0, RejectCode::AttestationTarget));
                assert!(r.heads.is_empty());
            }
            Op::Repeat { target, stale } => {
                let (objects, lines) = (self.model.objects.len(), self.model.attestations.len());
                if objects + lines == 0 {
                    return;
                }
                let i = target % (objects + lines);
                let (result, _) = if i < objects {
                    let (obj, _) = &self.model.objects[i];
                    let bytes = obj.bytes.clone();
                    let wrong = [NOBODY.to_string()];
                    self.write(|log| match stale {
                        true => log.append_if_heads(&bytes, &wrong),
                        false => log.append(&bytes),
                    })
                } else {
                    let line = self.model.attestations[i - objects].0.clone();
                    self.write(|log| log.attest(&line))
                };
                let first = match i < objects {
                    true => &self.model.objects[i].1,
                    false => &self.model.attestations[i - objects].1,
                };
                let want = Receipt {
                    duplicate: true,
                    ..first.clone()
                };
                assert_eq!(result.unwrap(), want);
            }
            Op::Batch { subject: s, key } => {
                let current = self.model.record(s);
                let op = if current.is_empty() {
                    "create"
                } else {
                    "update"
                };
                let obj = self.model.object(op, RECORD_SCHEMA, s, &current);
                let line = self.model.line(&obj.id, "author", key);
                let seq = self.model.size() + 1;
                let items = [
                    Item::Object {
                        bytes: &obj.bytes,
                        if_heads: Some(&current),
                    },
                    Item::Attestation { line: &line },
                    Item::Object {
                        bytes: &obj.bytes,
                        if_heads: None,
                    },
                ];
                let (receipts, received_at) = self.write(|log| log.commit(&items));
                let receipts = receipts.unwrap();
                assert_eq!(receipts.len(), 3);
                assert_new(&receipts[0], Kind::Object, &obj.id, seq, &received_at);
                let reference = attestation_ref(&evidence_core::json::parse(&line).unwrap());
                assert_new(
                    &receipts[1],
                    Kind::Attestation,
                    &reference,
                    seq + 1,
                    &received_at,
                );
                let again = Receipt {
                    duplicate: true,
                    ..receipts[0].clone()
                };
                assert_eq!(receipts[2], again);
                self.model.landed(2, &received_at);
                self.model
                    .fold(&evidence_core::json::parse(&obj.bytes).unwrap());
                self.model.objects.push((obj, receipts[0].clone()));
                self.model.attestations.push((line, receipts[1].clone()));
            }
            Op::BadBatch(s) => {
                let head = self.log().head().unwrap();
                let fresh = self.model.object("create", RECORD_SCHEMA, s, &[]);
                let other = (s + 1) % SUBJECTS;
                let second = self.model.object("create", RECORD_SCHEMA, other, &[]);
                let orphan = self
                    .model
                    .object("update", RECORD_SCHEMA, s, &[NOBODY.to_string()]);
                let items = [
                    Item::Object {
                        bytes: &fresh.bytes,
                        if_heads: None,
                    },
                    Item::Object {
                        bytes: &second.bytes,
                        if_heads: None,
                    },
                    Item::Object {
                        bytes: &orphan.bytes,
                        if_heads: None,
                    },
                ];
                let (result, _) = self.write(|log| log.commit(&items));
                let r = rejected(result);
                assert_eq!((r.index, r.code), (2, RejectCode::Core(Code::PrevMissing)));
                assert_eq!(self.log().head().unwrap(), head, "nothing lands");
            }
            Op::Checkpoint => {
                let head = self.log().head().unwrap();
                let (info, received_at) = self.write(|log| log.checkpoint(&StoreKey));
                let info = info.unwrap();
                assert_eq!((info.seq, info.size), (head.seq + 1, head.seq));
                assert_eq!(info.head, head.entry_hash);
                self.model.landed(2, &received_at);
                self.model.checkpoints.push(info);
            }
            Op::Reopen => {
                self.backend.reopen(&self.model.clock);
                assert_eq!(self.model.clock.remaining(), 0, "a reopen reads no clock");
                self.model.epoch += 1;
                assert_eq!(self.log().epoch(), self.model.epoch);
            }
            Op::ClockBack(millis) => self.model.now = back(self.model.now, millis),
        }
    }

    /// Attest `line` and expect a new entry.
    #[track_caller]
    fn attest_new(&mut self, line: Vec<u8>) {
        let seq = self.model.size() + 1;
        let (receipt, received_at) = self.write(|log| log.attest(&line));
        let receipt = receipt.unwrap();
        let reference = attestation_ref(&evidence_core::json::parse(&line).unwrap());
        assert_new(&receipt, Kind::Attestation, &reference, seq, &received_at);
        self.model.landed(1, &received_at);
        self.model.attestations.push((line, receipt));
    }
}

/// `t` plus `nanos` nanoseconds.
fn advance(t: Timestamp, nanos: u64) -> Timestamp {
    let total = u64::from(t.nanos) + nanos;
    Timestamp {
        secs: t.secs + (total / 1_000_000_000) as i64,
        nanos: (total % 1_000_000_000) as u32,
    }
}

/// `t` minus `millis` milliseconds.
fn back(t: Timestamp, millis: u32) -> Timestamp {
    let total =
        i128::from(t.secs) * 1_000_000_000 + i128::from(t.nanos) - i128::from(millis) * 1_000_000;
    Timestamp {
        secs: total.div_euclid(1_000_000_000) as i64,
        nanos: total.rem_euclid(1_000_000_000) as u32,
    }
}

fn same_set(a: &[String], b: &[String]) -> bool {
    let a: BTreeSet<&String> = a.iter().collect();
    let b: BTreeSet<&String> = b.iter().collect();
    a == b
}

/// Run a script against a new `B`.
fn run<B: Backend>(steps: &[Step]) -> Run<B> {
    let mut run = Run::<B>::init();
    for step in steps {
        run.step(step);
        assert_eq!(run.log().head().unwrap().seq, run.model.size());
    }
    run
}

// ---------------------------------------------------------------------------
// The properties

/// Check every property of a log at the end of a script. Returns the
/// `verify_log` report and the number of inclusion proofs checked.
fn check(log: &dyn LogRead, model: &Model, segment_size: u64) -> (StoreReport, usize) {
    let head = log.head().unwrap();
    assert_eq!(head.seq, model.size());

    // verify_log is ok.
    let report = verify_log(log, VerifyOptions::default()).unwrap();
    assert!(report.ok(), "verify_log: {:?}", report.findings);
    assert_eq!(report.source, Source::Store);
    assert_eq!(report.log.as_deref(), Some(log.log_id()));
    assert_eq!(report.head, Some((head.seq, head.entry_hash.clone())));

    // The lines: dense, with the epochs and received_at of the model, and
    // the receipts given for them.
    let lines = log.lines(1, ALL).unwrap();
    assert_eq!(lines.len() as u64, head.seq);
    let mut fold = Heads::default();
    let mut entries = Vec::new();
    let mut leaves = Vec::new();
    for (n, (seq, text)) in (1..).zip(&lines) {
        assert_eq!(*seq, n);
        let line = parse_line(text.as_bytes()).unwrap();
        let i = n as usize - 1;
        assert_eq!(line.entry.epoch, model.epochs[i], "epoch of seq {n}");
        assert_eq!(line.entry.received_at, model.received[i], "seq {n}");
        if let Body::Object(value) = &line.body {
            let prev: Vec<String> = value
                .get("prev")
                .and_then(Value::as_array)
                .unwrap()
                .iter()
                .map(|p| p.as_str().unwrap().to_string())
                .collect();
            fold.apply(
                &line.entry.reference,
                &subject_of(value),
                &lane_of(value),
                &prev,
            );
        }
        leaves.push(merkle::leaf_hash(line.entry_canonical.as_bytes()));
        entries.push(line.entry);
    }
    let objects = model.objects.iter().map(|(_, receipt)| receipt);
    let attestations = model.attestations.iter().map(|(_, receipt)| receipt);
    for receipt in objects.chain(attestations) {
        let entry = &entries[receipt.seq as usize - 1];
        assert_eq!(receipt.reference, entry.reference);
        assert_eq!(receipt.entry_hash, entry.hash());
        assert_eq!(receipt.received_at, entry.received_at);
    }
    assert_eq!(log.leaf_hashes(head.seq).unwrap(), leaves);

    // The heads projection is the fold over the lines, and the model's.
    let rows = log.head_rows().unwrap();
    assert_eq!(rows, fold.rows());
    let ours: Vec<_> = rows
        .iter()
        .filter(|(s, _, _)| s.system == SYSTEM)
        .cloned()
        .collect();
    assert_eq!(ours, model.heads.rows());
    for s in 0..SUBJECTS {
        let lanes = [RECORD.to_string()]
            .into_iter()
            .chain(ASSERT_SCHEMAS.iter().map(|schema| assert_lane(schema)));
        for lane in lanes {
            assert_eq!(
                log.heads(&subject(s), &lane).unwrap(),
                model.heads.get(&subject(s), &lane)
            );
        }
    }

    // Every checkpoint's root and head recompute.
    let checkpoints = log.checkpoints().unwrap();
    assert_eq!(checkpoints, model.checkpoints);
    for c in &checkpoints {
        let size = c.size as usize;
        assert_eq!(c.root, merkle::to_text(&merkle::root(&leaves[..size])));
        assert_eq!(c.head, entries[size - 1].hash());
        assert_eq!(entries[c.seq as usize - 1].reference, c.id);
        assert_eq!(
            entries[c.seq as usize].kind,
            Kind::Attestation,
            "its witness"
        );
    }

    // The inclusion proof of every seq verifies at the latest checkpoint's
    // size and at the head, and at each earlier checkpoint's size, those of
    // its first and last seq do. (A checkpoint's witness is always newer, so
    // the head is above every checkpoint's size.)
    let latest = checkpoints.last().map(|c| c.size);
    let mut sizes: Vec<u64> = checkpoints.iter().map(|c| c.size).collect();
    sizes.push(head.seq);
    let mut proofs = 0;
    for &size in &sizes {
        let root = merkle::root(&leaves[..size as usize]);
        let named = checkpoints
            .iter()
            .find(|c| c.size == size)
            .map(|c| c.id.clone());
        let seqs: Vec<u64> = if Some(size) == latest || size == head.seq {
            (1..=size).collect()
        } else {
            vec![1, size]
        };
        for seq in seqs {
            let p = inclusion(log, seq, size).unwrap();
            assert!(p.verify(), "seq {seq} in size {size}");
            assert_eq!(p.leaf_hash, leaves[seq as usize - 1]);
            assert_eq!(p.root, root);
            assert_eq!(p.entry, entries[seq as usize - 1]);
            assert_eq!(p.checkpoint, named);
            proofs += 1;
        }
    }

    // Consecutive checkpoints, and the latest and the head, have
    // consistency proofs that verify.
    for pair in sizes.windows(2) {
        let p = consistency(log, pair[0], pair[1]).unwrap();
        assert!(p.verify(), "{} to {}", pair[0], pair[1]);
        assert_eq!(p.old_root, merkle::root(&leaves[..pair[0] as usize]));
        assert_eq!(p.new_root, merkle::root(&leaves[..pair[1] as usize]));
    }

    // The export holds exactly the lines, each with its LF, and its
    // manifest the log id, the head and the latest checkpoint. (verify_log
    // has verified these lines; verifying the export again would only
    // repeat the signature checks.)
    let files = export_files(log, segment_size).unwrap();
    let m = &files.manifest;
    let (manifest, segments) = files.files.split_last().unwrap();
    assert_eq!(manifest.1, m.canonical().into_bytes());
    assert_eq!(Manifest::parse(&manifest.1).as_ref(), Ok(m));
    assert_eq!(m.log, log.log_id());
    assert_eq!(
        (m.size, m.head.as_str()),
        (head.seq, head.entry_hash.as_str())
    );
    assert_eq!(
        m.checkpoint,
        checkpoints.last().map(|c| (c.id.clone(), c.size))
    );
    assert_eq!(m.segments.len(), segments.len());
    for (listed, (file, bytes)) in m.segments.iter().zip(segments) {
        assert_eq!(&listed.file, file);
        assert_eq!(listed.hash, canon::sha256_hex(bytes));
    }
    let exported: Vec<u8> = segments.iter().flat_map(|(_, b)| b.clone()).collect();
    let mut want = Vec::new();
    for (_, line) in &lines {
        want.extend_from_slice(line.as_bytes());
        want.push(b'\n');
    }
    assert!(exported == want, "the segments hold the lines");
    (report, proofs)
}

/// The SQLite part of T5, for a script that ran against both backends:
///
/// - the two hold the same log (leaf hashes, heads and checkpoints), so every
///   property [`check`] found of `mem` holds of the store too, and
///   `SqliteLog` exports the bytes `MemLog` does;
/// - a separate `SqliteReader` sees what the writer holds;
/// - `verify_store` of the store directory is ok, with the same head and
///   covered checkpoint as `verify_log`;
/// - the export imports into a store that verifies, with the same head
///   entry hash, and holds the same log.
///
/// `live` is the `verify_log` report of `mem`.
fn check_sqlite(mem: &dyn LogRead, live: &StoreReport, sqlite: &Sqlite, segment_size: u64) {
    same_log(mem, sqlite.log(), segment_size);
    let reader = SqliteReader::open(&sqlite.dir()).unwrap();
    assert_eq!(
        reader.lines(1, ALL).unwrap(),
        sqlite.log().lines(1, ALL).unwrap()
    );
    same_log(sqlite.log(), &reader, segment_size);
    let report = verify_store(&sqlite.dir(), VerifyOptions::default()).unwrap();
    assert!(report.ok(), "verify_store: {:?}", report.findings);
    assert_eq!(report.source, Source::Store);
    assert_eq!((&report.head, &report.covered), (&live.head, &live.covered));

    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("export");
    export(sqlite.log(), &out, segment_size).unwrap();
    let into = tmp.path().join("imported");
    let imported = import(&out, &into).unwrap();
    assert!(imported.ok(), "import: {:?}", imported.findings);
    assert_eq!(imported.head, live.head, "import(export) has the same head");
    let head = sqlite.log().head().unwrap();
    assert_eq!(imported.head, Some((head.seq, head.entry_hash)));
    let reader = SqliteReader::open(&into).unwrap();
    same_log(mem, &reader, segment_size);
}

proptest! {
    // proptest's default configuration: 256 cases unless PROPTEST_CASES
    // says otherwise.
    #![proptest_config(ProptestConfig::default())]

    /// Every script runs against `MemLog` and `SqliteLog`; the two must
    /// hold the same log, and the SQLite store must survive the export and
    /// import round trip.
    #[test]
    fn random_scripts_keep_every_property(
        steps in script(),
        segment_size in 1u64..=40,
    ) {
        let mem = run::<Mem>(&steps);
        let (live, _) = check(mem.log(), &mem.model, segment_size);
        let sqlite = run::<Sqlite>(&steps);
        check_sqlite(mem.log(), &live, &sqlite.backend, segment_size);
    }
}

/// A fixed script that takes every kind of operation at least once, so
/// each is known to run whatever proptest generates.
#[test]
fn every_operation_runs() {
    let ops = [
        Op::Create(0),
        Op::Create(0),
        Op::Merge(0),
        Op::Update {
            subject: 0,
            stale: true,
        },
        Op::Update {
            subject: 1,
            stale: true,
        },
        Op::Update {
            subject: 0,
            stale: false,
        },
        Op::Update {
            subject: 0,
            stale: true,
        },
        Op::Assert {
            subject: 0,
            schema: 0,
        },
        Op::Assert {
            subject: 0,
            schema: 1,
        },
        Op::Assert {
            subject: 0,
            schema: 0,
        },
        Op::Attest {
            target: 7,
            key: 0,
            approve: true,
        },
        Op::AttestMissing(2),
        Op::Checkpoint,
        Op::Repeat {
            target: 0,
            stale: true,
        },
        Op::Repeat {
            target: usize::MAX,
            stale: false,
        },
        Op::Batch { subject: 2, key: 1 },
        Op::Batch { subject: 2, key: 2 },
        Op::BadBatch(3),
        Op::ClockBack(3_600_000),
        Op::Create(4),
        Op::Checkpoint,
        Op::Reopen,
        Op::Checkpoint,
        Op::Delete(0),
        Op::Update {
            subject: 0,
            stale: false,
        },
        Op::Reopen,
        Op::ClockBack(1),
        Op::Create(1),
        Op::Checkpoint,
    ];
    let steps: Vec<Step> = ops
        .into_iter()
        .enumerate()
        .map(|(i, op)| Step {
            tick: (i as u64 % 3) * 400_000,
            op,
        })
        .collect();
    let mem = run::<Mem>(&steps);
    let sqlite = run::<Sqlite>(&steps);

    assert_eq!(mem.model.checkpoints.len(), 4);
    assert_eq!(mem.model.epoch, 3);
    let (live, proofs) = check(mem.log(), &mem.model, 4);
    check_sqlite(mem.log(), &live, &sqlite.backend, 4);
    assert!(proofs > 0);
    // received_at never fell. The clock stepped back an hour before the
    // create of S4, which therefore repeats the received_at of the entry
    // before it.
    let received = &mem.model.received;
    assert!(received.windows(2).all(|w| ts(&w[0]) <= ts(&w[1])));
    let s4 = mem
        .model
        .objects
        .iter()
        .find(|(obj, _)| {
            let value = evidence_core::json::parse(&obj.bytes).unwrap();
            subject_of(&value) == subject(4)
        })
        .map(|(_, receipt)| receipt.seq as usize)
        .unwrap();
    assert_eq!(received[s4 - 1], received[s4 - 2]);
}
