# Roadmap: from evidence layer to memory

Status: decisions confirmed by the project maintainer on 26 November 2026 (recorded as D11-D13 in [decisions.md](decisions.md); the rest apply as written [below](#decisions-confirmed-26-november-2026)) and on 27 November 2026 ([the edge track and approvals](#decisions-confirmed-27-november-2026)). The build specification for the first milestone is [m1-store-spec.md](m1-store-spec.md). This stays the only roadmap. Not an official publication of any agency; it describes no agency's process (see the [README](../README.md) notice).

## What's next, in short

Today the layer can create, sign and verify single evidence objects. To become an organisation's memory it needs four more things, in this order:

1. **A safe place to keep what it records.** A store that can only be added to, where anyone can check, offline or in a browser, that nothing was changed or removed since a signed fingerprint of its history was taken.
2. **Capture from an existing database, without changing it.** Read-only capture turns each change in a register into evidence, with no schema change in the register.
3. **Continuous comparison in both directions.** On a schedule, the store and the database are compared, and every difference (including edits made outside the normal process) is recorded as evidence. Nothing is repaired automatically.
4. **Memory reads.** Staff and applications can ask what a record said on any date, under which schema, who changed it and why, and get an answer they can verify.

Only after that does the layer accept requests to change records: first in shadow mode, where requests are recorded and compared with what staff actually did, and later delivered through the database's own interface, where a change counts as done only when it is read back and seen to have landed. Writing to any real system stays off until the system's owner agrees, one kind of record at a time.

## Principles

- **Observe before writing.** The layer starts read-only and stays "beside, never instead" ([D3](decisions.md)) until a write path is separately approved.
- **The log is authoritative and lossless.** Every observation, request, state change and reconciliation result is itself an evidence object in the store. There is no best-effort mirror that can silently drop events.
- **Record changes are observations.** `create`, `update` and `delete` in the log only ever describe changes seen in a system of record. Anything the layer or an application states itself (a request, a decision, a check) is an `assert`, so the log never claims a change it did not see.
- **Approvals count only under published rules.** Who may approve, and which approvals each kind of record needs, is published data: approver lists and approval rules, versioned and approved before use; how a list or rule set is changed, and by whom, is proposed in [approvals.md](approvals.md#53-changing-a-list-or-a-rule-set) and stays open until keys are bound at M5. Whether an object is approved is a view computed from the log at a stated position, never a stored field, and it has no effect outside this layer ([approvals.md](approvals.md)).
- **One comparator.** The same comparison decides whether a write landed and whether the store and the database still agree.
- **Two clocks.** An object's `at` is its writer's claim. The store adds its own position and received time outside the object, and every query says which clock it uses.
- **Privacy fails closed.** Every captured column must be classified before capture starts. Values that are not classified as non-personal are kept by reference, outside the log, with keyed hashes, because an append-only log cannot forget.
- **evidence/0 does not change.** Store facts go in a separate `store/0` spec; domain and connector rules go in profiles under `ext`.
- **Every format has vectors.** Formats that others must reproduce get an independent JavaScript reference, as the object format does today ([D7](decisions.md)).

## Milestones

| # | Milestone | Rough size* |
|---|---|---|
| M1 | Verifiable store: append-only log, checkpoints, offline verification | M-L: about 5 engineer-weeks (about a quarter of it the JS reference) |
| M2 | Read-only capture from a synthetic register | L: about 5 engineer-weeks |
| M3 | Two-way reconciliation, report-only, recorded as evidence | M: about 3 engineer-weeks |
| M4 | Memory reads: index, query API, bundles, read-only service | L: about 4-5 engineer-weeks for the index, queries, bundles and service; the additions of 27 November 2026 are not yet sized (M4b prime-coordinate column: 1-2 weeks, optional) |
| M5 | Real-data gate: trust, time, replication, privacy operations | M-L: about 4-5 engineer-weeks, plus external legal and security review time |
| M6 | Requests and the outbox as evidence, in shadow mode (no write code) | L: about 4 engineer-weeks |
| M7 | Write-through to the synthetic database only (writes opt-in) | L: about 5-6 engineer-weeks |
| M8 | (optional) Postgres and vendor-platform sources | M-L: about 3-4 engineer-weeks |
| M9 | (optional) centr/0 profile, register views and prime coordinates | M: about 2-3 engineer-weeks, only once a dictionary and a consumer exist |

\*Sizes are estimates for one engineer. The real-data gate (M5) also depends on legal and security review, and on agreeing data access with each real system's owner, which run on their own timelines. These sizes predate the additions of 27 November 2026, which are not yet sized: the approval-status and disagreement views, read labels and grant-scoped tokens (M4); the assert intake (late M4 or early M6); wider key binding, key-bound approver lists and the id directory (M5); approval rules as data and continuity notes (M6; the timing of notes is open); and the edge track.

### M1 Verifiable store: append-only log, checkpoints, offline verification

Give evidence objects and attestations a durable, single-writer home. Each entry gets the store's own position and received time. Signed checkpoints, a structured integrity report and an export that anyone can verify offline complete it. No system of record is touched.

**Deliverables**

- spec/store-0.md, plus JSON Schemas for log-entry/0, the store/0 genesis, checkpoint/0, segments/0 and store-report/0. docs/decisions.md gains D11 (observe before writing), D12 (storage) and D13 (two clocks).
- evidence-core gains PrevLookup, check_prev and verify_lookup as additions only. Options, Store, api::verify and every binding stay exactly as they are.
- crates/evidence-log, pure, building for wasm32 and PyO3: log entries, the RFC 9162 Merkle tree with inclusion and consistency proofs, genesis and checkpoint objects, lanes and heads, the received-time and service-time rules, a streaming verifier and an export verifier.
- crates/evidence-store: SqliteLog (log.db with append-only triggers, a heads projection written in the same transaction, a file lease plus writer epoch), MemLog as the reference, compare-and-set append_if_heads, all-or-nothing batches, signed checkpoints, export and import, and verify_store.
- CLI: evidence store init|append|attest|checkpoint|get|heads|log|verify|export|import|prove. `evidence verify --store` also accepts a store directory.
- WASM and Python gain verify_export, entry_hash, merkle_root, verify_inclusion and verify_consistency. A static page verifies an export offline in a browser.
- tools/reference/log.mjs, an independent JS implementation, generates test-vectors/store/: entries, merkle, 6 valid exports and 18 tampered exports.

**Acceptance**

- Every test-vectors/store case produces the expected canonical store-report/0 byte for byte in Rust, the JS reference, WASM and Python. Each tampered case produces its named code: for example, a flipped object byte gives S_OBJECT_INVALID with core E_ID_MISMATCH and nothing else.
- SqliteLog and MemLog each replay every valid case's inputs.json with the scripted clock and test seed, and export files byte-identical to the JS-generated vectors.
- All 20 existing object vectors go through the CLI store as follows. 001-005 land. 101-109 and 114 are refused with their case code. 110 gives E_PREV_MISSING and 111 gives E_PREV_ORDER. 112, 113 and 115 come back as duplicates of 003. Their attestation lines give E_SIGNATURE_INVALID, a duplicate, and E_ATTESTATION_SHAPE respectively. `store get` on 003 with --require approve gives E_ATTESTATION_MISSING before the approval is attested and ok after. `store verify` is clean.
- One contract suite passes against both MemLog and SqliteLog. A proptest (256 cases in CI) runs random appends, forks, merges, stale compare-and-set calls, attestations, checkpoints and backward clock steps, and these invariants hold: verify is clean, the heads table equals a recompute, every inclusion and consistency proof verifies, MemLog and SqliteLog exports are identical, and import(export) has the same head.
- Crash test: SIGKILL at 50 random points during CLI appends (1,000 nightly). After each kill, `store verify` is clean and every acknowledged receipt is present.
- A second writer gets S_LOCKED. An SQL UPDATE or DELETE on entries, objects or attestations aborts. With the triggers dropped, an edited object is reported as S_TRIGGERS_MISSING plus S_OBJECT_INVALID, and a deleted heads row is reported as S_PROJECTION_DRIFT.
- With a clock that steps back an hour, received_at never decreases and checkpoints still verify. evidence-core's existing vector tests pass unmodified. CI fails if rusqlite reaches the core, the pure crate, the WASM build or the Python build.

### M2 Read-only capture from a synthetic register

Turn every change in an existing database into evidence using only read access and no schema change. Capture is idempotent, survives crashes and refuses to run on unclassified data. This is the first demo that is directly relevant to consumer energy resources (CER) records.

**Deliverables**

- demo/installation-register (synthetic): SQLite built from test-vectors/fixtures/installation-v12.ddl.sql unchanged, plus a synthetic installer table with personal fields and a v13 migration.
- crates/register-sim: seeded workloads of inserts, updates, deletes, re-creates, bursts, capture pauses, edits made outside change tracking, the v12 to v13 schema change, SoR clock skew and a restore from backup. Its oracle gives the expected projected state at every step.
- crates/evidence-model, pure, built for WASM and Python:
  - mapping/0: every live column is classified inline, keyed, reference or omit, and key, version, time, provenance and SoR-computed columns are declared. It also defines versioned normalisation of NUMERIC, DATE, TIMESTAMP and big integers, and a catalogue fingerprint.
  - project().
  - change-key/0, computed over the comparable projection, which leaves out SoR-computed, version and provenance columns.
  - The capture/0 profile check, with codes P_MAPPING_UNCLASSIFIED, P_SUBJECT_KEY_MISMATCH, P_CHANGE_KEY_MISMATCH and P_CAPTURE_SHAPE.
- capture/0 observation rules:
  - The first sighting of a row is op=assert with observation=baseline, in the record lane.
  - create, update and delete are used only for observed changes.
  - A delete carries the last state. It is written only on a positive delete signal, or after a complete scan confirmed by a keyed NotFound read.
  - at is the later of the SoR time and the prev objects' at, and the raw SoR time is kept in ext.
  - The source position stays out of the object.
- evidence-store schema v2: cursors and change_keys tables, and an Item::Cursor that commits in the same transaction as the observations. blobs/sha256/ holds reference-mode content outside the log, where it can be erased and is hash-checked on read. Capture is the only writer of the record lane and appends with append_if_heads.
- crates/evidence-connect (I/O):
  - Traits SorReader {read, scan, catalogue_fingerprint} and ChangeSource {poll}.
  - SnapshotDiffSource, with a completeness guard.
  - ChangeTableSource, using triggers in the demo only.
  - ChangeFileSource, for Debezium-style JSON Lines extracts handed over offline.
  - capture_once, with Clock, Signer and KeyedDigest injected.
  - capture-run asserts carrying counts.
- CLI commands `evidence capture once` and `evidence mapping check`. test-vectors/changekey and test-vectors/capture, with parity in the JS reference.

**Acceptance**

- The changekey and capture vectors (row, mapping, heads and clock in; exact object bytes or a P_ code out) pass in Rust, JS, WASM and Python. An unlisted column gives P_MAPPING_UNCLASSIFIED. A subject key that differs from the key column gives P_SUBJECT_KEY_MISMATCH.
- Oracle E2E with seed 42, 1,000 rows and 5,000 operations, including 200 deletes, re-creates and the v12 to v13 change. In change-table mode, after every step, each subject's record-lane head content hash equals the oracle's projection, or is a delete carrying the last state. Snapshot mode matches at each poll and marks its observations as observed. Replaying the same history from change files reaches the same head content hashes.
- A second capture run appends nothing. After a SIGKILL at a random point and a rerun, the final heads are the same and no change key is duplicated.
- The SoR is opened read-only, and an attempted write fails. The SoR file hash is identical before and after capture. The D3 test deletes the store directory and finds the SoR byte-identical, with nothing lost.
- Seeded personal sentinel values in keyed or omitted columns appear nowhere in log.db, the exports or the blobs. Reference-mode values appear only in blobs/.
- The v12 to v13 change stops capture for that type with a schema_changed finding while other types continue. Capture for that type resumes once a new mapping assert is published.
- When the SoR time is earlier than the head's at, the object's at equals the prev object's at, the raw time is kept, and the object verifies without E_PREV_ORDER. A failed snapshot page never writes a delete. A restore that reissues old version numbers with new content is still captured.

### M3 Two-way reconciliation, report-only, recorded as evidence

Prove continuously and verifiably that the store and the database agree. Where they do not, name exactly where, including edits made outside change tracking.

**Deliverables**

- evidence-model::compare, a single comparator that uses the same normalisation as projection; classify(); the reconcile/0 profile.
- reconcile_once, forward and reverse:
  - Forward pass: covers every record-lane head. A rotating cursor reaches all subjects every K runs, and every subject touched since the last run is also checked.
  - Reverse pass: works by scan.
  - Classes: match, lagging, untracked_edit (the hash differs but the version is unchanged), differs (the SoR has no versions), missing, unchecked (becomes unreachable after K runs), sor_regressed (a restore from backup), coalesced (skipped versions; informational), unobserved and resurrected.
  - partial:true names any scope whose page failed.
- Each run is recorded as a reconcile-report/0 assert, chained by prev per scope. Resolutions are asserts by a human actor carrying an approve attestation, with disposition legitimate_edit, capture_defect or escalated. Notifications carry only the class, the key and field names.
- Reconciliation runs on a connection whose SQLite authorizer denies every write.
- CLI `evidence reconcile once|show`; test-vectors/compare.

**Acceptance**

- The compare vectors pass in Rust, JS, WASM and Python.
- Each injected drift gives exactly its golden finding:
  - an edit made with change tracking off: untracked_edit[status];
  - an out-of-band delete: missing;
  - an insert while capture is paused: unobserved, then match once capture runs;
  - a capture pause after updates: lagging, then match;
  - an unreachable SoR: unchecked everywhere, never missing, then unreachable after K runs;
  - a restore from backup: sor_regressed.
- An untouched workload gives zero findings, covering 6.6 vs 6.600, dates, time zones and NULL.
- A failed reverse-scan page gives partial:true and names the scope. Running again on the same inputs gives the same report content hash.
- Reports verify under the core and reconcile/0, and chain per scope. A resolution by a service gives P_RESOLVER_NOT_HUMAN. Report and notification bytes contain no seeded values. The authorizer test records zero write attempts.

### M4 Memory reads: index, query API, bundles, read-only service

Make the store the place where staff and applications ask what a record said, under which schema, who changed it, as of when, and why. These are the first memory reads.

Design input: [turn-reader.md](turn-reader.md), the reader tools and AI agents call before they rely on evidence: what it verifies, its read receipt, and when a read that informs an action is recorded as an `assert`. Its open decisions (where a decision's basis goes, completeness at a checkpoint, an intake for asserts) bear on the index and bundles below. Also [approvals.md](approvals.md), which proposes approver lists and approval rules as published data, an approval-status view, and hooks for tools; its views and labels are M4 deliverables below.

**Deliverables**

- crates/evidence-index: index.db is derived from log.db behind a watermark (seq plus entry hash) and rebuilds deterministically. Index-vs-log reconciliation reports index_missing, index_extra and index_stale.
- Query API:
  - get: fresh verification on every read. Objects that fail are dropped and listed in rejected[].
  - by_subject: log order, with a stable seq cursor.
  - state: current, forked, deleted or absent, the latest assert per schema, and capture lag.
  - history: ancestors and descendants, with depth, via and truncated.
  - as_of on both clocks: claimed `at`, or recorded, meaning the largest seq with received_at at or before t.
  - Faceted filter.
  - Reverse lookups by content hash, schema hash and signer.
  Every answer carries the watermark, the clock used, the echoed limits, and a truncated flag computed by fetching limit+1.
- Bundles: objects, attestations, entries, a covering checkpoint and inclusion proofs. `evidence bundle verify` works in the CLI, WASM and Python. A bundle can be sealed as an assert, which answers 'what the register showed on date X'.
- crates/evidenced (axum and tokio): one process per store, holding the lease. Its read-only HTTP API mirrors the CLI. Bearer tokens map to actors and carry the actor's kind (`human`, `service` or `agent`), with read scopes per subject.system that are closed by default. A token issued to a tool acting for someone takes its read scopes from that person's grant (`approval/grant@0`), never wider.
- The approval-status view ([approvals.md](approvals.md)): for one object at a watermark, one of a fixed list of statuses (approved, awaiting approval, contested, superseded, unknown), with the rule set used, each step and who filled it, the escalations, the positions recorded, the lines that did not count and why, what would move the status on (`next`), and the label `not-established-before-key-binding` until M5. It is computed from the log and writes nothing. Lineage-style vectors and an independent JavaScript reference ([D7](decisions.md#d7-the-vectors-are-the-contract)).
- A disagreement view per subject, for the edge profile ([cer-edge.md](cer-edge.md)): more than one head in a lane, a parent with children in more than one lane, and the differences that the approval rules' comparisons find between a record and a linked statement. It is computed from `prev` links over the whole history and writes nothing, with vectors for cases that must be flagged and cases that must not.
- Read labels: every item a read returns gets the first label that fits: `observed` (a record lane written by capture), `note` (a continuity note), `tool-stated` (any other assert made for someone else, with `on_behalf_of`, whatever the actor's kind; every agent's assert has one) or `stated` (anything else, including a record-lane object not written by capture). Nothing is returned as observed unless capture wrote it.
- The read-basis profile ([turn-reader open decision 2](turn-reader.md#2-the-basis-profile), working name `read-basis/0`): its shape and the `P_BASIS_*` checks, including `P_BASIS_STALE`, which the `basis-stale` condition in [approvals.md](approvals.md) uses. Vectors and an independent JavaScript reference.
- test-vectors/lineage: forks, merges, deletes, baselines, late arrivals, and as_of on both clocks. Optional M4b adds a prime-coordinate column, only once a published dictionary exists.
- Late M4, or early M6 at the latest: an intake for asserts ([turn-reader open decision 7](turn-reader.md#7-an-intake-for-asserts)). One request is one batch: the object and its attestation lines, with optional if_heads and write scopes per lane. Before appending, it runs the pure checks: actor id form, an author line signed as actor.id, the kind of on_behalf_of, the record-lane writer, and grants. It refuses with every failed code, and verification repeats the same checks.

**Acceptance**

- The lineage vectors pass in Rust, JS, WASM and Python.
- Golden queries on simulator data pass, including a late arrival that makes 'as of T (claimed)' differ from 'as of T (recorded)'. Every response carries a watermark and detects truncation.
- Dropping and rebuilding index.db gives an identical index digest, and a rebuild-equivalence proptest holds. Tampered index rows are classified as missing, extra or stale.
- An object byte changed behind the triggers is excluded from answers, listed in rejected[] and reported by `store verify`.
- A bundle verifies offline in the CLI, WASM and Python, and changing one byte makes it fail with a code.
- HTTP answers are byte-identical to CLI answers. A token scoped to one system gets 403 for the others, and a delegated token gets 403 outside its grant. Without the HMAC key, index.db holds no plaintext sentinel values.
- The approval-status vectors give the same status, steps and codes in Rust and the JavaScript reference at every log position they name.

### M5 Real-data gate: trust, time, replication, privacy operations

Make M1-M4 safe to run beside a real system of record on real data. That needs keys bound to people, independent time, copies off the machine, erasure, access control, and legal sign-off.

**Deliverables**

- Key-binding objects: an administrator's approve binds a key to an actor id, with a validity period and revocation. They cover persons, services, organisations, gateways and agents; an agent's binding names its sponsor, the person or organisation accountable for it, and a gateway's binding also names its log id and store key. The id is not the key, so a key can be rotated or revoked without the id changing. Verification gains S_KEY_UNBOUND and S_KEY_REVOKED, and M3 resolutions are re-checked against the bindings.
- Approver lists ([approvals.md](approvals.md)) become key-bound: an approval counts only when its key is bound to the listed id at the line's log position.
- The id directory, which maps pseudonymous actor ids to people, is held off the log and access-controlled, with its own access log. It is never appended to the store.
- Signer backends for PKCS#11 (tested against SoftHSM in CI) and a cloud KMS. RFC 3161 timestamp tokens are recorded on checkpoints as witness attestations.
- Sealed exports are replicated to an object-lock bucket and/or a git remote held by another party. A pull either completes or changes nothing. Consistency proofs run between published checkpoints, with codes S_REPLICA_TRUNCATED and S_REPLICA_FORK.
- Erasure deletes a blob and appends a redaction assert approved by a bound key, and verify then reports the blob as erased, not missing. A leak scanner covers the log, exports, blobs, the index, logs and HTTP responses, and its seeded sentinels include directory entries (synthetic ids and the personal details they map to) that exist only in the directory, so finding one anywhere else shows that the directory leaked.
- A container image, a runbook, health and metrics (capture lag, last reconcile, open findings, watermark) and a security description against the Australian Signals Directorate's Information Security Manual (ISM). A legal review covers the store's status under the Privacy Act 1988 (APP 11), the Archives Act 1983 (including records disposal under section 24) and the Freedom of Information Act 1982.

**Acceptance**

- Key-binding vectors: an unbound key gives S_KEY_UNBOUND. For a line held in a store, revocation is judged by the entry's received_at, because an attestation's `at` is chosen by its signer: a line received after revocation fails, and one received before it passes. For a line outside a store, the result is unknown unless a witness line or a timestamp token shows when it existed.
- A recorded TSA token fixture verifies offline in Rust, WASM and Python.
- A truncated replica gives S_REPLICA_TRUNCATED and a rewritten one gives S_REPLICA_FORK. A clean replica verifies with no access to the primary, and a store restored from it has the same head.
- After an erasure, `store verify` reports the blob as erased, and every id and the chain still verify. The leak scanner finds no seeded sentinel value for an unprivileged role.
- The signer contract suite passes for the seed, PKCS#11 and KMS backends.

### M6 Requests and the outbox as evidence, in shadow mode (no write code)

Let applications hand over a request to change a record instead of acting on the database. Requests get a de-duplication key, a published allow-list, a compare-and-set state machine and resolution by named people, all recorded as evidence. In shadow mode, measure how often requests match what staff actually did.

**Deliverables**

- register/0: the write allow-list held as data (kinds, operations, fields, types, limits; deletes forbidden by default). It is published as an assert, and every request carries its {ref, hash}.
- The approval rules for requests are data too: P_APPROVAL_REQUIRED and the two-person rule for an overwrite on a stale base are written as `approval/rules@0` rules ([approvals.md](approvals.md)), not as code.
- sync/0 profile. The request (intent) is an assert on the SoR subject. Its content is the full proposed comparable projection, built at intake from the base head plus the requested field changes. The actor is a person, or a service with on_behalf_of, which is required. Whether an agent may file a request under a grant is open. One candidate rule: a tool files a request only when a person starts it, for a change that person could request themselves, with that person in on_behalf_of. ext carries op, base, base_sor_version, change_key, app, and the register and mapping refs.
- Intake checks the person's identity. A register violation is refused with field:code and nothing is stored. Dedupe on the change key returns the existing request. The request object is itself the queued state, so intake is one atomic append.
- Transitions are value-free asserts on subject evidence-sync/intent/<id>, appended with append_if_heads.
  - Members are a closed list: intent, from, to, reason code, attempt, epoch, receipt, sor_version, observed_hash.
  - Retry, discard and park are allowed only for a human carrying an approve attestation from a bound key.
  - An overwrite on a stale base needs two people.
  - Every chained append uses the service-time rule.
- The working outbox view in index.db is rebuilt from the transitions. The state() view overlays a caller's pending requests. Reconciliation compares requests with observations (shadow_match, shadow_differs, superseded) and the outbox view with the transitions. HTTP: POST /intents, GET /intents/{id}, and a held list.
- Proof that the default build has no delivery path: a compile_fail doctest, plus a cargo feature-graph check that no build for real data enables the `writes` feature.
- Continuity notes for tools, proposed for M6 with the timing open ([turn-reader open decision 13](turn-reader.md#13-continuity-for-tools)), built on the assert intake once M5's erasure exists: by-reference asserts under a grant, in a dedicated notes system, labelled `note` and recalled only within the grant's window. No recall endpoint, ranking or similarity search.

**Acceptance**

- The outbox vectors enumerate every (state, event, actor kind) triple. Allowed edges give the expected transition and all others give P_INVALID_TRANSITION. A retry by a service gives P_RESOLVER_NOT_HUMAN, and a retry without approval gives P_APPROVAL_REQUIRED.
- The same request from two people at different times gives one request object. A register violation is refused, and its sentinel value is absent from every byte of the store.
- Two concurrent transitions on one request: exactly one lands, the other gets S_STALE_HEADS, and the log never shows two heads.
- Shadow E2E: simulator staff apply the requested change by hand, and the report shows shadow_match through an equal change key, with no change to the SoR schema. A different manual change gives shadow_differs.
- The rebuilt outbox view equals the working view. The observe build compiles with no delivery code, shown by the compile_fail test.

### M7 Write-through to the synthetic database only (writes opt-in)

Close the loop on the demo database. Changes are delivered through its own write interface, and a change counts as done only when a read-back with the same comparator proves it and capture later sees it. Writing to any real system stays a separate decision for that system's owner.

**Deliverables**

- A cargo feature `writes`, never enabled in builds for real data.
  - SorWriter delivers to the demo database through its own write procedures.
  - An ev_provenance side table (record key, SoR version, change key, request id, person, app) is written in the same SoR transaction. Business tables are unchanged.
  - Optimistic concurrency uses row_version.
  - Creates read provenance first, so a retried create is idempotent.
- Worker:
  - 'sending' is made durable before delivery.
  - Outcomes are typed: Delivered, PartiallyApplied, Rejected, Unknown.
  - An unknown outcome is resolved by read-back, and the change is resent only if the read-back proves it did not land.
  - Confirmation is read-back plus compare(), with (sor_version, hash) recorded on the confirmed transition. Capture stays the only writer of the record lane.
  - Stale results are held, never merged.
  - At most one request per subject is in flight, and due selection cannot starve other requests.
- Safety rails:
  - Writes are off by default.
  - A kill-switch file stops delivery; an unreadable file counts as engaged, and read-backs continue.
  - Per-app caps skip only that app.
  - The store is bound to one SoR.
  - A wire guard derived from the register runs as the SQLite authorizer.
- Reconciliation adds orphan (a provenance row unknown to the log), confirmed_unobserved, observed_unconfirmed and stuck_sending. A fault-script wrapper injects a silent drop, a partial apply, an apply followed by a timeout, a stale version, an unknown error and an unavailable read-back.

**Acceptance**

- Each scripted fault ends in its documented state with a value-free reason code.
- A proptest over random fault sequences keeps four invariants:
  - no change key is applied twice (checked against the simulator's apply log);
  - nothing is confirmed without a passing read-back;
  - stale updates are never merged;
  - every confirmed request is observed within N capture cycles or reported as confirmed_unobserved.
- A crash between delivery and confirmation leads to a read-back and a confirmation, with no second write.
- With the kill-switch file present or unreadable, nothing is delivered and read-backs continue. One app at its cap does not delay another. A tampered plan that touches an unlisted column is denied by the authorizer.
- A fabricated provenance row shows up as orphan. The D3 test deletes the store and finds every confirmed write in the SoR with business tables unchanged; the last export lists the requests that were still pending.

### M8 (optional) Postgres and vendor-platform sources

Capture from realistic infrastructure once a real system's technology is known, without losing changes on a crash.

**Deliverables**

- A Postgres polling source.
- Logical decoding using pg_logical_slot_peek_changes, then pg_replication_slot_advance only after a durable commit (or streaming replication with feedback after commit). The consuming get_changes is never used.
- A CI service image that includes wal2json. An HTTP API adapter for vendor platforms.
- PgLog behind the same traits, only if a system owner's operations require their own database estate.

**Acceptance**

- The M2 and M3 suites pass unchanged against Postgres.
- A crash between reading a change and committing it loses nothing.
- A dropped replication slot surfaces as a capture gap, never silently.

### M9 (optional) centr/0 profile, register views and prime coordinates

Answer domain queries and publish registers as views over the store. Prime coordinates appear only as a derived, checked export field.

**Deliverables**

- A centr/0 profile verifier and vectors, including the absent cases from docs/centr-profile-notes.md.
- Register snapshots as asserts whose content is a query result at a watermark.
- A published facet dictionary, and a derived coordinate column that is never part of an id. Every coordinate carries its dictionary's profile id and indexes key on (profile, coordinate), as in [Kernel/2](kernel-2.md#33-representation-declared). A shared registry of prime ranges is optional.

**Acceptance**

- The centr/0 vectors pass, including the absent cases.
- A register snapshot recomputed at its recorded watermark is byte-identical.
- On 100k objects, divisibility queries return exactly the same rows as facet queries, every coordinate is carried as a decimal string and decodes to exactly its facets, and an unknown facet gives a finding.

## Edge track

The edge track adds records about consumer energy resources devices: an installer's commissioning record, a network connection record, and a gateway's own settings or measurement summary. The profile is a draft in [cer-edge.md](cer-edge.md); the approvals it relies on are in [approvals.md](approvals.md). The track sits under the milestones above, so this stays the only roadmap. Nothing in it is built.

- **Two kinds of edge object, chosen by record type.** A record held in another system enters as a record change observed by a capture service, in the record lane. A gateway's own settings or measurement summary is an `assert` by the gateway, with an opaque service id.
- **The installer approves the captured record** with an `approve` attestation. A published approver list says whose approvals count, and approval rules say when a listed reviewer is also needed.
- **No writes and no automated decisions.** The edge track adds no write path to another system, and only synthetic data is used until M5. A difference between two records is a prompt for review, not a finding, and a person resolves it. Each status says what is still needed and who can still supply it, so uncertainty is shared and a person can decide.

| Step | Content | When |
|---|---|---|
| EG1 | The edge profile and the `approval/0` shapes (approver list, rules, position, resolution, grant), with vectors built from the profile's examples; a synthetic edge workload for the M2 simulator; a gateway deployment profile ([cer-edge.md section 6](cer-edge.md#6-moving-records-between-sites-proposed)) | Alongside M2 |
| EG2 | Sync between sites (ingest, or one replica per gateway), the receipt profile, exports that end at a checkpoint, and a check for two histories signed by one key, with vectors | With M4 |
| EG3 | Gateway key binding (signing key, log id and store key), how gateways get their devices' keyed digests, the transport decision and an ARM build. A trial with real devices only after M5, with legal review and the agreement of the device owners and of the systems' owners | After M5 |
| EG4 | Edge rules that need more than one listed person in the same role (`count` above 1), only if a use needs them. Before keys are bound, two names need not be two people | After M5 |

The edge track is not yet sized; each step adds to the milestone it sits under.

## Decisions (confirmed 26 November 2026)

1. **Record D11 now: observe first, writes later only through explicit requests**
   - Options: (a) Record D11 now and keep D3 as written until a write path is separately approved. (b) Amend D3 now to describe the end state. (c) Say nothing until M6.
   - Recommended: (a), using the wording in the M1 spec. Record ops in the log are only observations of the system of record. Requests come later, checked against an allow-list the SoR owner publishes and delivered through the SoR's own interface. They are off by default and enabled per record type. Turning the layer off never loses a confirmed change, but once writes are on, pending requests depend on the layer being available.
   - Why: It says exactly what a first deployment with real data would and would not do. It also fixes the rule that shapes M2's objects. And it is honest about the one way a write path weakens D3's 'turning it off loses nothing'.
2. **Spec impact: freeze evidence/0 or bump it**
   - Options: (a) Freeze evidence/0. Add a separate store/0 spec (log-entry/0, checkpoint/0, segments/0, store-report/0) now, and later profiles under ext (capture/0, mapping/0, reconcile/0, sync/0). (b) evidence/0.1 with received time or an intent key inside the object. (c) evidence/1.
   - Recommended: (a)
   - Why: Received time and position are facts about one particular store. Inside the object they would give the same object a different id in each store. Status is already defined as a view, and the change key is derived from the object, so nothing found so far needs a bump. A bump would invalidate every binding and vector.
3. **Authoritative storage engine**
   - Options: SQLite log.db as the authority, plus exported JSON Lines segments, plus a derived index.db from M4 / JSONL segments as the authority with a SQLite index / git as primary / Postgres / a pure-Rust KV store (redb, fjall)
   - Recommended: SQLite, through rusqlite with the bundled build, as the authority. Segments are the portable and conformance form. Git or object storage hold replicas only. Postgres goes behind the same traits only if a system owner's operations require their own database estate.
   - Why: One transaction covers the append, the heads, and (from M2) the capture cursor and the change-key uniqueness constraint. Triggers make the tables append-only inside storage itself. It needs no server, and auditors can open the file. Because the export defines conformance, a later backend can be proven equivalent. Making JSONL the authority means hand-written crash recovery and pushes uniqueness into a derived index. Git has no compare-and-set or transactions.
4. **What data and keys are allowed before the real-data gate (M5)**
   - Options: (a) Synthetic or already-public data only, with a development seed file for the store key, until M5 delivers KMS/HSM, key binding and legal review. (b) Real data from M2 with development keys. (c) KMS from M1.
   - Recommended: (a)
   - Why: An attestation proves only that the holder of a key signed (spec 'Not in v0'). Signatures and approvals made with development seeds must not be presented as accountability evidence. Anything wrongly put into an append-only log cannot be taken out.
5. **Scope of the two-implementation rule (D7) for new formats**
   - Options: A JS reference for every new module / a JS reference only for formats and pure rules that third parties must reproduce / Rust-only golden files
   - Recommended: A JS reference for what others must reproduce: the log entry, Merkle tree, checkpoint and export verification (M1); change key, projection and normalisation (M2); the comparator (M3); lineage (M4). Rust-only golden files for query answers and reconciliation reports.
   - Why: It keeps the byte-identity guarantee where it matters, for about a quarter more effort on those milestones.
6. **Reserve create/update/delete in the log for observations of the system of record**
   - Options: Reserve them, so that requests, outcomes, decisions and checks are all asserts / let requests use op=update with a role marker
   - Recommended: Reserve them. A first sighting of an existing row is an assert (a baseline) in the record lane, never a fabricated create.
   - Why: The log then never claims an SoR change it did not see. A request can also be matched to the change it caused by an equal change key, with no change to the SoR schema.
7. **Capture route for real data (needed before M2 ends)**
   - Options: (a) Periodic snapshot comparison over a read-only connection to a replica or extract. (b) Change files from the system owner's own change-data-capture tooling, handed over as files. (c) Database log-based CDC granted to the layer. (d) The SoR vendor's API. (e) Triggers in the SoR.
   - Recommended: Start with (a) or (b), neither of which changes the SoR. Move to (c) for record types where full history matters. Never use (e) outside the demo.
   - Why: These have the smallest security footprint and are the fastest to agree. Snapshots miss intermediate versions, so every observation records how it was seen. M2 builds both (a) and (b) against the synthetic register, so it does not wait on access to any real system.
8. **Privacy defaults for captured records**
   - Options: Inline whole rows / inline only columns classified as non-personal, everything else by reference with a keyed salt / never inline
   - Recommended: Every column must be classified, or capture refuses to start. The default is by reference: bytes go in an erasable blob store outside the log, with a keyed salt so hashes cannot be guessed. Only columns classified as non-personal are inline. Personal values that must be linked become keyed digests. The data owner holds the keys, one per link domain, in its key service.
   - Why: The log cannot forget. A bare hash of a low-entropy value can be reversed by guessing. And a CI leak test only means something if classification fails closed.
9. **Policy on claimed times**
   - Options: Reject objects whose `at` is well after the store's received time / accept and count them / rewrite them
   - Recommended: Accept and count them (store report counts.at_after_received, threshold 5 minutes), with a per-source option to reject later. Never rewrite a writer's `at`. Only objects that services author in a chain, and capture, use the later-of rule, and capture keeps the raw SoR time.
   - Why: Rejecting would lose evidence because of one misconfigured clock, and rewriting would break ids. Counting makes the problem visible.
10. **Unknown profile keys under ext at intake**
   - Options: Reject / store them and report them as unchecked
   - Recommended: Store them. Only the store's own subjects are reserved. From M2, the profiles the store knows (capture/0 and later ones) are always checked, and any others are listed as unchecked.
   - Why: Objects carrying other bodies' profiles can then be kept and verified at the core level, even though the store does not understand those profiles.

## Decisions (confirmed 27 November 2026)

1. **The edge work is a profile and an edge track under this roadmap.** The profile is [cer-edge.md](cer-edge.md), the steps are EG1-EG4 in the [edge track](#edge-track), and this stays the only roadmap.
2. **Edge objects follow the record type.** A record held in another system enters as a record change observed by a capture service, in the record lane. A gateway's own settings or measurement summary is an `assert` by the gateway.
3. **The installer approves; a published approver list says whose approvals count.** The installer signs `approve` on the captured record. Approver lists and approval rules are published data ([approvals.md](approvals.md)).
4. **What matters is how uncertainty is shared and how decisions are enabled, not who runs which system.** The layer records where there is doubt and where a person's decision is still needed, and makes no decision itself.
5. **Approval is spread in that spirit; ids are pseudonyms; tools are actors.** Approval rests with the parties on published approver lists, and actor ids are pseudonymous. An AI agent is one kind of actor, not only a reader or contributor. The core and the hooks are the same for every tool: grants, verified reads, recorded checks and doubt, witnessing, and continuity notes kept between sessions. Whether a tool may ever fill an approval step is still open; the proposal is that it never does ([approvals.md open question 4](approvals.md#15-open-questions)).

## Risks

- This repository assumes nothing about any real system of record: its technology, whether a replica or extract is available, whether CDC is permitted, whether it is a vendor platform with only an API, and whether it has version columns. This shapes M2 and M3. Mitigation: SorReader and ChangeSource traits with snapshot, change-file, CDC and HTTP adapters, so that capture can follow whichever route a system's owner agrees to.
- Privacy and records law. An append-only log cannot forget, so a field classified wrongly is permanent. Mitigation: fail-closed mapping, CI sentinel leak tests, an erasable blob store outside the log, synthetic data only until M5, and legal advice before any real data on the Privacy Act (APP 11), the Archives Act 1983 and FOI.
- False drift from normalisation (NUMERIC 6.6 vs 6.600, DATE vs TIMESTAMP, time zones, NULL, trailing spaces) will quickly erode trust in reconciliation. Mitigation: one versioned normalisation shared by projection and the comparator, covered by vectors, plus a CI clean run that must report zero findings.
- Until M5, signatures prove only that a key holder signed. Approvals and resolutions are not accountability evidence before key binding. Before KMS and independent time are in place, a stolen store key could forge history after the last externally witnessed checkpoint.
- Polling loses history. Snapshot capture sees only the state at each poll, so intermediate versions are never evidenced. Objects and the UI must say 'as observed', and CDC should be used where completeness matters.
- The change key has subtleties. SoR-computed columns (updated_at, row versions, trigger output) must be left out of the comparable projection, or confirmations never match. An identical manual change looks like a confirmation: that is deliberate in shadow mode, but once writes are on, telling them apart needs the provenance side table, which the SoR owner must approve.
- Single node, single writer, so no high availability until replication and a warm standby (M5). The flock lease needs a local disk, not NFS. An fsync on every commit limits the append rate; capture batches its commits, and the rate is measured on reference hardware before any real use.
- Timeline: about 31 engineer-weeks for one engineer to reach writes to the synthetic database (8-9 months with contingency), with memory reads at about week 18 and the write half last. These figures predate the additions of 27 November 2026 and the edge track, which will lengthen them once sized. Mitigation: each milestone ships on its own, and a second engineer can build M4 alongside M3.
- Perception: a system's owner may read any write path as drift toward 'instead'. Mitigation: record D11 now, ship builds for real data that provably contain no delivery code, enable writes per record type under the system owner's published allow-list, always deliver through the SoR's own interface, and treat writes to any real system as a separate decision.
- Dependency weight goes against the project's minimal-dependency stance (D8, D9): rusqlite bundles C SQLite, and axum and tokio arrive in M4. Mitigation: keep them out of evidence-core, evidence-log, the WASM build and the Python build, enforced by a CI guard.
- Postgres logical decoding tooling in Rust is thin, and the consuming slot function loses changes on a crash. Use peek followed by advance after a durable commit, and a custom CI image with wal2json. Keep it optional until a real system's stack is known.
- Scope creep toward fuzzy recall (embeddings, ranking, token or prime indexes). Retrieval stays exact, bounded and verifiable, and prime coordinates appear only as a derived export field. Continuity notes for tools are governed asserts under a grant, not a recall service.
- A review status can be misread as a finding. Statuses are review statuses with no effect outside this layer, and a difference between records is a prompt for review, not a finding. Before M5 no approval is accountability evidence, and every status output says so.
- Public-repo hygiene: synthetic data and pseudonymous actor ids only; new profile vectors use the opaque id form of [approvals.md](approvals.md#3-parties-and-ids-proposed). Small inconsistencies slip in easily (an early vector's subject key disagreed with its content, since fixed); M2's subject-key check rejects such a mismatch in captured records.
