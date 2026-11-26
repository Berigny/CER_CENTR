//! T4, the contract suite (`docs/m1-store-spec.md` section 11), with the
//! library parts of T9 (two clocks) and T10 (export and import): the rules
//! every writer follows, run against `MemLog` and `SqliteLog`.
//!
//! Each scenario is a function generic over [`Backend`] (`tests/common`).
//! The `contract!` macro turns every scenario into a test for one backend,
//! so a backend joins the suite with one `Backend` impl and one `contract!`
//! line at the bottom of this file. Each scenario ends with [`audit`], which
//! checks what the log says about itself against its own lines and that
//! `verify_log` finds nothing, and then with the backend's own
//! [`Backend::check`]: for `SqliteLog`, a separate `SqliteReader` sees the
//! same log and `verify_store` (triggers and `meta.log_id` included) is ok.
//!
//! Every write goes through [`Store`], which pushes exactly one clock
//! reading before it and asserts afterwards that the reading was used. The
//! suite therefore also checks that every transaction reads the clock
//! exactly once, rejected and empty batches included. A refusal that must
//! read nothing is given no reading: a `ScriptClock` with no reading left
//! panics when read.
//!
//! Data: the object, attestation and key vectors in `test-vectors/`, and
//! synthetic objects built here. Nothing is real.

use evidence_core::attest;
use evidence_core::canon;
use evidence_core::json::{self, Value};
use evidence_core::object;
use evidence_core::time::Timestamp;
use evidence_core::Code;
use evidence_log::{
    attestation_ref, merkle, parse_line, Body, Checkpoint, Digest, Entry, Genesis, Kind, Source,
    StoreReport, SubjectKey, VerifyOptions,
};
use evidence_store::{
    export, export_files, import, read_export_dir, verify_export_dir, verify_log, verify_store,
    CheckpointInfo, Head, Item, LogRead, LogWrite, Receipt, RejectCode, ScriptClock, Signer,
    SqliteReader, StoreError, StoredAttestation, StoredObject,
};

mod common;
use common::{
    error_code, names, nth_line, read_vector, rejected, same_log, sealed, seed, store_key, ts,
    Backend, Mem, Obj, Sqlite, ALL,
};

// ---------------------------------------------------------------------------
// The harness

/// The store's name in every scenario.
const NAME: &str = "contract";

/// A log under test and the clock its writer reads.
struct Store<B: Backend> {
    backend: B,
    clock: ScriptClock,
}

impl<B: Backend> Store<B> {
    /// A store named [`NAME`], keyed by the store-demo seed, created with
    /// the one clock reading `at`.
    fn init(at: &str) -> Store<B> {
        let clock = ScriptClock::default();
        clock.push(ts(at));
        let backend = match B::init(NAME, &store_key(), &clock) {
            Ok(backend) => backend,
            Err(e) => panic!("init: {e}"),
        };
        assert_eq!(clock.remaining(), 0, "init reads the clock once");
        Store { backend, clock }
    }

    fn read(&self) -> &dyn LogRead {
        self.backend.log()
    }

    fn head(&self) -> Head {
        self.read().head().unwrap()
    }

    /// One write, given the one clock reading `at`, which it must use.
    #[track_caller]
    fn write<T>(&mut self, at: &str, op: impl FnOnce(&mut dyn LogWrite) -> T) -> T {
        self.clock.push(ts(at));
        let out = op(self.backend.log_mut());
        assert_eq!(
            self.clock.remaining(),
            0,
            "a transaction reads the clock exactly once"
        );
        out
    }

    /// One write that must read no clock: none is given.
    #[track_caller]
    fn unread<T>(&mut self, op: impl FnOnce(&mut dyn LogWrite) -> T) -> T {
        op(self.backend.log_mut())
    }

    #[track_caller]
    fn append(&mut self, at: &str, object: &Obj) -> Result<Receipt, StoreError> {
        self.write(at, |log| log.append(&object.bytes))
    }

    #[track_caller]
    fn append_if_heads(
        &mut self,
        at: &str,
        object: &Obj,
        expected: &[String],
    ) -> Result<Receipt, StoreError> {
        self.write(at, |log| log.append_if_heads(&object.bytes, expected))
    }

    #[track_caller]
    fn attest(&mut self, at: &str, line: &[u8]) -> Result<Receipt, StoreError> {
        self.write(at, |log| log.attest(line))
    }

    #[track_caller]
    fn commit(&mut self, at: &str, items: &[Item<'_>]) -> Result<Vec<Receipt>, StoreError> {
        self.write(at, |log| log.commit(items))
    }

    /// A checkpoint signed with the store key.
    #[track_caller]
    fn checkpoint(&mut self, at: &str) -> Result<CheckpointInfo, StoreError> {
        self.write(at, |log| log.checkpoint(&store_key()))
    }

    /// Close and reopen the writer. Reads no clock.
    fn reopen(&mut self) {
        self.backend.reopen(&self.clock);
        assert_eq!(self.clock.remaining(), 0);
    }
}

/// When a scenario ends, the backend's own checks ([`Backend::check`]): for
/// `SqliteLog`, a separate reader agrees with the writer and `verify_store`
/// is ok.
impl<B: Backend> Drop for Store<B> {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            self.backend.check();
        }
    }
}

/// Check what the log says about itself against its own lines, then verify
/// it:
///
/// - seqs are dense from 1 to the head, every entry carries the log id, and
///   each chains to the one before (seq 1 to the log id);
/// - `leaf_hashes` are the leaf hashes of the entries, and refuse to go
///   past the head;
/// - every object line's object is held with its seq, `received_at` and
///   canonical bytes, and every attestation line is among its target's
///   attestations, which come in seq order;
/// - `verify_log` is ok (no finding, projection drift included), with
///   source `store`, this log id and this head.
///
/// Returns the report.
#[track_caller]
fn audit(log: &dyn LogRead) -> StoreReport {
    let head = log.head().unwrap();
    let lines = log.lines(1, 100_000).unwrap();
    assert_eq!(
        lines.len() as u64,
        head.seq,
        "one line per seq up to the head"
    );
    let mut prev = log.log_id().to_string();
    let mut leaves = Vec::new();
    for (n, (seq, text)) in (1..).zip(&lines) {
        assert_eq!(*seq, n, "seqs are dense");
        let line = parse_line(text.as_bytes()).unwrap_or_else(|code| panic!("seq {seq}: {code}"));
        assert_eq!(line.entry.seq, *seq);
        assert_eq!(line.entry.log, log.log_id());
        assert_eq!(
            line.entry.prev, prev,
            "seq {seq} chains to the entry before"
        );
        prev = line.entry.hash();
        leaves.push(merkle::leaf_hash(line.entry_canonical.as_bytes()));
        let body = canon::canonicalize(line.body.value());
        match &line.body {
            Body::Object(_) => {
                let held = log.object(&line.entry.reference).unwrap();
                let expected = StoredObject {
                    id: line.entry.reference.clone(),
                    seq: *seq,
                    received_at: line.entry.received_at.clone(),
                    bytes: body,
                };
                assert_eq!(held, Some(expected));
            }
            Body::Attestation(value) => {
                let target = value.get("object").and_then(Value::as_str).unwrap();
                let held = log.attestations(target).unwrap();
                assert!(held.windows(2).all(|w| w[0].seq < w[1].seq));
                let expected = StoredAttestation {
                    hash: line.entry.reference.clone(),
                    seq: *seq,
                    line: body,
                };
                assert!(held.contains(&expected), "seq {seq} is held");
            }
        }
    }
    assert_eq!(prev, head.entry_hash, "the head is the last entry");
    assert_eq!(log.leaf_hashes(head.seq).unwrap(), leaves);
    assert_eq!(log.leaf_hashes(0).unwrap(), Vec::<Digest>::new());
    assert_eq!(error_code(log.leaf_hashes(head.seq + 1)), "E_USAGE");
    let report = verify_log(log, VerifyOptions::default()).unwrap();
    assert!(report.ok(), "verify_log: {:?}", report.findings);
    assert_eq!(report.source, Source::Store);
    assert_eq!(report.log.as_deref(), Some(log.log_id()));
    assert_eq!(report.head, Some((head.seq, head.entry_hash)));
    report
}

/// The parsed entry of the line at `seq`.
fn entry_at(log: &dyn LogRead, seq: u64) -> Entry {
    let lines = log.lines(seq, 1).unwrap();
    assert_eq!(lines.len(), 1, "seq {seq} is held");
    parse_line(lines[0].1.as_bytes()).unwrap().entry
}

/// The held object with this id, parsed.
fn stored(log: &dyn LogRead, id: &str) -> Value {
    let object = log.object(id).unwrap().expect("held");
    json::parse(object.bytes.as_bytes()).unwrap()
}

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap()
}

// ---------------------------------------------------------------------------
// Data

/// An object vector: `test-vectors/objects/<case>/object.json`, as the file
/// spells it (pretty-printed).
fn vector(case: &str) -> Obj {
    let bytes = read_vector(&format!("objects/{case}/object.json"));
    let id = object::check(&bytes).map(|o| o.id).unwrap_or_default();
    Obj { id, bytes }
}

const V001: &str = "001-create-inline";
const V002: &str = "002-update-with-prev";
const V003: &str = "003-agent-assertion-approved";
const V004: &str = "004-external-content";
const V005: &str = "005-profile-extension";
const V110: &str = "110-prev-missing";
const V111: &str = "111-prev-after-object";

/// Author and approve lines for 003, then the author line for 001.
const A003: &str = "objects/003-agent-assertion-approved/attestations.jsonl";
/// The author line for 003 again, then an approve line edited after signing.
const A112: &str = "objects/112-signature-invalid/attestations.jsonl";
/// An approve line whose key is not 32 bytes of base64url.
const A115: &str = "objects/115-attestation-bad-key/attestations.jsonl";

/// Line `n`, counting from 1, of a JSON Lines vector file.
fn line(path: &str, n: usize) -> Vec<u8> {
    nth_line(&read_vector(path), n)
}

/// The same JSON, spelled differently: a space after the opening brace.
fn respell(json: &[u8]) -> Vec<u8> {
    let mut out = b"{ ".to_vec();
    out.extend_from_slice(&json[1..]);
    out
}

/// The subject of every synthetic `thing`.
const THING: [&str; 2] = ["contract-test", "thing"];
const RECORD_SCHEMA: &str = "contract-test/thing@3";
const CHECK_SCHEMA: &str = "contract-test/check@1";
const CHECK_LANE: &str = "assert:contract-test/check";

fn subject(system: &str, r#type: &str, key: &str) -> SubjectKey {
    SubjectKey {
        system: system.into(),
        r#type: r#type.into(),
        key: key.into(),
    }
}

/// A synthetic object about `contract-test/thing/<key>`.
fn thing(op: &str, schema: &str, key: &str, note: &str, at: &str, prev: &[&str]) -> Obj {
    build(op, schema, [THING[0], THING[1], key], note, at, prev)
}

/// A sealed synthetic object: inline content `{"note":…}`, the actor
/// `service:contract-test`.
fn build(op: &str, schema: &str, subject: [&str; 3], note: &str, at: &str, prev: &[&str]) -> Obj {
    let inline = Value::Object(vec![("note".into(), Value::String(note.into()))]);
    let prev: Vec<String> = prev.iter().map(|p| p.to_string()).collect();
    sealed(op, schema, subject, inline, "contract-test", at, &prev)
}

fn obj(o: &Obj) -> Item<'_> {
    Item::Object {
        bytes: &o.bytes,
        if_heads: None,
    }
}

fn obj_if<'a>(o: &'a Obj, heads: &'a [String]) -> Item<'a> {
    Item::Object {
        bytes: &o.bytes,
        if_heads: Some(heads),
    }
}

fn att(line: &[u8]) -> Item<'_> {
    Item::Attestation { line }
}

/// A signer that holds the store key but does not sign exactly what it is
/// asked to.
struct Tampered(Tamper);

#[derive(Clone, Copy, Debug)]
enum Tamper {
    /// Another purpose.
    Purpose,
    /// Another object.
    Object,
    /// Another `at`.
    At,
    /// The same instant as asked, spelled differently.
    AtSpelling,
    /// Another signer.
    Signer,
    /// Signed with another key, which the line names.
    OtherKey,
    /// The asked content, with a signature over other content.
    Forged,
    /// Not an attestation.
    Garbage,
    /// Exactly what was asked, spelled with a space: accepted, and stored
    /// in canonical form.
    Respelled,
}

const TAMPERS: [Tamper; 8] = [
    Tamper::Purpose,
    Tamper::Object,
    Tamper::At,
    Tamper::AtSpelling,
    Tamper::Signer,
    Tamper::OtherKey,
    Tamper::Forged,
    Tamper::Garbage,
];

impl Signer for Tampered {
    fn public_key(&self) -> String {
        store_key().public_key()
    }

    fn sign(
        &self,
        object_id: &str,
        purpose: &str,
        signer: &str,
        at: &str,
    ) -> Result<String, StoreError> {
        let key = store_key();
        match self.0 {
            Tamper::Purpose => {
                let other = if purpose == "author" {
                    "approve"
                } else {
                    "author"
                };
                key.sign(object_id, other, signer, at)
            }
            Tamper::Object => key.sign(&canon::sha256_hex(b"other"), purpose, signer, at),
            Tamper::At => key.sign(object_id, purpose, signer, "2026-01-01T00:00:00.000Z"),
            Tamper::AtSpelling => {
                let instant = at.strip_suffix(".000Z").map(|s| format!("{s}Z"));
                let at = instant.unwrap_or_else(|| at.replace('Z', "0Z"));
                key.sign(object_id, purpose, signer, &at)
            }
            Tamper::Signer => key.sign(object_id, purpose, "service:someone-else", at),
            Tamper::OtherKey => seed("assessor-01").sign(object_id, purpose, signer, at),
            Tamper::Forged => {
                let other = key.sign(object_id, purpose, signer, "2026-01-01T00:00:00.000Z")?;
                let value = json::parse(other.as_bytes()).unwrap();
                Ok(canon::canonicalize(
                    &value.with("at", Value::String(at.into())),
                ))
            }
            Tamper::Garbage => Ok("not an attestation".into()),
            Tamper::Respelled => {
                let line = key.sign(object_id, purpose, signer, at)?;
                Ok(String::from_utf8(respell(line.as_bytes())).unwrap())
            }
        }
    }
}

/// A signer whose public key is not a key.
struct NoKey;

impl Signer for NoKey {
    fn public_key(&self) -> String {
        "ed25519:not-a-key".into()
    }

    fn sign(&self, _: &str, _: &str, _: &str, _: &str) -> Result<String, StoreError> {
        Err(StoreError::Usage("no key".into()))
    }
}

// ---------------------------------------------------------------------------
// Scenarios: T4

/// Genesis at seq 1 and its author attestation at seq 2, at epoch 1, with
/// the clock reading truncated to milliseconds for both `at` and
/// `received_at`.
fn genesis_is_at_seq_1_and_2<B: Backend>() {
    let store = Store::<B>::init("2026-11-26T09:30:15.123456789Z");
    let log = store.read();
    let at = "2026-11-26T09:30:15.123Z";
    let key = store_key().public_key();
    let genesis = Genesis::new(NAME, &key).unwrap();
    assert_eq!(log.genesis(), &genesis);
    let object = genesis.object(at);
    let log_id = text(&object, "id");
    assert_eq!(log.log_id(), log_id);
    assert_eq!(store.backend.log().epoch(), 1);
    assert_eq!(log.head().unwrap().seq, 2);

    let lines = log.lines(1, 10).unwrap();
    assert_eq!(lines.len(), 2);
    let first = parse_line(lines[0].1.as_bytes()).unwrap();
    let expected = Entry {
        log: log_id.into(),
        seq: 1,
        kind: Kind::Object,
        reference: log_id.into(),
        received_at: at.into(),
        epoch: 1,
        prev: log_id.into(),
    };
    assert_eq!(
        first.entry, expected,
        "seq 1's log, ref and prev are the log id"
    );
    assert_eq!(
        canon::canonicalize(first.body.value()),
        canon::canonicalize(&object)
    );

    let second = parse_line(lines[1].1.as_bytes()).unwrap();
    let author = second.body.value();
    let expected = Entry {
        seq: 2,
        kind: Kind::Attestation,
        reference: attestation_ref(author),
        prev: first.entry.hash(),
        ..first.entry.clone()
    };
    assert_eq!(second.entry, expected);
    let parsed = attest::parse(canon::canonicalize(author).as_bytes()).unwrap();
    parsed.verify_signature().unwrap();
    assert_eq!(parsed.object, log_id);
    assert_eq!(parsed.purpose, "author");
    assert_eq!(parsed.signer, genesis.signer);
    assert_eq!(text(author, "key"), key);
    assert_eq!(
        text(author, "at"),
        at,
        "the author line is dated as the genesis"
    );

    let lane = "assert:evidence-store/genesis";
    let store_subject = subject("evidence-store", "log", NAME);
    assert_eq!(
        log.heads(&store_subject, lane).unwrap(),
        vec![log_id.to_string()]
    );
    assert_eq!(
        log.head_rows().unwrap(),
        vec![(store_subject, lane.to_string(), log_id.to_string())]
    );
    assert_eq!(log.checkpoints().unwrap(), Vec::new());
    let held = log.attestations(log_id).unwrap();
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].seq, 2);

    let report = audit(log);
    assert_eq!(report.checked.entries, 2);
    assert_eq!(report.checked.objects, 1);
    assert_eq!(report.checked.attestations, 1);
}

/// `init` refuses a bad name or key with `E_USAGE` before it reads the
/// clock, and a signer whose author line does not check with `S_STORE_KEY`
/// after its one reading. A signer that spells its line differently is
/// accepted, and the line is stored canonically.
fn init_checks_its_arguments_before_the_clock<B: Backend>() {
    fn refused<B: Backend>(name: &str, signer: &dyn Signer, clock: &ScriptClock) -> StoreError {
        match B::init(name, signer, clock) {
            Ok(_) => panic!("init {name:?} was accepted"),
            Err(e) => e,
        }
    }

    // No readings: init must not read the clock.
    let clock = ScriptClock::default();
    let long = "a".repeat(64);
    for name in [
        "",
        "Contract",
        "-contract",
        "contract store",
        "contract/x",
        &long,
    ] {
        let e = refused::<B>(name, &store_key(), &clock);
        assert_eq!(e.code_str(), "E_USAGE", "{name:?}");
    }
    assert_eq!(refused::<B>(NAME, &NoKey, &clock).code_str(), "E_USAGE");

    for tamper in TAMPERS {
        clock.push(ts("2026-11-26T00:00:00Z"));
        let e = refused::<B>(NAME, &Tampered(tamper), &clock);
        assert_eq!(e.code_str(), "S_STORE_KEY", "{tamper:?}");
        assert_eq!(clock.remaining(), 0, "{tamper:?}: one reading");
    }

    // A name at the length limit, and a respelled author line.
    clock.push(ts("2026-11-26T00:00:00Z"));
    let at_limit = B::init(&"a".repeat(63), &Tampered(Tamper::Respelled), &clock);
    let Ok(at_limit) = at_limit else {
        panic!("a 63-character name and a respelled line are accepted");
    };
    assert_eq!(clock.remaining(), 0);
    assert_eq!(at_limit.log().genesis().name, "a".repeat(63));
    audit(at_limit.log());
    at_limit.check();
}

/// Seqs are dense, every entry chains to the one before, and each receipt
/// describes the entry it names. Objects are stored in canonical form.
fn seq_is_dense_and_the_chain_holds<B: Backend>() {
    let mut store = Store::<B>::init("2026-11-26T00:00:00Z");
    let author_001 = line(A003, 3);
    let receipts = [
        store.append("2026-11-26T00:00:01Z", &vector(V001)).unwrap(),
        store.append("2026-11-26T00:00:02Z", &vector(V002)).unwrap(),
        store.attest("2026-11-26T00:00:03Z", &author_001).unwrap(),
        store.append("2026-11-26T00:00:04Z", &vector(V004)).unwrap(),
        store.append("2026-11-26T00:00:05Z", &vector(V005)).unwrap(),
    ];
    let author_ref = attestation_ref(&json::parse(&author_001).unwrap());
    let expected = [
        (Kind::Object, vector(V001).id),
        (Kind::Object, vector(V002).id),
        (Kind::Attestation, author_ref),
        (Kind::Object, vector(V004).id),
        (Kind::Object, vector(V005).id),
    ];
    for (n, (receipt, (kind, reference))) in (3..).zip(receipts.iter().zip(expected)) {
        assert_eq!(receipt.seq, n);
        assert_eq!(receipt.kind, kind);
        assert_eq!(receipt.reference, reference);
        assert_eq!(
            receipt.received_at,
            format!("2026-11-26T00:00:0{}.000Z", n - 2)
        );
        assert!(!receipt.duplicate);
        let entry = entry_at(store.read(), n);
        assert_eq!(receipt.entry_hash, entry.hash());
        assert_eq!(
            (
                entry.kind,
                &entry.reference,
                &entry.received_at,
                entry.epoch
            ),
            (kind, &receipt.reference, &receipt.received_at, 1)
        );
    }
    let last = receipts.last().unwrap();
    assert_eq!(
        store.head(),
        Head {
            seq: 7,
            entry_hash: last.entry_hash.clone()
        }
    );

    // Stored in canonical form, whatever the spelling given.
    let given = vector(V001).bytes;
    let held = store.read().object(&vector(V001).id).unwrap().unwrap();
    assert_eq!(held.seq, 3);
    assert_eq!(
        held.bytes,
        canon::canonicalize(&json::parse(&given).unwrap())
    );
    assert_ne!(held.bytes.as_bytes(), given.as_slice());

    // lines: from a seq, at most a limit.
    let log = store.read();
    let seqs = |from, limit| -> Vec<u64> {
        let lines = log.lines(from, limit).unwrap();
        lines.into_iter().map(|(seq, _)| seq).collect()
    };
    assert_eq!(seqs(4, 2), vec![4, 5]);
    assert_eq!(seqs(6, 10), vec![6, 7]);
    assert_eq!(seqs(8, 10), Vec::<u64>::new());
    assert_eq!(seqs(1, 0), Vec::<u64>::new());
    assert_eq!(seqs(0, 1), vec![1]);

    // Nothing else is held.
    assert_eq!(log.object(&canon::sha256_hex(b"absent")).unwrap(), None);
    assert_eq!(log.attestations(&vector(V004).id).unwrap(), Vec::new());
    assert_eq!(
        log.heads(&subject(THING[0], THING[1], "absent"), "record")
            .unwrap(),
        Vec::<String>::new()
    );
    audit(log);
}

/// Appending a held object again returns its receipt with `duplicate:
/// true` and appends nothing, whatever the spelling.
fn a_duplicate_append_returns_the_same_receipt<B: Backend>() {
    let mut store = Store::<B>::init("2026-11-26T00:00:00Z");
    let v001 = vector(V001);
    let first = store.append("2026-11-26T00:00:01Z", &v001).unwrap();
    let again = store.append("2026-11-26T00:00:02Z", &v001).unwrap();
    let duplicate = Receipt {
        duplicate: true,
        ..first.clone()
    };
    assert_eq!(again, duplicate);
    let canonical = Obj {
        id: v001.id.clone(),
        bytes: store
            .read()
            .object(&v001.id)
            .unwrap()
            .unwrap()
            .bytes
            .into_bytes(),
    };
    let respelled = Obj {
        id: v001.id.clone(),
        bytes: respell(&canonical.bytes),
    };
    assert_eq!(
        store.append("2026-11-26T00:00:03Z", &canonical).unwrap(),
        duplicate
    );
    assert_eq!(
        store.append("2026-11-26T00:00:04Z", &respelled).unwrap(),
        duplicate
    );
    assert_eq!(store.head().seq, 3);
    assert_eq!(store.head().entry_hash, first.entry_hash);
    audit(store.read());
}

/// evidence/0 step 6 at intake: every prev held, none dated after the
/// object, checked in list order.
fn prev_missing_and_prev_order<B: Backend>() {
    let mut store = Store::<B>::init("2026-11-26T00:00:00Z");
    let r = rejected(store.append("2026-11-26T00:00:01Z", &vector(V110)));
    assert_eq!(r.index, 0);
    assert_eq!(r.code, RejectCode::Core(Code::PrevMissing));
    assert_eq!(r.code_str(), "E_PREV_MISSING");
    assert!(r.heads.is_empty());
    assert_eq!(store.head().seq, 2, "a rejected item is not stored");

    let v001 = vector(V001);
    store.append("2026-11-26T00:00:02Z", &v001).unwrap();
    let r = rejected(store.append("2026-11-26T00:00:03Z", &vector(V111)));
    assert_eq!((r.index, r.code_str()), (0, "E_PREV_ORDER"));
    assert!(r.heads.is_empty());

    let v002 = vector(V002);
    assert_eq!(store.append("2026-11-26T00:00:04Z", &v002).unwrap().seq, 4);

    // List order decides: the first prev that fails gives the code.
    let absent = canon::sha256_hex(b"never appended");
    let between = "2026-11-21T00:30:00Z"; // after 001, before 002
    let key = "INS-TEST-000001";
    let missing_first = thing(
        "update",
        RECORD_SCHEMA,
        key,
        "m",
        between,
        &[&absent, &v001.id],
    );
    let r = rejected(store.append("2026-11-26T00:00:05Z", &missing_first));
    assert_eq!(r.code_str(), "E_PREV_MISSING");
    let held_first = thing(
        "update",
        RECORD_SCHEMA,
        key,
        "h",
        between,
        &[&v001.id, &absent],
    );
    let r = rejected(store.append("2026-11-26T00:00:06Z", &held_first));
    assert_eq!(r.code_str(), "E_PREV_MISSING");
    let later_first = thing(
        "update",
        RECORD_SCHEMA,
        key,
        "l",
        between,
        &[&v002.id, &absent],
    );
    let r = rejected(store.append("2026-11-26T00:00:07Z", &later_first));
    assert_eq!(r.code_str(), "E_PREV_ORDER");
    // A prev dated exactly at the object's own `at` is in order.
    let same_time = thing(
        "update",
        RECORD_SCHEMA,
        key,
        "s",
        "2026-11-21T01:00:00Z",
        &[&v002.id],
    );
    store.append("2026-11-26T00:00:08Z", &same_time).unwrap();
    assert_eq!(store.head().seq, 5);
    audit(store.read());
}

/// Objects about store subjects (`subject.system` = `evidence-store`) are
/// refused, the store's own genesis included: that check comes before the
/// duplicate check.
fn store_subjects_are_reserved<B: Backend>() {
    let mut store = Store::<B>::init("2026-11-26T00:00:00Z");
    let log_id = store.read().log_id().to_string();
    let at = "2026-11-25T00:00:00Z";
    let checkpoint_like = build(
        "assert",
        "evidence-store/checkpoint@0",
        ["evidence-store", "checkpoint", &log_id],
        "x",
        at,
        &[],
    );
    let r = rejected(store.append("2026-11-26T00:00:01Z", &checkpoint_like));
    assert_eq!(r.code, RejectCode::ReservedSubject);
    assert_eq!((r.index, r.code_str()), (0, "S_RESERVED_SUBJECT"));

    let other_type = build(
        "create",
        RECORD_SCHEMA,
        ["evidence-store", "thing", "k"],
        "x",
        at,
        &[],
    );
    let r = rejected(store.append("2026-11-26T00:00:02Z", &other_type));
    assert_eq!(r.code_str(), "S_RESERVED_SUBJECT");

    let genesis = Obj {
        id: log_id.clone(),
        bytes: store
            .read()
            .object(&log_id)
            .unwrap()
            .unwrap()
            .bytes
            .into_bytes(),
    };
    let r = rejected(store.append("2026-11-26T00:00:03Z", &genesis));
    assert_eq!(r.code_str(), "S_RESERVED_SUBJECT", "not a duplicate");
    assert_eq!(store.head().seq, 2);

    // Only that exact system is reserved.
    let near = build(
        "create",
        RECORD_SCHEMA,
        ["evidence-store-x", "thing", "k"],
        "x",
        at,
        &[],
    );
    store.append("2026-11-26T00:00:04Z", &near).unwrap();
    audit(store.read());
}

/// Attestation intake: shape, then a held target, then the signature, then
/// duplicates.
fn attestation_rules<B: Backend>() {
    let mut store = Store::<B>::init("2026-11-26T00:00:00Z");
    let v003 = vector(V003);
    let author = line(A003, 1);
    let approve = line(A003, 2);

    let r = rejected(store.attest("2026-11-26T00:00:01Z", &author));
    assert_eq!(r.code, RejectCode::AttestationTarget);
    assert_eq!((r.index, r.code_str()), (0, "S_ATTESTATION_TARGET"));
    store.append("2026-11-26T00:00:02Z", &v003).unwrap();

    let shapes = [
        b"not json".to_vec(),
        b"{}".to_vec(),
        b"{\"v\":\"attestation/0\"}".to_vec(),
        line(A115, 1),
    ];
    for bad in shapes {
        let r = rejected(store.attest("2026-11-26T00:00:03Z", &bad));
        assert_eq!(r.code_str(), "E_ATTESTATION_SHAPE");
    }
    let r = rejected(store.attest("2026-11-26T00:00:04Z", &line(A112, 2)));
    assert_eq!(r.code, RejectCode::Core(Code::SignatureInvalid));
    assert_eq!(store.head().seq, 3, "nothing rejected is stored");

    let landed = store.attest("2026-11-26T00:00:05Z", &author).unwrap();
    assert_eq!((landed.seq, landed.kind), (4, Kind::Attestation));
    let approved = store.attest("2026-11-26T00:00:06Z", &approve).unwrap();
    assert_eq!(approved.seq, 5);

    // Duplicates: the same line, respelled, and 112's identical author line.
    let duplicate = Receipt {
        duplicate: true,
        ..landed.clone()
    };
    for again in [author.clone(), respell(&author), line(A112, 1)] {
        assert_eq!(
            store.attest("2026-11-26T00:00:07Z", &again).unwrap(),
            duplicate
        );
    }
    assert_eq!(store.head().seq, 5);

    let held = store.read().attestations(&v003.id).unwrap();
    let canonical = |l: &[u8]| canon::canonicalize(&json::parse(l).unwrap());
    assert_eq!(
        held,
        vec![
            StoredAttestation {
                hash: landed.reference.clone(),
                seq: 4,
                line: canonical(&author),
            },
            StoredAttestation {
                hash: approved.reference.clone(),
                seq: 5,
                line: canonical(&approve),
            },
        ]
    );
    audit(store.read());
}

/// `if_heads`: compare-and-set against `heads(subject, lane)`, as a set.
fn if_heads_compare_and_set<B: Backend>() {
    let mut store = Store::<B>::init("2026-11-26T00:00:00Z");
    let none: &[String] = &[];
    let v001 = vector(V001);
    let v002 = vector(V002);
    let v3 = build(
        "update",
        "rec-registry/installation@12",
        ["rec-registry", "installation", "INS-TEST-000001"],
        "third",
        "2026-11-22T00:00:00Z",
        &[&v002.id],
    );

    // Empty: the subject and lane have no heads yet.
    store
        .append_if_heads("2026-11-26T00:00:01Z", &v001, none)
        .unwrap();
    // Stale: they have one now, and the current heads come back.
    let r = rejected(store.append_if_heads("2026-11-26T00:00:02Z", &v002, none));
    assert_eq!(r.index, 0);
    assert_eq!(r.code, RejectCode::StaleHeads);
    assert_eq!(r.code_str(), "S_STALE_HEADS");
    assert_eq!(r.heads, vec![v001.id.clone()]);
    // Matching.
    let landed = store
        .append_if_heads(
            "2026-11-26T00:00:03Z",
            &v002,
            std::slice::from_ref(&v001.id),
        )
        .unwrap();
    // A duplicate's if_heads is not evaluated.
    let again = store
        .append_if_heads("2026-11-26T00:00:04Z", &v002, none)
        .unwrap();
    assert_eq!(
        again,
        Receipt {
            duplicate: true,
            ..landed
        }
    );
    let r = rejected(store.append_if_heads(
        "2026-11-26T00:00:05Z",
        &v3,
        std::slice::from_ref(&v001.id),
    ));
    assert_eq!(r.heads, vec![v002.id.clone()]);
    // A set: repeated ids do not matter.
    store
        .append_if_heads(
            "2026-11-26T00:00:06Z",
            &v3,
            &[v002.id.clone(), v002.id.clone()],
        )
        .unwrap();

    // A fork has two heads, given in either order.
    let at = "2026-11-25T00:00:00Z";
    let fork_a = thing("create", RECORD_SCHEMA, "fork", "a", at, &[]);
    let fork_b = thing("create", RECORD_SCHEMA, "fork", "b", at, &[]);
    store.append("2026-11-26T00:00:07Z", &fork_a).unwrap();
    store.append("2026-11-26T00:00:08Z", &fork_b).unwrap();
    let mut both = vec![fork_a.id.clone(), fork_b.id.clone()];
    both.sort();
    let merge = thing(
        "update",
        RECORD_SCHEMA,
        "fork",
        "m",
        at,
        &[&fork_a.id, &fork_b.id],
    );
    let r = rejected(store.append_if_heads(
        "2026-11-26T00:00:09Z",
        &merge,
        std::slice::from_ref(&v3.id),
    ));
    assert_eq!(
        r.heads, both,
        "the heads of the object's own subject and lane"
    );
    both.reverse();
    store
        .append_if_heads("2026-11-26T00:00:10Z", &merge, &both)
        .unwrap();

    // Lanes are separate: an assert about the same subject compares its own.
    let check = thing("assert", CHECK_SCHEMA, "fork", "c", at, &[]);
    store
        .append_if_heads("2026-11-26T00:00:11Z", &check, none)
        .unwrap();
    let fork = subject(THING[0], THING[1], "fork");
    let log = store.read();
    assert_eq!(log.heads(&fork, "record").unwrap(), vec![merge.id.clone()]);
    assert_eq!(
        log.heads(&fork, CHECK_LANE).unwrap(),
        vec![check.id.clone()]
    );
    audit(log);
}

/// A batch is one transaction: all of it lands, or none of it. Items are
/// checked in order against the log plus the earlier items.
fn a_batch_is_all_or_nothing<B: Backend>() {
    let mut store = Store::<B>::init("2026-11-26T00:00:00Z");
    let v001 = vector(V001);
    let v002 = vector(V002);
    let v004 = vector(V004);
    let v110 = vector(V110);
    let v3 = build(
        "update",
        "rec-registry/installation@12",
        ["rec-registry", "installation", "INS-TEST-000001"],
        "third",
        "2026-11-22T00:00:00Z",
        &[&v002.id],
    );
    let author_001 = line(A003, 3);
    let before = store.head();
    let lines_before = store.read().lines(1, 100).unwrap();

    // [valid, valid, invalid]: nothing lands, and the rejection names index 2.
    let r = rejected(store.commit(
        "2026-11-26T00:00:01Z",
        &[obj(&v001), obj(&v004), obj(&v110)],
    ));
    assert_eq!((r.index, r.code_str()), (2, "E_PREV_MISSING"));
    assert_eq!(store.head(), before);
    assert_eq!(store.read().lines(1, 100).unwrap(), lines_before);
    assert_eq!(store.read().object(&v001.id).unwrap(), None);

    // A later item's if_heads sees the earlier items, and so do the heads a
    // stale rejection gives back.
    let stale = [v001.id.clone()];
    let r = rejected(store.commit(
        "2026-11-26T00:00:02Z",
        &[obj(&v001), obj(&v002), obj_if(&v3, &stale)],
    ));
    assert_eq!((r.index, r.code_str()), (2, "S_STALE_HEADS"));
    assert_eq!(r.heads, vec![v002.id.clone()]);
    assert_eq!(store.head(), before);

    // A prev, a target and a duplicate may be earlier items.
    let current = [v002.id.clone()];
    let receipts = store
        .commit(
            "2026-11-26T00:00:03Z",
            &[
                obj(&v001),
                obj(&v002),
                att(&author_001),
                obj(&v001),
                obj_if(&v3, &current),
            ],
        )
        .unwrap();
    let seqs: Vec<u64> = receipts.iter().map(|r| r.seq).collect();
    assert_eq!(seqs, vec![3, 4, 5, 3, 6], "consecutive seqs in batch order");
    let duplicates: Vec<bool> = receipts.iter().map(|r| r.duplicate).collect();
    assert_eq!(duplicates, vec![false, false, false, true, false]);
    assert_eq!(
        receipts[3],
        Receipt {
            duplicate: true,
            ..receipts[0].clone()
        }
    );
    for receipt in &receipts {
        assert_eq!(receipt.received_at, "2026-11-26T00:00:03.000Z");
    }
    assert_eq!(store.head().entry_hash, receipts[4].entry_hash);

    // Only duplicates: nothing appended, one clock reading all the same.
    let again = store
        .commit("2026-11-26T00:00:04Z", &[obj(&v001), att(&author_001)])
        .unwrap();
    assert_eq!(again[0], receipts[3]);
    assert_eq!(
        again[1],
        Receipt {
            duplicate: true,
            ..receipts[2].clone()
        }
    );
    // An empty batch.
    assert_eq!(
        store.commit("2026-11-26T00:00:05Z", &[]).unwrap(),
        Vec::new()
    );
    assert_eq!(store.head().seq, 6);

    // A rejection after a duplicate and a valid item still names its own
    // index, and the valid item does not land.
    let reserved = build(
        "create",
        RECORD_SCHEMA,
        ["evidence-store", "thing", "k"],
        "x",
        "2026-11-25T00:00:00Z",
        &[],
    );
    let r = rejected(store.commit(
        "2026-11-26T00:00:06Z",
        &[obj(&v001), obj(&v004), obj(&reserved)],
    ));
    assert_eq!((r.index, r.code_str()), (2, "S_RESERVED_SUBJECT"));
    assert_eq!(store.head().seq, 6);
    assert_eq!(
        store.append("2026-11-26T00:00:07Z", &v004).unwrap().seq,
        7,
        "nothing of a rejected batch used a seq"
    );
    audit(store.read());
}

/// The heads fold: an update replaces its prev, a second create forks the
/// record lane, a merge joins it, and a prev in another lane changes
/// nothing there.
fn the_heads_fold_handles_update_fork_and_merge<B: Backend>() {
    let mut store = Store::<B>::init("2026-11-26T00:00:00Z");
    let s = subject(THING[0], THING[1], "k");
    let record = |store: &Store<B>| store.read().heads(&s, "record").unwrap();
    let forks = |store: &Store<B>| {
        verify_log(store.read(), VerifyOptions::default())
            .unwrap()
            .forks
    };

    let a = thing(
        "create",
        RECORD_SCHEMA,
        "k",
        "a",
        "2026-11-25T00:00:01Z",
        &[],
    );
    store.append("2026-11-26T00:00:01Z", &a).unwrap();
    assert_eq!(record(&store), vec![a.id.clone()]);

    let b = thing(
        "update",
        RECORD_SCHEMA,
        "k",
        "b",
        "2026-11-25T00:00:02Z",
        &[&a.id],
    );
    store.append("2026-11-26T00:00:02Z", &b).unwrap();
    assert_eq!(record(&store), vec![b.id.clone()]);

    let c = thing(
        "create",
        RECORD_SCHEMA,
        "k",
        "c",
        "2026-11-25T00:00:03Z",
        &[],
    );
    store.append("2026-11-26T00:00:03Z", &c).unwrap();
    let mut fork = vec![b.id.clone(), c.id.clone()];
    fork.sort();
    assert_eq!(record(&store), fork);
    assert_eq!(forks(&store), 1);

    let m = thing(
        "update",
        RECORD_SCHEMA,
        "k",
        "m",
        "2026-11-25T00:00:04Z",
        &[&b.id, &c.id],
    );
    store.append("2026-11-26T00:00:04Z", &m).unwrap();
    assert_eq!(record(&store), vec![m.id.clone()]);
    assert_eq!(forks(&store), 0);

    // An assert about the same subject is in its own lane; its prev in the
    // record lane changes nothing there.
    let x = thing(
        "assert",
        CHECK_SCHEMA,
        "k",
        "x",
        "2026-11-25T00:00:05Z",
        &[&m.id],
    );
    store.append("2026-11-26T00:00:05Z", &x).unwrap();
    assert_eq!(record(&store), vec![m.id.clone()]);
    assert_eq!(
        store.read().heads(&s, CHECK_LANE).unwrap(),
        vec![x.id.clone()]
    );

    let d = thing(
        "delete",
        RECORD_SCHEMA,
        "k",
        "d",
        "2026-11-25T00:00:06Z",
        &[&m.id],
    );
    store.append("2026-11-26T00:00:06Z", &d).unwrap();
    assert_eq!(record(&store), vec![d.id.clone()]);

    let log = store.read();
    let mut rows = vec![
        (
            subject("evidence-store", "log", NAME),
            "assert:evidence-store/genesis".to_string(),
            log.log_id().to_string(),
        ),
        (s.clone(), CHECK_LANE.to_string(), x.id.clone()),
        (s.clone(), "record".to_string(), d.id.clone()),
    ];
    rows.sort();
    assert_eq!(log.head_rows().unwrap(), rows);
    audit(log);
}

/// `received_at` is the later of the clock, truncated to milliseconds, and
/// the last entry's: a clock that steps back repeats it.
fn received_at_never_decreases<B: Backend>() {
    let mut store = Store::<B>::init("2026-11-26T00:00:10Z");
    let at = "2026-11-25T00:00:00Z";
    let steps = [
        ("2026-11-26T00:00:20Z", "2026-11-26T00:00:20.000Z"),
        ("2026-11-26T00:00:05Z", "2026-11-26T00:00:20.000Z"),
        ("2026-11-26T00:00:21.987654321Z", "2026-11-26T00:00:21.987Z"),
        ("2026-11-26T00:00:21.9879Z", "2026-11-26T00:00:21.987Z"),
        ("2026-11-25T00:00:00Z", "2026-11-26T00:00:21.987Z"),
        ("2026-11-26T00:00:21.988Z", "2026-11-26T00:00:21.988Z"),
    ];
    for (n, (reading, received)) in steps.into_iter().enumerate() {
        let o = thing("create", RECORD_SCHEMA, &format!("k{n}"), "n", at, &[]);
        let receipt = store.append(reading, &o).unwrap();
        assert_eq!(receipt.received_at, received, "reading {reading}");
    }
    let info = store.checkpoint("2026-11-26T00:00:00Z").unwrap();
    assert_eq!(
        entry_at(store.read(), info.seq).received_at,
        "2026-11-26T00:00:21.988Z"
    );
    assert_received_never_decreases(store.read());
    audit(store.read());
}

/// Every writer epoch is stamped into its entries; reopening adds one and
/// reads no clock.
fn reopen_increments_the_epoch<B: Backend>() {
    let mut store = Store::<B>::init("2026-11-26T00:00:00Z");
    assert_eq!(store.backend.log().epoch(), 1);
    store.reopen();
    assert_eq!(store.backend.log().epoch(), 2);
    let receipt = store.append("2026-11-26T00:00:01Z", &vector(V001)).unwrap();
    assert_eq!(entry_at(store.read(), receipt.seq).epoch, 2);
    store.reopen();
    store.reopen();
    assert_eq!(store.backend.log().epoch(), 4);
    let info = store.checkpoint("2026-11-26T00:00:02Z").unwrap();
    assert_eq!(entry_at(store.read(), info.seq).epoch, 4);
    assert_eq!(entry_at(store.read(), info.seq + 1).epoch, 4);
    // Duplicates and rejections write nothing, so stamp nothing.
    store.append("2026-11-26T00:00:03Z", &vector(V001)).unwrap();
    let epochs: Vec<u64> = (1..=store.head().seq)
        .map(|seq| entry_at(store.read(), seq).epoch)
        .collect();
    assert_eq!(epochs, vec![1, 1, 2, 4, 4]);
    audit(store.read());
}

/// A checkpoint covers every entry before it: its root recomputes from the
/// leaf hashes, its `at` follows the service rule, its witness is by the
/// store key, and anything else signing it is refused with `S_STORE_KEY`.
fn checkpoints<B: Backend>() {
    let mut store = Store::<B>::init("2026-11-26T00:00:00Z");
    store.append("2026-11-26T00:00:01Z", &vector(V001)).unwrap();
    store.append("2026-11-26T00:00:02Z", &vector(V002)).unwrap();
    let log_id = store.read().log_id().to_string();
    let genesis = store.read().genesis().clone();
    let lane = "assert:evidence-store/checkpoint";
    let checkpoint_subject = subject("evidence-store", "checkpoint", &log_id);

    let before = store.head();
    let first = store.checkpoint("2026-11-26T00:00:03.999999Z").unwrap();
    let leaves = store.read().leaf_hashes(4).unwrap();
    let root = merkle::to_text(&merkle::root(&leaves));
    assert_eq!(
        first,
        CheckpointInfo {
            id: first.id.clone(),
            seq: 5,
            size: 4,
            root: root.clone(),
            head: before.entry_hash.clone(),
        }
    );
    assert_eq!(store.read().checkpoints().unwrap(), vec![first.clone()]);
    assert_eq!(store.head().seq, 6);

    // The object: built from its content, the genesis signer, its `at` (the
    // clock, truncated) and no prev.
    let first_at = "2026-11-26T00:00:03.999Z";
    let content = Checkpoint {
        log: log_id.clone(),
        size: 4,
        head: before.entry_hash.clone(),
        root,
    };
    let expected = content.object(&genesis, first_at, None);
    assert_eq!(text(&expected, "id"), first.id);
    let held = store.read().object(&first.id).unwrap().unwrap();
    assert_eq!(held.seq, 5);
    assert_eq!(held.bytes, canon::canonicalize(&expected));

    // The witness: seq 6, by the store key, dated as the checkpoint.
    let witnesses = store.read().attestations(&first.id).unwrap();
    assert_eq!(witnesses.len(), 1);
    assert_eq!(witnesses[0].seq, 6);
    let witness = attest::parse(witnesses[0].line.as_bytes()).unwrap();
    witness.verify_signature().unwrap();
    assert_eq!(witness.purpose, "witness");
    assert_eq!(witness.signer, genesis.signer);
    assert_eq!(text(&witness.value, "key"), genesis.key);
    assert_eq!(text(&witness.value, "at"), first_at);
    // One transaction: one received_at for both.
    for seq in [5, 6] {
        assert_eq!(entry_at(store.read(), seq).received_at, first_at);
    }
    assert_eq!(
        store.read().heads(&checkpoint_subject, lane).unwrap(),
        vec![first.id.clone()]
    );

    // Directly after another, with the clock behind: the size covers the
    // first witness, `at` is the previous checkpoint's, `prev` names it.
    let second = store.checkpoint("2026-11-26T00:00:01Z").unwrap();
    assert_eq!((second.seq, second.size), (7, 6));
    let object = stored(store.read(), &second.id);
    assert_eq!(text(&object, "at"), first_at);
    assert_eq!(
        object.get("prev"),
        Some(&Value::Array(vec![Value::String(first.id.clone())]))
    );
    assert_eq!(
        store.read().heads(&checkpoint_subject, lane).unwrap(),
        vec![second.id.clone()]
    );

    // A clock ahead gives its own reading.
    store.append("2026-11-26T00:00:04Z", &vector(V004)).unwrap();
    let third = store.checkpoint("2026-11-26T00:10:00.5Z").unwrap();
    assert_eq!((third.seq, third.size), (10, 9));
    let object = stored(store.read(), &third.id);
    assert_eq!(text(&object, "at"), "2026-11-26T00:10:00.500Z");
    assert_eq!(
        object.get("prev"),
        Some(&Value::Array(vec![Value::String(second.id.clone())]))
    );
    assert_eq!(
        store.read().checkpoints().unwrap(),
        vec![first.clone(), second.clone(), third.clone()]
    );

    // The wrong seed: refused before the clock is read; nothing written.
    let head = store.head();
    let wrong = store.unread(|log| log.checkpoint(&seed("assessor-01")));
    assert_eq!(error_code(wrong), "S_STORE_KEY");
    // The store key with a witness that does not check: one reading, and
    // nothing written.
    for tamper in TAMPERS {
        let refused = store.write("2026-11-26T00:20:00Z", |log| {
            log.checkpoint(&Tampered(tamper))
        });
        assert_eq!(error_code(refused), "S_STORE_KEY", "{tamper:?}");
    }
    assert_eq!(store.head(), head);
    assert_eq!(store.read().checkpoints().unwrap().len(), 3);

    // A respelled witness is accepted and stored in canonical form.
    let fourth = store
        .write("2026-11-26T00:30:00Z", |log| {
            log.checkpoint(&Tampered(Tamper::Respelled))
        })
        .unwrap();
    assert_eq!((fourth.seq, fourth.size), (12, 11));
    let report = audit(store.read());
    assert_eq!(report.covered, Some((11, fourth.id.clone())));
    assert_eq!(report.checked.checkpoints, 4);
}

// ---------------------------------------------------------------------------
// Scenarios: T4 and T10, export, verify and import

/// Export, verify and import round-trip (T4; the library part of T10). A
/// log with objects, attestations, two checkpoints, a reopen and an entry
/// after the latest checkpoint, at three segment sizes:
///
/// - `export` writes the bytes of `export_files` and nothing else; the
///   manifest names the log, its head and its latest checkpoint;
/// - `verify_export_dir` is ok, and its report is the live `verify_log`
///   report but for its source;
/// - `import` gives a store whose `verify_store` report is that report again
///   (source `store`, so the same head entry hash), which holds only
///   `log.db`, holds the same log as the writer and exports the same bytes,
///   and whose next writer appends at the highest epoch + 1;
/// - importing again, into the same place, is `S_EXISTS`.
fn export_verify_import_round_trip<B: Backend>() {
    let mut store = Store::<B>::init("2026-11-26T00:00:00Z");
    store.append("2026-11-26T00:00:01Z", &vector(V001)).unwrap();
    store.append("2026-11-26T00:00:02Z", &vector(V002)).unwrap();
    store.append("2026-11-26T00:00:03Z", &vector(V003)).unwrap();
    store
        .attest("2026-11-26T00:00:04Z", &line(A003, 1))
        .unwrap();
    store
        .attest("2026-11-26T00:00:05Z", &line(A003, 2))
        .unwrap();
    store.checkpoint("2026-11-26T00:00:06Z").unwrap();
    store.reopen();
    store.append("2026-11-26T00:00:07Z", &vector(V004)).unwrap();
    store
        .attest("2026-11-26T00:00:08Z", &line(A003, 3))
        .unwrap();
    let latest = store.checkpoint("2026-11-26T00:00:09Z").unwrap();
    store.append("2026-11-26T00:00:10Z", &vector(V005)).unwrap();
    let head = store.head();
    assert_eq!((latest.size, head.seq), (11, 14));
    let log = store.read();
    let live = audit(log);
    let live_value = canon::canonicalize(&live.to_value());

    let tmp = tempfile::tempdir().unwrap();
    for segment_size in [1, 4, 1000] {
        let what = format!("segment_size {segment_size}");
        let files = export_files(log, segment_size).unwrap();
        let out = tmp.path().join(format!("export-{segment_size}"));
        let manifest = export(log, &out, segment_size).unwrap();
        assert_eq!(manifest, files.manifest, "{what}");
        let mut want = files.files.clone();
        want.sort();
        assert_eq!(common::files_in(&out), want, "{what}");
        assert_eq!(manifest.log, log.log_id());
        assert_eq!(
            (manifest.size, &manifest.head),
            (head.seq, &head.entry_hash)
        );
        assert_eq!(manifest.checkpoint, Some((latest.id.clone(), latest.size)));
        assert_eq!(
            read_export_dir(&out).unwrap().manifest,
            manifest.canonical().into_bytes()
        );

        let report = verify_export_dir(&out, VerifyOptions::default()).unwrap();
        assert!(report.ok(), "{what}: {:?}", report.findings);
        assert_eq!(report.source, Source::Export);
        let as_store = report
            .to_value()
            .with("source", Value::String("store".into()));
        assert_eq!(canon::canonicalize(&as_store), live_value, "{what}");

        let into = tmp.path().join(format!("store-{segment_size}"));
        let imported = import(&out, &into).unwrap();
        assert!(imported.ok(), "{what}: {:?}", imported.findings);
        assert_eq!(imported.source, Source::Store);
        assert_eq!(
            imported.head,
            Some((head.seq, head.entry_hash.clone())),
            "{what}: import keeps the head entry hash"
        );
        assert_eq!(
            canon::canonicalize(&imported.to_value()),
            live_value,
            "{what}"
        );
        assert_eq!(names(&into), [evidence_store::sqlite::DB_FILE], "{what}");
        let reader = SqliteReader::open(&into).unwrap();
        same_log(log, &reader, segment_size);
        assert_eq!(reader.lines(1, ALL).unwrap(), log.lines(1, ALL).unwrap());
        assert!(
            export_files(&reader, segment_size).unwrap() == files,
            "{what}: the imported store exports the same bytes"
        );
        drop(reader);
        assert_eq!(error_code(import(&out, &into)), "S_EXISTS");
    }

    // The next writer of an imported store: epoch 3, after epochs 1 and 2.
    let into = tmp.path().join("store-4");
    let clock = ScriptClock::new(&["2026-11-26T00:00:11Z"]);
    let mut next = evidence_store::SqliteLog::open(&into, Box::new(clock.clone())).unwrap();
    assert_eq!(next.epoch(), 3);
    let receipt = next.checkpoint(&store_key()).unwrap();
    assert_eq!(receipt.size, head.seq);
    assert_eq!(entry_at(&next, receipt.seq).epoch, 3);
    assert_eq!(clock.remaining(), 0);
    drop(next);
    assert!(verify_store(&into, VerifyOptions::default()).unwrap().ok());
    assert_eq!(
        names(tmp.path()).len(),
        6,
        "three exports and three stores, no partial directory"
    );
}

// ---------------------------------------------------------------------------
// Scenarios: T9, two clocks

/// A clock that steps back an hour: `received_at` repeats rather than
/// decreasing, and a checkpoint after the step takes the previous
/// checkpoint's `at`, so the log verifies with no `E_PREV_ORDER`.
fn t9_a_clock_an_hour_behind<B: Backend>() {
    let mut store = Store::<B>::init("2026-11-26T10:00:00Z");
    let a = thing(
        "create",
        RECORD_SCHEMA,
        "a",
        "a",
        "2026-11-26T09:00:00Z",
        &[],
    );
    let b = thing(
        "create",
        RECORD_SCHEMA,
        "b",
        "b",
        "2026-11-26T09:00:00Z",
        &[],
    );
    let landed = store.append("2026-11-26T10:00:01Z", &a).unwrap();
    assert_eq!(landed.received_at, "2026-11-26T10:00:01.000Z");
    let first = store.checkpoint("2026-11-26T10:00:02Z").unwrap();
    let first_at = text(&stored(store.read(), &first.id), "at").to_string();
    assert_eq!(first_at, "2026-11-26T10:00:02.000Z");

    // An hour back.
    let landed = store.append("2026-11-26T09:00:03Z", &b).unwrap();
    assert_eq!(landed.received_at, "2026-11-26T10:00:02.000Z");
    let second = store.checkpoint("2026-11-26T09:00:04Z").unwrap();
    let object = stored(store.read(), &second.id);
    assert_eq!(text(&object, "at"), first_at);
    assert_eq!(
        entry_at(store.read(), second.seq).received_at,
        "2026-11-26T10:00:02.000Z"
    );

    assert_received_never_decreases(store.read());
    let report = audit(store.read());
    assert_eq!(report.covered, Some((second.size, second.id)));
}

/// A late claim is counted, never refused or rewritten: an object dated one
/// day after its `received_at` gives `at_after_received` 1, and one dated
/// exactly 300 seconds after is not counted.
fn t9_a_late_claim_is_counted_not_refused<B: Backend>() {
    let mut store = Store::<B>::init("2026-11-26T00:00:00Z");
    let late = thing(
        "create",
        RECORD_SCHEMA,
        "late",
        "l",
        "2026-11-27T00:00:01Z",
        &[],
    );
    let receipt = store.append("2026-11-26T00:00:01Z", &late).unwrap();
    assert_eq!(receipt.received_at, "2026-11-26T00:00:01.000Z");
    assert_eq!(
        text(&stored(store.read(), &late.id), "at"),
        "2026-11-27T00:00:01Z",
        "at is the writer's claim, kept as written"
    );
    let edge = thing(
        "create",
        RECORD_SCHEMA,
        "edge",
        "e",
        "2026-11-26T00:05:02Z",
        &[],
    );
    store.append("2026-11-26T00:00:02Z", &edge).unwrap();
    let report = audit(store.read());
    assert!(report.ok());
    assert_eq!(report.at_after_received, 1);
}

/// Every line's `received_at` is at least the one before, as instants.
fn assert_received_never_decreases(log: &dyn LogRead) {
    let lines = log.lines(1, 100_000).unwrap();
    let times: Vec<Timestamp> = lines
        .iter()
        .map(|(_, l)| ts(&parse_line(l.as_bytes()).unwrap().entry.received_at))
        .collect();
    assert!(times.windows(2).all(|w| w[0] <= w[1]), "{times:?}");
}

// ---------------------------------------------------------------------------
// The suite, per backend

/// One test per scenario, for one backend.
macro_rules! contract {
    ($backend:ty) => {
        contract!(@each $backend;
            genesis_is_at_seq_1_and_2,
            init_checks_its_arguments_before_the_clock,
            seq_is_dense_and_the_chain_holds,
            a_duplicate_append_returns_the_same_receipt,
            prev_missing_and_prev_order,
            store_subjects_are_reserved,
            attestation_rules,
            if_heads_compare_and_set,
            a_batch_is_all_or_nothing,
            the_heads_fold_handles_update_fork_and_merge,
            received_at_never_decreases,
            reopen_increments_the_epoch,
            checkpoints,
            export_verify_import_round_trip,
            t9_a_clock_an_hour_behind,
            t9_a_late_claim_is_counted_not_refused,
        );
    };
    (@each $backend:ty; $($scenario:ident),* $(,)?) => {
        $(
            #[test]
            fn $scenario() {
                crate::$scenario::<$backend>();
            }
        )*
    };
}

mod mem {
    contract!(crate::Mem);
}

mod sqlite {
    contract!(crate::Sqlite);
}
