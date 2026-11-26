# store/0

Status: draft for discussion. Version tag `store/0`; breaking changes bump the number.

store/0 is a companion to [evidence/0](evidence-0.md) and changes nothing in it. Objects and attestations are stored exactly as evidence/0 defines them; the store's own facts are kept beside them, never inside them.

## Purpose

A store/0 store gives evidence/0 objects and their attestations a durable, append-only home that anyone can check without trusting its operator.

- Every item is an entry in one log. The store gives each entry a dense position (`seq`) and the time the store received it (`received_at`). These facts live in the entry, outside the object, so the object's id never changes.
- Entries are hash-chained. The store signs checkpoints: RFC 9162 Merkle tree hashes over the entries. With a checkpoint, anyone can check that an entry is in the log (an inclusion proof) and that a later log extends an earlier one (a consistency proof).
- The log can be exported as canonical JSON Lines segments with a manifest. An export can be verified offline, and every conforming implementation produces a byte-identical report for it.
- A store has one writer at a time. Every entry records the writer epoch it was appended in.

The store records; it does not act. Record changes in the log (`create`, `update`, `delete`) are only ever observations of a system of record, and anything the store or an application states itself is an `assert` ([D11](../docs/decisions.md)). The store never writes to a system of record.

The reference implementation keeps the log in one SQLite file ([D12](../docs/decisions.md)). That file's layout is not part of store/0. The export is the portable form and defines conformance: another backend conforms if its exports verify and match.

### Terms

- **Hash:** `sha256:` followed by 64 lowercase hex digits, as in evidence/0.
- **Canonical form:** the [RFC 8785](https://www.rfc-editor.org/rfc/rfc8785) form of JSON parsed under evidence/0's strict rules (UTF-8 with no byte order mark, no duplicate members, no lone surrogates, no number with magnitude above 2^53 − 1).
- **Nesting limit:** arrays and objects nest at most 128 deep (`{}` and `[]` are depth 1), the limit of the evidence/0 reference parser. Deeper JSON is not strict JSON here: an object that deep is rejected at intake with `E_PARSE`, and an entry that deep is `S_ENTRY_INVALID`. A segment line wraps its item in one more object, so a line may nest 129 deep, and every object that evidence/0 accepts fits in a line.
- **Timestamp:** an evidence/0 timestamp, `YYYY-MM-DDTHH:MM:SS[.fraction]Z` with at most 9 fraction digits, that is a real calendar instant. Timestamps are compared as instants, not as text, so `2026-11-26T00:00:00Z` equals `2026-11-26T00:00:00.000Z`.
- **Entry hash:** `"sha256:"` + hex(SHA-256(canonical form of the entry)).
- **Log id:** the id of the log's genesis object (seq 1). It names the log everywhere else.
- **Held:** an object or attestation is held when the log has an entry for it.
- **Store subjects:** subjects whose `system` is `"evidence-store"`. They are reserved for the store's own objects.
- **Store key:** the Ed25519 key the genesis declares. It signs the genesis author attestation and every checkpoint witness.

### Code namespaces

- `E_` codes are evidence/0 object and attestation results only. store/0 defines no new `E_` codes. At intake an `E_` code is returned unchanged; in a report it is carried in a finding's `core` member.
- The store uses `S_` codes, listed under [Report and codes](#report-and-codes).
- Profiles use `P_` codes.

## Genesis

Seq 1 of every store is its **genesis**, an evidence/0 object whose id is the **log id**. Seq 2 is the genesis **author attestation**.

```json
{"v":"evidence/0","id":"sha256:…","op":"assert",
 "schema":{"ref":"evidence-store/genesis@0","hash":"sha256:369979e1a083afc7096df02394b74ebf42a0cf4712b2bd377df9fc0d1f57be03"},
 "subject":{"system":"evidence-store","type":"log","key":"cer-demo"},
 "content":{"media_type":"application/json","hash":"sha256:…",
   "inline":{"v":"store/0","name":"cer-demo","signer":"service:evidence-store/cer-demo","key":"ed25519:…"}},
 "actor":{"id":"service:evidence-store/cer-demo","kind":"service"},
 "at":"2026-11-26T00:00:00.000Z","prev":[]}
```

The genesis content (`content.inline`) has exactly these members. The JSON Schema is [`schema/store-genesis-0.schema.json`](schema/store-genesis-0.schema.json).

| Member | Rule |
|---|---|
| `v` | `"store/0"` |
| `name` | The store's name. Matches `^[a-z0-9][a-z0-9.-]{0,62}$`. |
| `signer` | `"service:evidence-store/"` + `name`. |
| `key` | The store key's Ed25519 public key, written as in evidence/0 (`ed25519:` + 32 bytes of unpadded canonical base64url). |

The genesis object around that content:

| Member | Value |
|---|---|
| `op` | `"assert"` |
| `schema.ref` | `"evidence-store/genesis@0"` |
| `schema.hash` | `GENESIS_SCHEMA_HASH` |
| `subject` | `{"system":"evidence-store","type":"log","key":<name>}` |
| `content.media_type` | `"application/json"`, with `content.inline` as above and `content.hash` its evidence/0 content hash. No `content.uri`. |
| `actor` | `{"id":<signer>,"kind":"service"}` |
| `at` | The writer's service time when the store is created (see [Two clocks](#two-clocks)). |
| `prev` | `[]` |
| `ext` | Absent. |

`GENESIS_SCHEMA_HASH` is `"sha256:"` + hex SHA-256 of the bytes of `schema/store-genesis-0.schema.json`:

```
sha256:369979e1a083afc7096df02394b74ebf42a0cf4712b2bd377df9fc0d1f57be03
```

The file is authoritative. Implementations hold the value as a constant and test it against the file.

A genesis is fully determined by its name, its key and its `at`. Building the object from those three values and sealing it (setting `id`) gives the same canonical bytes and the same log id in every implementation. A **valid genesis** is an object whose content is valid store/0 and which is exactly the object built from its own `name`, `key` and `at`.

**Author attestation.** Seq 2 is an attestation/0 line whose `object` is the log id, `purpose` is `author`, `signer` and `key` are those the genesis declares, and `at` equals the genesis `at`. Its valid signature proves that whoever created the log held the store key.

## Log entry

Every item in the log has one entry, `log-entry/0`. All members are required; no other members are allowed. The JSON Schema is [`schema/log-entry-0.schema.json`](schema/log-entry-0.schema.json).

```json
{"epoch":1,"kind":"object","log":"sha256:…","prev":"sha256:…","received_at":"2026-11-26T00:00:01.000Z","ref":"sha256:…","seq":3,"v":"log-entry/0"}
```

| Member | Type | Rule |
|---|---|---|
| `v` | string | `"log-entry/0"` |
| `log` | hash | The log id. |
| `seq` | integer | 1 or more, at most 2^53 − 1. Dense: entry n has seq n. |
| `kind` | string | `"object"` or `"attestation"`. |
| `ref` | hash | For an object, its id. For an attestation, `"sha256:"` + hex SHA-256 of the canonical attestation, including `sig`. |
| `received_at` | timestamp | When the store received the item. Never less than the previous entry's. Writers emit exactly 3 fraction digits; verifiers accept any valid timestamp. |
| `epoch` | integer | 1 or more: the writer epoch. Never less than the previous entry's. |
| `prev` | hash | The entry hash of seq − 1. For seq 1, the log id. |

- The entry hash covers every member, so `received_at` and the epoch are covered by the chain.
- For seq 1, `log` = `ref` = `prev` = the log id.
- Integers are JSON numbers with no fractional part. Like every JSON number under evidence/0, they are at most 2^53 − 1.

**Entry validity.** An entry is valid `log-entry/0` when it is a JSON object with exactly the members above, each of the type and pattern given, and its `received_at` is a real calendar instant. Anything else is `S_ENTRY_INVALID`. The rules that relate an entry to the rest of the log (dense seq, the chain, non-decreasing `received_at` and epoch, the log id, and the seq 1 equalities) are checked during [verification](#verification), each with its own code. Breaking one of them does not make the entry itself invalid.

**Writer epoch.** A store's epoch starts at 1 when it is created, and increases by one each time a writer opens it. Entries written by one writer session carry that session's epoch.

## Lanes and heads

A **subject key** is the triple (`subject.system`, `subject.type`, `subject.key`).

**Lane.** Every object belongs to one lane of its subject:

- `"record"` if `op` is `create`, `update` or `delete`;
- `"record"` if `op` is `assert` and `ext` has a member named `"capture/0"`. This is reserved for baseline observations by capture, so that heads stay valid when capture is added;
- otherwise `"assert:"` + `schema.ref` up to, not including, the first `@` (the whole ref if it has no `@`).

For example, an assert whose `schema.ref` is `photo-check/result@1` is in lane `assert:photo-check/result`. The genesis is in lane `assert:evidence-store/genesis`, and checkpoints are in lane `assert:evidence-store/checkpoint`.

**Heads.** `heads(S, L)` is the set of objects with subject key S and lane L that no object with subject key S and lane L lists in its `prev`. A `prev` that points at an object with a different subject key or lane does not change heads.

Every `prev` of an accepted object is held at a lower seq, so heads can be kept by a fold in seq order: for each object, remove its `prev` ids from `heads(S, L)` and add its own id.

**Forks.** A fork is a subject whose record lane has more than one head. A fork is not an error: a later object can merge it by listing every head in its `prev`. Reports count forks (`counts.forks`). store/0 defines no status words (current, forked, deleted, absent); those are views computed over heads, outside this spec.

## Intake

Intake is what a writer checks, in order, before it appends an item. A verifier never relies on intake having run: it checks the log on its own terms.

**Object:**

1. evidence/0 steps 1-5. On failure, reject with that `E_` code.
2. If `subject.system` is `"evidence-store"`, reject with `S_RESERVED_SUBJECT`. Only the store itself writes store subjects: the genesis when the store is created, and checkpoints.
3. If the id is already held, return the existing entry's receipt with `duplicate: true`. `if_heads` is not evaluated, so a compare-and-set append that is retried after it landed still succeeds.
4. evidence/0 step 6 against the held objects: every `prev` must be held and none may be dated after `at`. Failure gives `E_PREV_MISSING` or `E_PREV_ORDER`.
5. If `if_heads` is given, the set `heads(subject, lane)` must equal it exactly. An empty `if_heads` means the subject and lane must have no heads. Otherwise reject with `S_STALE_HEADS`, and return the current heads.
6. Append:
   - `seq` = the last entry's seq + 1;
   - `received_at` = the later of the writer's clock, truncated to milliseconds, and the last entry's `received_at` (see [Two clocks](#two-clocks));
   - `epoch` = the writer epoch;
   - `prev` = the last entry's entry hash;
   - the entry, the object and the heads change are written in the same transaction.

**Attestation line:**

1. Parse it as attestation/0. Failure gives `E_ATTESTATION_SHAPE`.
2. The target object (`object`) must be held. Otherwise `S_ATTESTATION_TARGET`.
3. The signature must verify. Failure gives `E_SIGNATURE_INVALID`.
4. If its ref is already held, return the existing entry's receipt with `duplicate: true`.
5. Append, as for an object.

**Rules for both:**

- An accepted item gets a **receipt**: its kind, ref, seq, entry hash, `received_at`, and `duplicate`.
- A batch of items is written in one transaction and is all or nothing. If any item is rejected, nothing in the batch is appended, and the rejection names the item's position in the batch. Duplicates inside a batch do not fail it.
- Rejected input is not stored anywhere (see [Privacy](#privacy)).

## Checkpoint and tree

Only the store writes checkpoints. Each checkpoint is one transaction with two entries: the checkpoint object, then its witness attestation.

```json
{"v":"evidence/0","id":"sha256:…","op":"assert",
 "schema":{"ref":"evidence-store/checkpoint@0","hash":"sha256:fa9f7b8bcfc8d2adba92090d2722e0ab9e89de8eb298d5641b7a074bc91ee540"},
 "subject":{"system":"evidence-store","type":"checkpoint","key":"<log id>"},
 "content":{"media_type":"application/json","hash":"sha256:…",
   "inline":{"v":"checkpoint/0","log":"<log id>","size":10,"head":"<entry hash of seq 10>","root":"sha256:<tree hash, entries 1..10>"}},
 "actor":{"id":"service:evidence-store/cer-demo","kind":"service"},
 "at":"…","prev":["<previous checkpoint id>"]}
```

The checkpoint content (`content.inline`) has exactly these members. The JSON Schema is [`schema/checkpoint-0.schema.json`](schema/checkpoint-0.schema.json).

| Member | Rule |
|---|---|
| `v` | `"checkpoint/0"` |
| `log` | The log id. |
| `size` | The seq of the last entry before the checkpoint's own entry. 1 or more. |
| `head` | The entry hash of seq = `size`. |
| `root` | The Merkle Tree Hash over entries 1 to `size` (below), written `"sha256:"` + hex. |

The checkpoint object around that content:

| Member | Value |
|---|---|
| `op` | `"assert"` |
| `schema.ref` | `"evidence-store/checkpoint@0"` |
| `schema.hash` | `CHECKPOINT_SCHEMA_HASH` |
| `subject` | `{"system":"evidence-store","type":"checkpoint","key":<log id>}` |
| `content.media_type` | `"application/json"`, with `content.inline` as above and `content.hash` its evidence/0 content hash. No `content.uri`. |
| `actor` | `{"id":<genesis signer>,"kind":"service"}` |
| `at` | Service time: the later of the store clock, truncated to milliseconds, and the previous checkpoint's `at` (see [Two clocks](#two-clocks)). |
| `prev` | `[]` for the first checkpoint; otherwise a list holding only the previous checkpoint's id. Writers must follow this; verifiers check only that `prev` holds at most one id (step j of [Body checks](#body-checks)). |
| `ext` | Absent. |

A checkpoint object is fully determined by its content, the genesis signer, its `at` and its `prev`.

`CHECKPOINT_SCHEMA_HASH` is `"sha256:"` + hex SHA-256 of the bytes of `schema/checkpoint-0.schema.json`:

```
sha256:fa9f7b8bcfc8d2adba92090d2722e0ab9e89de8eb298d5641b7a074bc91ee540
```

As with the genesis, the file is authoritative and implementations test their constant against it.

- **Witness attestation.** Purpose `witness`, `object` the checkpoint id, `signer` and `key` as in the genesis, and `at` equal to the checkpoint's `at`.
- **Wrong key.** If the key offered to sign a checkpoint differs from the genesis key, the request is refused with `S_STORE_KEY` before anything is written.

### Merkle tree

The tree is the [RFC 9162](https://www.rfc-editor.org/rfc/rfc9162) section 2.1.1 Merkle Tree Hash (MTH).

- **Leaves.** Leaf n is the canonical bytes of entry n: its UTF-8 bytes, without the LF that ends its segment line. The 0-based leaf index of the entry at seq n is n − 1.
- **Leaf hash.** SHA-256(0x00 ‖ leaf).
- **Node hash.** SHA-256(0x01 ‖ left ‖ right).
- **Tree hash.** For one leaf, MTH is its leaf hash. For n > 1 leaves, with k the largest power of two less than n, MTH(D[n]) = SHA-256(0x01 ‖ MTH(D[0:k]) ‖ MTH(D[k:n])). store/0 never uses the tree of zero leaves.

The leaf hash is not the entry hash. The entry hash is SHA-256 of the canonical entry alone; the leaf hash adds the 0x00 prefix that RFC 9162 uses to keep leaves and nodes apart.

### Proofs

- **Inclusion proofs** follow RFC 9162 section 2.1.3: PATH(m, D[n]) to produce one (2.1.3.1) and the algorithm in 2.1.3.2 to verify one, with leaf index m = seq − 1 and tree size n.
- **Consistency proofs** follow RFC 9162 section 2.1.4: PROOF(m, D[n]) to produce one (2.1.4.1) and the algorithm in 2.1.4.2 to verify one, from old size m to new size n.
- Each proof element is written `"sha256:"` + hex.
- Verification answers true or false; it never fails with an error. It is false when the index is not below the tree size, when the old size is not between 0 and the new size (exclusive at both ends), when the proof has the wrong number of elements, and when the computed root differs. These are the ranges RFC 9162 defines verification for. It is also false when a tree size is above 2^53 − 1, because a tree size is a seq.

## Export

An export is a directory holding `manifest.json` and numbered segment files. The export bytes depend only on the log content and `segment_size`, so two conforming writers produce identical exports of the same log.

### Manifest (segments/0)

`manifest.json` holds exactly the canonical form of the manifest, with no trailing newline. The JSON Schema is [`schema/segments-0.schema.json`](schema/segments-0.schema.json).

```json
{"checkpoint":{"id":"<latest checkpoint id>","size":10},"head":"<entry hash of seq 12>","log":"<log id>","segment_size":10000,"segments":[{"file":"000001.jsonl","first":1,"hash":"sha256:<file bytes>","last":12}],"size":12,"v":"segments/0"}
```

| Member | Rule |
|---|---|
| `v` | `"segments/0"` |
| `log` | The log id. |
| `size` | The number of entries exported: the seq of the last one. |
| `head` | The entry hash of seq = `size`. |
| `segment_size` | Lines per segment file, 1 or more. |
| `segments` | One member per segment file, in order: `file`, `first` and `last` (the seqs of its first and last lines), and `hash` (`"sha256:"` + hex SHA-256 of the file's bytes). |
| `checkpoint` | `{"id":…,"size":…}` of the latest checkpoint. Omitted if the log has no checkpoint. |

The segment list follows from `size` and `segment_size`. Segment i, counting from 1, is named with i in decimal, zero-padded to six digits, followed by `.jsonl` (`000001.jsonl`, `000002.jsonl`, …). It holds seqs (i − 1) × `segment_size` + 1 to the smaller of i × `segment_size` and `size`. Every segment except the last therefore holds exactly `segment_size` lines.

### Segment files

Each segment file holds one canonical line per entry, in seq order, each terminated by a single LF (0x0A).

- Object line: `{"entry":E,"object":O}`.
- Attestation line: `{"attestation":A,"entry":E}`.

E, O and A are the canonical forms of the entry, the object and the attestation. Written in this order, a line is already canonical, because RFC 8785 sorts the members exactly as written. The same lines, without their LF, are the lines of a live store.

## Verification

One algorithm verifies a live store and an export. It reads the lines in the order they are stored (for an export, the lines of each segment file, taking the files in manifest order) and processes each line in turn. Object verification under evidence/0 stops at the first failure; store verification does not. It collects every finding, because an auditor needs to know how much is damaged. Nothing is repaired.

A line that fails a line check is **unusable**: processing of that line stops there.

### Line checks

| Step | Check | Code |
|---|---|---|
| a | The line is strict JSON nesting at most 129 deep (see [Terms](#terms)), has exactly two members, `entry` and one of `object` or `attestation`, and equals its canonical form byte for byte. | `S_LINE_INVALID` |
| b | `entry` is valid `log-entry/0`. | `S_ENTRY_INVALID` |
| b2 | The line's other member is the one `entry.kind` names: `object` for `"object"`, `attestation` for `"attestation"`. | `S_LINE_INVALID` |
| c | `entry.log` equals the log id, which is taken from line 1. | `S_WRONG_LOG` |

### Sequence and clock checks

Each of these can add a finding, and processing of the line continues.

| Step | Check | Code |
|---|---|---|
| d | `entry.seq` equals the expected seq, which starts at 1. Either way, the expected seq becomes `entry.seq` + 1. | `S_SEQ_GAP` |
| e | `entry.prev` equals the previous line's entry hash, or the log id when `entry.seq` is 1. Skipped if the previous line was unusable. | `S_CHAIN_BREAK` |
| f | `entry.received_at` is not less than the previous usable line's. | `S_RECEIVED_REGRESSION` |
| g | `entry.epoch` is not less than the previous usable line's. | `S_EPOCH_REGRESSION` |

### Body checks

These stop at the first finding for the line.

**h. Reference.** `entry.ref` must equal the object's declared id, or the attestation's hash. Otherwise `S_REF_MISMATCH`. A ref that an earlier line already carried gives `S_DUPLICATE_REF`.

**i. Object.** evidence/0 steps 1-5 failing gives `S_OBJECT_INVALID`, with `core` set to the `E_` code. Step 6 runs against the objects at a lower seq, with three outcomes for a `prev`:

- held at a lower seq but dated after this object's `at`: `S_OBJECT_INVALID`, core `E_PREV_ORDER`;
- found only at a higher seq: `S_PREV_POSITION`;
- absent from the whole log: `S_OBJECT_INVALID`, core `E_PREV_MISSING`. This is decided at the end of the log.

**j. Store subjects.**

- Seq 1 must be a valid genesis whose id is the log id, and seq 2 must be its author attestation (see [Genesis](#genesis)). Otherwise `S_GENESIS_INVALID`.
- A **checkpoint** here is an object with a store subject whose `subject.type` is `"checkpoint"` and whose `actor.id` is the genesis signer. Any other object with a store subject, apart from the genesis at seq 1, gives `S_RESERVED_SUBJECT`.
- A checkpoint gives `S_CHECKPOINT_INVALID` unless its content is valid checkpoint/0 with `log` equal to the log id, and the object is exactly the one built from that content, the genesis signer, its `at` and its `prev` (see [Checkpoint and tree](#checkpoint-and-tree)). Its own `prev` is used for that, so `prev` must be `[]` or hold one id, but which id it holds is not checked here.
- That a checkpoint's `prev` names the previous checkpoint (`[]` for the first) is a writer rule, and verification does not check it. A `prev` that names some other object is subject only to step i: it must be held at a lower seq and not dated after the checkpoint. Every checkpoint is checked on its own against the lines, which is what an auditor relies on; the link adds nothing to that.
- A checkpoint whose `size` is not below its own seq, or whose `head` or `root` disagrees with the lines, gives `S_CHECKPOINT_MISMATCH`.

**k. Attestation.**

- A line that is not valid attestation/0 gives `S_ATTESTATION_INVALID`, core `E_ATTESTATION_SHAPE`.
- A signature that does not verify gives `S_ATTESTATION_INVALID`, core `E_SIGNATURE_INVALID`.
- A target object that is not held at a lower seq gives `S_ATTESTATION_TARGET`.

### End of log

- A `prev` absent from the whole log gives `S_OBJECT_INVALID`, core `E_PREV_MISSING`, at the seq of the object that lists it.
- A checkpoint with no valid `witness` attestation by the genesis key gives `S_CHECKPOINT_UNSIGNED`, at the checkpoint's seq.
- A log of fewer than two lines gives `S_GENESIS_INVALID`.

### Export only

These findings concern the manifest. They have no seq, so they sort before every line finding.

- A `manifest.json` that is not strict JSON, does not match segments/0, or whose segment list is not the one its `size` and `segment_size` give, gives `S_MANIFEST_INVALID`.
- A segment file whose SHA-256 differs from its manifest `hash` gives `S_SEGMENT_HASH`, with `file` set to its name.
- A manifest whose `size`, `head`, `log` or `checkpoint` disagrees with the lines gives `S_MANIFEST_MISMATCH`.

### Live store only

These are checked after the lines, against the store's own tables.

- Projections (the `heads` and `checkpoints` tables) that differ from a recompute over the lines give `S_PROJECTION_DRIFT`.
- Missing append-only triggers give `S_TRIGGERS_MISSING`. The reference store's triggers refuse UPDATE and DELETE on its log tables, aborting with the message `S_APPEND_ONLY`.
- A stored log id (`meta.log_id`) that differs from line 1 gives `S_META_MISMATCH`.

### Collecting and ordering findings

- Findings sort by `seq`, with findings that have no seq first, then by the code's position in the [report code table](#report-codes). Findings that tie on both keep input order: the order of their lines, or for `S_SEGMENT_HASH` the order of the files in the manifest.
- A verifier collects findings up to a limit, `max_findings` (default 1000, at least 1). When there are more, the report keeps the first `max_findings` in sort order and sets `truncated: true`.
- The findings kept are the first in sort order, not the first found. A finding decided late can therefore displace one found earlier: `E_PREV_MISSING` and `S_CHECKPOINT_UNSIGNED` are decided at the end of the log but sort at the seq of the line they concern, ahead of findings on later lines. This keeps the report independent of the order in which an implementation happens to find things. A verifier can meet it by keeping every finding until the end, or only the `max_findings` smallest by (`seq`, code, input order).

## Report and codes

Verification produces one `store-report/0` object. The JSON Schema is [`schema/store-report-0.schema.json`](schema/store-report-0.schema.json).

```json
{"v":"store-report/0","ok":false,"source":"export","log":"sha256:…",
 "head":{"seq":12,"entry_hash":"sha256:…"},
 "covered":{"size":10,"checkpoint":"sha256:…"},
 "checked":{"entries":12,"objects":7,"attestations":4,"checkpoints":1},
 "counts":{"forks":0,"at_after_received":0},
 "findings":[{"code":"S_OBJECT_INVALID","seq":4,"ref":"sha256:…","core":"E_ID_MISMATCH"}],
 "truncated":false}
```

| Member | Meaning |
|---|---|
| `v` | `"store-report/0"` |
| `ok` | `true` exactly when `findings` is empty. |
| `source` | `"store"` for a live store, `"export"` for an export. |
| `log` | The log id. Omitted when unknown. |
| `head` | `seq` and `entry_hash` of the last entry. Omitted when unknown or empty. |
| `covered` | `size` and `checkpoint` (its id) of the highest checkpoint that verified and has a valid witness. Omitted when there is none. |
| `checked` | How many `entries`, `objects`, `attestations` and `checkpoints` were checked. |
| `counts.forks` | The number of record-lane subjects with more than one head. |
| `counts.at_after_received` | The number of objects whose `at` is more than 300 seconds after their entry's `received_at`. Informational; never a finding (see [Two clocks](#two-clocks)). |
| `findings` | The findings, sorted as above. |
| `truncated` | `true` when findings were dropped because of `max_findings`. |

A **finding** has `code`, always, plus these where known:

- `seq` and `ref`: the seq and ref of the entry the finding is about;
- `core`: the evidence/0 code behind `S_OBJECT_INVALID` or `S_ATTESTATION_INVALID`, and only those;
- `file`: the segment file behind `S_SEGMENT_HASH`, and only that.

A report holds no free text, so every implementation emits identical bytes for the same input. Implementations may give human-readable detail elsewhere, for example on standard error, never in the report.

### Report codes

In sort order:

| Code | Where | Meaning |
|---|---|---|
| `S_MANIFEST_INVALID` | export | `manifest.json` is not strict JSON, does not match segments/0, or lists the wrong segments. |
| `S_SEGMENT_HASH` | export | A segment file does not hash to its manifest `hash`. `file` names it. |
| `S_MANIFEST_MISMATCH` | export | The manifest's `size`, `head`, `log` or `checkpoint` disagrees with the lines. |
| `S_LINE_INVALID` | line | Not strict JSON, wrong members, not in canonical form, or a body member that `entry.kind` does not name. |
| `S_ENTRY_INVALID` | line | The entry is not valid `log-entry/0`. |
| `S_WRONG_LOG` | line | `entry.log` is not the log id. |
| `S_SEQ_GAP` | line | `entry.seq` is not the expected seq. |
| `S_CHAIN_BREAK` | line | `entry.prev` is not the previous entry hash (or the log id at seq 1). |
| `S_RECEIVED_REGRESSION` | line | `received_at` is earlier than the previous usable line's. |
| `S_EPOCH_REGRESSION` | line | `epoch` is lower than the previous usable line's. |
| `S_REF_MISMATCH` | line | `entry.ref` is not the object's id or the attestation's hash. |
| `S_DUPLICATE_REF` | line | The ref appeared on an earlier line. |
| `S_GENESIS_INVALID` | line, end | Seq 1 is not a valid genesis or seq 2 is not its author attestation, or the log has fewer than two lines. |
| `S_RESERVED_SUBJECT` | line | An object with a store subject that is neither the genesis nor a checkpoint by the genesis signer. Also an intake rejection. |
| `S_OBJECT_INVALID` | line, end | The object fails evidence/0 steps 1-5, or step 6 within the log. `core` gives the `E_` code. |
| `S_PREV_POSITION` | line | A `prev` is held only at a higher seq. |
| `S_ATTESTATION_INVALID` | line | The attestation's shape or signature is invalid. `core` gives the `E_` code. |
| `S_ATTESTATION_TARGET` | line | The attestation's object is not held at a lower seq. Also an intake rejection. |
| `S_CHECKPOINT_INVALID` | line | A checkpoint by the genesis signer is not valid checkpoint/0 for this log. |
| `S_CHECKPOINT_MISMATCH` | line | A checkpoint's `size` is not below its seq, or its `head` or `root` disagrees with the lines. |
| `S_CHECKPOINT_UNSIGNED` | end | A checkpoint has no valid witness by the genesis key. |
| `S_PROJECTION_DRIFT` | live | The stored heads or checkpoints differ from a recompute. |
| `S_TRIGGERS_MISSING` | live | The append-only triggers are missing. |
| `S_META_MISMATCH` | live | The stored log id differs from line 1. |

### Intake and operation codes

These are returned by writers and tools, and never appear in a report. `S_RESERVED_SUBJECT` and `S_ATTESTATION_TARGET` are used both at intake and in reports.

| Code | Meaning |
|---|---|
| `S_STALE_HEADS` | A compare-and-set append found heads different from `if_heads`. The current heads are returned. |
| `S_STORE_KEY` | The key offered to sign a checkpoint is not the genesis key. Nothing is written. |
| `S_LOCKED` | Another writer holds the store. |
| `S_NOT_A_STORE` | The directory is not a store. |
| `S_EXISTS` | The target already exists: a store directory that is not empty, or an export or import destination. |
| `S_FORMAT` | The store's format version is not one this implementation supports. |
| `S_IO` | An I/O or storage-engine error. |

## Two clocks

The store keeps two clocks apart ([D13](../docs/decisions.md)).

- **`at`** is the writer's claim. It stays inside the object, and so inside its id. The store never changes it.
- **`seq` and `received_at`** are the store's facts. They live in the entry, outside the object. `seq` is dense and `received_at` never decreases.
- Questions about what the store knew on a date use the store's clock (`received_at`). Questions about what was true at a time, according to the writers, use `at`.

**Received time.** `received_at` is the later of the writer's clock, truncated to milliseconds, and the previous entry's `received_at`. A clock that steps backwards therefore repeats the last `received_at` rather than decreasing it. Writers format it with exactly 3 fraction digits (`YYYY-MM-DDTHH:MM:SS.mmmZ`). Truncating drops the digits after the milliseconds; it never rounds up.

**Earlier times finer than a millisecond.** Verifiers accept up to 9 fraction digits, so an imported log can hold a `received_at`, or a store-authored `at`, such as `2026-11-26T00:00:02.123456789Z`. Formatting that value with 3 digits would give a time below it. So when a writer takes the later of its clock and an earlier time (the previous `received_at` here, or a `prev` object's `at` under service time below), it first rounds that earlier time **up** to the next whole millisecond (`…02.124Z` in the example). A time already in whole milliseconds, which is everything a writer emits, is unchanged. The clock reading itself is still only truncated.

**Service time.** Objects that the store or its services author in a chain take `at` = the later of their clock, truncated to milliseconds, and the `at` of each of their `prev` objects, each rounded up to a whole millisecond as above. A clock step backwards therefore cannot break evidence/0's prev ordering rule (`E_PREV_ORDER`). The genesis has no `prev`, so its `at` is the clock truncated to milliseconds. A checkpoint's `prev` is the previous checkpoint.

**One reading per transaction.** A writer reads its clock once for each transaction (creating the store, a batch of items, or a checkpoint). That one reading gives `received_at` for every entry in the transaction and the `at` of any store-authored object in it.

**Late claims.** An object whose `at` is more than 300 seconds after its entry's `received_at` is counted in reports (`counts.at_after_received`). It is never rejected or rewritten.

## Privacy

- **Rejected input is not stored anywhere.** There is no quarantine of rejected bytes, because a rejected object can hold personal data.
- **An append-only log cannot forget.** Whatever is appended stays in every later export and copy. evidence/0's privacy rules apply in full: do not inline personal information, and remember that a bare hash of a guessable value does not hide it.
- **Synthetic or public data only, for now.** Until the pilot gate (M5 in the [roadmap](../docs/roadmap.md)), which adds trust, time, replication and privacy operations, only synthetic or public data goes into a store.
- The store's own objects, the genesis and checkpoints, hold no personal data.

## Conformance

The vectors in [`../test-vectors/store/`](../test-vectors/store/) are the contract ([D7](../docs/decisions.md)). They are generated by an independent JavaScript implementation, `tools/reference/log.mjs`, and checked by every other implementation in this repository.

| File | What a conforming implementation does |
|---|---|
| `entries.json` | For each `valid` case, produces `canonical` from `entry` and the entry hash `hash`. For each `invalid` case, rejects `input` with `S_ENTRY_INVALID`. |
| `merkle.json` | For each tree of `leaves` (leaf bytes are the UTF-8 of each string), produces `root`; produces each proof marked `valid: true`; and verifies every proof to its `valid` value. |
| `exports/<case>/` | Verifies the export to a report whose canonical form equals the canonical form of `case.json`'s `expect`, byte for byte. |
| `exports/<case>/inputs.json` | Valid cases only. A writer that replays the operations with the scripted clock and the test store key (`test-vectors/keys/store-demo.seed`) exports a manifest and segment files byte-identical to the committed ones. |

An implementation of the verifier conforms if it gives the expected result for every case. A writer conforms if, in addition, its exports for the valid cases match byte for byte. Another storage backend conforms if its exports verify and match.

The JSON Schemas for this spec are [`log-entry-0`](schema/log-entry-0.schema.json), [`store-genesis-0`](schema/store-genesis-0.schema.json), [`checkpoint-0`](schema/checkpoint-0.schema.json), [`segments-0`](schema/segments-0.schema.json) and [`store-report-0`](schema/store-report-0.schema.json). As with evidence/0, implementations check these rules in code, and `tools/check_schema_vectors.py` runs the published schemas over the same vectors so the two cannot drift ([D9](../docs/decisions.md)).
