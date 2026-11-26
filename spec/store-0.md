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
- **Log id:** the id of the log's genesis object (seq 1). It names the log everywhere else. A verifier reads it from the lines, as `entry.log` of the first readable line (see [Verification](#verification)).
- **Held:** at intake, an object or attestation is held when the log has an entry for it. In verification, an object is held from the first line where it passes the reference check and evidence/0 steps 1-5 (step i of [Body checks](#body-checks)).
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

**Writer epoch.** A store's epoch starts at 1 when it is created, and increases by one each time a writer opens it. Entries written by one writer session carry that session's epoch. A store rebuilt from an export continues from the highest epoch among its entries: the first writer to open it has that epoch + 1.

## Lanes and heads

A **subject key** is the triple (`subject.system`, `subject.type`, `subject.key`).

**Lane.** Every object belongs to one lane of its subject:

- `"record"` if `op` is `create`, `update` or `delete`;
- `"record"` if `op` is `assert` and `ext` has a member named `"capture/0"`. This is reserved for baseline observations by capture, so that heads stay valid when capture is added;
- otherwise `"assert:"` + `schema.ref` up to, not including, the first `@` (the whole ref if it has no `@`).

For example, an assert whose `schema.ref` is `photo-check/result@1` is in lane `assert:photo-check/result`. The genesis is in lane `assert:evidence-store/genesis`, and checkpoints are in lane `assert:evidence-store/checkpoint`.

**Heads.** `heads(S, L)` is the set of objects with subject key S and lane L that no object with subject key S and lane L lists in its `prev`. A `prev` that points at an object with a different subject key or lane does not change heads.

Every `prev` of an accepted object is held at a lower seq, so heads can be kept by a fold in seq order: for each object, remove its `prev` ids from `heads(S, L)` and add its own id.

The heads of a log are that fold over every object the log holds, the genesis and checkpoints included, in seq order. A verifier folds, in reading order, every object that is held in the sense of [verification](#body-checks): an object whose `prev` check or store-subject check later gives a finding is still folded, and an object that fails evidence/0 steps 1-5 is not. In a log with no findings this is exactly the writer's fold.

**Forks.** A fork is a subject whose record lane has more than one head. A fork is not an error: a later object can merge it by listing every head in its `prev`. Reports count forks (`counts.forks`). store/0 defines no status words (current, forked, deleted, absent); those are views computed over heads, outside this spec.

## Intake

Intake is what a writer checks, in order, before it appends an item. A verifier never relies on intake having run: it checks the log on its own terms.

**Object:**

1. evidence/0 steps 1-5. On failure, reject with that `E_` code.
2. If `subject.system` is `"evidence-store"`, reject with `S_RESERVED_SUBJECT`. Only the store itself writes store subjects: the genesis when the store is created, and checkpoints.
3. If the id is already held, return the existing entry's receipt with `duplicate: true`. `if_heads` is not evaluated, so a compare-and-set append that is retried after it landed still succeeds.
4. evidence/0 step 6 against the held objects: every `prev` must be held and none may be dated after `at`. Failure gives `E_PREV_MISSING` or `E_PREV_ORDER`.
5. If `if_heads` is given, the set `heads(subject, lane)` must equal it exactly, compared as sets (order and repeated ids do not matter). An empty `if_heads` means the subject and lane must have no heads. Otherwise reject with `S_STALE_HEADS`, and return the current heads.
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
- The items of a batch are checked in order, each against the log as it would be with the batch's earlier accepted items appended. A `prev` or an attestation's target may be an object earlier in the batch, `if_heads` is compared with the heads after the earlier items, and an item that repeats an earlier item of the batch returns that item's receipt with `duplicate: true`. The new entries take consecutive seqs in batch order and, because the writer reads its clock once per transaction (see [Two clocks](#two-clocks)), all have the same `received_at`.
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
- **Wrong key.** If the key offered to sign a checkpoint differs from the genesis key, or the witness it produces does not verify under the genesis key, the request is refused with `S_STORE_KEY` before anything is written.
- **When.** A writer may write a checkpoint at any time after the store is created, directly after another checkpoint included: `size` then covers the previous checkpoint and its witness.

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

How a verifier splits a file into lines, including a missing final LF, an empty line and CR LF, is under [Export only](#export-only).

## Verification

One algorithm verifies a live store and an export. It reads the lines in **reading order**: for a live store, its lines in seq order; for an export, the lines of each segment file (see [Export only](#export-only)), taking the files in manifest order. It processes each line in turn. Object verification under evidence/0 stops at the first failure; store verification does not. It collects every finding, because an auditor needs to know how much is damaged. Nothing is repaired.

An **earlier line** or a **later line** is one before or after the current line in reading order. In a log with no `S_SEQ_GAP` finding, reading order is seq order, so an earlier line is one with a lower seq.

A line that fails a line check is **unusable**: processing of that line stops there. An unusable line is not counted in `checked`, is never the report's `head`, is not a leaf of the tree that checkpoints are checked against, and is skipped when steps f and g look for the previous usable line. Every other line is **usable**.

The **log id** of the lines is `entry.log` of the first line that passes steps a, b and b2. That is normally line 1, the genesis entry; if line 1 is unusable, it is the first line after it that passes them. When no line passes them, the lines have no log id and the report omits `log`.

### Line checks

| Step | Check | Code |
|---|---|---|
| a | The line is strict JSON nesting at most 129 deep (see [Terms](#terms)), has exactly two members, `entry` and one of `object` or `attestation`, and equals its canonical form byte for byte. | `S_LINE_INVALID` |
| b | `entry` is valid `log-entry/0`. | `S_ENTRY_INVALID` |
| b2 | The line's other member is the one `entry.kind` names: `object` for `"object"`, `attestation` for `"attestation"`. | `S_LINE_INVALID` |
| c | `entry.log` equals the log id. | `S_WRONG_LOG` |

### Sequence and clock checks

Each of these can add a finding, and processing of the line continues.

| Step | Check | Code |
|---|---|---|
| d | `entry.seq` equals the expected seq, which starts at 1. Either way, the expected seq becomes `entry.seq` + 1. Unusable lines do not change it. | `S_SEQ_GAP` |
| e | If `entry.seq` is 1, `entry.prev` equals the log id. Otherwise it equals the entry hash of the previous line; this is skipped when there is no previous line (the line is the first one read) or the previous line is unusable. | `S_CHAIN_BREAK` |
| f | `entry.received_at` is not less than the previous usable line's. Skipped for the first usable line. | `S_RECEIVED_REGRESSION` |
| g | `entry.epoch` is not less than the previous usable line's. Skipped for the first usable line. | `S_EPOCH_REGRESSION` |

### Body checks

Body checks stop at the first finding, so a usable line gets at most one body finding. Step h applies to every usable line, steps i and j to object lines, and step k to attestation lines.

**h. Reference.** For an object line, `entry.ref` must equal the object's `id` member; an object without a string `id` does not match. For an attestation line, it must equal the attestation's hash (see [Log entry](#log-entry)). Otherwise `S_REF_MISMATCH`. A ref that matches but that an earlier usable line carried as its `entry.ref`, whatever that line's findings, gives `S_DUPLICATE_REF`.

**i. Object.** evidence/0 steps 1-5 failing gives `S_OBJECT_INVALID`, with `core` set to the `E_` code.

An object that passes steps 1-5 is **held** from this line on: its id and `at` enter the id map, and the object enters the heads fold (see [Lanes and heads](#lanes-and-heads)). That does not depend on what step 6 or step j finds for it. An object that fails step h or steps 1-5 is never held, so a `prev` or an attestation that names it finds nothing.

Step 6 takes the object's `prev` ids in list order and stops at the first one that is not held on an earlier line, or that is held there but dated after this object's `at`:

- held on an earlier line but dated after: `S_OBJECT_INVALID`, core `E_PREV_ORDER`;
- not held on an earlier line: the line's finding is decided at the end of the log. It is `S_PREV_POSITION` if that id is held by then (so on a later line), and otherwise `S_OBJECT_INVALID`, core `E_PREV_MISSING`.

The `prev` ids after the one that stops step 6 are not examined, as in evidence/0's own step 6. So an object has at most one `prev` finding, which `prev` it concerns is known on its line, and the end of the log decides only that finding's code. Either outcome is a finding, so step j is skipped.

**j. Store subjects.** In this order:

1. An object at seq 1 must be a valid genesis whose id is the log id (see [Genesis](#genesis)). Otherwise `S_GENESIS_INVALID`. The first one that is valid is the log's **genesis**: its signer and key are the ones every later line is checked against. An object at seq 2 gives `S_GENESIS_INVALID`, because seq 2 is the author attestation.
2. Any other object with a store subject is a **checkpoint** when the genesis was found on an earlier line, its `subject.type` is `"checkpoint"`, and its `actor.id` is the genesis signer. Any other object with a store subject gives `S_RESERVED_SUBJECT`: a second genesis, a checkpoint by another actor, and every store-subject object read before the genesis was found.
3. A checkpoint gives `S_CHECKPOINT_INVALID` unless its content is valid checkpoint/0, the object is exactly the one built from that content, the genesis signer, its `at` and its `prev` (see [Checkpoint and tree](#checkpoint-and-tree)), so `prev` is `[]` or holds one id, and the content's `log` is the log id.
4. A checkpoint gives `S_CHECKPOINT_MISMATCH` unless its `size` is below its own seq, at least `size` usable lines come before it, the entry hash of the `size`-th of them is its `head`, and the tree hash over the canonical entries of the first `size` of them is its `root`.

The tree in item 4 has one leaf per usable line, in reading order. In a log with no unusable line and no `S_SEQ_GAP`, leaf n is entry n, which is exactly the tree of [Merkle tree](#merkle-tree), so a checkpoint is checked against what it signed. Lines that are missing, unusable or out of order before a checkpoint's `size` make it `S_CHECKPOINT_MISMATCH`.

That a checkpoint's `prev` names the previous checkpoint (`[]` for the first) is a writer rule, and verification does not check it. A `prev` that names some other object is subject only to step 6: it must be held on an earlier line and not dated after the checkpoint. Every checkpoint is checked on its own against the lines, which is what an auditor relies on; the link adds nothing to that.

Rust's `Checkpoint::from_object` covers item 3 except the `log` comparison, and takes the signer from the object's own actor. The verifier therefore checks the actor in item 2 (`S_RESERVED_SUBJECT`) and `log` in item 3 (`S_CHECKPOINT_INVALID`), which is what the JavaScript reference's `checkpointFromObject` does in one call.

**k. Attestation.** In this order:

1. A line that is not valid attestation/0 gives `S_ATTESTATION_INVALID`, core `E_ATTESTATION_SHAPE`.
2. A signature that does not verify gives `S_ATTESTATION_INVALID`, core `E_SIGNATURE_INVALID`. It is checked strictly, as evidence/0 requires: S is below the group order L; R, the first 32 bytes, equals the encoding of [S]B − [k]A, with k the SHA-512 of R, the key A and the message, reduced mod L; and neither A nor R is a point of small order (1, 2, 4 or 8), however it is encoded, including encodings whose y is not below 2^255 − 19. A check without the small-order rule, such as Node's, accepts signatures that verify for any message (cases 130 and 131).
3. A target object (`object`) that is not held on an earlier line gives `S_ATTESTATION_TARGET`.
4. An attestation at seq 2 gives `S_GENESIS_INVALID` unless the genesis was found on an earlier line and this is its author attestation: `object` is the log id, `purpose` is `author`, `signer` and `key` are the genesis signer and key, and `at` equals the genesis `at` as an instant. An attestation at seq 1 gives `S_GENESIS_INVALID`, because seq 1 is the genesis.

The genesis rule comes after the general ones, so an author attestation whose signature does not verify gives `S_ATTESTATION_INVALID`, core `E_SIGNATURE_INVALID`, as any other attestation would, and not `S_GENESIS_INVALID`.

A **valid witness** of a checkpoint is a usable attestation line with no body finding whose `purpose` is `witness`, whose `object` is the checkpoint's id, and whose `key` is the genesis key.

### End of log

- Each step 6 finding left open is decided (`S_PREV_POSITION` or `S_OBJECT_INVALID` with core `E_PREV_MISSING`), at the seq of the object that lists the `prev`.
- A checkpoint (item 2 of step j, whatever items 3 and 4 found) with no valid witness gives `S_CHECKPOINT_UNSIGNED`, at the checkpoint's seq.
- If no usable line has seq 1, or no usable line has seq 2, the log gives `S_GENESIS_INVALID` once, with no seq. This covers a log of fewer than two lines. A usable line at seq 1 or 2 that is not the genesis or its author attestation is reported at its own seq instead, by its own body checks.

### Export only

**Reading the segments.** A verifier is given the bytes of `manifest.json` and of the segment files, in the order the manifest lists them. A host passes one byte string for each file the manifest lists, using empty bytes for a file it cannot read, so that file gets `S_SEGMENT_HASH`. A host that cannot read a list from `manifest.json` passes the export's segment files in file-name order.

Each segment file is split into lines at every LF byte (0x0A), before any decoding:

- A line is the bytes before an LF, without the LF. The lines of all the files form one sequence: the previous line of a file's first line is the last line of the file before it.
- If a file does not end in LF, the bytes after its last LF form one more line, which is unusable with `S_LINE_INVALID` whatever its bytes are. Every line of a segment file ends in LF.
- An empty line (an LF at the start of a file or right after another LF) is a line of zero bytes, and so `S_LINE_INVALID`. Empty lines are not skipped.
- CR has no special meaning. A line that ends in CR LF keeps its CR, so it is not canonical: `S_LINE_INVALID`.
- An empty file has no lines.

**Manifest checks.** These findings concern the manifest. They have no seq, so they sort before every line finding.

1. `manifest.json` must be, byte for byte, the canonical form of a strict JSON value that matches segments/0 and whose segment list is the one its `size` and `segment_size` give (see [Manifest](#manifest-segments0)). Otherwise `S_MANIFEST_INVALID`, and the manifest is not used further: there is no `S_SEGMENT_HASH` or `S_MANIFEST_MISMATCH`.
2. Each listed file whose bytes were given and do not hash to its `hash` gives `S_SEGMENT_HASH`, with `file` set to its name.
3. After the lines, the manifest gives `S_MANIFEST_MISMATCH`, once, when any of these holds:
   - the number of segment files given is not the number the manifest lists;
   - `log` is not the log id of the lines, or the lines have none;
   - `size` and `head` are not the `seq` and `entry_hash` of the report's `head`, or the report has no `head`;
   - `checkpoint` does not hold the id and `size` of the latest checkpoint in the lines, or is present when the lines have none, or is absent when they have one. The **latest checkpoint** is the last line, in reading order, that is a checkpoint and has no `S_CHECKPOINT_INVALID` (item 3 of step j); an `S_CHECKPOINT_MISMATCH` does not exclude it.

The lines are verified whatever the manifest checks find, after `S_MANIFEST_INVALID` and `S_SEGMENT_HASH` included. A segment's `first` and `last` are checked only in item 1, against `size` and `segment_size`; they are not compared with the seqs of the lines in the file. A line that sits in a different file than `first` and `last` say is therefore not a finding in itself: what the lines hold is checked by steps a to k, and their end by `size` and `head`.

### Live store only

These are checked after the lines, against the store's own tables. Each gives at most one finding, with no seq.

- The projections give `S_PROJECTION_DRIFT` when the `heads` table differs in any row from the heads fold over the lines (see [Lanes and heads](#lanes-and-heads)), or the `checkpoints` table differs in any row from the lines that are checkpoints with no `S_CHECKPOINT_INVALID` (their seq, id, `size`, `root` and `head`). One recompute gives one finding, however many rows differ and in which table.
- Missing append-only triggers give `S_TRIGGERS_MISSING`. The reference store's triggers refuse UPDATE and DELETE on its log tables, aborting with the message `S_APPEND_ONLY`.
- A stored log id (`meta.log_id`) that differs from the log id of the lines, or lines that have none, give `S_META_MISMATCH`.

### Collecting and ordering findings

- Findings sort by `seq`, with findings that have no seq first, then by the code's position in the [report code table](#report-codes). Findings that tie on both keep the order of the lines they are about, a finding decided at the end of the log included; `S_SEGMENT_HASH` findings keep the order of the files in the manifest. A line has at most one finding of each code, so this order is total.
- A verifier collects findings up to a limit, `max_findings` (default 1000, at least 1). When there are more, the report keeps the first `max_findings` in sort order and sets `truncated: true`.
- The findings kept are the first in sort order, not the first found. A finding decided late can therefore displace one found earlier: `E_PREV_MISSING`, `S_PREV_POSITION` and `S_CHECKPOINT_UNSIGNED` are decided at the end of the log but sort at the seq of the line they concern, ahead of findings on later lines. This keeps the report independent of the order in which an implementation happens to find things. A verifier can meet it by keeping every finding until the end, or only the `max_findings` smallest by (`seq`, code, line order).

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
| `log` | The log id of the lines: `entry.log` of the first line that passes steps a, b and b2 (see [Verification](#verification)). Omitted when no line does. |
| `head` | `seq` and `entry_hash` of the last usable line in reading order: its `entry.seq` and its entry hash. Omitted when no line is usable. If the last line is unusable, `head` is the last usable line before it; an export whose manifest names the unusable line then also gives `S_MANIFEST_MISMATCH`. `log` and `head` are present together. |
| `covered` | `size` and `checkpoint` (its id) of the checkpoint with the largest `size` among those with no body finding and a valid witness; of several with that size, the first in reading order. Omitted when there is none. |
| `checked` | Counts of usable lines, each counted once. `entries`: every usable line. `attestations`: the usable attestation lines. `checkpoints`: the object lines that are checkpoints under item 2 of step j, whatever items 3 and 4 found. `objects`: every other usable object line, the genesis included. So `entries` = `objects` + `attestations` + `checkpoints`. Lines with findings are counted; unusable lines are not. |
| `counts.forks` | The number of record-lane subjects with more than one head, in the heads fold over the held objects (see [Lanes and heads](#lanes-and-heads)). |
| `counts.at_after_received` | The number of held objects whose `at` is more than 300 seconds after their entry's `received_at`, compared as instants (exactly 300 seconds is not counted). Informational; never a finding (see [Two clocks](#two-clocks)). |
| `findings` | The findings, sorted as above. |
| `truncated` | `true` when findings were dropped because of `max_findings`. |

In the example, the 12 usable lines are 7 objects (the genesis among them), 4 attestations (the genesis author attestation and a witness among them) and 1 checkpoint.

A **finding** has `code`, always, plus:

- `seq` and `ref`: the `entry.seq` and `entry.ref` of the line the finding is about. Every finding about a line has both, those decided at the end of the log included, except the findings of steps a, b and b2 (`S_LINE_INVALID` and `S_ENTRY_INVALID`), which have neither. `S_WRONG_LOG` has both: its entry is valid, only its log is wrong. The manifest findings, the live-store findings and the end-of-log `S_GENESIS_INVALID` have neither. So `S_GENESIS_INVALID` has seq 1 or 2, or no seq;
- `core`: the evidence/0 code behind `S_OBJECT_INVALID` or `S_ATTESTATION_INVALID`, and only those;
- `file`: the segment file behind `S_SEGMENT_HASH`, and only that.

A report holds no free text, so every implementation emits identical bytes for the same input. Implementations may give human-readable detail elsewhere, for example on standard error, never in the report.

### Report codes

In sort order:

| Code | Where | Meaning |
|---|---|---|
| `S_MANIFEST_INVALID` | export | `manifest.json` is not the canonical form of strict JSON, does not match segments/0, or lists the wrong segments. |
| `S_SEGMENT_HASH` | export | A segment file does not hash to its manifest `hash`. `file` names it. |
| `S_MANIFEST_MISMATCH` | export | The manifest's `size`, `head`, `log` or `checkpoint` disagrees with the lines, or it lists a different number of segment files than were given. At most once. |
| `S_LINE_INVALID` | line | Not strict JSON, wrong members, not in canonical form, or a body member that `entry.kind` does not name. |
| `S_ENTRY_INVALID` | line | The entry is not valid `log-entry/0`. |
| `S_WRONG_LOG` | line | `entry.log` is not the log id. |
| `S_SEQ_GAP` | line | `entry.seq` is not the expected seq. |
| `S_CHAIN_BREAK` | line | `entry.prev` is not the previous entry hash (or the log id at seq 1). |
| `S_RECEIVED_REGRESSION` | line | `received_at` is earlier than the previous usable line's. |
| `S_EPOCH_REGRESSION` | line | `epoch` is lower than the previous usable line's. |
| `S_REF_MISMATCH` | line | `entry.ref` is not the object's id or the attestation's hash. |
| `S_DUPLICATE_REF` | line | The ref appeared on an earlier usable line. |
| `S_GENESIS_INVALID` | line, end | Seq 1 is not a valid genesis or seq 2 is not its author attestation, or no usable line has seq 1 or none has seq 2 (no seq). |
| `S_RESERVED_SUBJECT` | line | An object with a store subject that is neither the genesis nor a checkpoint by the genesis signer. Also an intake rejection. |
| `S_OBJECT_INVALID` | line, end | The object fails evidence/0 steps 1-5, or step 6 within the log. `core` gives the `E_` code. |
| `S_PREV_POSITION` | line, end | A `prev` is held only on a later line. |
| `S_ATTESTATION_INVALID` | line | The attestation's shape or signature is invalid. `core` gives the `E_` code. |
| `S_ATTESTATION_TARGET` | line | The attestation's object is not held on an earlier line. Also an intake rejection. |
| `S_CHECKPOINT_INVALID` | line | A checkpoint by the genesis signer is not valid checkpoint/0 for this log. |
| `S_CHECKPOINT_MISMATCH` | line | A checkpoint's `size` is not below its seq, or its `head` or `root` disagrees with the usable lines before it. |
| `S_CHECKPOINT_UNSIGNED` | end | A checkpoint has no valid witness by the genesis key. |
| `S_PROJECTION_DRIFT` | live | The stored heads or checkpoints differ from a recompute. At most once. |
| `S_TRIGGERS_MISSING` | live | The append-only triggers are missing. At most once. |
| `S_META_MISMATCH` | live | The stored log id differs from the log id of the lines. At most once. |

### Intake and operation codes

These are returned by writers and tools, and never appear in a report. `S_RESERVED_SUBJECT` and `S_ATTESTATION_TARGET` are used both at intake and in reports. An invalid argument to a store operation, such as a store name that does not match the name pattern, a segment size of 0 or a proof position outside the log, gives evidence/0's `E_USAGE`, as in the [programming interfaces](#programming-interfaces).

| Code | Meaning |
|---|---|
| `S_STALE_HEADS` | A compare-and-set append found heads different from `if_heads`. The current heads are returned. |
| `S_STORE_KEY` | The key offered to sign a checkpoint is not the genesis key, or what it signs does not verify under the genesis key. Nothing is written. |
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

## Programming interfaces

The WebAssembly module and the Python package expose the store's pure functions to other languages. Both wrap the same Rust code, so their results are byte-identical to the command-line tool's. They keep the conventions of the evidence/0 functions in those bindings:

- **Bytes.** A byte input is a string under its name (its UTF-8 bytes), or standard base64 under `<name>_base64` when the exact bytes matter. A list of byte inputs is an array of strings under `<name>`, or an array of standard base64 strings under `<name>_base64`. If both forms are given, `<name>` is used. Python takes `bytes` or `str` (encoded as UTF-8) wherever bytes are expected.
- **Hashes.** A leaf hash, a root and each proof element are text, `"sha256:"` followed by 64 lowercase hex digits, never base64. A proof is an array of them in the order RFC 9162 produces them, from the leaf upwards.
- **Integers.** An index or size is an integer from 0 to 2^53 − 1, and `max_findings` an integer from 1 to 2^53 − 1. In Python, an `int` that is not a `bool`. An integer outside its range is an invalid argument; the rule of [Proofs](#proofs) that a tree size above 2^53 − 1 verifies to `false` is for library functions that take wider integers, such as Rust's `u64`.
- **Errors.** Invalid arguments give `E_USAGE`: a missing or mistyped argument, a hash not written as above, an integer out of range. In WebAssembly that is the response `{"error":{"code":"E_USAGE","detail":…}}`; in Python, `EvidenceError` with `code` `E_USAGE`. Verification outcomes are never errors: proof checks answer `true` or `false` (see [Proofs](#proofs)), and `verify_export` returns a report however damaged the export is.

WebAssembly functions (`{"fn": <name>, "args": {…}}`):

| fn | args | result |
|---|---|---|
| `entry_hash` | `input`: log-entry/0 JSON text | The entry hash, `"sha256:…"` over the canonical form of the parsed entry. Input that is not strict JSON or not valid log-entry/0 gives `E_USAGE` whose `detail` begins with `S_ENTRY_INVALID`. |
| `merkle_root` | `leaves`: a list of leaf bytes | The tree hash, `"sha256:…"`. Each leaf is hashed as a leaf (0x00 prefix) first. An empty list gives the RFC 9162 hash of the empty tree, SHA-256 of no bytes. |
| `verify_inclusion` | `leaf_hash`, `index` (0-based: seq − 1), `size`, `proof`, `root` | `true` or `false` |
| `verify_consistency` | `old_size`, `new_size`, `old_root`, `new_root`, `proof` | `true` or `false` |
| `verify_export` | `manifest`: the bytes of `manifest.json`; `segments`: a list of segment file bytes in manifest order (see [Export only](#export-only)); `max_findings`: optional, default 1000 | The `store-report/0` object, with `source` `"export"`. |

`verify_inclusion` takes the leaf hash, not the leaf: that is what `store prove` prints (`leaf_hash`), and a caller that has the entry computes it as SHA-256(0x00 ‖ canonical entry). `merkle_root` takes leaf bytes, as `merkle.json` gives them.

The JavaScript wrapper names them `entryHash(input)`, `merkleRoot(leaves)`, `verifyInclusion(leafHash, index, size, proof, root)`, `verifyConsistency(oldSize, newSize, oldRoot, newRoot, proof)` and `verifyExport(manifest, segments, { maxFindings })`.

Python functions (`evidence_layer`), with the arguments and results of the table above, positional or by keyword:

```python
entry_hash(data) -> str                       # EvidenceError E_USAGE, detail "S_ENTRY_INVALID…"
merkle_root(leaves) -> str
verify_inclusion(leaf_hash, index, size, proof, root) -> bool
verify_consistency(old_size, new_size, old_root, new_root, proof) -> bool
verify_export(manifest, segments, max_findings=1000) -> dict   # the report, parsed
```

## Conformance

The vectors in [`../test-vectors/store/`](../test-vectors/store/) are the contract ([D7](../docs/decisions.md)). They are generated by an independent JavaScript implementation, `tools/reference/log.mjs`, and checked by every other implementation in this repository: Rust, the JavaScript reference, the WebAssembly module and the Python package.

| File | What a conforming implementation does |
|---|---|
| `entries.json` | For each `valid` case, produces `canonical` from `entry` and the entry hash `hash`. For each `invalid` case, rejects `input` with `S_ENTRY_INVALID` (through the bindings, `E_USAGE` whose `detail` begins with `S_ENTRY_INVALID`). |
| `merkle.json` | For each tree of `leaves` (leaf bytes are the UTF-8 of each string), produces `root`; produces each proof marked `valid: true`; and verifies every proof to its `valid` value. |
| `exports/<case>/case.json` | Verifies the export in that directory (`manifest.json` and the segment files, in manifest order), with `max_findings` when `case.json` has it, to a report whose canonical form equals the canonical form of `expect`, byte for byte. For `proofs`, see below. |
| `exports/<case>/inputs.json` | Valid cases only. A writer that replays the operations with the scripted clock and the test store key (`test-vectors/keys/store-demo.seed`) exports a manifest and segment files byte-identical to the committed ones. |

An implementation of the verifier conforms if it gives the expected result for every case. A writer conforms if, in addition, its exports for the valid cases match byte for byte. Another storage backend conforms if its exports verify and match.

### Export case files

Each case is a directory `exports/<NNN>-<name>/` holding `case.json`, the export (`manifest.json` and `000001.jsonl`, …), and for valid cases `inputs.json`. The two JSON files are written like the other vector files, with two-space indentation and a final LF; implementations compare canonical forms, so the layout only keeps the generator's output deterministic.

A test passes the bytes of `manifest.json` and of the directory's `NNNNNN.jsonl` files in file-name order, which is the manifest's order in every case, including those whose manifest is invalid. Every file is read as raw bytes.

**`case.json`** has exactly these members:

| Member | |
|---|---|
| `description` | Required. One sentence saying what the case holds. |
| `expect` | Required. The golden `store-report/0`. |
| `max_findings` | Only in a case that tests truncation: verify with this limit. Absent means the default, 1000. |
| `proofs` | Only in `003-attestations-two-checkpoints`, as below. |

```json
"proofs": {
  "inclusion": [{"seq": 1, "size": 11, "leaf_hash": "sha256:…", "proof": ["sha256:…"], "root": "sha256:…"}],
  "consistency": [{"old_size": 5, "new_size": 11, "old_root": "sha256:…", "new_root": "sha256:…", "proof": ["sha256:…"]}]
}
```

- `inclusion` has one member for each seq from 1 to the `size` of the export's latest checkpoint, in seq order, against that checkpoint's `size` and `root`. `leaf_hash` is the leaf hash of that seq's entry.
- `consistency` has one member, from the first checkpoint's `size` and `root` to the second's.
- Every proof is valid. Each implementation verifies each one to `true`, calling `verify_inclusion` with index `seq` − 1. An implementation that produces proofs (the Rust crate and the JavaScript reference) also produces each one from the export's lines and compares it byte for byte.

**`inputs.json`** has exactly these members, all required:

```json
{"name":"cer-demo","seed":"store-demo","segment_size":10000,
 "clock":["2026-11-26T00:00:00.000Z","2026-11-26T00:00:01.000Z","2026-11-26T00:00:02.000Z",
          "2026-11-26T00:00:03.000Z","2026-11-26T00:00:04.000Z"],
 "ops":[{"op":"init"},
        {"op":"append","file":"objects/001-create-inline/object.json"},
        {"op":"attest","file":"objects/003-agent-assertion-approved/attestations.jsonl","line":3},
        {"op":"append","object":{"…":"an inline synthetic object"}},
        {"op":"reopen"},{"op":"checkpoint"}]}
```

| Member | |
|---|---|
| `name` | The store's name, for the genesis. |
| `seed` | The test key: the seed is `test-vectors/keys/<seed>.seed`, and its public key is the `<seed>` member of `test-vectors/keys/public.json`. |
| `segment_size` | Lines per segment file for the export. |
| `clock` | The scripted clock: one timestamp per operation that reads the clock, consumed in order. It holds exactly as many as the operations need. |
| `ops` | The operations, in order. Each runs as one transaction and succeeds: nothing is rejected and nothing is a duplicate. |

| Operation | What it does | Reads the clock |
|---|---|---|
| `{"op":"init"}` | Creates the store at epoch 1: the genesis and its author attestation. Always the first operation, and only there. | yes |
| `{"op":"append","file":<path>}` or `{"op":"append","object":<object>}` | Appends one object: the bytes of the file (a path under `test-vectors/`), or the canonical form of `object`. | yes |
| `{"op":"attest","file":<path>,"line":<n>}` | Appends line n (counting from 1, without its LF) of that JSON Lines file. | yes |
| `{"op":"reopen"}` | Closes the writer and opens the store again, so the writer epoch increases by one. | no |
| `{"op":"checkpoint"}` | Writes a checkpoint and its witness, signed with the seed. | yes |

After the last operation the store is exported with `segment_size`.

Tampered cases have no `inputs.json`: they cannot be replayed through intake.

### Export cases

Valid cases (0xx) verify with `ok: true`.

| Case | What it holds |
|---|---|
| `001-genesis` | `init` only. |
| `002-records-and-checkpoint` | The genesis; object vectors 001 and 002; the author line for 001 (line 3 of `objects/003-agent-assertion-approved/attestations.jsonl`); a checkpoint. |
| `003-attestations-two-checkpoints` | Case 002, then object vector 003 with its author and approve lines, object vector 004, a `reopen`, and a second checkpoint. `case.json` has `proofs`. |
| `004-multi-segment` | The operations of 003 with `segment_size` 4. |
| `005-fork` | Two creates for one subject: `counts.forks` is 1. |
| `006-clock-backwards` | A clock that steps backwards: equal `received_at` values, and a second checkpoint whose `at` equals the first's. |
| `007-late-claim` | Three versions of one record, dated 300 s and 1 ns, exactly 300 s, and one day after their entries' `received_at`: `counts.at_after_received` is 2, because exactly 300 s is not counted. |

A tampered case (1xx) is either **edited**, the files of a valid case changed afterwards, with the manifest's segment hashes recomputed for the changed files and nothing else in it changed, unless the case says so; or **built**, written by the reference writer with one rule broken on purpose, so that its chain, tree and manifest are otherwise consistent. The generator asserts that each golden report holds the codes listed; where the list says "only", it asserts that the report holds exactly those findings.

| Case | How it is made | Report |
|---|---|---|
| `101-object-byte-flipped` | Edited 002: one byte in a string in object 002's content changed, keeping the line canonical. Nothing depends on object 002. | Only `S_OBJECT_INVALID` (`E_ID_MISMATCH`): leaves cover entry bytes, not object bytes. |
| `102-entry-removed` | Edited 004: the line of object 002 removed from its segment. | `S_SEQ_GAP`, `S_CHAIN_BREAK`, `S_CHECKPOINT_MISMATCH`, and no `S_MANIFEST_MISMATCH`: `first` and `last` are not compared with the lines. |
| `103-entries-swapped` | Edited 002: the lines of objects 001 and 002 swapped. | `S_SEQ_GAP`, `S_CHAIN_BREAK`, `S_PREV_POSITION`, and `S_CHECKPOINT_MISMATCH`: checkpoint leaves follow reading order. |
| `104-prev-after-child` | Built: an object written before the object its `prev` names. | `S_PREV_POSITION` |
| `105-checkpoint-root-wrong` | Built: a witnessed checkpoint with a wrong `root`. | `S_CHECKPOINT_MISMATCH` |
| `106-received-regression` | Built. | `S_RECEIVED_REGRESSION` |
| `107-epoch-regression` | Built. | `S_EPOCH_REGRESSION` |
| `108-attestation-signature-invalid` | Built: an attestation whose signature does not verify. | `S_ATTESTATION_INVALID` (`E_SIGNATURE_INVALID`) |
| `109-attestation-before-object` | Built. | `S_ATTESTATION_TARGET` |
| `110-checkpoint-unsigned` | Built: a checkpoint with no witness. | `S_CHECKPOINT_UNSIGNED` |
| `111-wrong-log` | Built: one entry whose `log` is another log's id. | `S_WRONG_LOG` |
| `112-line-not-canonical` | Edited. | `S_LINE_INVALID` |
| `113-duplicate-ref` | Built: an item appended a second time. | `S_DUPLICATE_REF` |
| `114-genesis-missing` | Edited 002: the genesis line removed. | `S_GENESIS_INVALID` with no seq, `S_SEQ_GAP` and `S_ATTESTATION_TARGET` at seq 2, `S_RESERVED_SUBJECT` at the checkpoint, `S_MANIFEST_MISMATCH`; no `S_CHAIN_BREAK`, because the first line read has no previous line. |
| `115-prev-dated-after` | Built. | `S_OBJECT_INVALID` (`E_PREV_ORDER`) |
| `116-segment-hash-mismatch` | Edited 002: object 002 changed as in 101, and the manifest left as it was. | Only `S_SEGMENT_HASH` and `S_OBJECT_INVALID` (`E_ID_MISMATCH`): lines are verified after `S_SEGMENT_HASH`. |
| `117-reserved-subject` | Built. | `S_RESERVED_SUBJECT` |
| `118-ref-mismatch` | Built. | `S_REF_MISMATCH` |
| `119-final-lf-missing` | Edited 002: the segment file's final LF removed, so its last line, the witness, is unusable. | Only `S_MANIFEST_MISMATCH`, `S_LINE_INVALID` (no seq) and `S_CHECKPOINT_UNSIGNED`. `head` is the checkpoint's line, `covered` is absent, and `checked` leaves out the last line. |
| `120-blank-line` | Edited 001: one more LF at the end of the segment file, an empty last line. | Only `S_LINE_INVALID`. `head` is seq 2, which the manifest names. |
| `121-crlf` | Edited 001: every line ends in CR LF. | Only `S_MANIFEST_MISMATCH`, `S_LINE_INVALID` twice and `S_GENESIS_INVALID` with no seq. No `log` or `head`; every `checked` count is 0. |
| `122-first-line-unusable` | Edited 002: a space inserted after the first `{` of the genesis line. | Only `S_MANIFEST_MISMATCH`, `S_LINE_INVALID`, `S_GENESIS_INVALID` with no seq, `S_SEQ_GAP` and `S_ATTESTATION_TARGET` at seq 2, and `S_RESERVED_SUBJECT` at the checkpoint. `log` is the log id of case 002, read from line 2. |
| `123-genesis-author-signature-invalid` | Built 001: the author attestation's signature does not verify. | Only `S_ATTESTATION_INVALID` (`E_SIGNATURE_INVALID`) at seq 2. |
| `124-genesis-author-wrong-at` | Built 001: the author attestation is signed by the store key but dated one millisecond after the genesis. | Only `S_GENESIS_INVALID` at seq 2. |
| `125-invalid-object-not-held` | Edited 002: object 001 changed as 101 changes object 002. | Only `S_OBJECT_INVALID` (`E_ID_MISMATCH`) at object 001, `S_OBJECT_INVALID` (`E_PREV_MISSING`) at object 002, and `S_ATTESTATION_TARGET` at the author line for 001: an object that fails steps 1-5 is not held. |
| `126-prev-list-order` | Built: the genesis, then the synthetic objects below. | Only `S_OBJECT_INVALID` (`E_PREV_MISSING`) at M, `S_OBJECT_INVALID` (`E_PREV_ORDER`) at N, and `S_PREV_POSITION` at P. `counts.forks` is 0. |
| `127-manifest-invalid` | Edited 002: an LF appended to `manifest.json`, and object 002 changed as in 101 with the segment hash left as it was. | Only `S_MANIFEST_INVALID` and `S_OBJECT_INVALID` (`E_ID_MISMATCH`): no `S_SEGMENT_HASH` after `S_MANIFEST_INVALID`, and the lines are still verified. |
| `128-checkpoint-recognition` | Built 002, then a checkpoint-shaped object whose actor is not the genesis signer, then a checkpoint by the genesis signer whose content `log` is not the log id, each followed by a witness by the store key. The manifest's `checkpoint` is still the checkpoint of 002. | Only `S_RESERVED_SUBJECT` and `S_CHECKPOINT_INVALID`. `checked.checkpoints` is 2, and `covered` is the checkpoint of 002. |
| `129-findings-truncated` | The export of 125, with `max_findings` 2. | Only the first two findings of 125, and `truncated: true`. The `E_PREV_MISSING` decided at the end of the log displaces the `S_ATTESTATION_TARGET` found before it. |
| `130-witness-nonce-zero` | Built 002, with the checkpoint's witness signed by the store key at nonce 0, so its R is the identity point. | Only `S_CHECKPOINT_UNSIGNED` at the checkpoint and `S_ATTESTATION_INVALID` (`E_SIGNATURE_INVALID`) at the witness. `covered` is absent. |
| `131-attestation-small-order-key` | Built: object vector 001, then two approve lines whose key is the identity point, written with y = 1 and with y = 2^255 − 18, each with R = B and S = 1. | Only `S_ATTESTATION_INVALID` (`E_SIGNATURE_INVALID`) at each of the two lines. |
| `132-entry-invalid` | Edited 005: the entry of the first create gets one more member, the line kept canonical. | Only `S_ENTRY_INVALID` with no seq and `S_SEQ_GAP` at the next line, seq 4: an unusable line does not move the expected seq. `head` is seq 4 and `checked.entries` is 3. |
| `133-genesis-not-exact` | Built 001: the genesis object has an `ext` member and is sealed again. Its content and everything else are as built, and its author line is signed by the store key over its id. | Only `S_GENESIS_INVALID` at seq 1 and at seq 2: the genesis must be exactly the object built from its own name, key and `at`. |
| `134-attestation-key-padded` | Built: object vector 001, then its author line with `=` padding added to the key. | Only `S_ATTESTATION_INVALID` (`E_ATTESTATION_SHAPE`). |
| `135-object-steps-2-3-5` | Built: object vector 004 with an unknown member, object vector 004 dated 30 February, and object vector 001 with a `content.hash` its inline content does not hash to, each sealed again. | Only `S_OBJECT_INVALID` with core `E_SHAPE`, `E_TIMESTAMP` and `E_CONTENT_HASH_MISMATCH`, in that order. |

Case 126 holds, after the genesis, these objects for one synthetic subject S, in this order. X, Y and Z are ids that no line holds, and T is a time before every `received_at` in the case.

1. A1: create S, `at` T, `prev` `[]`.
2. A2: create S with other content, `at` T, `prev` `[]`. S's record lane now has two heads.
3. M: update S, `at` T − 5 s, `prev` `[X, A1, A2]`. The first `prev` in list order, X, is not held, so M gives `E_PREV_MISSING`, not `E_PREV_ORDER`. M is held, so it merges A1 and A2.
4. N: update S, `at` T − 6 s, `prev` `[M, Z]`. M is held and dated after N: `E_PREV_ORDER`, at once; Z is not examined.
5. P: an assert about S in another lane, `at` T, `prev` `[L, Y]`. L is held only on a later line: `S_PREV_POSITION`; Y is not examined.
6. L: an assert about S in P's lane, `at` T, `prev` `[]`.

The JSON Schemas for this spec are [`log-entry-0`](schema/log-entry-0.schema.json), [`store-genesis-0`](schema/store-genesis-0.schema.json), [`checkpoint-0`](schema/checkpoint-0.schema.json), [`segments-0`](schema/segments-0.schema.json) and [`store-report-0`](schema/store-report-0.schema.json). As with evidence/0, implementations check these rules in code, and `tools/check_schema_vectors.py` runs the published schemas over the same vectors so the two cannot drift ([D9](../docs/decisions.md)).
