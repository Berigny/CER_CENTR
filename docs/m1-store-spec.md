# M1 build specification: a verifiable append-only evidence store

**Status:** complete; the acceptance tests pass in CI (27 November 2026). Decisions confirmed 26 November 2026 (D11-D13 in [decisions.md](decisions.md)). Phases A (formats, reference, vectors), B (export verifier and bindings), C and D (the store library: in-memory and SQLite logs, export, import, verification, proofs) and E (the command-line store commands, crash test, browser page, CI, nightly runs and docs) are done, and every acceptance test of section 11 passes locally and in CI, so M1 is done by section 1.
**Size:** about 5 engineer-weeks for one experienced Rust engineer. That includes the JavaScript reference work D7 requires, roughly a quarter of the effort.
**Dependencies:** nothing outside this repository.
**System of record:** not touched; no SoR is read or written.

## 1. Goal and definition of done

M1 gives evidence/0 objects and their attestations a durable home. Every later milestone writes into it, and anyone can check it without trusting the operator:

- **Log.** An authoritative append-only log in one SQLite file (`log.db`). Every entry gets a dense position (`seq`) and the time the store received it (`received_at`). Entries are hash-chained and covered by signed RFC 9162 checkpoints.
- **Concurrency.** One writer at a time, enforced by an OS file lease plus a writer epoch stamped into every entry, and compare-and-set on per-subject heads.
- **Integrity report.** `store-report/0` is structured and collects every finding rather than stopping at the first.
- **Export.** A portable export (canonical JSON Lines segments plus a manifest) that gives byte-identical results when verified offline by the CLI, the WebAssembly module, the Python package and a static browser page.

M1 is done when:
- every acceptance test in section 11 passes in CI;
- the existing evidence/0 vectors (`test-vectors/objects`, `canon.json`, `numbers.json`) and the public evidence-core API are unchanged;
- `spec/store-0.md`, D11 to D13 and the README changes are merged.

What runs where: libraries plus the CLI on one host. There is no network service until M4. The pure crate (`evidence-log`) also runs in browsers, Node, .NET and Java hosts through the WASM module, and in Python.

## 2. Decisions recorded with this milestone (docs/decisions.md)

**D11. Observe before writing.** The evidence layer starts by only reading. It records what it observes in a system of record and what people and services assert, using read-only access and no change to the system of record.

- Record changes in the log (`create`, `update`, `delete`) are only ever observations of a system of record. Anything the layer or an application states itself is an `assert`.
- Applications may later request changes through the layer, but only:
  - as explicit requests checked against an allow-list the system-of-record owner publishes;
  - delivered through the system of record's own interface;
  - off by default, and enabled per record type with the system-of-record owner's agreement.
- While the layer only reads, D3 holds exactly. Once requests flow through it, turning it off still loses no confirmed change. Requests not yet delivered, however, exist only in the layer, so applications then depend on it being available.

**D12. Storage.** One SQLite file, `log.db`, is the authoritative log.

- Entries, objects, attestations and the projections that must agree with them are written in one transaction. The projections are heads now, and capture cursors and change keys from M2.
- Triggers refuse UPDATE and DELETE on the log tables.
- Query indexes (from M4) live in separate files that can be deleted and rebuilt.
- The portable form is an export of canonical JSON Lines segments with a manifest. It defines conformance: another backend, such as Postgres, conforms if its exports verify and match.
- Git or object storage receives exports as off-machine copies. Neither is ever the primary store.

**D13. Two clocks.**

- `at` stays the writer's claim and stays inside the id.
- The store adds its own facts outside the object, in the log entry: a dense `seq` and `received_at`, which never decreases.
- Questions about what the store knew on a date use the store's clock. Questions about what was true at a time, according to the writers, use `at`.
- Objects that the store or its services author in a chain take `at` = the later of their clock and their predecessors' `at`. A clock step backwards therefore cannot break the prev ordering rule (E_PREV_ORDER, verify.rs:114-130).

## 3. Workspace layout (new and changed files)

```
Cargo.toml                                   + members/default-members: crates/evidence-log, crates/evidence-store
                                             + [workspace.package] rust-version = "1.89"  (File::try_lock; rusqlite 0.43 MSRV)
spec/store-0.md                              NEW  normative: formats, intake, verification order, codes
spec/schema/log-entry-0.schema.json          NEW
spec/schema/store-genesis-0.schema.json      NEW  (content.inline of the genesis object)
spec/schema/checkpoint-0.schema.json         NEW  (content.inline of a checkpoint)
spec/schema/segments-0.schema.json           NEW  (export manifest)
spec/schema/store-report-0.schema.json       NEW
crates/evidence-core/src/verify.rs           CHANGED, additive only (section 4)
crates/evidence-log/                         NEW  pure: no fs, net, clock, process or threads
  Cargo.toml          deps: evidence-core (path), sha2 = "0.11.2"; denies clippy's disallowed_methods and disallowed_types
  clippy.toml         the fs, net, process, thread and clock items those lints find (T14)
  src/lib.rs          re-exports; pub const STORE_SPEC: &str = "store/0"
  src/entry.rs        log-entry/0
  src/line.rs         segment lines
  src/merkle.rs       RFC 9162 tree hash, inclusion and consistency proofs
  src/genesis.rs      store/0 genesis object
  src/checkpoint.rs   checkpoint/0 object
  src/lane.rs         lanes, subject keys, heads fold
  src/clock.rs        received-time and service-time rules, millisecond formatting
  src/manifest.rs     segments/0 manifest
  src/report.rs       finding codes, Finding, StoreReport
  src/verify.rs       streaming Verifier, verify_export
  src/schemas.rs      schema refs and hashes (constants, tested against spec/schema)
  tests/vectors.rs    runs test-vectors/store
crates/evidence-store/                       NEW  native library with I/O
  Cargo.toml          deps: evidence-core, evidence-log,
                            rusqlite = { version = "0.43", default-features = false, features = ["bundled"] },
                            libc = "0.2" (unix only: open flags, mkfifo in tests)
                      dev-deps: proptest = "1", tempfile = "3"
  src/lib.rs
  src/clock.rs        Clock, SystemClock, ScriptClock
  src/signer.rs       Signer, SeedSigner
  src/api.rs          LogRead, LogWrite, Item, Receipt, Rejected, StoreError, ...
  src/intake.rs       intake rules shared by MemLog and SqliteLog
  src/mem.rs          MemLog (reference implementation)
  src/sqlite/mod.rs   SqliteLog (writer, holds lease), SqliteReader (read-only)
  src/sqlite/db.rs    every query and insert over one log.db connection
  src/sqlite/schema.sql
  src/sqlite/lease.rs
  src/export.rs       export, export_files, read_export_dir
  src/import.rs       import
  src/verify.rs       verify_log, verify_store, verify_export_dir
  src/prove.rs        inclusion and consistency proofs
  src/files.rs        partial directories, durable renames, reading only regular files
  src/hook.rs         named points where the crate's unit tests run code (no-ops in other builds)
  tests/common/mod.rs  tests/contract.rs  tests/sqlite_only.rs  tests/init_killed.rs
  tests/relative_paths.rs  tests/props.rs  tests/writer_vectors.rs
  examples/bench_append.rs
crates/evidence-cli/Cargo.toml               + evidence-log, evidence-store; dev-deps: tempfile = "3", rusqlite (as evidence-store's, to damage stores in tests); rust-version.workspace
crates/evidence-cli/README.md                NEW  the commands, their output, exit status and codes
crates/evidence-cli/src/main.rs              CHANGED: `store` dispatched before option parsing; `verify --store <store dir>`; USAGE;
                                             arguments read as OsString; output written and checked, never println!
crates/evidence-cli/src/store_cmd.rs         NEW  section 8: arguments, output, exit status, the commands
crates/evidence-cli/tests/store_cli.rs       NEW  T3, CLI contract, T7 (CLI), T10, T11 (Rust), T12
crates/evidence-cli/tests/store_crash.rs     NEW  T6 (#![cfg(unix)])
crates/evidence-store/README.md              NEW
crates/evidence-wasm/Cargo.toml, src/lib.rs  CHANGED: + evidence-log, 5 ABI functions
crates/evidence-wasm/js/evidence.mjs, evidence.d.ts, test.mjs     CHANGED
crates/evidence-py/Cargo.toml, src/lib.rs, python/evidence_layer/__init__.py, _native.pyi   CHANGED
crates/evidence-py/tests/test_store_vectors.py   NEW
crates/evidence-py/tests/test_cli_proofs.py  NEW  T11 through Python (EVIDENCE_BIN)
tools/reference/log.mjs                      NEW  independent JS implementation (writer + verifier)
tools/make-vectors.mjs, tools/check-vectors.mjs, tools/check_schema_vectors.py   CHANGED
tools/check-proofs.mjs                       NEW  T11 through WebAssembly in Node (EVIDENCE_BIN)
tools/verify-page/index.html, page.mjs, page.css   NEW  static offline verifier (section 9)
tools/verify-page/smoke.mjs                  NEW  Playwright smoke test of the page over every export case
tools/verify-page/README.md                  NEW  serving and using the page, its security rules, the smoke test, the manual check
test-vectors/store/...                       NEW (section 10)
test-vectors/keys/store-demo.seed            NEW (+ one member in keys/public.json)
test-vectors/README.md, README.md, docs/decisions.md   CHANGED
spec/store-0.md                              CHANGED in phase E: + S_NOT_HELD (operation code of `store get`)
.github/workflows/ci.yml                     CHANGED;  .github/workflows/nightly.yml  NEW
```

## 4. evidence-core: additive changes only

```rust
// crates/evidence-core/src/verify.rs

/// Where step 6 looks up the claimed time of a `prev` object.
pub trait PrevLookup {
    /// `at` of a verified object with this id, or None if it is not held.
    fn prev_at(&self, id: &str) -> Option<Timestamp>;
}
impl PrevLookup for Store {
    fn prev_at(&self, id: &str) -> Option<Timestamp> { self.at(id) }
}

/// Step 6 on its own: every prev resolves, and none is dated after `checked.at`.
/// Returns E_PREV_MISSING or E_PREV_ORDER exactly as `verify` does.
pub fn check_prev(checked: &crate::object::Checked, lookup: &dyn PrevLookup) -> Result<(), Error>;

#[derive(Default)]
pub struct LookupOptions<'a> {
    pub attestations: Option<&'a [u8]>,
    pub prev: Option<&'a dyn PrevLookup>,
    pub require: &'a [String],
}

/// The same pipeline as `verify`, with step 6 run against any lookup.
pub fn verify_lookup(bytes: &[u8], opts: &LookupOptions) -> Report;
```

- `verify(bytes, &Options)` keeps its signature. It now delegates to `verify_lookup`, passing `opts.store.map(|s| s as &dyn PrevLookup)`.
- `run()` calls `check_prev` for step 6.
- Unchanged: `Options`, `Store`, `api::VerifyArgs`, `api::verify`, every error code, and the WASM and Python `verify`.
- `time::now` (time.rs:86, called only from main.rs:89) stays where it is. No new crate calls it, and clocks are injected.

## 5. Formats (normative; this text goes into spec/store-0.md)

Code namespaces:
- `E_` codes remain evidence/0 object results only.
- The store uses `S_` codes.
- Profiles (from M2) use `P_` codes.

### 5.1 Genesis object (store/0)

Seq 1 of every store is this evidence/0 object, and its id is the **log id**:

```json
{"v":"evidence/0","id":"sha256:…","op":"assert",
 "schema":{"ref":"evidence-store/genesis@0","hash":"<GENESIS_SCHEMA_HASH>"},
 "subject":{"system":"evidence-store","type":"log","key":"cer-demo"},
 "content":{"media_type":"application/json","hash":"sha256:…",
   "inline":{"v":"store/0","name":"cer-demo","signer":"service:evidence-store/cer-demo","key":"ed25519:…"}},
 "actor":{"id":"service:evidence-store/cer-demo","kind":"service"},
 "at":"2026-11-26T00:00:00.000Z","prev":[]}
```

- `name` matches `^[a-z0-9][a-z0-9.-]{0,62}$`.
- `signer` = `service:evidence-store/` + name.
- `key` is the store key's Ed25519 public key.
- `GENESIS_SCHEMA_HASH` = sha256 of the bytes of `spec/schema/store-genesis-0.schema.json`. It is a constant in `evidence_log::schemas`, and a unit test checks it against the file.
- Seq 2 is the genesis **author** attestation: object = log id, purpose `author`, signer and key as declared, at = genesis at. It proves possession of the store key.

### 5.2 Log entry (log-entry/0)

All members are required; no other members are allowed.

| Member | Type | Rule |
|---|---|---|
| `v` | string | `"log-entry/0"` |
| `log` | hash | the log id |
| `seq` | integer | 1 or more, at most 2^53 − 1; dense, so entry n has seq n |
| `kind` | string | `"object"` or `"attestation"` |
| `ref` | hash | for an object, its id; for an attestation, `"sha256:"` + hex SHA-256 of the canonical attestation, including `sig` |
| `received_at` | timestamp | an evidence/0 timestamp, never less than the previous entry's. Writers emit exactly 3 fraction digits; verifiers accept any valid timestamp |
| `epoch` | integer | 1 or more; the writer epoch; never less than the previous entry's |
| `prev` | hash | the `entry_hash` of seq − 1; for seq 1, the log id |

- `entry_hash` = `"sha256:"` + hex(SHA-256(JCS(entry))). The epoch is therefore covered by the chain.
- For seq 1, `log` = `ref` = `prev` = the log id.
- Integers are JSON numbers with no fractional part.

### 5.3 Lanes and heads

- **Lane.** `lane(o)` is:
  - `"record"` if `o.op` is create, update or delete;
  - `"record"` if `o.op` = assert and `o.ext` has a `"capture/0"` member (reserved now for M2's baseline observations, so heads stay valid when M2 lands);
  - otherwise `"assert:"` + `schema.ref` up to the first `@` (the whole ref if there is no `@`).
- **Heads.** `heads(S, L)` = the objects with subject S and lane L that no object with subject S and lane L lists in its `prev`. A prev pointing at a different subject or lane does not change heads.
- M1 reports only `counts.forks`, the number of record-lane subjects with more than one head. Status words (current, forked, deleted, absent) are views added in M4.

### 5.4 Intake (what a writer checks, in order)

**Object:**
1. evidence/0 steps 1-5 (`object::check`). On failure, reject with that E_ code.
2. `subject.system` = `"evidence-store"` → reject with S_RESERVED_SUBJECT. Only init and checkpoint write those subjects, through an internal path.
3. If the id is already held, return the existing receipt with `duplicate: true`. `if_heads` is not evaluated, so a compare-and-set append that is retried after it landed still succeeds.
4. Step 6 against held objects (`verify::check_prev` with a lookup over `objects`). Failure gives E_PREV_MISSING or E_PREV_ORDER.
5. If `if_heads` is given, the set `heads(subject, lane)` must equal it exactly. Otherwise reject with S_STALE_HEADS and return the current heads.
6. Append:
   - seq = head + 1;
   - received_at = later of (clock truncated to ms, last received_at);
   - epoch = the writer epoch;
   - prev = the head's entry_hash;
   - insert into `entries` and `objects`, and update `heads`, in the same transaction.

**Attestation line:**
1. `attest::parse`. Failure gives E_ATTESTATION_SHAPE.
2. The target object must be held; otherwise S_ATTESTATION_TARGET.
3. `verify_signature`. Failure gives E_SIGNATURE_INVALID.
4. If the ref is already held, return a duplicate receipt.
5. Append.

**Rules for both:**
- A batch is all or nothing. Duplicates inside a batch do not fail it.
- Rejected input is not stored anywhere. There is no quarantine of bytes, because a rejected object can hold personal data.

### 5.5 Checkpoint (checkpoint/0)

Only the store writes checkpoints (`store checkpoint`). Each is one transaction with two entries: the checkpoint object, then its witness attestation.

```json
{"v":"evidence/0","id":"sha256:…","op":"assert",
 "schema":{"ref":"evidence-store/checkpoint@0","hash":"<CHECKPOINT_SCHEMA_HASH>"},
 "subject":{"system":"evidence-store","type":"checkpoint","key":"<log id>"},
 "content":{"media_type":"application/json","hash":"sha256:…",
   "inline":{"v":"checkpoint/0","log":"<log id>","size":10,"head":"<entry_hash of seq 10>","root":"sha256:<tree hash, entries 1..10>"}},
 "actor":{"id":"service:evidence-store/cer-demo","kind":"service"},
 "at":"…","prev":["<previous checkpoint id>"]}
```

- **size** = the seq of the last entry before the checkpoint's own entry.
- **prev** = `[]` for the first checkpoint.
- **root** = the RFC 9162 §2.1.1 Merkle Tree Hash over leaves 1..size. Leaf n is the canonical bytes of entry n. Leaf hash = SHA-256(0x00 ‖ leaf), node = SHA-256(0x01 ‖ left ‖ right), split at the largest power of two less than n. It is written as `"sha256:"` + hex.
- **at** = service time: the later of the store clock (truncated to ms) and the previous checkpoint's `at`.
- **Witness attestation:** purpose `witness`, signer and key as in the genesis, at = checkpoint at.
- **Wrong key:** if the signer key differs from the genesis key, the request is refused with S_STORE_KEY before anything is written.
- **Proofs:** inclusion and consistency proofs follow RFC 9162 §2.1.3-2.1.4, with 0-based leaf index = seq − 1.
- **Cost:** computing the root reads every entry, so it is O(n). A stored compact frontier is a later optimisation.

### 5.6 Export (segments/0)

An export is a directory containing:

- **`manifest.json`** (canonical JSON):
  `{"v":"segments/0","log":"<log id>","size":N,"head":"<entry_hash of seq N>","segment_size":10000,"segments":[{"file":"000001.jsonl","first":1,"last":10000,"hash":"sha256:<file bytes>"}],"checkpoint":{"id":"<latest checkpoint id>","size":<its size>}}`.
  The `checkpoint` member is omitted if the store has no checkpoint.
- **`NNNNNN.jsonl`** files, numbered from 000001. Each holds one canonical line per entry, LF-terminated, in seq order.
  - Object line: `{"entry":E,"object":O}`.
  - Attestation line: `{"attestation":A,"entry":E}`.
  - These are already canonical, because JCS sorts the members as written.
  - Every segment except the last holds exactly `segment_size` lines.

The export bytes depend only on the log content and `segment_size`.

### 5.7 Verification (one algorithm for a live store and for an export)

Lines are processed in order.

**Line checks.** If one of these fails, the line is unusable and processing of that line stops.
- a. The line is strict JSON, has exactly `{entry, object|attestation}`, and equals its canonical form byte for byte. Otherwise S_LINE_INVALID.
- b. `entry` matches log-entry/0. Otherwise S_ENTRY_INVALID.
- c. `entry.log` equals the log id, taken from line 1. Otherwise S_WRONG_LOG.

**Sequence and clock checks** (each can add a finding):
- d. `entry.seq` equals the expected seq. Otherwise S_SEQ_GAP. Either way, the expected seq becomes entry.seq + 1.
- e. `entry.prev` equals the previous line's entry_hash, or the log id when seq = 1. Otherwise S_CHAIN_BREAK. Skipped if the previous line was unusable.
- f. `received_at` is not less than the previous usable line's. Otherwise S_RECEIVED_REGRESSION.
- g. `epoch` is not less than the previous usable line's. Otherwise S_EPOCH_REGRESSION.

**Body checks** (stop at the first finding):
- h. `ref` equals the object's declared id or the attestation's hash; otherwise S_REF_MISMATCH. A ref seen before gives S_DUPLICATE_REF.
- i. **Object.** Steps 1-5 failing gives S_OBJECT_INVALID with `core` = the E_ code. Step 6 runs against objects at a lower seq, with three outcomes:
  - a prev held at a lower seq but dated later gives S_OBJECT_INVALID, core E_PREV_ORDER;
  - a prev that appears only at a higher seq gives S_PREV_POSITION;
  - a prev absent from the whole log gives S_OBJECT_INVALID, core E_PREV_MISSING. This is decided at the end of the log.
- j. **Reserved subjects.**
  - Seq 1 must be a valid genesis and seq 2 its author attestation; otherwise S_GENESIS_INVALID.
  - Any other evidence-store object that is not a checkpoint by the genesis signer gives S_RESERVED_SUBJECT.
  - A checkpoint whose content is not valid checkpoint/0 gives S_CHECKPOINT_INVALID.
  - A checkpoint whose size is not below its own seq, or whose head or root disagrees with the lines, gives S_CHECKPOINT_MISMATCH.
- k. **Attestation.** A bad shape gives S_ATTESTATION_INVALID, core E_ATTESTATION_SHAPE. A bad signature gives S_ATTESTATION_INVALID, core E_SIGNATURE_INVALID. A target object that is not at a lower seq gives S_ATTESTATION_TARGET.

**End of log:**
- A checkpoint with no valid witness by the genesis key gives S_CHECKPOINT_UNSIGNED, at the checkpoint's seq.
- Fewer than two lines gives S_GENESIS_INVALID.

**Export only, checked before the lines:**
- An invalid manifest shape gives S_MANIFEST_INVALID.
- A segment file whose hash differs gives S_SEGMENT_HASH, with `file` set.
- A manifest whose size, head, log or checkpoint disagrees with the lines gives S_MANIFEST_MISMATCH.

**Live store only, checked after the lines:**
- `heads` or `checkpoints` tables that differ from a recompute give S_PROJECTION_DRIFT, and so does anything else the store keeps beside the canonical texts and reads to answer or to write that differs from what the lines give: the other columns of `entries`, `objects` and `attestations`, item rows that no line shows, and indexes that differ from their tables (SQLite's integrity check). One finding at most.
- Append-only triggers that are missing or changed, or any other trigger, give S_TRIGGERS_MISSING.
- A `meta.log_id` that differs from line 1, a stored name, store key or signer that is not the genesis line's, or a stored writer epoch below the epoch of a line, gives S_META_MISMATCH. One finding at most.

**Collecting and ordering findings.**
- Object verification stops at the first failure. Store verification does not: it collects every finding up to `max_findings` (default 1000) and then sets `truncated: true`, because an auditor needs to know how much is damaged.
- Findings sort by seq, with findings that have no seq first, then by the code's position in the table below.
- Nothing is repaired.

### 5.8 Report (store-report/0)

```json
{"v":"store-report/0","ok":false,"source":"export","log":"sha256:…",
 "head":{"seq":12,"entry_hash":"sha256:…"},
 "covered":{"size":10,"checkpoint":"sha256:…"},
 "checked":{"entries":12,"objects":7,"attestations":4,"checkpoints":1},
 "counts":{"forks":0,"at_after_received":0},
 "findings":[{"code":"S_OBJECT_INVALID","seq":4,"ref":"sha256:…","core":"E_ID_MISMATCH"}],
 "truncated":false}
```

- `ok` = `findings` is empty.
- `source` is `"store"` or `"export"`.
- `log`, `head` and `covered` are omitted when unknown or empty.
- `covered` is the highest checkpoint that verified and has a valid witness.
- `counts.at_after_received` counts objects whose `at` is more than 300 s after their entry's received_at. It is informational (see roadmap decision 9).
- A finding has `code` (always), plus `seq`, `ref`, `core` and `file` where known. There is **no free text**, so every implementation emits identical bytes. The CLI prints human-readable detail to stderr only.

**Report codes**, in sort order:
S_MANIFEST_INVALID, S_SEGMENT_HASH, S_MANIFEST_MISMATCH, S_LINE_INVALID, S_ENTRY_INVALID, S_WRONG_LOG, S_SEQ_GAP, S_CHAIN_BREAK, S_RECEIVED_REGRESSION, S_EPOCH_REGRESSION, S_REF_MISMATCH, S_DUPLICATE_REF, S_GENESIS_INVALID, S_RESERVED_SUBJECT, S_OBJECT_INVALID, S_PREV_POSITION, S_ATTESTATION_INVALID, S_ATTESTATION_TARGET, S_CHECKPOINT_INVALID, S_CHECKPOINT_MISMATCH, S_CHECKPOINT_UNSIGNED, S_PROJECTION_DRIFT, S_TRIGGERS_MISSING, S_META_MISMATCH.

**Intake and operation codes** (never in a report): S_STALE_HEADS, S_STORE_KEY, S_LOCKED, S_NOT_A_STORE, S_EXISTS, S_FORMAT, S_IO, and S_NOT_HELD (the CLI's `store get` of an id the store does not hold; section 8). S_RESERVED_SUBJECT and S_ATTESTATION_TARGET are used both at intake and in reports.

## 6. crate evidence-log (pure)

```rust
// entry.rs
pub const ENTRY_V: &str = "log-entry/0";
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum Kind { Object, Attestation }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry { pub log: String, pub seq: u64, pub kind: Kind, pub reference: String,
                   pub received_at: String, pub epoch: u64, pub prev: String }
impl Entry {
    pub fn to_value(&self) -> Value;
    pub fn canonical(&self) -> String;                                // JCS
    pub fn hash(&self) -> String;                                     // "sha256:…"
    pub fn from_value(v: &Value) -> Result<Entry, FindingCode>;       // S_ENTRY_INVALID
}
// line.rs
pub enum Body { Object(Value), Attestation(Value) }
pub struct Line { pub entry: Entry, pub entry_canonical: String, pub body: Body }
pub fn render_line(entry_canonical: &str, kind: Kind, body_canonical: &str) -> String;  // no trailing LF
pub fn parse_line(bytes: &[u8]) -> Result<Line, FindingCode>;     // S_LINE_INVALID / S_ENTRY_INVALID
pub fn attestation_ref(attestation: &Value) -> String;
// merkle.rs
pub type Digest = [u8; 32];
pub fn leaf_hash(leaf: &[u8]) -> Digest;
pub fn root(leaf_hashes: &[Digest]) -> Digest;
pub fn inclusion_proof(leaf_hashes: &[Digest], index: u64) -> Vec<Digest>;
pub fn verify_inclusion(leaf_hash: &Digest, index: u64, size: u64, proof: &[Digest], root: &Digest) -> bool;
pub fn consistency_proof(leaf_hashes: &[Digest], old_size: u64) -> Vec<Digest>;
pub fn verify_consistency(old_size: u64, new_size: u64, old_root: &Digest, new_root: &Digest, proof: &[Digest]) -> bool;
pub fn to_text(d: &Digest) -> String;  pub fn from_text(s: &str) -> Option<Digest>;
// genesis.rs
pub struct Genesis { pub name: String, pub signer: String, pub key: String }
impl Genesis {
    pub fn new(name: &str, key: &str) -> Result<Genesis, FindingCode>;
    pub fn object(&self, at: &str) -> Value;                                   // sealed
    pub fn from_object(o: &evidence_core::object::Checked) -> Result<Genesis, FindingCode>;
}
// checkpoint.rs
pub struct Checkpoint { pub log: String, pub size: u64, pub head: String, pub root: String }
impl Checkpoint {
    pub fn object(&self, genesis: &Genesis, at: &str, prev: Option<&str>) -> Value;   // sealed
    pub fn from_object(o: &evidence_core::object::Checked) -> Result<Checkpoint, FindingCode>;
}
// lane.rs
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SubjectKey { pub system: String, pub r#type: String, pub key: String }
pub const RECORD: &str = "record";
pub fn subject_of(object: &Value) -> SubjectKey;
pub fn lane_of(object: &Value) -> String;
#[derive(Default, Clone, PartialEq, Eq)] pub struct Heads { /* BTreeMap<(SubjectKey, String), BTreeSet<String>> */ }
impl Heads {
    pub fn apply(&mut self, id: &str, subject: &SubjectKey, lane: &str, prev: &[String]);
    pub fn get(&self, subject: &SubjectKey, lane: &str) -> Vec<String>;   // sorted
    pub fn record_forks(&self) -> u64;
    pub fn rows(&self) -> Vec<(SubjectKey, String, String)>;               // for drift checks
}
// clock.rs
pub fn next_received(now: Timestamp, last: Option<Timestamp>) -> Timestamp;   // truncate to ms, max with last
pub fn service_at(now: Timestamp, prev_ats: &[Timestamp]) -> Timestamp;       // truncate to ms, max
pub fn format_millis(t: Timestamp) -> String;                                  // YYYY-MM-DDTHH:MM:SS.mmmZ
// manifest.rs
pub struct SegmentRef { pub file: String, pub first: u64, pub last: u64, pub hash: String }
pub struct Manifest { pub log: String, pub size: u64, pub head: String, pub segment_size: u64,
                      pub segments: Vec<SegmentRef>, pub checkpoint: Option<(String, u64)> }
impl Manifest { pub fn to_value(&self) -> Value; pub fn parse(bytes: &[u8]) -> Result<Manifest, FindingCode>;
                pub fn file_name(index: usize) -> String; }   // 1 -> "000001.jsonl"
// report.rs
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum FindingCode { ManifestInvalid, SegmentHash, ManifestMismatch, LineInvalid, EntryInvalid, WrongLog,
    SeqGap, ChainBreak, ReceivedRegression, EpochRegression, RefMismatch, DuplicateRef, GenesisInvalid,
    ReservedSubject, ObjectInvalid, PrevPosition, AttestationInvalid, AttestationTarget, CheckpointInvalid,
    CheckpointMismatch, CheckpointUnsigned, ProjectionDrift, TriggersMissing, MetaMismatch }
impl FindingCode { pub fn as_str(self) -> &'static str; }       // declaration order = sort order
pub struct Finding { pub code: FindingCode, pub seq: Option<u64>, pub reference: Option<String>,
                     pub core: Option<evidence_core::Code>, pub file: Option<String> }
#[derive(Clone, Copy)] pub enum Source { Store, Export }
pub struct Checked { pub entries: u64, pub objects: u64, pub attestations: u64, pub checkpoints: u64 }
pub struct StoreReport { pub source: Source, pub log: Option<String>, pub head: Option<(u64, String)>,
    pub covered: Option<(u64, String)>, pub checked: Checked, pub forks: u64, pub at_after_received: u64,
    pub findings: Vec<Finding>, pub truncated: bool }
impl StoreReport { pub fn ok(&self) -> bool; pub fn to_value(&self) -> Value; }
// verify.rs
pub struct VerifyOptions { pub max_findings: usize, pub source: Source }   // Default: 1000, Export
pub struct Verifier { /* leaf hashes, entry hashes, id -> (seq, at), refs, Heads, checkpoints, findings */ }
impl Verifier {
    pub fn new(opts: VerifyOptions) -> Verifier;
    pub fn push_line(&mut self, line: &[u8]);
    pub fn add_finding(&mut self, f: Finding);                  // manifest- and live-only findings
    pub fn heads(&self) -> &Heads;
    pub fn checkpoints(&self) -> &[(u64 /*seq*/, String /*id*/, Checkpoint)];
    pub fn finish(self) -> StoreReport;
}
pub fn verify_export(manifest: &[u8], segments: &[&[u8]], opts: VerifyOptions) -> StoreReport;
// schemas.rs
pub const GENESIS_SCHEMA_REF: &str = "evidence-store/genesis@0";
pub const GENESIS_SCHEMA_HASH: &str = "sha256:…";      // test: equals sha256(spec/schema/store-genesis-0.schema.json)
pub const CHECKPOINT_SCHEMA_REF: &str = "evidence-store/checkpoint@0";
pub const CHECKPOINT_SCHEMA_HASH: &str = "sha256:…";   // test: equals sha256(spec/schema/checkpoint-0.schema.json)
```

The Verifier keeps per-entry hashes in memory: about 200 MB per million entries, which is fine at the volumes in view. A bounded-memory verifier is a later improvement.

## 7. crate evidence-store (I/O)

The interfaces below are the code's: `crates/evidence-store/src`. The rules of this crate's writers are written out in full as rustdoc in `src/api.rs` (module docs) and in each module; 7.5 lists them in brief. The rules every writer must follow are normative in `spec/store-0.md`. Two private modules hold shared plumbing and no interface: `src/sqlite/db.rs` (every query and insert over one `log.db` connection, used by the writer, the reader, the writer's `IntakeState` and import) and `src/files.rs` (the partial-directory and durable-rename steps of export, import and init).

### 7.1 On-disk layout

```
<store>/
  log.db            authoritative SQLite (WAL); log.db-wal, log.db-shm alongside while open
  LOCK              writer lease (File::try_lock); text "pid <pid>\nepoch <n>\n" for diagnostics only
  log.db.init       only while SqliteLog::init builds the store, or after an init that was killed
```

- Seed files are never kept in the store directory.
- Exports go to a directory the caller names. Export and import build in `<target>.partial-<pid>-<n>` beside their target, for the first `n` from 1 whose name `create_dir` can create, and rename it into place, so a crash never leaves a half-written target. A partial directory that a killed attempt left is skipped and left alone, so it never blocks a retry, even by a later process with the same pid (pids repeat, for example in containers); it can be deleted by hand. S_EXISTS only when the first 1000 names all exist.
- `SqliteLog::init` builds the new store as `log.db.init` and renames it to `log.db` once it is complete, so `log.db` only ever names a complete store. An init killed at any moment leaves a store, or a directory holding nothing but `LOCK` and `log.db.init` with SQLite's files beside it, which the next `init` takes.
- The store must be on a local disk, because flock is unreliable on NFS.

### 7.2 log.db schema (src/sqlite/schema.sql; PRAGMA user_version = 1)

```sql
PRAGMA application_id = 1163281456;   -- 0x45564430, 'EVD0'
PRAGMA user_version = 1;
CREATE TABLE meta (k TEXT PRIMARY KEY, v TEXT NOT NULL) STRICT;
  -- rows written by code: format='store/0', log_id, name, store_signer, store_key, writer_epoch (decimal text)
CREATE TABLE entries (
  seq INTEGER PRIMARY KEY CHECK (seq >= 1),
  kind TEXT NOT NULL CHECK (kind IN ('object','attestation')),
  ref TEXT NOT NULL UNIQUE,
  received_at TEXT NOT NULL,
  epoch INTEGER NOT NULL,
  entry_hash TEXT NOT NULL UNIQUE,
  entry TEXT NOT NULL                       -- canonical log-entry/0
) STRICT;
CREATE TABLE objects (
  id TEXT PRIMARY KEY,
  seq INTEGER NOT NULL UNIQUE REFERENCES entries(seq),
  at TEXT NOT NULL, op TEXT NOT NULL,
  system TEXT NOT NULL, type TEXT NOT NULL, key TEXT NOT NULL, lane TEXT NOT NULL,
  bytes TEXT NOT NULL                       -- canonical object
) STRICT;
CREATE INDEX objects_subject ON objects(system, type, key, lane, seq);
CREATE TABLE attestations (
  hash TEXT PRIMARY KEY,
  object TEXT NOT NULL REFERENCES objects(id),
  seq INTEGER NOT NULL UNIQUE REFERENCES entries(seq),
  purpose TEXT NOT NULL, signer TEXT NOT NULL, key TEXT NOT NULL, at TEXT NOT NULL,
  line TEXT NOT NULL                        -- canonical attestation
) STRICT;
CREATE INDEX attestations_object ON attestations(object);
-- projections, written in the same transaction, recomputed by store verify
CREATE TABLE heads (system TEXT NOT NULL, type TEXT NOT NULL, key TEXT NOT NULL, lane TEXT NOT NULL,
  id TEXT NOT NULL REFERENCES objects(id), PRIMARY KEY (system, type, key, lane, id)) STRICT, WITHOUT ROWID;
CREATE TABLE checkpoints (seq INTEGER PRIMARY KEY REFERENCES entries(seq), id TEXT NOT NULL UNIQUE,
  size INTEGER NOT NULL, root TEXT NOT NULL, head TEXT NOT NULL) STRICT;
-- append-only, enforced inside storage
CREATE TRIGGER entries_no_update BEFORE UPDATE ON entries BEGIN SELECT RAISE(ABORT, 'S_APPEND_ONLY'); END;
CREATE TRIGGER entries_no_delete BEFORE DELETE ON entries BEGIN SELECT RAISE(ABORT, 'S_APPEND_ONLY'); END;
-- the same two triggers on objects and on attestations
```

- `schema.sql` runs once, inside the transaction that creates a store (init) or rebuilds one (import). `application_id` and `user_version` live in the database header and roll back with that transaction, so a store that failed half way holds no schema.
- Connection settings are not in `schema.sql`: `journal_mode` cannot change inside a transaction and `foreign_keys` does nothing inside one. Every connection runs them when it opens. Writers run `WRITER_PRAGMAS`: `journal_mode = WAL`, `synchronous = FULL`, `foreign_keys = ON`, `busy_timeout = 5000`. Readers open with `SQLITE_OPEN_READ_ONLY` and run `READER_PRAGMAS`: `foreign_keys = ON`, `busy_timeout = 5000`, `query_only = ON`.
- **Reading a store the reader may not write.** SQLite reads a WAL database through `log.db-wal` and `log.db-shm` (beside the file a symlinked `log.db` leads to), creating them when absent, which needs write access to the store directory. Files a process creates belong to its user and primary group, and a writer that cannot open them cannot open the store until someone removes them. Write access to `log.db` is not enough to avoid that: a member of the store's group can write a group-writable `log.db`, but the files it creates (in a directory without the setgid bit) are of its own group, and root's are handed to `log.db`'s owner only after they are created, so a root reader killed in between leaves files of root. So a reader lets SQLite create them only if this process can write `log.db` and its effective user owns it. `SqliteReader::open` first asks SQLite whether this process can write `log.db` (opened read-write, SQLite opens a file it cannot write read-only and says so) and compares `log.db`'s owner with the effective user (on Unix; elsewhere a process that can write `log.db` counts as its owner), then:
  - it can and owns it: an ordinary read-only connection; when its first read fails with SQLITE_CANTOPEN or SQLITE_READONLY (a read-only file system, a directory of another user) and the store has no WAL content, it opens `log.db` as an immutable file instead (below); with WAL content, S_IO, as SQLite reports it;
  - otherwise, and the store has no WAL content: `log.db` as an immutable file;
  - otherwise, the store has WAL content and `log.db-shm` exists (a writer is open, or one did not close): an ordinary read-only connection, which reads the WAL, current frames included, through SQLite's read-only WAL support and creates nothing (unless that writer closes and removes the files in the moment between the check and the read); this may now and then fail with S_IO (SQLITE_READONLY_CANTINIT), and a retry reads. Without `log.db-shm`: S_IO.

  "No WAL content" is `log.db-wal` absent or an empty regular file, SQLite's own rule: `log.db` then holds the whole log and no writer has it open. An immutable file is opened by URI `file://<absolute path>?mode=ro&immutable=1`, the path percent-encoded, and nothing is created; `SqliteReader::is_immutable` says so. SQLite does not watch an immutable file, so the reader notes, when it opens and by absolute path, `LOCK`'s text and file status (or its absence), then that the WAL has no content, then `log.db`'s file status (device, inode, size, modification and change times); after every read transaction it checks that the WAL still has no content and `LOCK` and `log.db` are unchanged, and otherwise that read is S_IO and a new reader is needed. A writer creates or rewrites `LOCK` before it changes `log.db`, has WAL content while it writes, and changes `log.db` when it closes, so a writer is noticed even if `LOCK` is removed afterwards or the process changes its working directory. `verify_store`, export and proofs read through `SqliteReader`, so all of them work on a read-only copy of a store that was closed cleanly.
- Each write is `BEGIN IMMEDIATE … COMMIT`.
- Readers run each call in its own read transaction. `SqliteReader::snapshot` runs several calls in one; `verify_store` uses it, so the lines and the projections it compares are of one moment while a writer appends.
- Opening (`SqliteLog::open`, `SqliteReader::open`) checks, in order: `log.db` exists (else S_NOT_A_STORE, and nothing is created); `log.db-wal` and `log.db-shm`, symlinks followed, are absent or regular files (else S_IO: SQLite opens a file it cannot write read-only, and a FIFO opened read-only waits for ever for a writer); it is an SQLite database with this `application_id` (else S_NOT_A_STORE); `user_version` is 1 (else S_FORMAT); its tables, indexes and views are exactly those of a fresh in-memory database built from `schema.sql` (same type, name, table and `sql`, and nothing else: no view, no virtual table), checked before any table is read, so a `log.db` whose `entries` is a view that never ends is refused rather than read forever (else S_NOT_A_STORE); `meta.format` is `store/0` (S_FORMAT if it is something else); every other `meta` row is present and valid (else S_NOT_A_STORE). The log id and the genesis (name, signer, key) come from `meta`; a reader never reads the lines to open. `verify_store` checks `meta` against them (S_META_MISMATCH).
- `SqliteLog::open` also refuses, with S_IO and before it takes the lease: a trigger that `schema.sql` does not create, or one of the six with other SQL (a trigger could drop or change what it writes while it reports success; a missing trigger it allows, and `verify_store` reports it); and a `meta` name, signer or key that is not the genesis line's (it would sign checkpoints with that key), or a `meta.log_id` that is not the `ref` and `log` of the genesis line's entry (it would put that log id into every entry and checkpoint it appends, which no later edit of `meta` undoes). Under the lease it refuses a `log.db` that is no longer the file it opened (S_IO; S_NOT_A_STORE if it is gone), and a `writer_epoch` below the epoch of the last entry (S_IO).
- `verify_store` gives S_TRIGGERS_MISSING unless the triggers are exactly the six, each with the `tbl_name` and `sql` that a fresh in-memory database built from `schema.sql` gives it, and no others.
- Every path given to SQLite is a plain file name: a relative one is given as `./<path>`, because the bundled SQLite is built with `SQLITE_USE_URI` and takes any name that starts with `file:` as a URI, so a store directory named `file:x` would otherwise open another file.

### 7.3 Lease and epoch

1. `SqliteLog::open` runs the open checks of 7.2 first, so `LOCK` is never created outside a store.
2. It opens `<dir>/LOCK` with create, read and write (never truncating; on Unix with `O_NOFOLLOW` and `O_NONBLOCK`), checks that it is a regular file with one link (a symlink, a file with another hard link, or anything else is S_IO, so a planted link cannot make a writer overwrite another file), and calls `std::fs::File::try_lock()` (stable since Rust 1.89). `WouldBlock` gives `StoreError::Locked { holder }`, where `holder` is the file's text (at most 4096 bytes). The lock is per open file, so a second open in the same process is refused too.
3. On success it checks that `log.db` is still the file it opened, then runs, in one transaction, `writer_epoch = writer_epoch + 1`, then rewrites LOCK's diagnostic text.

- `SqliteLog::init` takes the lease before it creates `log.db.init` (and so before `log.db`), and writes `writer_epoch` 1.
- A writer that creates `LOCK` creates it as its own user. On Unix, when that user is root and `log.db` belongs to another user (an imported store has no `LOCK` until a writer opens it), the new `LOCK` is handed to `log.db`'s user and group, as SQLite does with `log.db-wal` and `log.db-shm`, so that the store's owner can still take the lease. Like SQLite's, the hand-over follows the create: a root writer killed between the two leaves a `LOCK` of root, which the owner removes. A `LOCK` that is already there keeps its owner.
- A failed `init` removes the `LOCK` it created while it still holds the lease. A process that opened that file before and locks it after would hold a file that is no longer the store's `LOCK`, so on Unix `Lease::acquire` checks, after `try_lock`, that `<dir>/LOCK` still names the file it locked (device and inode), and otherwise opens the name again (up to 8 times, then S_LOCKED).
- The lock is held for the lifetime of the `SqliteLog` value and released when it drops.
- The lock belongs to the open file, and a process started with `fork` and `exec` holds a copy of every file its parent had open until its `exec`. A lease dropped while another thread starts a process can stay held until that `exec`, so an open right after the drop can get S_LOCKED with no other writer about; a caller that starts processes while it reopens a store retries after a moment. The test that starts processes (`tests/init_killed.rs`) has a test binary of its own for this reason.
- Every entry carries the epoch. `MemLog` starts at epoch 1; `MemLog::reopen` adds one, with no clock reading, as closing and reopening a `SqliteLog` does.
- `import` sets `writer_epoch` to the highest epoch in the export, so the first writer to open the result appends at that + 1.

### 7.4 Modules, traits and types

```rust
// clock.rs
pub trait Clock: Send { fn now(&self) -> Timestamp; }
pub struct SystemClock;
#[derive(Clone)] pub struct ScriptClock { /* Arc<Mutex<VecDeque<Timestamp>>>: clones share one queue */ }
impl ScriptClock {
    pub fn new(times: &[&str]) -> ScriptClock;                 // panics on a reading that is not a timestamp
    pub fn from_timestamps(times: impl IntoIterator<Item = Timestamp>) -> ScriptClock;
    pub fn push(&self, time: Timestamp);
    pub fn remaining(&self) -> usize;                          // readings left, across clones
}                                                              // now() panics when none is left (a test clock)
// signer.rs
pub trait Signer {
    fn public_key(&self) -> String;                                          // "ed25519:…"
    fn sign(&self, object_id: &str, purpose: &str, signer: &str, at: &str) -> Result<String, StoreError>; // canonical line
}
pub struct SeedSigner { seed: [u8; 32] }          // development only; wraps evidence_core::attest::sign
impl SeedSigner { pub fn from_hex(hex: &str) -> Result<SeedSigner, StoreError>;      // E_USAGE
                  pub fn from_file(path: &Path) -> Result<SeedSigner, StoreError>; } // S_IO, E_USAGE
// api.rs
pub struct Head { pub seq: u64, pub entry_hash: String }                                  // + to_value
pub struct Receipt { pub kind: Kind, pub reference: String, pub seq: u64, pub entry_hash: String,
                     pub received_at: String, pub duplicate: bool }                       // + to_value
pub struct StoredObject { pub id: String, pub seq: u64, pub received_at: String, pub bytes: String }
pub struct StoredAttestation { pub hash: String, pub seq: u64, pub line: String }
pub struct CheckpointInfo { pub id: String, pub seq: u64, pub size: u64, pub root: String, pub head: String } // + to_value
pub enum Item<'a> { Object { bytes: &'a [u8], if_heads: Option<&'a [String]> }, Attestation { line: &'a [u8] } }
pub enum RejectCode { Core(evidence_core::Code), StaleHeads, ReservedSubject, AttestationTarget }
pub struct Rejected { pub index: usize, pub code: RejectCode, pub detail: String, pub heads: Vec<String> }
impl Rejected { pub fn new(index: usize, code: RejectCode, detail: impl Into<String>) -> Rejected;
                pub fn code_str(&self) -> &'static str; }      // the E_ code, or S_STALE_HEADS etc.
pub enum StoreError { Rejected(Rejected), StoreKey, Locked { holder: String },
    NotAStore(PathBuf), Exists(PathBuf), Format(String), Usage(String), Sqlite(String), Io(String) }
impl StoreError { pub fn code_str(&self) -> &'static str; }   // + Display, Error, From<Rejected | io::Error | rusqlite::Error>

pub trait LogRead {
    fn log_id(&self) -> &str;
    fn genesis(&self) -> &Genesis;
    fn head(&self) -> Result<Head, StoreError>;
    fn lines(&self, from_seq: u64, limit: usize) -> Result<Vec<(u64, String)>, StoreError>;  // (seq, canonical line)
    fn object(&self, id: &str) -> Result<Option<StoredObject>, StoreError>;
    fn attestations(&self, object_id: &str) -> Result<Vec<StoredAttestation>, StoreError>;
    fn heads(&self, subject: &SubjectKey, lane: &str) -> Result<Vec<String>, StoreError>;
    fn head_rows(&self) -> Result<Vec<(SubjectKey, String, String)>, StoreError>;  // every heads row, as Heads::rows
    fn checkpoints(&self) -> Result<Vec<CheckpointInfo>, StoreError>;
    fn leaf_hashes(&self, upto: u64) -> Result<Vec<Digest>, StoreError>;          // E_USAGE above the head
}
pub trait LogWrite: LogRead {
    fn epoch(&self) -> u64;
    /// One transaction: every item lands, or none does.
    fn commit(&mut self, items: &[Item<'_>]) -> Result<Vec<Receipt>, StoreError>;
    fn append(&mut self, object: &[u8]) -> Result<Receipt, StoreError>;                       // provided
    fn append_if_heads(&mut self, object: &[u8], expected: &[String]) -> Result<Receipt, StoreError>; // provided
    fn attest(&mut self, line: &[u8]) -> Result<Receipt, StoreError>;                          // provided
    fn checkpoint(&mut self, signer: &dyn Signer) -> Result<CheckpointInfo, StoreError>;
}
// intake.rs: the intake rules, shared by every writer. Planners read through IntakeState and write nothing;
// a writer reads its clock once, plans, then applies the plan's entries in order.
pub struct Tip { pub seq: u64, pub entry_hash: String, pub received_at: Timestamp }
pub trait IntakeState {
    fn tip(&self) -> Result<Tip, StoreError>;
    fn receipt(&self, reference: &str) -> Result<Option<Receipt>, StoreError>;   // held ref → its receipt
    fn object_at(&self, id: &str) -> Result<Option<Timestamp>, StoreError>;      // held object → at
    fn heads(&self, subject: &SubjectKey, lane: &str) -> Result<Vec<String>, StoreError>;
    fn latest_checkpoint(&self) -> Result<Option<CheckpointInfo>, StoreError>;
    fn root(&self, size: u64) -> Result<Digest, StoreError>;
}
pub struct NewEntry { pub entry: Entry, pub canonical: String, pub hash: String, pub leaf: Digest,
                      pub line: String, pub item: NewItem }                       // + receipt()
pub enum NewItem { Object(NewObject), Attestation(NewAttestation) }               // the objects / attestations rows
pub struct Plan { pub receipts: Vec<Receipt>, pub entries: Vec<NewEntry>, pub checkpoint: Option<CheckpointInfo> }
pub fn plan_commit(state: &dyn IntakeState, log_id: &str, epoch: u64, now: Timestamp, items: &[Item<'_>]) -> Result<Plan, StoreError>;
pub fn new_genesis(name: &str, signer: &dyn Signer) -> Result<Genesis, StoreError>;                    // E_USAGE
pub fn plan_genesis(genesis: &Genesis, signer: &dyn Signer, now: Timestamp) -> Result<(String, Plan), StoreError>;
pub fn check_store_key(genesis: &Genesis, signer: &dyn Signer) -> Result<(), StoreError>;            // S_STORE_KEY
pub fn plan_checkpoint(state: &dyn IntakeState, genesis: &Genesis, log_id: &str, epoch: u64,
                       signer: &dyn Signer, now: Timestamp) -> Result<Plan, StoreError>;
pub fn sign_checked(signer: &dyn Signer, genesis: &Genesis, object_id: &str, purpose: &str, at: &str)
    -> Result<evidence_core::attest::Attestation, StoreError>;                                       // S_STORE_KEY
// mem.rs
pub struct MemLog;  impl MemLog {
    pub fn init(name: &str, signer: &dyn Signer, clock: Box<dyn Clock>) -> Result<MemLog, StoreError>;
    pub fn reopen(&mut self); }                                    // epoch += 1, no clock reading
// sqlite/mod.rs, sqlite/lease.rs
pub const DB_FILE, INIT_FILE, APPLICATION_ID, USER_VERSION, FORMAT, META_KEYS, SCHEMA_SQL, WRITER_PRAGMAS, READER_PRAGMAS, TRIGGERS;
pub struct SqliteLog;  impl SqliteLog {
    pub fn init(dir: &Path, name: &str, signer: &dyn Signer, clock: Box<dyn Clock>) -> Result<SqliteLog, StoreError>; // dir absent, empty, or a killed init's
    pub fn open(dir: &Path, clock: Box<dyn Clock>) -> Result<SqliteLog, StoreError>;   // takes the lease, epoch += 1
    pub fn dir(&self) -> &Path; }
pub struct SqliteReader; impl SqliteReader {
    pub fn open(dir: &Path) -> Result<SqliteReader, StoreError>;   // immutable when it cannot write log.db or dir (7.2)
    pub fn is_immutable(&self) -> bool;
    pub fn dir(&self) -> &Path;
    pub fn snapshot<T>(&self, f: impl FnOnce(&SqliteReader) -> Result<T, StoreError>) -> Result<T, StoreError>;
    pub fn has_append_only_triggers(&self) -> Result<bool, StoreError>; }
pub const LOCK_FILE: &str = "LOCK";
pub struct Lease;  impl Lease {
    pub fn acquire(dir: &Path) -> Result<Lease, StoreError>;      // S_LOCKED
    pub fn record(&mut self, epoch: u64) -> Result<(), StoreError>;
    pub fn holder(dir: &Path) -> Result<String, StoreError>; }
// export.rs
pub const DEFAULT_SEGMENT_SIZE: u64 = 10_000;
pub struct ExportFiles { pub manifest: Manifest, pub files: Vec<(String, Vec<u8>)> }  // segments in order, then manifest.json
pub fn export_files(log: &dyn LogRead, segment_size: u64) -> Result<ExportFiles, StoreError>;   // in memory, for tests
pub fn export(log: &dyn LogRead, out: &Path, segment_size: u64) -> Result<Manifest, StoreError>;
pub fn partial_path(target: &Path, n: u32) -> Result<PathBuf, StoreError>;                     // <target>.partial-<pid>-<n>
pub struct ExportBytes { pub manifest: Vec<u8>, pub segments: Vec<Vec<u8>> }                     // + segment_slices
pub fn read_export_dir(dir: &Path) -> Result<ExportBytes, StoreError>;                         // regular files only
// import.rs
pub fn import(export_dir: &Path, into: &Path) -> Result<StoreReport, StoreError>;
// verify.rs
pub fn verify_log(log: &dyn LogRead, opts: VerifyOptions) -> Result<StoreReport, StoreError>;   // lines + projection drift
pub fn verify_store(dir: &Path, opts: VerifyOptions) -> Result<StoreReport, StoreError>;        // + derived rows, triggers, meta
pub fn verify_export_dir(dir: &Path, opts: VerifyOptions) -> Result<StoreReport, StoreError>;   // read_export_dir + verify_export
// prove.rs
pub struct InclusionProof { pub seq: u64, pub size: u64, pub entry: Entry, pub leaf_hash: Digest,
                            pub proof: Vec<Digest>, pub root: Digest, pub checkpoint: Option<String> }   // + to_value, verify
pub struct ConsistencyProof { pub old_size: u64, pub new_size: u64, pub old_root: Digest,
                              pub new_root: Digest, pub proof: Vec<Digest> }                           // + to_value, verify
pub fn inclusion(log: &dyn LogRead, seq: u64, size: u64) -> Result<InclusionProof, StoreError>;
pub fn consistency(log: &dyn LogRead, old_size: u64, new_size: u64) -> Result<ConsistencyProof, StoreError>;
```

- `SqliteLog`, `SqliteReader` and `MemLog` implement `evidence_core::verify::PrevLookup`: `prev_at(id)` is the parsed `at` of the held object, or None. It has no error channel, so a storage error also reads as None; `evidence verify --store <store dir>` and `store get` use it. Intake does not: it resolves prevs through `IntakeState`, which reports errors.
- `StoreError::code_str`: Rejected → the item's code; StoreKey → S_STORE_KEY; Locked → S_LOCKED; NotAStore → S_NOT_A_STORE; Exists → S_EXISTS; Format → S_FORMAT; Usage → E_USAGE (evidence/0's existing code for invalid arguments, as in the bindings); Sqlite and Io → S_IO.

### 7.5 Writer rules (in brief; rustdoc in src/api.rs has them in full)

- **Clock reads.** Each `init`, `commit` and `checkpoint` reads the clock exactly once. `commit` reads it first, before any intake check, so a rejected, all-duplicate or empty batch uses one reading (the reference writer also reads before it checks). `init` and `checkpoint` read it after their argument checks: a refusal with E_USAGE, S_EXISTS or (key mismatch) S_STORE_KEY reads nothing. That one reading gives every entry of the transaction the same `received_at`, through `next_received`, and store-authored objects their `at`, through `service_at`.
- **Batches.** Items are checked in order against the log plus the batch's earlier accepted items: a `prev` or an attestation target may be earlier in the batch, and `if_heads` (compared as a set; empty means no heads) sees the earlier items' heads changes. A duplicate of a held item or of an earlier batch item returns that receipt with `duplicate: true`, adds no entry and does not fail the batch. The first rejection fails the batch with `Rejected.index` = its position from 0; `Rejected.heads` is filled for S_STALE_HEADS only. Receipts come back one per item, in batch order.
- **init** writes, in one transaction at epoch 1, the genesis at seq 1 (`at` = `service_at(now, [])`) and its author attestation at seq 2, plus the genesis's heads row. `SqliteLog::init` refuses a path that exists and is neither an empty directory nor one holding only what a killed init leaves (`LOCK`, `log.db.init`, `log.db.init-journal`, `-wal`, `-shm`) with S_EXISTS. If the path is absent it creates it with its parents and syncs the parent of every directory it created, before anything else (S_IO if a parent cannot be synced, for example one this user can write but not read). It takes the lease (S_LOCKED if held), checks the directory again under it (S_EXISTS if `log.db` or anything else appeared), removes those leftovers, reads the clock, and builds the store in `log.db.init`: schema, `meta` and the plan's rows in one transaction. It closes that connection (which checkpoints and removes the WAL; S_IO if a WAL with content is left), syncs the file, renames it to `log.db` (S_EXISTS if `log.db` appeared), syncs the store directory, and only then opens `log.db` as the writer and records epoch 1 in `LOCK`. A failure after the lease is taken and before the rename removes what this call created (`log.db.init` and its files, and `LOCK` if it created it) while still holding the lease. Once `log.db` is in place it is a complete store, which another process may already have opened, so a failure after the rename (the directory sync, opening the writer, recording the lease) removes nothing: the error is returned and the store stays, as a killed init would leave it. After any failure, the directories this call created are removed if empty, nearest first. Nothing is removed after S_LOCKED, after a lease that could not be taken for another reason, or after the S_EXISTS of the check under the lease: the `LOCK` there may be another init's, so a `LOCK` this call created can stay behind, which the next `init` takes.
- **checkpoint** refuses a signer whose public key is not the genesis key (S_STORE_KEY) before reading the clock. Then, in one transaction: the checkpoint object at seq size + 1 over entries 1..size (every entry before it), with `prev` = the previous checkpoint's id (none for the first) and `at` = `service_at(now, [previous at])`; its witness at seq size + 2; and its `checkpoints` row. A checkpoint directly after another is allowed, as in the reference writer: its size covers the previous witness.
- **Signer output is checked.** The author and witness lines must parse, verify, and attest exactly what was asked with the genesis key; otherwise S_STORE_KEY and nothing is written. They are stored in canonical form.
- **Reopening.** `SqliteLog::open` takes the lease and increments `writer_epoch`; `MemLog::reopen` increments the epoch. A replay of an `inputs.json` `reopen` drops the `SqliteLog` and opens it again, handing it a clone of the same `ScriptClock` (clones share one queue), or calls `MemLog::reopen`.
- **export** fixes `size` and `head` from `head()` first, reads only lines 1..size (S_IO if they are not exactly those seqs), and names in the manifest the last checkpoint with seq ≤ size. It writes `<out>.partial-<pid>-<n>` (7.1), syncs every file and the directory, then renames to `<out>`; S_EXISTS if `<out>` exists; E_USAGE unless 1 ≤ `segment_size` ≤ 2^53 − 1; the partial directory is removed on failure.
- **import** works in five steps: S_EXISTS if `<into>` exists; `verify_export`, returning the report and writing nothing if it is not ok; `<into>.partial-<pid>-<n>` (7.1) built in one transaction with the exact canonical rows, the recomputed `heads` and `checkpoints`, and `writer_epoch` = the highest epoch (no `LOCK` until a writer opens it); `verify_store` on it (not ok: removed, report returned); then sync and rename. A partial directory is removed on any failure. Before the rename one read through a writable connection settles the WAL, so the imported store holds `log.db` alone.
- **Never half a target.** Export and import share one placement step: sync the partial directory, S_EXISTS if the target appeared meanwhile, rename, sync the parent. If the parent cannot be synced after the rename, the target is removed again, so an error never leaves an export or a store behind. A partial directory that a killed export or import left stays, and never blocks a later one. A failed `SqliteLog::init` removes only the files it created, never a `log.db` once it is in place, and directories only if it created them and they are empty; another `init` racing it on the same directory gets S_LOCKED (it holds the lease) or S_EXISTS (its `log.db` exists), and its files are left alone. If the failing `init` created the directory and removes it as it fails, a racing `init` that had already found it may get S_IO (the directory is gone); nothing is damaged, and a retry creates the store. A killed `init` leaves a complete store or a directory the next `init` takes.
- **Limits and damage.** Seqs, sizes and epochs are at most 2^53 − 1. A stored `writer_epoch` outside 1 to 2^53 − 1 is S_NOT_A_STORE when the store is opened, and an open that would pass 2^53 − 1 is S_IO; so is an append to a log at seq 2^53 − 1. A planner also refuses with S_IO, writing nothing, a stored log id that is not a hash and a clock reading whose millisecond form is not a timestamp (outside the years 0000 to 9999). `leaf_hashes` is E_USAGE above the head and S_IO for a missing seq below it; export (a missing seq, or a line holding an LF) and prove (a line that is missing, does not parse or does not match its leaf, or a checkpoint of a size asked whose root is not the computed one or whose object fails the checks under **prove** below) give S_IO for damaged content, while `verify_store` reports the same damage as findings. `verify_store` reads `meta` again inside its snapshot. The writer refuses damage it would carry into every later entry: a commit or checkpoint whose last entry's `entry_hash`, `seq`, `received_at` or `epoch` column is not that of its entry text is S_IO, and so is an open of a store whose `meta` (log id, name, signer or key) disagrees with the genesis line, whose last entry's `epoch` column is not its entry text's, or whose `writer_epoch` is below the epoch of the last entry's text (7.2).
- **Readers leave WAL files.** A read-only connection to a store with no writer open may leave `log.db-wal` and `log.db-shm` behind; this is SQLite's WAL behaviour, and the next writer to open and close the store removes them. Only a reader whose user owns `log.db` and can write it lets SQLite create them, so they belong to the store's owner; any other reader (another user, a member of the store's group, root, or one that cannot write `log.db` or the store directory) creates nothing, and reads `log.db` as an immutable file when the store has no WAL content (7.2).
- **verify_log** works for any `LogRead`: every line through `evidence_log::Verifier` with source `store`, then one S_PROJECTION_DRIFT if `head_rows()` or `checkpoints()` differ from the fold. **verify_store** adds, all in one reader snapshot: S_PROJECTION_DRIFT (still at most one) if SQLite's `PRAGMA integrity_check` fails, if any column of an `entries`, `objects` or `attestations` row differs from its line (`entries.entry` must be the line's canonical entry, not merely render to the same line), if an `objects` or `attestations` row is at no entry of its kind, or if a `checkpoints` row has a negative `seq` or `size` (checked before the projections are read, so such a row is a finding, not S_IO); S_TRIGGERS_MISSING unless the triggers are exactly the six; and S_META_MISMATCH (at most one) unless `meta.log_id`, `name`, `store_key` and `store_signer` are the lines' and `writer_epoch` is at least every line's epoch. These are what lookups and the writer read, so a store that verifies answers from what its lines say. **verify_export_dir** reads an export as `spec/store-0.md` asks a host to and runs `verify_export`. `read_export_dir` (used by it and by import) reads only regular files, a symlink followed to one, each opened without blocking (`O_NONBLOCK`, its type checked again on the open file) and read only up to the size it reports: a listed segment that is a FIFO, a device, a socket, a directory, or a file that holds more than its size says (a `/proc` file reports size 0 and may never end) reads as empty bytes (S_SEGMENT_HASH), and such a `manifest.json` is S_IO, so an export from elsewhere cannot block a read or make it endless. Memory is the sum of the sizes the export's files report.
- **prove** returns the shapes of section 8 (`InclusionProof::to_value`, `ConsistencyProof::to_value`); `checkpoint` names the first checkpoint, in seq order, whose size is `size`, omitted if there is none (a log this crate writes has at most one per size; an imported one may have several). A checkpoint of a size a proof covers (`size`, or either size of a consistency proof) whose root is not the computed root is S_IO: the stored entries are not the ones it signed, and a proof never names a signed checkpoint whose root it does not have. The `checkpoints` rows are a projection that nothing keeps from being edited, so the proof also reads the checkpoint object each such row names, and checks its stored bytes again: S_IO unless it is held, its bytes pass evidence/0 steps 1-5 and hash to the id the row names, it is built exactly as the store builds a checkpoint, by the store signer, a held witness by the store key signs that id, and its content has the log's id, that size and the computed root. E_USAGE unless 1 ≤ seq ≤ size ≤ head, or 1 ≤ old_size < new_size ≤ head. The default size stays in the CLI.

## 8. CLI

The store commands are `evidence store <command>`, in `crates/evidence-cli/src/store_cmd.rs`; `main.rs` dispatches to them before it parses any option, and adds the store form of `evidence verify --store`. This section is the contract; `crates/evidence-cli/README.md` repeats it for users. The object commands (`canon`, `verify`, `sign`, …) keep their behaviour and their error handling (unknown options ignored, errors on standard error only, nothing on standard output). In every command, object or store, a failed write to standard output is an error with exit status 2, never a panic, and a failed write to standard error is ignored.

### 8.1 Conventions of every store command

**Arguments.**
- Every argument is UTF-8 text; one that is not (a path with such a name) is `E_USAGE`, for the object commands too.
- A path is never empty: an empty `<dir>`, `<export-dir>`, input file, `--seed`, `--out` or `--into` is `E_USAGE`, as it would otherwise name the working directory. The strings of `store heads` (system, type, key, `--lane`) are not paths, and may be empty.
- The syntax of the object commands: an argument that starts with `--` names an option and takes the next argument as its value, whatever it is (`--if-heads none`; `--out --x` names a directory `--x`). Every other argument is positional. No option goes without a value, so an option that is the last argument is `E_USAGE`.
- Each command accepts only the options shown for it below; any other is `E_USAGE`. Each option may be given once, except `--require`, which repeats. An option shown without brackets is required. Positional arguments are exactly those shown; `<x>...` means one or more.
- **Numbers** (`--from`, `--limit`, `--seq`, `--size`, `--to`, `--max-findings`, `--segment-size`): 1 to 16 ASCII digits, leading zeros allowed, no sign, with a value from 1 to 2^53 − 1. Anything else is `E_USAGE`.
- **Ids** (`store get <id>`, `--if-heads`): `sha256:` and 64 lowercase hex digits. Anything else is `E_USAGE`.
- **Purposes** (`--require`): `author`, `approve` or `witness`. Anything else is `E_USAGE`.
- Every check of the command line runs before any file is read or any store is opened, so an `E_USAGE` from the command line never creates, locks or changes anything. The library adds its own `E_USAGE` for what only it can judge: a store name, a seed file's content, a proof position above the head.
- `evidence store help` (or `-h`, `--help`) prints the usage text on standard output and exits 0. `evidence store` with no command, or with an unknown one, is `E_USAGE`.

**Standard output** holds only canonical JSON (RFC 8785): one value per line, each line ending in LF, written whole and flushed before the command goes on. Members appear in canonical order, so the shapes below list them in that order. Nothing else is printed there.

**Standard error** holds human-readable detail, each line starting `evidence: `. It is not part of the contract, with one exception: for an operation error, the first line starts `evidence: <code>`, or `evidence: <file>: <code>` and `evidence: <file>:<line>: <code>` when the error concerns one input. A usage error is followed by the usage text. A rejected item gets one line, `evidence: <file>[:<line>]: <code>: <detail>`.

**Exit status.**

| Status | Meaning | Standard output |
|---|---|---|
| 0 | Success: every item accepted (duplicates included), or a report that is ok. | The result. |
| 1 | A refusal that is a result: an item rejected at intake (`append`, `attest`), or a report that is not ok (`get`, `verify`, `import`). | The whole result, as for 0. |
| 2 | An operation error: `E_USAGE`, `S_LOCKED`, `S_STORE_KEY`, `S_NOT_A_STORE`, `S_EXISTS`, `S_FORMAT`, `S_IO` or `S_NOT_HELD`. | Last line `{"code":"<code>","ok":false}`, with `"file"` (and `"line"`) when the error concerns one input of `append` or `attest`, for example `{"code":"S_IO","file":"b.json","ok":false}`. Lines printed before it stay true: they report work that was committed. |

The error line carries no detail: detail differs between implementations and runs, so it goes to standard error. `S_STORE_KEY` is an operation error (exit 2), not a rejection. `S_NOT_HELD` is the CLI's addition to the operation codes (`spec/store-0.md`): `store get` names a well-formed id that the store does not hold.

**Clock, key and lease.**
- `init`, `append`, `attest` and `checkpoint` write through `SqliteLog` with `SystemClock`. `init` and `checkpoint` sign with `SeedSigner::from_file(--seed)`, read before the store is opened.
- A writer holds the lease for the whole command and releases it when the process ends. A lease held by another writer is `S_LOCKED` at once: no waiting, no retry.
- `get`, `heads`, `log`, `verify`, `export`, `prove` and `evidence verify --store` read through `SqliteReader`, take no lease, and so run while a writer appends. `import` opens no existing store.
- There is no scripted clock, no hidden option and no environment variable. Tests that need fixed times use the library (T2); the CLI tests assert seqs, codes and hashes, never times.

**Store or export.** A directory holds a store when `<dir>/log.db` exists as any kind of file (looked up with `symlink_metadata`, so a dangling symlink counts), and an export when `<dir>/manifest.json` exists the same way. A `log.db` that is not a store is then an error from opening it (`S_NOT_A_STORE`, `S_FORMAT`), never a reason to read the directory another way. A lookup that fails because the file is not there, or because `<dir>` is not a directory, means it does not exist; any other failure (a directory this user cannot search, a symlink loop) is `S_IO`, as it is for every command that opens the store.

### 8.2 Commands

**`evidence store init <dir> --name <name> --seed <seed-file>`**
- Reads the seed (`S_IO`; `E_USAGE` unless it holds 64 hex digits), then `SqliteLog::init` (`E_USAGE` for a name that is not a store name; `S_EXISTS`; `S_LOCKED`; `S_IO`). The writer is dropped at once.
- Prints `{"head":{"entry_hash":"sha256:…","seq":2},"key":"ed25519:…","log":"sha256:…","name":"…"}`: the log id, the genesis name and key, and the head. Exit 0.

**`evidence store append <dir> <object.json>... [--if-heads <id>[,<id>…] | --if-heads none]`**
- `--if-heads` needs exactly one file (`E_USAGE` otherwise). `none` means the subject and lane must have no heads; otherwise the ids, separated by commas, are compared with the heads as a set (repeats allowed).
- Opens the writer before it reads any file. Then, for each file in argument order: read it whole, commit it alone (one transaction per file), print its line, go on. A line is printed after its commit and before the next file is read.
- Accepted, a duplicate included: `{"duplicate":false,"entry_hash":"sha256:…","file":"<argument>","kind":"object","ok":true,"received_at":"…","ref":"sha256:…","seq":7}`. `file` is the argument as given.
- Rejected at intake: `{"code":"E_ID_MISMATCH","file":"…","ok":false}`; for `S_STALE_HEADS` also `"heads":[…]`, the current heads sorted, `[]` included. A rejection does not stop the run: the next file is tried.
- An operation error stops the run at once: a file that cannot be read (`S_IO`, with `file`), or an error from the store (its code, with `file`). Files after it are not read; files before it stay committed.
- Exit 0 if every file was accepted; 1 if at least one was rejected and there was no operation error; 2 after an operation error, whatever came before.

**`evidence store attest <dir> <attestations.jsonl>...`**
- As `append`, for each file in argument order, with one transaction per line. A file is read whole and split at LF. A line whose bytes are all ASCII whitespace, an empty line included, is skipped. Lines are numbered from 1, blank ones counted: the numbering of `evidence verify --attestations`. A CR before the LF stays in the line (it is JSON whitespace).
- Lines as for `append`, plus `"line":n`: `{"duplicate":true,"entry_hash":"…","file":"…","kind":"attestation","line":1,"ok":true,"received_at":"…","ref":"…","seq":8}` and `{"code":"E_SIGNATURE_INVALID","file":"…","line":2,"ok":false}`. An operation error carries `file`, and `line` when it concerns one line. A file with no line that is not blank prints nothing.
- The same exit status as `append`.

**`evidence store checkpoint <dir> --seed <seed-file>`**
- Reads the seed, opens the writer, writes a checkpoint. `S_STORE_KEY` (exit 2, nothing written) when the seed is not the store key.
- Prints `{"checkpoint":"sha256:…","head":"sha256:…","root":"sha256:…","seq":11,"size":10}`. Exit 0.

**`evidence store get <dir> <id> [--require <purpose>]...`**
- Reads, in one reader snapshot, the object with this id, its line, and the line of every held attestation whose `object` is the id. An id the store holds no object for (an attestation's ref included) is `S_NOT_HELD`.
- Each line it reads must be a segment line (as `store log` checks) at its own seq, whose entry's `ref` is the item it was read for: the object's line holds an object whose `id` member is the id, and each attestation's line an attestation whose hash is its `ref` and whose `object` is the id. Anything else is `S_IO` (a damaged store): `get` never prints, or reports on, an item other than the one asked for.
- Prints `{"attestations":[{"attestation":A,"entry":E},…],"entry":E,"object":O,"report":R}`:
  - `object`: the stored canonical object;
  - `entry`: its log-entry/0;
  - `attestations`: every held attestation of the object, in seq order, each as its log line;
  - `report`: the evidence/0 report (`ok`, `id`, `code`, `detail`, `content_checked`, `attestations`) that `verify_lookup` gives for the stored canonical bytes (the object as its line holds it), with the stored canonical attestation lines joined by LF as `attestations`, the store as the prev lookup, and the `--require` purposes. The stored bytes are checked again on every read, not trusted: an object whose bytes were changed but keep its `id` gives a report that is not ok (`E_ID_MISMATCH`).
- Exit 0 if the report is ok, 1 if not.
- A storage error during a prev lookup is `S_IO` (exit 2), never `E_PREV_MISSING`: the lookup asks `LogRead::object` when `prev_at` gives None.

**`evidence store heads <dir> <system> <type> <key> [--lane <lane>]`**
- Prints `{"heads":["sha256:…"],"lane":"record"}`, the heads sorted. The lane is `record` unless `--lane` gives another (`assert:<schema ref up to the first @>`). Any strings are accepted; a subject or lane with no objects prints `"heads":[]`. Exit 0.

**`evidence store log <dir> [--from <seq>] [--limit <n>]`**
- Prints the lines of seqs `--from` (default 1) onwards, at most `--limit` (default 100), in seq order. Each is exactly a segment line, `{"entry":E,"object":O}` or `{"attestation":A,"entry":E}`, followed by LF, so `store log --limit <head seq>` prints the concatenation of an export's segment files. Nothing is printed past the head. Exit 0.
- Each line is checked before it is printed: it must pass the line checks of verification (`evidence_log::parse_line`: canonical JSON, the two members, a valid entry of the body's kind), and its entry's `seq` must be its own. A line that does not, which only a damaged store holds, is `S_IO`; the lines printed before it stay. The content of the items is not checked: that is `store verify`'s work.
- It reads at most 1,024 lines at a time, so its memory does not grow with `--limit`.

**`evidence store verify <dir> [--max-findings <n>]`**
- A store: `verify_store` (source `store`). Otherwise an export: `verify_export_dir` (source `export`). Otherwise `S_NOT_A_STORE`. A directory holding both is read as a store.
- Prints the store-report/0. Exit 0 if it is ok, 1 if not. `--max-findings` defaults to 1000. For every export case, the output is canonical(`expect`) and LF.

**`evidence store export <dir> --out <dir> [--segment-size <n>]`**
- Reads the store without the lease. `--out` must not exist (`S_EXISTS`), and its parent must. `--segment-size` defaults to 10,000.
- Prints the manifest: exactly the bytes of the `manifest.json` it wrote, and LF. Exit 0.

**`evidence store import <export-dir> --into <dir>`**
- An export that does not verify: prints its report (source `export`), exits 1, and leaves nothing at `--into` or beside it.
- Otherwise prints the `verify_store` report of the new store (source `store`) and exits 0; if that report is not ok, it exits 1 and leaves nothing.
- `S_EXISTS` when `--into` exists, `S_NOT_A_STORE` when the export has no `manifest.json`, `S_IO`: exit 2.

**`evidence store prove <dir> --seq <n> [--size <m>]`** and **`evidence store prove <dir> --from <old-size> --to <new-size>`**
- Exactly one form; any other combination of the four options, none of them included, is `E_USAGE`.
- Inclusion: the size is `--size`, or else the `size` of the latest checkpoint; a store with no checkpoint needs `--size` (`E_USAGE`). `E_USAGE` unless 1 ≤ seq ≤ size ≤ head. Prints `{"checkpoint":"sha256:…","entry":E,"leaf_hash":"sha256:…","proof":["sha256:…"],"root":"sha256:…","seq":n,"size":m}`. `checkpoint` is the first checkpoint, in seq order, whose size is m; it is omitted when there is none.
- Consistency: `E_USAGE` unless 1 ≤ old size < new size ≤ head. Prints `{"new_root":"sha256:…","new_size":…,"old_root":"sha256:…","old_size":…,"proof":["sha256:…"]}`.
- Exit 0. A proof is checked with `verify_inclusion(leaf_hash, seq − 1, size, proof, root)` or `verify_consistency(old_size, new_size, old_root, new_root, proof)`, in Rust, WebAssembly or Python (section 9).

### 8.3 `evidence verify <file> --store <dir>`

- When `<dir>` holds a store, it is read through `SqliteReader`, in one snapshot, as the prev lookup only. Stored attestations are not used: attestations come from `--attestations`, as before. A storage error during a lookup is an error, not `E_PREV_MISSING`.
- The output and exit status are those of the folder form: for a store that holds the objects a folder holds, the report bytes are identical (T12).
- Errors follow the object commands: a message on standard error, exit 2, nothing on standard output. An unknown `--require` purpose gives the folder form's message (`usage: purpose "x" must be one of [...]`, then the usage text); a store that cannot be opened or read, or a `<dir>` of which it cannot be told whether it holds `log.db` (8.1, "Store or export"), gives `evidence: <dir>: <code>: <detail>`. Both forms print the report the same way, so a failed write to standard output gives the same message and exit 2.
- When `<dir>` holds no `log.db`, nothing changes: every `*.json` file in it, in name order, byte for byte as before. An empty `<dir>` is never a store: the folder form refuses it, as before.

## 9. Bindings and the browser page

**WASM** (evidence-wasm `dispatch`; byte inputs accept a string or `<name>_base64`, as today):

| fn | args | result |
|---|---|---|
| `entry_hash` | `input` (log-entry/0 JSON) | string; E_USAGE whose detail names S_ENTRY_INVALID |
| `merkle_root` | `leaves` (list of leaf inputs) | `"sha256:…"` |
| `verify_inclusion` | `leaf_hash`, `index`, `size`, `proof` (list), `root` | bool |
| `verify_consistency` | `old_size`, `new_size`, `old_root`, `new_root`, `proof` | bool |
| `verify_export` | `manifest`, `segments` (list, manifest order), `max_findings`? | store-report/0 |

**JS wrapper** (`crates/evidence-wasm/js/evidence.mjs` and `.d.ts`): `entryHash`, `merkleRoot`, `verifyInclusion`, `verifyConsistency`, `verifyExport(manifest, segments, {maxFindings})`. Bytes go to the module as base64 inside the one JSON request string: from `Buffer` in Node, and in browsers from `btoa` over chunks of 3 × 8192 bytes, because Chromium's `btoa` returns `""` instead of throwing once its result would pass the longest string (2^29 − 24 characters). A base64 text of any other length than 4 × ⌈n / 3⌉ throws `RangeError`, so the module is never given shorter bytes; an input too large for one string throws the engine's own error, never `E_USAGE`.

**Python** (`evidence_layer`): `entry_hash(bytes)`, `merkle_root(list[bytes])`, `verify_inclusion(...)`, `verify_consistency(...)`, `verify_export(manifest: bytes, segments: list[bytes], max_findings=1000) -> dict`.

**Browser page** (`tools/verify-page/`: `index.html`, `page.mjs`, `page.css`; smoke test `smoke.mjs`; `README.md`):

- **Static.** Three files, no build step, no dependencies, no inline script or style. It runs `verifyExport`, the CI-tested function, and nothing of its own decides a finding.
- **Serving.** Build the module (`sh tools/build-wasm.sh`), serve the repository root over HTTP, for example `python3 -m http.server 8000 --bind 127.0.0.1`, and open `http://127.0.0.1:8000/tools/verify-page/`. A `file://` URL does not work: browsers load neither ES modules nor the `.wasm` from files. To host it elsewhere, copy `tools/verify-page/` and `crates/evidence-wasm/pkg/` keeping their relative layout: the page loads `../../crates/evidence-wasm/pkg/evidence.mjs`, which fetches `evidence_wasm.wasm` beside itself.
- **Content-Security-Policy**, a meta tag, exactly: `default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; object-src 'none'; base-uri 'none'; form-action 'none'`. Chromium refuses `WebAssembly.compile` under `default-src 'self'` alone, so `script-src` adds `'wasm-unsafe-eval'` (and nothing else: no `'unsafe-eval'`, no `'unsafe-inline'`). `'self'` covers the module scripts, the stylesheet and the fetch of the `.wasm` (`connect-src` falls back to `default-src`). Checked in Chromium with the layout above.
- **No network calls.** The page requests only its own files from its own origin: `index.html`, `page.css`, `page.mjs`, `evidence.mjs` and `evidence_wasm.wasm`. The export is read locally with the File API and sent nowhere. No fonts, analytics or other origins. The page declares no icon, so a browser may also ask the same origin for `/favicon.ico`, as for any page (full Chromium does; the headless shell does not).
- **Loading.** `page.mjs` imports the wrapper with a dynamic `import()` inside `try`, then calls `load()`. If either fails, the status says to build the module and serve the repository root over HTTP, and the inputs stay disabled.
- **Choosing the export**, three equivalent ways:
  - a folder input (`<input id="folder" type="file" webkitdirectory>`): only the files directly in the chosen folder are used (a `webkitRelativePath` of exactly two parts); subfolders are ignored;
  - a files input (`<input id="files" type="file" multiple>`): the export's files;
  - a drop zone (`#drop`): one dropped folder gives the files directly in it (`webkitGetAsEntry`, calling `readEntries` until it returns none); anything else dropped gives the dropped files, folders among them ignored.

  Files are known by their names. Two files with one name, or no `manifest.json`, is an error and nothing is verified. When no file is left because everything given was a folder or inside a subfolder, the status says so (not "no files were chosen"). Bytes come from `File.arrayBuffer()` as a `Uint8Array` and are never decoded.
- **Size limit**: `manifest.json` and the segment files it uses may hold at most 400,000,000 bytes together (`MAX_BYTES` in `page.mjs`); ignored files do not count. The page checks `File.size` of the manifest before reading it, and of the segments once the manifest gives their names, and checks the bytes read once more before verifying. Above the limit the status says how many MB the files hold and to use `evidence store verify`, and nothing is verified. The reason is the wrapper's request string: 400 MB is 533.3 million characters as base64, under Chromium's 2^29 − 24 (about 536.9 million). The CLI has no such limit.
- **Segment order**, the host rule of `spec/store-0.md` ("Export only") and `read_export_dir`: first `verifyExport(manifest, [])`. If its findings include `S_MANIFEST_INVALID` (which depends on the manifest bytes alone), the segments are the chosen files whose names are six ASCII digits then `.jsonl`, in name order. Otherwise they are the files `JSON.parse(manifest).segments` lists, in that order, each the bytes of that file, or empty bytes when it was not chosen. Other files (`case.json`, `inputs.json`) are ignored. This rule gives every export case's `expect`.
- **Max findings**: a number input `#max-findings`, default 1000, from 1 to 2^53 − 1. Changing it verifies the chosen files again.
- **Display**: ok or not ok, source, log id, head (seq and entry hash), covered (size and checkpoint), `checked`, `counts`, `truncated`, a findings table (code, seq, ref, core, file), the files used in order with their sizes (a listed file not chosen is shown as missing), and the canonical report, `ev.canonicalize(JSON.stringify(report))`, in `<pre id="report">`.
- **Untrusted text.** Everything from the export or the report reaches the page through `textContent` or `createTextNode`. `page.mjs` uses no `innerHTML`, `outerHTML`, `insertAdjacentHTML`, `document.write`, `eval` or `Function`, and makes no URL from the export's content.
- **State for tests**: `document.body.dataset.state` is `loading`, `ready`, `verifying`, `done` or `error`, and `document.body.dataset.runs` counts finished verifications from 0. `#status` holds a one-line message.
- **Smoke test** (`node tools/verify-page/smoke.mjs`; CI job `page`):
  - loads Playwright through `createRequire(import.meta.url)`, so a global install on `NODE_PATH` or a local `node_modules` works; exits 1 if Playwright or the built module is missing (it never skips);
  - serves the repository root itself: `node:http` on 127.0.0.1, port 0, GET only, paths outside the root refused, types for `.html`, `.mjs`, `.css`, `.json` and `.wasm` (`application/wasm`);
  - opens the page in Chromium (`chromium.launch()`) and waits for `ready`;
  - for each of the 42 export cases: sets `#max-findings` to the case's `max_findings` or 1000, gives the case directory to the folder input (`setInputFiles` with a directory), waits for `runs` to grow, and compares the text of `#report` with canonical(`expect`), computed in Node through the same module;
  - once more through the files input, with case 004's manifest and four segments given in a shuffled order;
  - once through the drop zone: case 102 dropped as a folder through the DevTools protocol (`Input.dispatchDragEvent` with the folder's path), which gives the page a real directory entry;
  - case 004 through the folder input beside a file with a 244-character name, at 375 and at 1280 pixels wide, with the name listed in `#ignored`;
  - after every run, checks that the page is no wider than the window (`scrollWidth` not above `clientWidth`);
  - four refusals, each with its exact status and no result shown: a sparse 450 MB segment file, a sparse 450 MB `manifest.json` (both refused by size, unread), two export folders dropped together, and a folder that holds only subfolders; the folders are made in a temporary directory and removed at the end;
  - fails on any console error or page error (a CSP violation included), on any request to another origin, and if `page.mjs` contains one of the sinks listed above;
  - prints `page smoke: 46 runs and 4 refusals ok` and exits 0, or lists every failure and exits 1.
- **Manual check** (not automated): dragging an export folder, and its files, from a file manager onto the page, in Chromium and in a browser built on another engine. `tools/verify-page/README.md` lists the steps, and `tools/verify-page/index.html` says how to serve the page.

## 10. Test vectors (generated by tools/make-vectors.mjs via tools/reference/log.mjs)

```
test-vectors/store/
  entries.json   valid [{name, entry, canonical, hash}]; invalid [{name, input, error:"S_ENTRY_INVALID"}]
  merkle.json    [{name, leaves:[string], root, inclusion:[{index, proof, valid}], consistency:[{old_size, old_root, proof, valid}]}]
  exports/<case>/case.json      {description, expect:<store-report/0>} (+ "proofs" in 003)
  exports/<case>/inputs.json    valid cases only: the operation script that built the export
  exports/<case>/manifest.json, 000001.jsonl, …
```

**entries.json**
- Valid: the genesis entry (seq 1, prev = log = ref); an object entry; an attestation entry; received_at with 9 fraction digits; epoch 7.
- Invalid: seq 0; seq 1.5; an unknown member; prev missing; kind `"blob"`; received_at with an offset; prev that is not a hash.

**merkle.json**
- Leaves are `"leaf-0" … "leaf-(n-1)"` for n in 1, 2, 3, 4, 5, 7, 8, 9, 16, 17 and 100.
- Inclusion proofs for the first, last and middle index.
- Consistency proofs from old sizes 1, 2, 3 and n−1.
- Negative cases, each expected to return false: one proof byte flipped, and an index out of range.

**inputs.json format**
```json
{"name":"demo","seed":"store-demo","segment_size":10000,
 "clock":["2026-11-26T00:00:00.000Z","2026-11-26T00:00:01.000Z"],
 "ops":[{"op":"init"},{"op":"append","file":"objects/001-create-inline/object.json"},
        {"op":"attest","file":"objects/003-agent-assertion-approved/attestations.jsonl","line":3},
        {"op":"append","object":{"…":"inline synthetic object"}},{"op":"reopen"},{"op":"checkpoint"}]}
```
- Each init, append, attest line and checkpoint consumes one clock reading.
- `reopen` increments the epoch.
- The store key is `test-vectors/keys/store-demo.seed`, derived like the other test keys from `sha256("evidence/0 test-vector key: store-demo")`.

The table below is the starting list. The normative and complete table is in [spec/store-0.md](../spec/store-0.md) ("Export cases"). Phase B added 007-late-claim and the tampered cases 119-135. They pin the line splitting, genesis, held-object, checkpoint-recognition and truncation rules, plus the strict Ed25519 checks (small-order keys and a nonce-0 witness).

**Export cases, valid (0xx)**
- 001-genesis
- 002-records-and-checkpoint: genesis; object vectors 001 and 002; the author line for 001; a checkpoint.
- 003-attestations-two-checkpoints: adds 003 with its author and approve lines, 004, a reopen, a second checkpoint, and a consistency proof.
- 004-multi-segment: case 003 with segment_size 4.
- 005-fork: two creates for one subject, giving counts.forks = 1 and ok = true.
- 006-clock-backwards: equal received_at values, and a second checkpoint whose at equals the first's.

**Export cases, tampered (1xx)**

Each case is built so that its golden report contains the named code, and the generator asserts that it does:
- 101-object-byte-flipped → S_OBJECT_INVALID, core E_ID_MISMATCH, and nothing else (leaves cover entry bytes, not object bytes)
- 102-entry-removed → S_SEQ_GAP, plus whatever follows from it, per the golden report
- 103-entries-swapped → S_SEQ_GAP and S_CHAIN_BREAK
- 104-prev-after-child → S_PREV_POSITION
- 105-checkpoint-root-wrong → S_CHECKPOINT_MISMATCH
- 106-received-regression → S_RECEIVED_REGRESSION
- 107-epoch-regression → S_EPOCH_REGRESSION
- 108-attestation-signature-invalid → S_ATTESTATION_INVALID, core E_SIGNATURE_INVALID
- 109-attestation-before-object → S_ATTESTATION_TARGET
- 110-checkpoint-unsigned → S_CHECKPOINT_UNSIGNED
- 111-wrong-log → S_WRONG_LOG
- 112-line-not-canonical → S_LINE_INVALID
- 113-duplicate-ref → S_DUPLICATE_REF
- 114-genesis-missing → S_GENESIS_INVALID
- 115-prev-dated-after → S_OBJECT_INVALID, core E_PREV_ORDER
- 116-segment-hash-mismatch → S_SEGMENT_HASH
- 117-reserved-subject → S_RESERVED_SUBJECT
- 118-ref-mismatch → S_REF_MISMATCH

`test-vectors/README.md` gains a store section that describes these files.

## 11. Tests and CI (the acceptance criteria)

- **T1. Store vectors in four implementations.** `entries.json`, `merkle.json` and every export case give the expected result in Rust (`crates/evidence-log/tests/vectors.rs`), the JS reference (`tools/check-vectors.mjs`), WASM (`js/test.mjs`) and Python (`tests/test_store_vectors.py`). For exports, canonical(report) equals canonical(case.expect) byte for byte.
- **T2. Writer agreement** (`crates/evidence-store/tests/writer_vectors.rs`). Replaying each 0xx `inputs.json` with `ScriptClock` and the test seed, into both `SqliteLog` and `MemLog`, exports a manifest and segments byte-identical to the committed files.
- **T3. The 20 object vectors of M1 (001-005 and 101-115) through the CLI store** (`crates/evidence-cli/tests/store_cli.rs`, `t3_object_vectors_through_a_cli_store`), on a store created by `store init` with `store-demo.seed`. Each `store append` below is one invocation per file.
  1. Append 001-005: exit 0, `duplicate: false`, at seqs 3-7 (003 at seq 5).
  2. `store get <003 id> --require approve` exits 1; its report has code E_ATTESTATION_MISSING, which is case 113's code, and `attestations` is `[]`.
  3. `store attest` 003's `attestations.jsonl`: exit 0. Three lines land at seqs 8-10: author and approve for 003, and author for 001.
  4. Append each 1xx object:
     - 101-109 and 114 are refused (exit 1) with exactly their case.json code;
     - 110 gives E_PREV_MISSING;
     - 111 gives E_PREV_ORDER;
     - 112, 113 and 115 (003's object) exit 0 with `duplicate: true` at seq 5.
  5. Attest the attestation files of 112, 113 and 115. 112 gives a duplicate author line and E_SIGNATURE_INVALID for the approve line (exit 1). 113 gives a duplicate (exit 0). 115 gives E_ATTESTATION_SHAPE (exit 1).
  6. `store get <003 id> --require approve` exits 0, with report attestations {approve: 1, author: 1}, `entry.seq` 5, and 003's two attestation lines.
  7. `store verify` is ok, with checked {entries: 10, objects: 6, attestations: 4, checkpoints: 0}.
  8. `store checkpoint` prints size 10 at seq 11, and `store verify` then gives covered.size = 10.
- **CLI contract** (`store_cli.rs`). The rules of section 8 each have a test: usage errors (exit 2, exactly `{"code":"E_USAGE","ok":false}`, nothing created or changed, empty paths included), operation errors (each code, last on standard output, `S_FORMAT` and a store that cannot be looked up included), runs mixing accepted and rejected items, `--if-heads` and `store heads`, `store log` (its lines equal an export's segments), stores damaged behind the triggers (`store get` and `store log` give `S_IO` rather than print a line that is not the one asked for), and standard output that cannot be written (exit 2 in every command).
- **T4. Contract suite** (`tests/contract.rs`), run against both `MemLog` and `SqliteLog`:
  - genesis is at seq 1 and 2;
  - seq is dense and the chain is correct;
  - a duplicate append returns the same receipt;
  - E_PREV_MISSING and E_PREV_ORDER;
  - S_RESERVED_SUBJECT;
  - S_ATTESTATION_TARGET, E_SIGNATURE_INVALID, and duplicate attestations;
  - `if_heads` matching, stale (S_STALE_HEADS returns the current heads), and `none`;
  - a batch [valid, valid, invalid] leaves nothing appended and reports index 2;
  - the heads fold handles update, fork and merge;
  - received_at is monotone under a backwards clock;
  - a checkpoint's root recomputes, its `at` follows the service rule, and a wrong seed gives S_STORE_KEY;
  - export, verify and import round-trip.
- **T5. Property test** (`tests/props.rs`; 256 cases in CI, 4,096 nightly). Random scripts of up to 200 operations over 5 subjects: creates, updates with current or stale heads, deletes, asserts in 2 schemas, merges, attestations, checkpoints, reopens and backwards clock steps. These must hold:
  - `verify_store` is ok;
  - the `heads` table equals the fold over the lines;
  - every checkpoint root recomputes, and the inclusion proof for every seq verifies;
  - consecutive checkpoints have verifying consistency proofs;
  - `MemLog` and `SqliteLog` exports are byte-identical;
  - import(export) has the same head entry_hash.
- **T6. Crash test** (`crates/evidence-cli/tests/store_crash.rs`, `#![cfg(unix)]`; the test process runs only CLI processes and never opens a store itself).
  - **Objects.** 2,000 synthetic objects generated by the test, the same on every run: object i is about subject `crash-test`/`item`/`item-<i mod 20>`; the first of each subject is a `create` with `prev` [], the rest `update`s whose `prev` is the subject's previous object; `at` = 2026-11-26T00:00:00Z + i seconds; inline content `{"n":i}`; sealed with `evidence_core::api`. Written once per test binary as `0000.json` to `1999.json`.
  - **Append killed** (`append_killed_at_random_keeps_every_printed_receipt`). Each iteration: `store init` a new store; spawn `store append <store> 0000.json … 1999.json` with standard output piped; choose k from 0 to 1,999, read k receipt lines, wait a random 0 to 999 µs, and send SIGKILL (`Child::kill`). Then read what the pipe still holds (a final piece without LF is ignored) and check:
    - each printed line is the receipt of file i at seq i + 3, in order;
    - `store verify` is ok;
    - every printed receipt is in `store log` at its seq, with its `ref`, entry hash and `received_at`;
    - the head is the last printed seq or one more (a commit whose line the kill cut off); with none printed, seq 2 or 3;
    - `store append` of the next file not held lands at head + 1, so the killed writer's lease is gone, with epoch 3 (2 or 3 if no receipt was printed: the kill may have come before the writer took its epoch); `store verify` is still ok.
  - **Import killed** (`import_killed_at_random_leaves_no_store_or_a_complete_one`). Once: append all 2,000 objects (exit 0, head seq 2,002), export, and time one `store import` of the export: T. Each iteration: spawn `store import <export> --into <new>`, wait a random time from 0 to 1.2 T, send SIGKILL. Then either `<new>` does not exist, or the kill came after the rename and `store verify <new>` is ok with the manifest's head. `<new>.partial-*` directories may remain; when `<new>` does not exist, `store import` into it again exits 0.
  - **Iterations.** `EVIDENCE_CRASH_ITERS` sets the append iterations: 10 when unset, 50 in CI, 1,000 nightly. The import test runs ⌈iterations / 5⌉. The random choices come from a small generator seeded by `EVIDENCE_CRASH_SEED`, or by the system time when it is unset; the seed is printed to standard error so a failure can be repeated. No new dependency: `Child::kill` is SIGKILL on Unix.
- **T7. Lease** (`tests/sqlite_only.rs`, `tests/init_killed.rs` and `store_cli.rs`; unit tests in `src/sqlite`):
  - a second `SqliteLog::open` returns `Locked`;
  - the CLI exits 2, printing exactly `{"code":"S_LOCKED","ok":false}`, while another process holds the lease (`store_cli.rs`, `lease_held_by_another_process_gives_s_locked`: the test process holds it through `SqliteLog::open`, and `store append` and `store checkpoint` are refused while `get`, `heads`, `log`, `verify`, `export` and `prove` succeed);
  - readers work while a writer holds the lease;
  - the epoch increments on each open and appears in the entries;
  - `Lease::acquire` starts again when `LOCK` is removed or replaced between its open and its lock, never follows a `LOCK` that is a symlink, and refuses one with another hard link; a `LOCK` it creates as root beside a `log.db` of another user belongs to that user;
  - an `init` killed at any moment leaves a store or a directory the next `init` takes (`tests/init_killed.rs`, a binary of its own because it starts processes); a failed `init` removes nothing once `log.db` is in place, and no `LOCK` it does not hold; it syncs the parents of the directories it creates before it builds;
  - `SqliteLog::open` refuses a `log.db` replaced before it took the lease;
  - a reader that cannot write `log.db`, or whose user does not own it (shown as root), creates nothing; a `log.db-wal` or `log.db-shm` that is a FIFO is S_IO for the reader and the writer, never a read that blocks; a reader of an immutable file notices a writer by `log.db` itself (a `LOCK` removed after the writer closed, a working directory changed after a relative open), and by the WAL beside the target of a symlinked `log.db`;
  - stores in directories whose relative names start with `file:` are the ones read and written (`tests/relative_paths.rs`, a binary of its own because it changes the working directory).
- **T8. Append-only and drift** (`tests/sqlite_only.rs`):
  - a raw rusqlite UPDATE or DELETE on `entries`, `objects` or `attestations` fails with "S_APPEND_ONLY";
  - after `DROP TRIGGER` and an edit to one object's bytes, `verify_store` reports S_TRIGGERS_MISSING and S_OBJECT_INVALID (core E_ID_MISMATCH) at that seq;
  - deleting a `heads` row gives S_PROJECTION_DRIFT;
  - an edit behind the triggers to any derived column, an item row no line shows, a line split differently between its columns, or a damaged index gives S_PROJECTION_DRIFT, and an edit to the last entry's hash, time or epoch stops the writer;
  - a `checkpoints` row with a negative seq or size gives S_PROJECTION_DRIFT, not S_IO; one moved to another size with the root the entries give there gives S_PROJECTION_DRIFT, and a proof that would name it is S_IO;
  - an extra or changed trigger gives S_TRIGGERS_MISSING and the writer refuses the store; a `meta` log id, key, name or epoch that disagrees with the lines gives S_META_MISMATCH and the writer refuses the store;
  - a table that is a view (one that never ends included), an extra table, index or view, or a table changed in place is S_NOT_A_STORE at once.
- **T9. Two clocks:**
  - with a clock that steps back an hour between appends, received_at never decreases;
  - a checkpoint taken after the step has `at` equal to the previous checkpoint's `at` and verifies without E_PREV_ORDER;
  - an object claiming an `at` one day after its received_at gives counts.at_after_received = 1 and ok = true.
- **T10. Export and import:**
  - a round trip keeps an identical head entry_hash (`store_cli.rs`, `export_import_round_trip_keeps_the_head`; `contract.rs` for the library);
  - importing any 1xx case exits 1, prints a report that is not ok, and leaves nothing: no `<into>`, no `<into>.partial-*` (`import_of_a_tampered_export_writes_nothing`);
  - `store verify <export>` prints canonical(`expect`) and exits 0 for every 0xx case and 1 for every 1xx case, with `--max-findings` where `case.json` has `max_findings` (`verify_every_export_case`).
- **T11. Proofs.** One procedure, in three places:
  - **The store.** `store init` (name `proofs`, `store-demo.seed`); append object vectors 001-005; attest 003's `attestations.jsonl`; `store checkpoint` three times (sizes 10, 12 and 14; head 16).
  - **The checks.**
    - Inclusion: `store prove --seq n` for every seq from 1 to 14 (the latest checkpoint's size), and `--seq n --size 10` for every seq up to 10. Each verifies. Its `leaf_hash` is SHA-256(0x00 ‖ canonical `entry`). Its `root` is the merkle root of the canonical entries that `store log` prints for seqs 1 to size. For a checkpoint size, `root` and `checkpoint` are what `store checkpoint` printed.
    - Consistency: `store prove --from a --to b` for every 1 ≤ a < b ≤ 16. Each verifies, and its roots are the merkle roots of the first a and b entries.
    - A changed byte: the last hex digit replaced (`0` by `1`, any other by `0`) in the first proof element, the leaf hash, a root: each makes verification return false.
  - **Rust**: `store_cli.rs`, `prove_output_verifies_and_a_changed_byte_does_not`, with `evidence_log::merkle`.
  - **WebAssembly in Node**: `tools/check-proofs.mjs`, through `crates/evidence-wasm/pkg/evidence.mjs` (`verifyInclusion`, `verifyConsistency`, `merkleRoot`, `canonicalize`). The CLI binary is `EVIDENCE_BIN`; unset, the script exits 2 with a message (it never skips). Exit 0 when every check passes, 1 otherwise. CI job `wasm`.
  - **Python**: `crates/evidence-py/tests/test_cli_proofs.py`, through `evidence_layer`. It reads `EVIDENCE_BIN`; unset, the tests are skipped (unittest reports them as skipped) unless `CI` is `true`, when they fail. CI job `python`, which builds the CLI and sets `EVIDENCE_BIN`. GitHub Actions sets `CI=true` in every job.
- **T12. `evidence verify --store`** (`store_cli.rs`, `verify_with_a_store_matches_the_folder_of_objects`). For object vectors 002, 110 and 111, the report bytes and the exit status are identical whether `--store` points at a store directory holding 001 or at the case's existing `store/` folder. 003 with its attestations and `--require approve`, against a store holding 001, gives the bytes it gives without `--store`: stored attestations are not used.
- **T13. Core unchanged:**
  - `crates/evidence-core/tests/vectors.rs` passes unmodified;
  - `verify_lookup` with a `Store` gives the same reports as `verify` on every object vector;
  - on pull requests, no committed object, canon or number vector on `main` is changed or removed; new vectors are added only under a recorded decision, as [D15](decisions.md#d15-step-1-faults-are-reported-in-a-fixed-order) added 116 to 138. The `vectors` job, before it regenerates anything:
    ```sh
    git fetch --no-tags --depth=1 origin main
    git diff --exit-code --diff-filter=DMRT FETCH_HEAD -- test-vectors/objects test-vectors/canon.json test-vectors/numbers.json
    ```
    `actions/checkout` fetches one commit, so the step fetches `main` itself; no `fetch-depth: 0` is needed.
- **T14. Purity guards** (step of the `rust` job). `cargo tree` must succeed and print something for each crate, so a typo in a crate name cannot pass. Each `grep` must exit 1 (nothing found), so a typo in the path cannot pass either:
  ```sh
  for p in evidence-core evidence-log evidence-wasm evidence-py; do
    out=$(cargo tree -p "$p" -e normal --prefix none --locked)
    test -n "$out"
    if echo "$out" | grep -qE '^(rusqlite|libsqlite3-sys|tokio|axum) '; then echo "$p is not pure"; exit 1; fi
  done
  # grep exits 1 when nothing matches: a match (0) or an error (2) fails the step.
  s=0; grep -rnE 'std::(fs|net|process|thread)|SystemTime|Instant|UNIX_EPOCH' crates/evidence-log/src || s=$?
  test "$s" -eq 1
  # The same modules in a braced import, such as use std::{fs, io::Read};
  s=0; grep -rlzP 'std::\{[^;]*\b(fs|net|process|thread)\b' crates/evidence-log/src || s=$?
  test "$s" -eq 1
  ```
  The two `grep`s read text, so they miss a use written another way, such as `use std as s;` then `s::fs::read`. The item-level guard is `crates/evidence-log/clippy.toml`: it lists, as `disallowed-types` and `disallowed-methods`, the items that reach the file system (`std::fs`, and the `std::path::Path` methods that read it), the network, processes, threads and the clock (`SystemTime` and `Instant`, their `now` and `elapsed` included, so `UNIX_EPOCH.elapsed()` is found too). `crates/evidence-log/Cargo.toml` denies both lints, so the `cargo clippy --all-targets` step of the `rust` job fails on any use in the library or its unit tests. `crates/evidence-log/tests/vectors.rs` reads the vectors from disk and allows the two lints for itself only.

  Since 28 November 2026 the step also covers `evidence-coordinate`, built after M1 ([coordinate/0](../spec/coordinate-0.md)): `cargo tree` for the crate, and both `grep`s over `crates/evidence-coordinate/src`. Its `clippy.toml` holds the same lists, and its `Cargo.toml` denies the same two lints.

**CI changes** (`.github/workflows/ci.yml`; on push and pull request, `permissions: contents: read`; jobs run as a user that is not root):
- `rust`: as now, with `EVIDENCE_CRASH_ITERS: 50` on the `cargo test --locked` step (T3, T6, T7, T10, T11, T12 run there), then the T14 step.
- `wasm`: as now, then `cargo build -p evidence-cli --locked` and `node tools/check-proofs.mjs` with `EVIDENCE_BIN: target/debug/evidence` (T11). It still uploads `crates/evidence-wasm/pkg/` as the artefact `evidence-wasm-pkg`.
- `python` (3.10 and 3.13): as now, plus `cargo build -p evidence-cli --locked` before the tests, which run with `EVIDENCE_BIN: ${{ github.workspace }}/target/debug/evidence` (T11).
- `vectors`: the T13 step first (pull requests only: `if: github.event_name == 'pull_request'`), then as now.
- `page` (new, `needs: wasm`): checkout; `actions/download-artifact@v4` of `evidence-wasm-pkg` into `crates/evidence-wasm/pkg`; Node 22; then
  ```sh
  npm install --no-save --no-package-lock --prefix "$RUNNER_TEMP/pw" playwright@1.63.0
  "$RUNNER_TEMP/pw/node_modules/.bin/playwright" install --with-deps chromium
  NODE_PATH="$RUNNER_TEMP/pw/node_modules" node tools/verify-page/smoke.mjs
  ```
  Playwright is pinned to the version the page smoke test was written against, and installed outside the repository, which keeps no `package.json`.
- `check_schema_vectors.py` validates entries, manifests, genesis and checkpoint contents, and reports against the new schemas.

**Nightly** (`.github/workflows/nightly.yml`; `schedule: cron "23 3 * * *"` and `workflow_dispatch`; `permissions: contents: read`; three jobs on ubuntu-latest, each with `timeout-minutes`):
- `props` (120 min): `cargo test --locked -p evidence-store --test props` with `PROPTEST_CASES: 4096` (T5). Debug profile, as in CI: overflow checks stay on.
- `crash` (180 min): `cargo test --locked -p evidence-cli --test store_crash` with `EVIDENCE_CRASH_ITERS: 1000` (T6).
- `bench` (120 min): `cargo build --release --locked -p evidence-store --example bench_append`, then `target/release/examples/bench_append 1000000 --batch 100 > bench-batch-100.json` and `target/release/examples/bench_append 1000000 --batch 1 > bench-batch-1.json` (one object per transaction, as `store append` commits; about 15 minutes on a 4-core machine, against about 6 for `--batch 100`), then `actions/upload-artifact@v4` of both files as `bench-append` (`if: always()`, `if-no-files-found: error`). Each file is the example's one JSON line: append throughput, checkpoint and verify times, and `ok`. The numbers are informational, not a gate, and are not committed; a run whose log does not verify fails the job.

## 12. Documentation

- **`spec/store-0.md`** covers: purpose; genesis; log entry; lanes and heads; intake; checkpoint and tree; export; verification order; report and codes; two clocks; privacy; conformance (the vectors are the contract, D7). The privacy section says that no rejected bytes are stored, and that only synthetic or public data goes into a store before the M5 gate. Phase E adds one row to "Intake and operation codes": `S_NOT_HELD` (section 8). Nothing else in it changes.
- **`README.md`** changes:
  - Layout: `crates/evidence-cli` also runs stores; a row for `tools/verify-page/`; "in progress" dropped from the M1 rows once section 1's definition of done holds.
  - Quick start gains a "Store" section. Every command in it is run before it is committed, and its output shown as in the object examples (hashes shortened with `…`). It uses a temporary directory and the public test key, says that key is for tests only, and goes: `store init`, `store append` of object vectors 001-003, `store attest` of 003's attestations, `store get` of 003 with `--require approve`, `store checkpoint`, `store verify`, `store export`, `store verify` of the export, `store prove --seq 3`, and verifying the export in the browser (build the module, `python3 -m http.server 8000 --bind 127.0.0.1` from the repository root, open `http://127.0.0.1:8000/tools/verify-page/`).
  - Checks gains `node tools/check-proofs.mjs` (with `EVIDENCE_BIN`) and the page smoke test.
  - Status and next steps are updated to the roadmap order.
- **`crates/evidence-cli/README.md`** (new): the object and store commands, what each prints, the exit status and the operation codes, in brief, pointing to section 8 for the full contract.
- **`tools/verify-page/README.md`** (new): serving and using the page, how it orders segments, its security rules, the smoke test and the manual check.
- **`crates/evidence-store/README.md`** (new): what the crate is (the writer, the reader, export, import, verification and proofs), the store directory layout, the lease and epoch in one paragraph each, a short Rust example with `SqliteLog::init`, `append`, `checkpoint` and `verify_store`, and links to `spec/store-0.md` and the rustdoc of `src/api.rs`.
- **`docs/decisions.md`** gains D11-D13 (section 2).
- **`test-vectors/README.md`** gains the store files (done) and one sentence on who runs them: the CLI tests (T3 over the object vectors, T10 over every export case) and the page smoke test.
- **`docs/m1-store-spec.md`**: the status line, and section 3 kept in step with the files that exist.

## 13. Explicitly out of scope for M1

- Any connection to a system of record: capture, mappings, projection, change keys, blobs and by-reference content (M2).
- Reconciliation (M3).
- Derived `index.db`, queries beyond get, heads and log, bundles, and any HTTP service (M4).
- Requests, outbox and write paths (M6-M7).
- KMS or HSM signers, key binding and revocation, RFC 3161 timestamps, replication or publishing (export writes files only), and erasure (all M5).
- Quarantine of rejected bytes. It is never done.
- Validation of profile data under `ext`. Unknown keys are stored unchecked, and only the store's own subjects are reserved.
- A Postgres backend, multiple writers, high availability, and compaction or segment rotation inside `log.db`.
- Bounded-memory verification. Prime coordinates.
- The crash test on Windows; it runs on Linux and macOS only.

## 14. Build order inside the milestone

- **Week 1.** Draft `spec/store-0.md` and the schemas. Make the evidence-core additive change. Write `evidence-log` entry, line, merkle, genesis, checkpoint, lane and clock modules, with the matching parts of `tools/reference/log.mjs`. Generate `entries.json` and `merkle.json`.
- **Week 2.** Build the Verifier, report, manifest and `verify_export` in Rust and in JS, the JS writer, and the export vectors (0xx and 1xx). Add the WASM and Python functions and their tests (T1).
- **Week 3.** In `evidence-store`: api, intake, `MemLog` and the contract suite. Then `SqliteLog` schema, lease, commit, heads and checkpoint (T4, T7, T8, T9).
- **Week 4.** Export and import, `verify_store` live checks, prove, the proptest, and the writer-vector test (T2, T5, T10, T11).
- **Week 5.** CLI commands and tests (T3, T6, T12), the browser page, docs, CI and the nightly workflow, and the benchmark (T13, T14).