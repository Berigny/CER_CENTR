# Roadmap: from evidence layer to memory

Status: decisions confirmed by the project maintainer on 26 November 2026 (recorded as D11-D13 in [decisions.md](decisions.md); the rest apply as written [below](#decisions-confirmed-26-november-2026)), on 27 November 2026 ([the edge track and approvals](#decisions-confirmed-27-november-2026)) and on 28 November 2026 ([prime coordinates](#decisions-confirmed-28-november-2026)). The design notes' open questions were settled on 28 November 2026 ([below](#open-questions-settled-confirmed-28-november-2026)); those that need a decision outside the project are marked gated. The build specification for the first milestone is [m1-store-spec.md](m1-store-spec.md). This stays the only roadmap. Not an official publication of any agency; it describes no agency's process (see the [README](../README.md) notice).

## What's next, in short

Today the layer can create, sign and verify single evidence objects. To become an organisation's memory it needs four more things, in this order:

1. **A safe place to keep what it records.** A store that can only be added to, where anyone can check, offline or in a browser, that nothing was changed or removed since a signed fingerprint of its history was taken.
2. **Capture from an existing database, without changing it.** Read-only capture turns each change in a register into evidence, with no schema change in the register.
3. **Continuous comparison in both directions.** On a schedule, the store and the database are compared, and every difference (including edits made outside the normal process) is recorded as evidence. Nothing is repaired automatically.
4. **Memory reads.** Staff and applications can ask what a record said on any date, under which schema, who changed it and why, and get an answer they can verify. Faceted queries give the same rows in any implementation, from the same dictionary version, watermark and query ([M4b](#m4b-coordinate-column-and-factor-table)); the coordinate arithmetic they rest on, [coordinate/0](../spec/coordinate-0.md), is already built.

Only after that does the layer accept requests to change records: first in shadow mode, where requests are recorded and compared with what staff actually did, and later delivered through the database's own interface, where a change counts as done only when it is read back and seen to have landed. Writing to any real system stays off until the system's owner agrees, one kind of record at a time.

## Principles

- **Observe before writing.** The layer starts read-only and stays "beside, never instead" ([D3](decisions.md)) until a write path is separately approved.
- **The log is authoritative and lossless.** Every observation, request, state change and reconciliation result is itself an evidence object in the store. There is no best-effort mirror that can silently drop events.
- **Record changes are observations.** `create`, `update` and `delete` in the log only ever describe changes seen in a system of record. Anything the layer or an application states itself (a request, a decision, a check) is an `assert`, so the log never claims a change it did not see.
- **Approvals count only under published rules.** Who may approve, and which approvals each kind of record needs, is published data: approver lists and approval rules, versioned and approved before use. A new version counts only with approvals from at least two distinct administrators named in the version it replaces, and until M5 the first version is anchored by deployment configuration, and every status says so ([approvals.md 5.3](approvals.md#53-changing-a-list-or-a-rule-set)); M5's security review revisits that anchor. Whether an object is approved is a view computed from the log at a stated position, never a stored field, and it has no effect outside this layer ([approvals.md](approvals.md)).
- **One comparator.** The same comparison decides whether a write landed and whether the store and the database still agree.
- **Two clocks.** An object's `at` is its writer's claim. The store adds its own position and received time outside the object, and every query says which clock it uses.
- **Privacy fails closed.** Every captured column must be classified before capture starts. Values that are not classified as non-personal are kept by reference, outside the log, with keyed hashes, because an append-only log cannot forget.
- **evidence/0 does not change.** Store facts go in a separate `store/0` spec; domain and connector rules go in profiles under `ext`. The one clarification is [D15](decisions.md#d15-step-1-faults-are-reported-in-a-fixed-order): bytes with more than one step-1 fault report `E_PARSE`, then `E_NUMBER_RANGE`, then `E_DUPLICATE_KEY`, and every single-fault input and existing vector keeps its result.
- **Every format has vectors.** Formats that others must reproduce get an independent JavaScript reference, as the object format does today ([D7](decisions.md)).

## Milestones

| # | Milestone | Rough size* |
|---|---|---|
| M1 | Verifiable store: append-only log, checkpoints, offline verification | M-L: about 5 engineer-weeks (about a quarter of it the JS reference) |
| M2 | Read-only capture from a synthetic register | L: about 5 engineer-weeks |
| M3 | Two-way reconciliation, report-only, recorded as evidence | M: about 3 engineer-weeks |
| M4 | Memory reads: index, query API, bundles, read-only service | L: about 4-5 engineer-weeks for the index, queries, bundles and service; the additions of 27 and 28 November 2026 are not yet sized (see the sizing steps below); M4b (coordinate column and factor table, 1-2 weeks) is planned |
| M5 | Real-data gate: trust, time, replication, privacy operations | M-L: about 4-5 engineer-weeks, plus external legal and security review time |
| M6 | Requests and the outbox as evidence, in shadow mode (no write code) | L: about 4 engineer-weeks |
| M7 | Write-through to the synthetic database only (writes opt-in) | L: about 5-6 engineer-weeks |
| M8 | (optional) Postgres and vendor-platform sources | M-L: about 3-4 engineer-weeks |
| M9 | (optional) centr/0 profile, register views and a published dictionary | M: about 2-3 engineer-weeks for centr/0, register views and the dictionary, only once a dictionary and a consumer exist (sized before the coordinate column moved to M4b); Kernel/2, listed since 28 November 2026, is not yet sized |

\*Sizes are estimates for one engineer. The real-data gate (M5) also depends on legal and security review, and on agreeing data access with each real system's owner, which run on their own timelines. These sizes predate the additions of 27 and 28 November 2026, which are not yet sized: the approval-status and disagreement views, read labels, grant-scoped tokens, `read-basis/0`, the awaiting-approval report and the position across logs (M4); the assert intake (late in M4, early M6 at the latest); wider key binding, key-bound approver lists and the id directory (M5); approval rules as data, and continuity notes and their recall (M6); Kernel/2 (M9); and the edge and analytics tracks.

**Sizing steps** (settled 28 November 2026). No new estimate is given here until each step is done:

1. Size EG1 before M2 starts, since it runs alongside M2. EG2 is sized within M4, and EG3 and EG4 after M5, once gateway key custody, the digest route and the transport are decided.
2. Size M4 now, at least roughly, from the design notes, with everything now landing on it: the 27 November additions, `read-basis/0`, the assert intake, the awaiting-approval report, the position across logs, EG2 and AN1, and M4b's 1-2 weeks now that it carries the acceptance test. If the total grows well past the base, split M4 into a base milestone and a named follow-on (views, labels, intake), sized separately. AN2 is sized with the intake, and AN3 after M5.
3. Size M5's additions now, together with the lead times of its external reviews, because M5 gates every real-data step. Size M6's additions, continuity notes included, now that their timing is settled.
4. Then, before M2 starts, refresh the timeline in [Risks](#risks) and decide on those numbers whether a second engineer builds M4 alongside M3.

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
- The 20 object vectors of M1 (001-005 and 101-115) go through the CLI store as follows. 001-005 land. 101-109 and 114 are refused with their case code. 110 gives E_PREV_MISSING and 111 gives E_PREV_ORDER. 112, 113 and 115 come back as duplicates of 003. Their attestation lines give E_SIGNATURE_INVALID, a duplicate, and E_ATTESTATION_SHAPE respectively. `store get` on 003 with --require approve gives E_ATTESTATION_MISSING before the approval is attested and ok after. `store verify` is clean.
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
  - mapping/0: every live column is classified inline, keyed, reference or omit, and key, version, time, provenance and SoR-computed columns are declared. It also defines versioned normalisation of NUMERIC, DATE, TIMESTAMP and big integers, and a catalogue fingerprint. A record id that is not opaque is captured as a keyed digest, through a key column declared keyed, never in the clear. Whether a real system's ids are opaque is assessed by its data owner, with the M5 privacy and legal review, at capture set-up.
  - project().
  - change-key/0, computed over the comparable projection, which leaves out SoR-computed, version and provenance columns.
  - The capture/0 profile check, with codes P_MAPPING_UNCLASSIFIED, P_SUBJECT_KEY_MISMATCH, P_CHANGE_KEY_MISMATCH and P_CAPTURE_SHAPE.
  - One shared report for profile findings: for each object, the first failure in the profile's fixed check order; the findings of all objects collected and sorted as store/0 sorts its own ([store/0](../spec/store-0.md#collecting-and-ordering-findings)). Profiles the store does not know are listed in it as unchecked. Later profiles (`approval/0` in EG1, `read-basis/0` in M4, Kernel/2 in M9) report in the same shape.
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
- Before M2 cuts its capture and change-key vectors, its first regeneration of the object and store vectors renames the placeholder system `rec-registry` and its capture service `rec-registry-cdc` to `example-registry` and `example-registry-cdc`. The ids of most object vectors, and the store export cases built from them, change; the JavaScript generator and the Rust tests change together ([D7](decisions.md#d7-the-vectors-are-the-contract)).

**Acceptance**

- The changekey and capture vectors (row, mapping, heads and clock in; exact object bytes or a profile report out) pass in Rust, JS, WASM and Python. An unlisted column gives P_MAPPING_UNCLASSIFIED. A subject key that differs from the key column, or from its keyed digest where the key column is keyed, gives P_SUBJECT_KEY_MISMATCH.
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
  - K is two parameters: one for coverage (all subjects every K runs) and one for when unchecked becomes unreachable. A deployment sets both, and each report records them, so the same inputs still give the same report content hash.
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

Design input: [turn-reader.md](turn-reader.md), the reader tools and AI agents call before they rely on evidence: what it verifies, its read receipt, and when a read that informs an action is recorded as an `assert`. Its decisions, settled on 28 November 2026 ([turn-reader open decisions](turn-reader.md#open-decisions)), shape the index and bundles below: a decision's basis goes in one `ext` profile, `read-basis/0`; how to prove completeness to a reader that does not hold the whole log is left to the bundle design; and the assert intake comes late in M4 (early M6 at the latest). Also [approvals.md](approvals.md), which proposes approver lists and approval rules as published data, an approval-status view, and hooks for tools; its views and labels are M4 deliverables below.

**Built ahead of M4: coordinate/0.** [coordinate/0](../spec/coordinate-0.md) says how a list of facets becomes one coordinate and back, using a dictionary that gives each facet its own prime: the dictionary format and its id, the coordinate as a canonical decimal string below 2^1024, five operations (encode, decode, divides, check and extends) with their codes, a tagged form, and rules for indexes. The dictionary format, its id and the five operations are built: crates/evidence-coordinate in Rust, tools/reference/coordinate.mjs as the independent JavaScript implementation, and test-vectors/coordinate/ on synthetic dictionaries, one of them Kernel/2's. The tagged form is specified, with no implementation or vectors yet. M4b, the first index format that reads it, defines its error codes and vectors, named in coordinate/0's `P_COORDINATE_` code space so that M4's bundles reuse them. The rules for indexes are what M4b below builds. coordinate/0 is a separate format outside the core ([D6](decisions.md#d6-one-small-core-many-profiles)) and changes nothing in evidence/0 or store/0. M4b below builds the index column on it.

**Deliverables**

- crates/evidence-index: index.db is derived from log.db behind a watermark (seq plus entry hash) and rebuilds deterministically. Index-vs-log reconciliation reports index_missing, index_extra and index_stale.
- Query API:
  - get: fresh verification on every read. Objects that fail are dropped and listed in rejected[].
  - by_subject: log order, with a stable seq cursor.
  - state: current, forked, deleted or absent, the latest assert per schema, and capture lag.
  - history: ancestors and descendants, with depth, via and truncated.
  - as_of on both clocks: claimed `at`, or recorded, meaning the largest seq with received_at at or before t.
  - Faceted filter over one short published list of non-personal operational facets, shared with M4b and AN1 and classified like captured columns ([analytics.md section 4](analytics.md#4-trends-proposed)), with coordinates and factor rows from [M4b](#m4b-coordinate-column-and-factor-table). Actor ids and approval status are never facets.
  - Reverse lookups by content hash, schema hash and signer.
  Every answer carries the watermark, the clock used, the echoed limits, and a truncated flag computed by fetching limit+1.
- Bundles: objects, attestations, entries, a covering checkpoint and inclusion proofs. `evidence bundle verify` works in the CLI, WASM and Python. A bundle can be sealed as an assert, which answers 'what the register showed on date X'. How to prove completeness (that these are all the heads at a checkpoint) to a reader that does not hold the whole log is deliberately left to M4's bundle design, which chooses between a commitment to all heads in a new checkpoint version and a keyed digest of subject and lane in a new log-entry version; either is a store/0 version change, and evidence/0 is unchanged ([turn-reader decision 10](turn-reader.md#10-proving-completeness-to-a-remote-reader)). Until one is built, such a reader reports *membership verified, completeness unknown*, and bundles and EG2 exports carry no completeness proof. Shipping the log prefix up to a checkpoint, or holding a local store, gives completeness meanwhile.
- crates/evidenced (axum and tokio): one process per store, holding the lease. Its read-only HTTP API mirrors the CLI. Bearer tokens map to actors and carry the actor's kind (`human`, `service` or `agent`), with read scopes per subject.system that are closed by default. A token issued to a tool acting for someone takes its read scopes from that person's grant (`approval/grant@0`), never wider. An item derived from more than one system, such as an aggregate analytics flag or a row of the analytics projection, is readable only with scopes for every system it draws on ([analytics.md 5.4](analytics.md#54-the-flag)).
- The approval-status view ([approvals.md](approvals.md)): for one object at a watermark, one of a fixed list of statuses (approved, awaiting approval, contested, superseded, unknown), with the rule set used, each step and who filled it, the escalations, the positions recorded, the lines that did not count and why, what would move the status on (`next`), and the label `not-established-before-key-binding` until M5. Each object is judged under the rule set in force at its own log position, and list membership at each approval line's own position; a later rule set changes no past status and is reported as `P_RULES_CHANGED`, for information. An object whose rule says it is compared, not approved, such as a captured connection record, is `unknown` with `P_COMPARED_NOT_APPROVED`, never `P_RULE_MISSING`. It is computed from the log and writes nothing. Lineage-style vectors, including vectors with more than one rule-set version, and an independent JavaScript reference ([D7](decisions.md#d7-the-vectors-are-the-contract)).
- The awaiting-approval report ([approvals.md section 11](approvals.md#11-advisory-reports-proposed)), built on the view: objects that have awaited approval, or stayed contested, for longer than a waiting time the deployment publishes as data, each with what it still needs. It lists objects, never people. The approval-spread report comes after M5, because it means little before keys are bound.
- A disagreement view per subject, for the edge profile ([cer-edge.md](cer-edge.md)): more than one head in a lane, a parent with children in more than one lane, the differences that the approval rules' comparisons find between a record and a linked statement and, only under a rule that names `record-differs`, between two captured records about one device. Separate detectors' chains in one flag lane are not a disagreement: heads there are grouped by the flag's detector, and only a fork in one detector's own chain is flagged ([analytics.md 5.4](analytics.md#54-the-flag)). It is computed from `prev` links over the whole history and writes nothing, with vectors for cases that must be flagged and cases that must not.
- A position across logs: a pair, a log id and a position in that log. At a receiving site (EG2) the views use it for the receipted positions of each gateway's replica. Content stays in its origin log; the views include a replica's position only once a receipt in the receiver's log names it, and order those positions by the receipt's position in the receiver's log, then by origin log position within the receipt, so the same receiver position always gives the same answer ([cer-edge.md section 6](cer-edge.md#6-moving-records-between-sites-proposed)).
- Read labels: every item a read returns gets the first label that fits: `observed` (a record lane written by capture), `note` (a continuity note), `tool-stated` (any other assert made for someone else, with `on_behalf_of`, whatever the actor's kind; every agent's assert has one) or `stated` (anything else, including a record-lane object not written by capture). Nothing is returned as observed unless capture wrote it.
- The read-basis profile `read-basis/0` ([turn-reader decision 2](turn-reader.md#2-the-basis-profile)), whose name is fixed before any EG1 vector cites it: its shape, a closed list of `left_out` reasons, an optional `scopes[]` member for a whole scope read, and the `P_BASIS_*` checks, including `P_BASIS_STALE` (a stale basis is recorded and detected), which the `basis-stale` condition in [approvals.md](approvals.md) uses, and `P_BASIS_CITED_NOT_READ`. Derived values appear in coordinate/0's tagged form; the vectors use a synthetic coordinate/0 profile, not Kernel/2 values. Vectors and an independent JavaScript reference. The read receipt gets a JSON Schema built from the basis's `as_of` and `heads` blocks, with no JavaScript reference.
- test-vectors/lineage: forks, merges, deletes, baselines, late arrivals, and as_of on both clocks. M4b ([below](#m4b-coordinate-column-and-factor-table)) adds a coordinate column and factor rows on a synthetic dictionary; it does not wait for a published one.
- Late in M4, or early M6 at the latest: an intake for asserts ([turn-reader decision 7](turn-reader.md#7-an-intake-for-asserts)). One request is one batch: the object and its attestation lines, with optional if_heads and write scopes per lane, and a place for other lanes' heads, so that M6 can add an optional same-transaction check without changing the shape. Before appending, it runs the pure checks: actor id form, an author line signed as actor.id, the kind of on_behalf_of, the record-lane writer, and grants.
  - It refuses a malformed identity (P_ACTOR_ID_FORM), and a write that fails an applicable authority check (P_AUTHOR_NOT_ACTOR, P_SPONSOR_NOT_ACCOUNTABLE, P_RECORD_LANE_WRITER, P_GRANT_*) or integrity check (a signature, hash or link that does not verify). It names every failed P_ code; a failed integrity check gives the single E_ code of store/0's intake. A refused object is stored nowhere.
  - Doubts about content quality or meaning are recorded as findings, not refused: a value outside an expected range, records that disagree, a missing optional field, a doubtful reading.
  - Verification repeats every check either way ([approvals.md section 13](approvals.md#13-proposed-codes)).
- No recall endpoint. M4 does not serve a tool's recall of its own earlier turns and does not become a general memory service. Continuity notes and their recall come at [M6](#m6-requests-and-the-outbox-as-evidence-in-shadow-mode-no-write-code), after M5's erasure exists; M4 gives notes their read label only.

**Acceptance**

- The lineage vectors pass in Rust, JS, WASM and Python.
- Golden queries on simulator data pass, including a late arrival that makes 'as of T (claimed)' differ from 'as of T (recorded)'. Every response carries a watermark and detects truncation.
- Dropping and rebuilding index.db gives an identical index digest, and a rebuild-equivalence proptest holds. Tampered index rows are classified as missing, extra or stale.
- An object byte changed behind the triggers is excluded from answers, listed in rejected[] and reported by `store verify`.
- A bundle verifies offline in the CLI, WASM and Python, and changing one byte makes it fail with a code. A reader that holds only the bundle reports membership verified, completeness unknown.
- HTTP answers are byte-identical to CLI answers. A token scoped to one system gets 403 for the others, and for an aggregate flag that draws on others, and a delegated token gets 403 outside its grant. Without the HMAC key, index.db holds no plaintext sentinel values.
- The approval-status vectors give the same status, steps and codes in Rust and the JavaScript reference at every log position they name, and each object keeps the status the rule set in force at its position gives.
- The intake refuses each malformed id and each failed authority or integrity check, naming every failed P_ code (or the single E_ code of a failed integrity check), and stores nothing for it; an object with a content doubt lands, with its finding.

#### M4b Coordinate column and factor table

Planned, about 1-2 engineer-weeks. The faceted filter gets coordinates and answers from factor rows, as [coordinate/0](../spec/coordinate-0.md#indexing) sets out ([D14](decisions.md#d14-coordinates-travel-as-decimal-strings-indexes-answer-from-factors)). Synthetic data and a synthetic dictionary only.

**Deliverables**

- A synthetic `coordinate-dictionary/0` for the simulator's records, drawn from the published facet list that M4's filter uses, and a stated rule for which facets each record has. Installation and device records get a synthetic dictionary and profile tag of their own; Kernel/2 stays for certificate lifecycles.
- Reading a tagged form ([coordinate/0](../spec/coordinate-0.md#tagged-form)): error codes for a tagged form that is not three string members, whose profile does not match the named dictionary's, or whose dictionary the reader does not hold, named in coordinate/0's `P_COORDINATE_` code space, with vectors. M4's bundles reuse them.
- In index.db, behind the same watermark: one row per object with its profile and its coordinate, a decimal string in a text column; factor rows (profile, object, prime, exponent), one per prime, with an index on (profile, prime); and, for each profile, the id of the dictionary used. An object whose facets do not encode gets no rows and is reported with encode's error.
- The faceted filter decodes the query with coordinate/0 first, and returns the decode error for a query that does not decode, never an empty result. It then answers from the factor rows: every prime of the query, each with at least the query's exponent. Answers carry the watermark and the dictionary id.
- No arithmetic on coordinates in SQL integer or floating-point types, and no posting list kept as one value that is rewritten on every add. A new dictionary version is taken only if it extends the one held.

**Acceptance** (moved from M9 on 28 November 2026)

- On 100k synthetic objects, queries answered from the factor rows return exactly the same rows as divisibility computed with exact arithmetic, including coordinates above 2^63, and the same error for queries that do not decode.
- Every coordinate is a decimal string that decodes to exactly its facets.
- An unknown facet gives a finding, and the object is never indexed under a partial coordinate.
- The tagged-form vectors give the same codes in Rust and the JavaScript reference.

### M5 Real-data gate: trust, time, replication, privacy operations

Make M1-M4 safe to run beside a real system of record on real data. That needs keys bound to people, independent time, copies off the machine, erasure, access control, legal sign-off, and, for each real system, an agreement with its owner on read access and on how each captured column is classified, starting with one system and one record type.

**Deliverables**

- Key-binding objects: an administrator's approve binds a key to an actor id, with a validity period and revocation. They cover persons, services, organisations, gateways and agents; an agent's binding names its sponsor, the person or organisation accountable for it, and a gateway's binding also names its log id and store key. An organisation's approvals are signed by persons listed as acting for it, so approvals need person keys only. The id is not the key, so a key can be rotated or revoked without the id changing. Each binding names its signature algorithm, and an id may have more than one bound key, so a post-quantum key can be added later without changing any id. Verification gains S_KEY_UNBOUND and S_KEY_REVOKED, and M3 resolutions are re-checked against the bindings.
- Approver lists ([approvals.md](approvals.md)) become key-bound: an approval counts only when its key is bound to the listed id at the line's log position.
- The id directory, which maps pseudonymous actor ids to people, is held off the log and access-controlled, with its own access log. It is never appended to the store. Each party has one pseudonym per directory.
- Signer backends for PKCS#11 (tested against SoftHSM in CI) and a cloud KMS. RFC 3161 timestamp tokens are recorded on checkpoints as witness attestations. Which backend and which timestamp authorities a deployment uses are its own parameters, under the system owner's security policy.
- Sealed exports are replicated to at least one copy held by a party independent of the store's operator: an object-lock bucket, a git remote, or both, named by the deployment. A pull either completes or changes nothing. Consistency proofs run between published checkpoints, with codes S_REPLICA_TRUNCATED and S_REPLICA_FORK.
- Erasure deletes a blob and appends a redaction assert, and verify then reports the blob as erased, not missing. By default a redaction counts only when it is approved under a published rule, with a bound key, by a role the data owner names on an approver list (for example its records manager); it cites the data owner's disposal authority by reference, and the blob is deleted only after a check that no disposal rule or pending access request stops it. A leak scanner covers the log, exports, blobs, the index, logs and HTTP responses, and its seeded sentinels include directory entries (synthetic ids and the personal details they map to) that exist only in the directory, so finding one anywhere else shows that the directory leaked.
- A container image, a runbook (including how to restore from a replica within a stated recovery time), health and metrics (capture lag, last reconcile, open findings, watermark) and a security description against the Australian Signals Directorate's Information Security Manual (ISM), for an external review started early. It covers the store, the M4 service and its tokens, the signer backends, the replicas, the id directory, the gateways and the link-domain key service. A legal review covers the store's status under the Privacy Act 1988 (APP 11), the Archives Act 1983 (including records disposal under section 24) and the Freedom of Information Act 1982, and also the sources [analytics.md section 8](analytics.md#8-privacy-records-and-fairness-proposed) lists, pseudonymous approval history, and how erasure fits records disposal rules. Its scope is set now, and it is commissioned before M5 starts.

**Gated:** the legal review, the security review, each real system's owner and data owner, the participating organisations and each deployment decide the points listed under [open questions settled](#open-questions-settled-confirmed-28-november-2026), among them the key-binding design, the pseudonym form, which body keeps the directory, who authorises an erasure, and who holds the replica. Until each has decided, the defaults above run on synthetic data only, and nothing real proceeds.

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
- sync/0 profile. The request (intent) is an assert on the SoR subject. Its content is the full proposed comparable projection, built at intake from the base head plus the requested field changes. The actor is a person, or a tool (a service or an agent) with on_behalf_of, which is then required. A tool of any kind, an agent included, files a request only when a person starts it, for a change that person could request themselves, with that person in on_behalf_of; the tool's grant carries a request scope (an EG1 shape). How intake tells that a person started a tool's request (for example that person's `approve` in the same batch, or a grant for that one request) is fixed in M6's design, before the outbox vectors are cut. ext carries op, base, base_sor_version, change_key, app, and the register and mapping refs.
- Intake checks the person's identity. A register violation is refused with field:code and nothing is stored. Dedupe on the change key returns the existing request. The request object is itself the queued state, so intake is one atomic append.
- Transitions are value-free asserts on subject evidence-sync/intent/<id>, appended with append_if_heads.
  - Members are a closed list: intent, from, to, reason code, attempt, epoch, receipt, sor_version, observed_hash.
  - Retry, discard and park are allowed only for a human carrying an approve attestation from a bound key.
  - An overwrite on a stale base needs two people.
  - Every chained append uses the service-time rule.
- The working outbox view in index.db is rebuilt from the transitions. The state() view overlays a caller's pending requests. Reconciliation compares requests with observations (shadow_match, shadow_differs, superseded) and the outbox view with the transitions. HTTP: POST /intents, GET /intents/{id}, and a held list.
- Proof that the default build has no delivery path: a compile_fail doctest, plus a cargo feature-graph check that no build for real data enables the `writes` feature.
- M6's design decides whether to add an optional check, in the same transaction as the append, that the lanes a basis read still have the heads it names: per decision schema, and only for actions that write nothing to the system of record. The intake's request shape already has a place for it.
- Continuity notes for tools and their recall, built at M6 on the assert intake, once M5's erasure exists ([turn-reader decision 13](turn-reader.md#13-continuity-for-tools)): by-reference asserts under a grant, in a dedicated notes system, labelled `note`, and returned to the grantee only while the grant is in force and its recall window is open on the store's clock. Recall is an exact read of the grant's notes subject, with no ranking or similarity search. By default a grant's `recall_until` is no later than its `valid_until`. Until the M5 legal review decides what a note body may hold, a body holds ids, codes and hashes only; how long bodies are kept waits for the data owner and each deployment's records authority, with that review.

**Acceptance**

- The outbox vectors enumerate every (state, event, actor kind) triple. Allowed edges give the expected transition and all others give P_INVALID_TRANSITION. A retry by a service gives P_RESOLVER_NOT_HUMAN, and a retry without approval gives P_APPROVAL_REQUIRED.
- The same request from two people at different times gives one request object. A register violation is refused, and its sentinel value is absent from every byte of the store.
- Two concurrent transitions on one request: exactly one lands, the other gets S_STALE_HEADS, and the log never shows two heads.
- Shadow E2E: simulator staff apply the requested change by hand, and the report shows shadow_match through an equal change key, with no change to the SoR schema. A different manual change gives shadow_differs.
- The rebuilt outbox view equals the working view. The observe build compiles with no delivery code, shown by the compile_fail test.
- A request filed by a tool that no person started is refused. Notes come back labelled `note`, never `observed`, and only inside the grant's recall window.

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
  - every confirmed request is observed within N capture cycles or reported as confirmed_unobserved, where N is a parameter per source, set from its capture route and cadence.
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

### M9 (optional) centr/0 profile, register views and a published dictionary

Answer domain queries and publish registers as views over the store, with a real published dictionary for the coordinates that M4b indexes, and build Kernel/2 for certificate lifecycles. Coordinates stay derived and checked, and never part of an id.

**Deliverables**

- A centr/0 profile verifier and vectors. It covers first the three public registers the [notes](centr-profile-notes.md) name (certified products, installers and suppliers), then installations, with a table of what each record type must and may contain and a must-pass and a must-fail vector for each rule, including the absent cases from docs/centr-profile-notes.md.
- Register snapshots as asserts whose content is a query result at a watermark.
- A real published facet dictionary, a `coordinate-dictionary/0` versioned by content, in place of M4b's synthetic one, with the profile's published mapping from source values to facets. It is drafted from public category lists only, labelled unofficial (not issued by any agency), kept under a profile id the project controls, and each version is published as an evidence `assert`, so which version was in force at a log position can be proved. Installation and device records get a dictionary and profile tag of their own. Every coordinate carries its dictionary's profile id and indexes key on (profile, coordinate), with the coordinate's name where a profile derives more than one ([coordinate/0](../spec/coordinate-0.md#tagged-form); [Kernel/2](kernel-2.md#33-representation-declared)). A shared registry of prime ranges is optional. **Gated:** an external publisher and a first consumer decide when a real dictionary ships. Until then every coordinate is computed on a synthetic dictionary, and none for a real record.
- Kernel/2 ([kernel-2.md](kernel-2.md)): its events, in the record lane, its two coordinates S and H, and its gates, under the rules settled on 28 November 2026 ([Kernel/2 section 8](kernel-2.md#8-open-questions)): the three-row contradiction table; a constrained `remove` list, with no event that exits a block and also names its entry or one of its states, and no facet in both `add` and `remove`; an exit only from a block already entered; S as a set; merges in topological order, with ties broken by (`at`, id); and retirement adding `registry.executed` only. Its `P_KERNEL_` codes, with event-chain vectors (merges, `remove` lists, failed events and a chain that mixes versions) and range vectors at the 2^1024 bound, in the shared profile report, and an independent JavaScript reference. Nothing of Kernel/2 beyond coordinate/0 is built before M9.

**Acceptance**

- The centr/0 vectors pass, including the absent cases.
- A register snapshot recomputed at its recorded watermark is byte-identical.
- The published dictionary loads under coordinate/0 with its stated id, each later version extends the one before it, and M4b's acceptance test passes with it in place of the synthetic dictionary.
- The Kernel/2 vectors give the same S, H and codes in Rust and the JavaScript reference, including merges, `remove` lists, failed events, a chain that mixes versions and the 2^1024 bound, and the values of Kernel/2's section 5 example are unchanged.

## Edge track

The edge track adds records about consumer energy resources devices: an installer's commissioning record, a network connection record, and a gateway's own settings or measurement summary. The profile is a draft in [cer-edge.md](cer-edge.md); the approvals it relies on are in [approvals.md](approvals.md). The track sits under the milestones above, so this stays the only roadmap. Nothing in it is built.

- **Two kinds of edge object, chosen by record type.** A record held in another system enters as a record change observed by a capture service, in the record lane. A gateway's own settings or measurement summary is an `assert` by the gateway, with an opaque service id.
- **The installer approves the captured record** with an `approve` attestation. A published approver list says whose approvals count, and approval rules say when a listed reviewer is also needed.
- **Connection records are compared, not approved.** A captured network connection record has an explicit rule with no approval step. The source system's own workflow stays authoritative for the connection itself; this layer records and compares.
- **No writes and no automated decisions.** The edge track adds no write path to another system, and only synthetic data is used until M5. A difference between two records is a prompt for review, not a conclusion, and a person resolves it: one resolution, approved by one listed reviewer, never a count of positions. Each status says what is still needed and who can still supply it, so uncertainty is shared and a person can decide.

| Step | Content | When |
|---|---|---|
| EG1 | The edge profile and the `approval/0` shapes, with vectors built from the profile's examples: approver list (with `for` on entries and a reason code on versions), rules (with a role-to-list member), position, resolution and grant (with its request scope). The example rule set: the installer's approval only, with a reviewer added when a named recorded condition holds; the example's five conditions plus `follow-up-pending`; `check-flagged` counting the form check's two results from the reviewer role or tools under a reviewer's grant; the five reconciliation classes; no analytics detector opted in; `basis-stale` and `record-differs` left to deployments; and connection records compared, not approved, so EG1 needs no organisation signing ([approvals.md 5.2](approvals.md#52-approval-rules)). A synthetic edge workload for the M2 simulator. A gateway deployment profile ([cer-edge.md section 6](cer-edge.md#6-moving-records-between-sites-proposed)): batched commits, acknowledged only once committed; restore or a new log, by case; rotation, never pruning | Alongside M2 |
| EG2 | Sync between sites: content stays in its origin log. The receiver keeps a verified replica of each gateway's log and records a receipt `assert` in its own log for each position it accepts (origin log id, position and entry hash); its views include only receipted positions, ordered by the receipt's position in its log and then by origin log position ([cer-edge.md section 6](cer-edge.md#6-moving-records-between-sites-proposed)). Full ingest is not adopted. The receipt profile, exports that end at a checkpoint and carry no completeness proof, and a check for two histories signed by one key, with vectors | With M4 |
| EG3 | Gateway key binding (signing key, log id and store key; by default generated on the gateway, and held in hardware for any trial with real devices), the digest route (the gateway asks the data owner's key service at commissioning or replacement), one or two transports the operator's security policy allows, an ARM build, and the commit interval measured on target hardware. Each of these waits for the party who decides it, as do where installers sign and who holds their keys ([gated](#open-questions-settled-confirmed-28-november-2026)). A trial with real devices only after M5, with legal review and the agreement of the device owners and of the systems' owners | After M5 |
| EG4 | Edge rules that need more than one listed person in the same role (`count` above 1), only if a use needs them. Before keys are bound, two names need not be two people | After M5 |

The edge track is not yet sized; each step adds to the milestone it sits under. EG1 is sized before M2 starts, EG2 within M4's sizing, and EG3 and EG4 after M5.

## Analytics track (proposed)

The analytics track would add trends and anomaly flags computed from the log. The design is a draft in [analytics.md](analytics.md). Like the edge track, it sits under the milestones above, so this stays the only roadmap. Nothing in it is built, and it uses synthetic data only until M5.

- **The log is the evidence base, not the engine.** Counts and baselines run in a projection rebuilt from the log behind a watermark, and every trend and flag names the log position and clocks it used, so anyone holding the log can recompute it.
- **Flags are facts against published rules.** A flag states what was measured and the published threshold that fired. It is a prompt for review, not a conclusion, and it adds a review step only under the published approval rules ([approvals.md](approvals.md)). It never changes an approved object's status by itself, and the layer makes no automated decisions. Conditions about a device, such as its firmware or battery capacity, are detectors of this kind with published rules, never scores.
- **Facts, not scores.** A flag states a measure, the published rule or threshold that fired, and a result code; `analysis/flag@0` has no score member. Advisory model scores about devices, product models or data quality are not adopted and not planned ([analytics.md section 3](analytics.md#3-the-scoring-question)). Nothing scores, ranks or profiles people or households: trends are never grouped by person, installer, reviewer, gateway or household, and small counts are suppressed.

| Step | Content | When |
|---|---|---|
| AN1 | The detector rule set and flag shapes, with no score member, and a compact run report for every run; a projection in a service of its own, rebuilt from the log behind a watermark; rule-based detectors with must-fire and must-not-fire vectors on the M2 simulator's workloads; an independent JavaScript reference for the shapes and the pure methods | With M4 |
| AN2 | Trends with small-count suppression; record-level flags routed to review through `check-flagged` in the approval-status view, for detectors a deployment opts in; aggregate flags in advisory reports; outcomes recorded by people as `analysis/outcome@0` | After the assert intake (late in M4, early M6 at the latest) |
| AN3 | Real data, only after M5 and legal review, with the agreement of each system's owner, and once the gated points of [analytics.md section 12](analytics.md#12-open-questions) are decided. No model work: flags stay facts only | After M5 |

The analytics track is not yet sized; each step adds to the milestone it sits under. AN1 is sized inside M4, AN2 with the intake, and AN3 after M5.

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
7. **Capture route for real data (needed before the first real capture, after M5; corrected on 28 November 2026 from "before M2 ends")**
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
5. **Approval is spread in that spirit; ids are pseudonyms; tools are actors.** Approval rests with the parties on published approver lists, and actor ids are pseudonymous. An AI agent is one kind of actor, not only a reader or contributor. The core and the hooks are the same for every tool: grants, verified reads, recorded checks and doubt, witnessing, and continuity notes kept between sessions. A tool never fills an approval step or settles a disagreement: settled on 28 November 2026 as a rule of the `approval/0` profile, not of the core ([approvals.md open question 4](approvals.md#15-open-questions)).

## Decisions (confirmed 28 November 2026)

1. **Reproducible faceted queries join the main proposition.** The [README](../README.md) now says: "Third parties can reproduce both the evidence and the exact results of faceted queries over it: the same dictionary version, watermark and query give the same rows in any implementation. A coordinate decides which records a query returns, not whether a record was classified correctly or captured completely."
2. **Prime coordinates come forward.** A short spec for computing coordinates, [coordinate/0](../spec/coordinate-0.md), is written now, with vectors on synthetic dictionaries, a Rust implementation and an independent JavaScript one. In M4, the coordinate column (M4b) moves from optional to planned, and the equivalence acceptance test moves from M9 to [M4b](#m4b-coordinate-column-and-factor-table), on synthetic data. M9 keeps the centr/0 profile, register views and a real published dictionary.
3. **Two wording fixes.** The [Risks](#risks) no longer list prime indexes with embeddings as fuzzy recall: prime coordinates are exact. The README no longer says a faceted query is "a divisibility test on one integer field, which any database or language can run": the coordinate defines what a query means, and databases answer it from a factor table.
4. **Coordinates travel as decimal strings, and indexes answer from factors** ([D14](decisions.md#d14-coordinates-travel-as-decimal-strings-indexes-answer-from-factors)). SQL integers are 64-bit. SQLite turns a decimal-text coordinate that does not fit into a floating-point number and returns a wrong remainder, silently: on SQLite 3.53, the product of the first 16 primes, a 65-bit number stored as text, gave 33 for `% 53` instead of 0, and an 89-bit product gave 33 too. PostgreSQL's `numeric` takes `mod` exactly at any size coordinate/0 allows. So the canonical coordinate is a decimal string. Indexes store the factorisation as rows (profile, object, prime, exponent) with an index on (profile, prime), and answer "has all these primes with at least these exponents". Divisibility is the reference meaning, and an acceptance test is to show that both give the same rows, including coordinates past 2^63. Whole posting lists are not stored as one value rewritten on every add. Divisibility does not make retrieval faster, since a scan reads every row; speed comes from the indexed factor rows. The arithmetic is long established: it is the idea behind Gödel numbering.
5. **Kernel/2 and the device index are separate questions.** coordinate/0 decides none of [turn-reader decision 9](turn-reader.md#9-kernel2-rules-the-reader-depends-on) (Kernel/2's scope, the order within an event, and whether installations and devices get their own dictionary), the device faceted index in [cer-edge.md](cer-edge.md), or [Kernel/2's own open questions](kernel-2.md#8-open-questions). They were settled later the same day ([below](#open-questions-settled-confirmed-28-november-2026)).

## Open questions settled (confirmed 28 November 2026)

The project maintainer adopted the design notes' proposals as the working rules, with the definite choices listed below. Each note's open questions now carry a decision, and the milestones above follow them. A decision that belongs to someone outside the project is marked gated in its note, with the default the build uses on synthetic data; nothing real proceeds on a gated point until that party has decided. One question stays open: how one installation job is kept from needing two sign-offs, until it is clear which duplication is meant ([cer-edge.md question 6](cer-edge.md#10-open-questions)).

The maintainer's judgement calls:

1. **Connection records.** EG1's example rule set gives captured network connection records an explicit rule: compared, not approved, with no approval step. The source system's own workflow stays authoritative for the connection itself; this layer records and compares ([approvals.md 5.2](approvals.md#52-approval-rules)).
2. **Continuity notes at M6.** Notes and their recall are built at M6, on the assert intake, after M5's erasure exists. M4 serves no recall endpoint and does not become a general memory service ([turn-reader decision 13](turn-reader.md#13-continuity-for-tools)).
3. **Intake refusals.** The assert intake refuses malformed identities, and writes that fail an applicable authority or integrity check, naming every failed code. Doubts about content quality or meaning are recorded as findings, not refused ([turn-reader decision 7](turn-reader.md#7-an-intake-for-asserts); [approvals.md section 13](approvals.md#13-proposed-codes)).
4. **Sync between sites.** Content stays in its origin log. A receiving site keeps a verified replica of each gateway's log and records a receipt in its own log for each position it accepts. Its views include only receipted positions, in one fixed order across logs ([cer-edge.md section 6](cer-edge.md#6-moving-records-between-sites-proposed)).
5. **Retirement.** A Kernel/2 retirement adds `registry.executed` only, not `universal.exit`, so universal state that stays true of a retired unit stays in S ([Kernel/2 section 8](kernel-2.md#8-open-questions)).
6. **The remove list.** A Kernel/2 event may carry a constrained `remove` list to change a state, and must not exit and re-enter the same block. Since exits are not applied first, an event also must not exit a block and add or remove one of its states ([Kernel/2 section 8](kernel-2.md#8-open-questions); [turn-reader decision 9](turn-reader.md#9-kernel2-rules-the-reader-depends-on)).
7. **Facts-only analytics.** Flags are facts against published rules, which anyone holding the log can recompute exactly; AN1's `analysis/flag@0` has no score member ([analytics.md section 3](analytics.md#3-the-scoring-question)).
8. **Step-1 precedence.** For bytes with more than one step-1 fault, evidence/0 reports `E_PARSE`, then `E_NUMBER_RANGE`, then `E_DUPLICATE_KEY`, backed by vectors with several faults ([D15](decisions.md#d15-step-1-faults-are-reported-in-a-fixed-order)).
9. **Completeness proofs.** The choice between the two proposed constructions is left, deliberately, to M4's bundle design. Until then, a reader that does not hold the whole log reports *membership verified, completeness unknown* ([turn-reader decision 10](turn-reader.md#10-proving-completeness-to-a-remote-reader)).

Gated decisions, by who decides. Each note gives the default.

- **The M5 legal review:** the pseudonym form (M5); what a continuity note's body may hold (before notes are built at M6) and, with the data owner and each deployment's records authority, how long it is kept (before notes hold real data); who authorises an erasure, with the data owner (M5); how long flags and run reports are kept, with the records authority of the body that holds them, and, with that same body, whether a detector is an AI use case under the Digital Transformation Agency's policy (AN3); and, with deployments and data owners, the small-count minimum N, the hold-back rule, exact numbers in flags, whether any aggregate or threshold leaves a deployment, and how a party named in a flagged record learns of it (AN3). Its scope is set now, and it is commissioned before M5 ([approvals.md questions 6 and 8](approvals.md#15-open-questions); [analytics.md section 12](analytics.md#12-open-questions); [cer-edge.md section 7](cer-edge.md#7-privacy)).
- **The M5 security review,** with each deployment's identity owner and security policy: the key-binding design and the signature algorithms it allows, whether gateway keys must be held in hardware, and the review's own scope. Before M5 and EG3 ([M5](#m5-real-data-gate-trust-time-replication-privacy-operations); [cer-edge.md question 5](cer-edge.md#10-open-questions)).
- **Each real system's owner and data owner:** read access and how each captured column is classified; the capture route; whether its record ids are opaque; who holds each link-domain key, and how a gateway gets its keyed digests; who may read analytics flags and the projection, with the deployment; the change volumes that set the append-rate target; any availability requirement; where the attendance-photo consistency check lives (by default in the system of record, with its result recorded as a check; M9); and whether the layer ever writes to the system, with the provenance side table that writes need. Before the first real capture after M5, EG3 and AN3, and before any real write ([cer-edge.md questions 3 and 4](cer-edge.md#10-open-questions); [26 November decisions 7 and 8](#decisions-confirmed-26-november-2026); [M7](#m7-write-through-to-the-synthetic-database-only-writes-opt-in)).
- **Each deployment,** with the bodies it names: who publishes each approver list and who its first administrators are; which body keeps the directory; who binds and revokes an installer's key; who holds the independent replica, with that party's agreement; the signer backend and timestamp authorities; the transport between sites; how each trial system's records are classified, with its owners; and reviewers' capacity for flags; and, with its legal review, what it publishes within the default boundary (the roles on its approver lists, its rule content, aggregate reports). Before M5, EG3 and AN3 ([approvals.md section 3](approvals.md#3-parties-and-ids-proposed) and [questions 1 and 6](approvals.md#15-open-questions); [cer-edge.md questions 3, 6 and 8](cer-edge.md#10-open-questions); [analytics.md question 2](analytics.md#12-open-questions)).
- **Participating organisations:** who acts for each, before M5 binds keys for it ([approvals.md question 9](approvals.md#15-open-questions)).
- **Installer-app vendors,** with the deployment: where installers sign `approve` and who holds their keys, before EG3 ([cer-edge.md question 6](cer-edge.md#10-open-questions)).
- **Device owners,** with the systems' owners and the legal review: whether and when a trial with real devices runs; and, with the deployment, who holds a gateway's keys. Before EG3 ([cer-edge.md question 5](cer-edge.md#10-open-questions); [section 8](cer-edge.md#8-fit-with-the-roadmap)).
- **Source operators and detector owners:** whether a real source's late claims are ever rejected, which, if ever built, is a recorded per-source policy above store/0, never in it; and a real source's settle window. After M5, before AN3 ([26 November decision 9](#decisions-confirmed-26-november-2026); [analytics.md question 10](analytics.md#12-open-questions)).
- **An external publisher and a first consumer:** when a real dictionary ships, for centr/0 and for installation and device records (M9; [turn-reader decision 9](turn-reader.md#9-kernel2-rules-the-reader-depends-on)).

## Risks

- This repository assumes nothing about any real system of record: its technology, whether a replica or extract is available, whether CDC is permitted, whether it is a vendor platform with only an API, and whether it has version columns. This shapes M2 and M3. Mitigation: SorReader and ChangeSource traits with snapshot, change-file, CDC and HTTP adapters, so that capture can follow whichever route a system's owner agrees to.
- Privacy and records law. An append-only log cannot forget, so a field classified wrongly is permanent. Mitigation: fail-closed mapping, CI sentinel leak tests, an erasable blob store outside the log, synthetic data only until M5, and legal advice before any real data on the Privacy Act (APP 11), the Archives Act 1983 and FOI, and, for analytics, on the sources [analytics.md section 8](analytics.md#8-privacy-records-and-fairness-proposed) lists.
- False drift from normalisation (NUMERIC 6.6 vs 6.600, DATE vs TIMESTAMP, time zones, NULL, trailing spaces) will quickly erode trust in reconciliation. Mitigation: one versioned normalisation shared by projection and the comparator, covered by vectors, plus a CI clean run that must report zero findings.
- Until M5, signatures prove only that a key holder signed. Approvals and resolutions are not accountability evidence before key binding. Before KMS and independent time are in place, a stolen store key could forge history after the last externally witnessed checkpoint.
- Polling loses history. Snapshot capture sees only the state at each poll, so intermediate versions are never evidenced. Objects and the UI must say 'as observed', and CDC should be used where completeness matters.
- The change key has subtleties. SoR-computed columns (updated_at, row versions, trigger output) must be left out of the comparable projection, or confirmations never match. An identical manual change looks like a confirmation: that is deliberate in shadow mode, but once writes are on, telling them apart needs the provenance side table, which the SoR owner must approve.
- Single node, single writer, so no high availability. M5 adds replicas, not a warm standby: recovery means restoring from a replica within a stated recovery time. A standby, which needs lease handover, epoch fencing and continuous replication, is added and sized only if a system's owner requires high availability, before any real write path. The flock lease needs a local disk, not NFS. An fsync on every commit limits the append rate; capture batches its commits, and so does a gateway, to limit flash wear, with its interval measured in EG3. The rate is measured on reference hardware before any real use; the target and the hardware are set per system once its owner supplies change volumes, with gateway hardware for EG3, and are not recorded here. Bounded-memory verification, a stored compact Merkle frontier and segment rotation inside log.db are not scheduled. They are taken up when a measured need shows: the trigger is set from the nightly bench_append numbers (append, checkpoint and verify times at 1,000,000 entries) and M5's volume estimate.
- Timeline: about 31 engineer-weeks for one engineer to reach writes to the synthetic database (8-9 months with contingency), with memory reads at about week 18 and the write half last. These figures predate the additions of 27 and 28 November 2026 and the edge and analytics tracks, which will lengthen them once sized, and leave out M4b (1-2 weeks), planned since 28 November 2026. Mitigation: each milestone ships on its own. The timeline is refreshed once the [sizing steps](#milestones) are done, before M2 starts, and whether a second engineer builds M4 alongside M3 is decided on those numbers.
- Perception: a system's owner may read any write path as drift toward 'instead'. Mitigation: record D11 now, ship builds for real data that provably contain no delivery code, enable writes per record type under the system owner's published allow-list, always deliver through the SoR's own interface, and treat writes to any real system as a separate decision.
- Dependency weight goes against the project's minimal-dependency stance (D8, D9): rusqlite bundles C SQLite, and axum and tokio arrive in M4. Mitigation: keep them out of evidence-core, evidence-log, the WASM build and the Python build, enforced by a CI guard.
- Postgres logical decoding tooling in Rust is thin, and the consuming slot function loses changes on a crash. Use peek followed by advance after a durable commit, and a custom CI image with wal2json. Keep it optional until a real system's stack is known.
- Scope creep toward fuzzy recall (embeddings, ranking, similarity search, token indexes). Retrieval stays exact, bounded and verifiable. Prime coordinates are not fuzzy recall: they are an exact derived index, answered from factor rows that are checked against divisibility, with no embeddings, ranking or similarity. Continuity notes for tools (M6) are governed asserts under a grant, read back only inside the grant's recall window, and M4 serves no recall endpoint.
- Coordinates in SQL. SQL integers are 64-bit and floating-point types are exact only to 2^53, so arithmetic on a larger coordinate can give wrong rows with no error (SQLite does so even for a coordinate stored as text). Mitigation: coordinates are decimal strings in text columns, queries are answered from factor rows, and M4b's acceptance test compares them with exact divisibility past 2^63 ([D14](decisions.md#d14-coordinates-travel-as-decimal-strings-indexes-answer-from-factors)).
- A review status can be misread as a conclusion about someone. Statuses are review statuses with no effect outside this layer, and a difference between records is a prompt for review, not a conclusion. Before M5 no approval is accountability evidence, and every status output says so.
- Public-repo hygiene: synthetic data and pseudonymous actor ids only; new profile vectors use the opaque id form of [approvals.md](approvals.md#3-parties-and-ids-proposed). Small inconsistencies slip in easily (an early vector's subject key disagreed with its content, since fixed); M2's subject-key check rejects such a mismatch in captured records.
