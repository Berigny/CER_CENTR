# Edge devices: a draft profile for consumer energy resources records

**Status:** draft for discussion, 27 November 2026. A proposed profile, and an edge track under the [roadmap](roadmap.md), which stays the only roadmap. Nothing here is built unless it is labelled **[built]**. This repository is not affiliated with any agency, and this document does not describe any agency's process.

**Scope:** records about edge devices, using synthetic data only until the pilot gate ([M5](roadmap.md#m5-pilot-gate-trust-time-replication-privacy-operations)). The layer never writes to another system. It makes no automated decisions and does not score households.

**Labels:** **[built]** is in the code at commit `a6bbb94`. **[proposed]** is a design for a later milestone. **[open]** is a decision still to be made.

**Names:** here "CER" means consumer energy resources (rooftop solar inverters, batteries, electric vehicle chargers and controllable loads). The Clean Energy Regulator is always named in full ([D1](decisions.md#d1-a-neutral-name)).

## In short

- **Records stay in their own systems.** An installer's commissioning app, a network connection portal and a gateway each keep their own records. This layer keeps verifiable evidence of what they held and said, beside them, and never writes to them.
- **Two kinds of edge object, chosen by record type.** A record held in another system enters as a record change observed by a capture service. A gateway's own settings or measurement summary is a statement (`assert`) signed by the gateway.
- **Approvals are signatures, and a published list says which count.** The installer approves the captured record. An approver list, published as data, says whose approvals count for which role.
- **Disagreement is shown, and a person resolves it.** When two records about one device differ, that is a prompt for review, not a finding. The system decides nothing; a listed person records the outcome.
- **Exports verify offline over any transport.** Objects are content-addressed and signed, so how a file travelled does not matter. A receiver relies only on what a signed checkpoint covers.
- **Built today:** the evidence format and the single-writer store (M1). **Proposed:** this profile, the approval-status view (M4), key binding (M5) and the edge track. Before M5, no signature here is accountability evidence.

## 1. Two kinds of edge object [proposed]

Records about one consumer energy device (an inverter, a battery, an electric vehicle charger) are kept in several systems: an equipment register, an installer's commissioning app, a network connection portal. Each keeps its own current state. Which kind of object enters the log depends on where the record lives:

| Record | Kept in | In the log | Lane | Signed by |
|---|---|---|---|---|
| Commissioning record | An installer's app (`installer-app-demo`) | `create`, `update` or `delete` by the capture service that observed it | `record` | Capture: `author`. Installer: `approve` |
| Connection record | A network portal (`network-portal-demo`) | The same pattern, under its own subject | `record` | Capture: `author`. Approvals as the published rules say |
| Gateway settings or measurement summary | The gateway (`edge-gateway-demo`) | `assert` by the gateway, `kind: "service"`, opaque id | `assert:edge-demo/gateway-settings`, or the summary's own schema | Gateway: `author`. Compared, not approved |
| Automated check of a record | A checking tool | `assert` with `on_behalf_of` naming the accountable person or organisation | `assert:edge-demo/form-check` | Tool: `author`. The `on_behalf_of` party: `approve` |

**A record change observed by capture.** The installer app's commissioning record, as capture saw it:

```json
{
  "v": "evidence/0",
  "id": "sha256:1b58519d6e2ea7b9dc42f2429f1f2d29a703b7c1bd3c4fcba85c5a6893f38455",
  "op": "create",
  "schema": { "ref": "installer-app-demo/commissioning@1", "hash": "sha256:1e96a0dfa88bb437f80a2cb90d5ed67cc67241b2d5466034d2044ec85825e584" },
  "subject": { "system": "installer-app-demo", "type": "commissioning", "key": "COM-TEST-000001" },
  "content": {
    "media_type": "application/json",
    "hash": "sha256:8b4199fa6bf7ff0e857561586ddaf06b93ad4adbd071120df3ec07b73c1f5875",
    "inline": {
      "commissioned_at": "2026-11-26T09:30:00Z",
      "commissioning_id": "COM-TEST-000001",
      "device_serial_digest": "hmac-sha256:48fd86a6794bad8917dd223d0c78290186611fec0760e74f2a75547d8d0c77c5",
      "export_limit_kw": 5,
      "installer_ref": "hmac-sha256:04d42d217df7830a613ded2960b601d244f8690c1772793fd0ff49d4a02f884d",
      "inverter_model": "INV-TEST-10K"
    }
  },
  "actor": { "id": "service:capture-installer-app-test", "kind": "service" },
  "at": "2026-11-26T09:35:00Z",
  "prev": []
}
```

- **`op` is `create`** because capture saw a new record. Later changes are `update`s that list it in `prev`. Record changes are only ever observations of a system of record ([D11](decisions.md#d11-observe-before-writing)), so only the capture services named in the deployment may write a record lane. store/0 does not enforce that, so a profile check flags any other writer (`P_RECORD_LANE_WRITER`).
- **No `ext` member here.** M2's capture/0 profile **[proposed]** adds an `ext` member to each captured object, keeping for example the raw source time ([M2](roadmap.md#m2-read-only-capture-from-a-synthetic-register)). It is left out of this example.
- **No personal identifiers are inline.** The device serial and the installer appear only as keyed digests (HMAC under a key the data owner holds, one key per link domain). A bare hash of a serial number can be reversed by guessing ([evidence/0 Privacy](../spec/evidence-0.md#privacy)). These values use a published test key. Other values are classified as [section 7](#7-privacy) says.
- **`5` and `5.0` have the same canonical form,** so the id does not depend on how the number was written.

**A statement by the gateway.** The gateway's own reading of its settings, half an hour after commissioning:

```json
{
  "v": "evidence/0",
  "id": "sha256:a37fb277a40307c5bfe642c7a145c5c8ff0f9ae3a9cb2968a6565f5f93cf06b3",
  "op": "assert",
  "schema": { "ref": "edge-demo/gateway-settings@1", "hash": "sha256:5afe0d2dcfdfd1933011b73386ff33b01dc0ddcdde2e380ad8b3300c87ac0c8a" },
  "subject": { "system": "edge-gateway-demo", "type": "inverter", "key": "DEV-TEST-000001" },
  "content": {
    "media_type": "application/json",
    "hash": "sha256:a5f88974702037ed612dd8d50b47f46614d68b32c12876b09c651c9f529405e0",
    "inline": {
      "device_serial_digest": "hmac-sha256:48fd86a6794bad8917dd223d0c78290186611fec0760e74f2a75547d8d0c77c5",
      "export_limit_kw": 10,
      "firmware": "sha256:0d948765dc775cdaca7ee92b3665d7be5a2e8048ec41db2789dc5c87e2ce4b7c",
      "read_at": "2026-11-26T10:00:00Z"
    }
  },
  "actor": { "id": "service:4275021583e8e644a10f5e5565e09600", "kind": "service" },
  "at": "2026-11-26T10:00:00Z",
  "prev": []
}
```

- **The gateway has an opaque id** because it sits at someone's premises, and a readable name could say where.
- **Subject keys are opaque record ids.** They are in the log in the clear. A device's subject key, such as `DEV-TEST-000001` here, is an opaque id assigned like the gateway's own actor id, never its serial number, a meter identifier or an address. A captured record keeps its own system's record id, such as `COM-TEST-000001`. Whether each system's record id is fit to be a subject key is [open question 3](#10-open-questions).
- **It states 10 kW where the record says 5 kW.** The two objects are about different subjects in different systems. Their only link is the equal `device_serial_digest`. [Section 4](#4-when-records-disagree) says what happens next.
- **A setting, not a measurement.** `export_limit_kw` is read from the device's configuration. A measurement summary states what was measured, for example `observed_max_export_kw` over a time window, because a measurement observes power, not a limit. Settings and measurement summaries are still information about a household; [section 7](#7-privacy) says how they are classified and kept.
- **Versions chain.** A later statement lists this one in `prev` and is appended with `if_heads`. In the checks, version 2 (5 kW) sent with `--if-heads none` was refused with `S_STALE_HEADS`, and sent with `--if-heads` set to version 1 it landed.

A faceted index for device records is not part of this profile. If one is wanted, it is a separate dictionary with its own profile tag at M9 ([turn-reader.md](turn-reader.md), open decision 9).

## 2. Store facts and two clocks [built]

- **Two clocks.** The object holds its writer's claim (`at`). The store keeps its own position (`seq`) and time (`received_at`) in a separate log entry, so an object's id is the same in every store ([store/0 Two clocks](../spec/store-0.md#two-clocks), [D13](decisions.md#d13-two-clocks)).
- **Late claims are counted, never rejected.** An object whose `at` is more than 300 seconds after its entry's `received_at` is counted in `counts.at_after_received`. It is never rewritten.
- **Checkpoints.** A checkpoint signed by the store key covers entries 1 to its `size`. `evidence store prove` gives inclusion and consistency proofs against it.
- **Compare-and-set.** `if_heads` refuses a stale append with `S_STALE_HEADS` and returns the current heads.
- **Time is claimed until M5.** Until M5 adds independent time, a gateway's `received_at` is that gateway's own claim.

## 3. Approvals [proposed]

### 3.1 Who signs what

| Who | Records | Signs |
|---|---|---|
| Capture | Record changes it observed | `author` |
| A gateway | Its own settings or measurements | `author` |
| The installer | Nothing new: accepts the captured record | `approve` |
| A listed reviewer | Accepts an object, or records a position or a resolution | `approve`, or `author` on their own assert |
| An automated check or an AI agent | Its output, as an `assert` with `on_behalf_of` | `author` |
| Anyone | That they saw an object at a time | `witness` |

**[built]** Signatures live outside the object, one attestation line each ([D4](decisions.md#d4-signatures-outside-the-object)). The purposes are `author`, `approve` and `witness`, and `evidence sign` refuses any other (`E_USAGE`). This is the installer's approval of the commissioning record:

```json
{"at":"2026-11-26T09:40:00Z","key":"ed25519:5wwKhX_d8QI9-z1hCe-ciALBs6LpLlThjWY1nqgTyYY","object":"sha256:1b58519d6e2ea7b9dc42f2429f1f2d29a703b7c1bd3c4fcba85c5a6893f38455","purpose":"approve","sig":"0eGRZlcJDWOZch5VJBks1nV6m8BG4tmTXGcfZ_o4W8KXyiogDATo8jOKVE4BcPAafFIuE50KWCWk40jZcD_UDQ","signer":"person:b0323effae5bd18bd0e5632e2b6cf284","v":"attestation/0"}
```

With capture's `author` line, `evidence verify --require author --require approve` gives `ok` with `{"approve":1,"author":1}`. With the author line alone, `--require approve` fails with `E_ATTESTATION_MISSING`. A listed reviewer's later `approve` makes it `{"approve":2,"author":1}`.

### 3.2 The approver list

Purpose alone cannot tell an installer's approval from a reviewer's, or from anyone else's: any valid `approve` line passes `--require approve`. So whose approvals count is published data, and an approval counts only when it comes from a listed approver: someone named on the approver list for the role a rule needs.

- **An approver list** (`approval/approver-list@0`) is an `assert` naming, by pseudonymous id, who may approve in which role, such as installer or reviewer. An installer's entry can carry the keyed reference the installer app uses, so a check can confirm that the installer who approved is the one the record names.
- **Approval rules** (`approval/rules@0`) say, for each kind of record, which roles must approve, how many distinct people in each, and which recorded facts add a review step.
- **Two administrators change them.** A new version of a list or rule set counts only with `approve` lines from two administrators named in the version it replaces. The example list (`sha256:0cdc21d0…`) carries one `author` and two `approve` lines (`{"approve":2,"author":1}`).
- **Keys are bound at M5.** Until then, list entries carry no keys, a signature cannot show that the signer holds any licence or accreditation, and approvals made with development keys are not accountability evidence ([roadmap decision 4](roadmap.md#decisions-confirmed-26-november-2026)).

The shapes, checks and codes are in the design note [approvals.md](approvals.md).

### 3.3 Automated checks and agents

- **A check informs; people decide.** A check result is an `assert` by the checking tool, naming the checked object by id and its rule set by hash. It never counts as an approval.
- **An AI agent is an actor** of kind `agent`, with `on_behalf_of` naming the accountable person or organisation. Its output counts as approved only through that party's `approve`. The example form check carries the tool's `author` line and the reviewer's `approve` (`{"approve":1,"author":1}`).

### 3.4 What the core counts today [built]

| Case | Core result | What the profile adds |
|---|---|---|
| The same `approve` line twice in one file | `ok`, `approve: 2` | Counts distinct people, not lines |
| `--require approve=2` | Usage error, exit 2 | The number required per role, as rule data |
| An agent's `approve` on its own output | `ok`, `approve: 1` | Only the `on_behalf_of` party's approval counts |
| A disagreement signed as its own purpose (`--purpose reject`) | Refused, `E_USAGE` | A position, recorded as an `assert` |

### 3.5 Review status is a view

Whether an object is approved is never a member of the object ([evidence/0 Not in v0](../spec/evidence-0.md#not-in-v0)). A view computes it from the log at a stated position, under the rules in force when the object was recorded. The answer is one of five words: **approved**, **awaiting approval**, **contested**, **superseded** or **unknown**. A missing rule, or a list with two current versions, gives unknown, and unknown is never read as approved or as refused. Until M5, every status says that accountability is not established. A status is a review status. It is not a finding about a device, an installer or a household, and it has no effect outside this layer.

## 4. When records disagree

What store/0 records today **[built]**, from the checks in the [appendix](#appendix-how-the-examples-were-checked):

| Case | store/0 | Result |
|---|---|---|
| Two capture `update`s replacing the same record version | Two heads in the record lane, counted | forks 1; an `update` listing both heads brings it to 0 |
| Two gateway statements replacing the same version | Two heads in an assert lane, not counted ([store/0 Lanes and heads](../spec/store-0.md#lanes-and-heads)) | heads 2; forks 0 |
| A statement lists P; a later one lists P and that statement | One head | Not a disagreement |
| Commissioning record 5 kW; gateway statement 10 kW | Different subjects | forks 0; not detected today |
| Same device in two systems (installer app 5 kW, network portal 10 kW) | Two subjects; nothing links them | forks 0; not detected today |

The approval layer adds **[proposed, M4]**:

- **Comparisons as rule data.** The rules name which values of a record to compare with which statements, linked by an equal keyed digest. When a current statement differs from the record, a review step is added.
- **A disagreement view** for each subject, built on M4's `state` and `history` reads and the approval-status view ([M4](roadmap.md#m4-memory-reads-index-query-api-bundles-read-only-service)): more than one head in a lane, a parent with children in more than one lane, and the differences the rules' comparisons find. It is computed from `prev` links over the whole history, with vectors for cases that must be flagged and cases that must not (the third row above).
- **Positions.** A listed reviewer who disagrees records a position (`approval/position@0`), an `assert` of their own. The object shows as contested until a person resolves it. A position never removes or alters an approval. A party can withdraw only their own approval, which then stops counting; the line stays in the log.
- **Resolution by a person.** A person records an `assert`, approved by a listed reviewer, naming the positions it settles and an outcome: the record stands, correct it at the source, correct it on the device, or escalate. This is the pattern of M3 resolutions. A profile check flags a resolution by a service (compare M3's `P_RESOLVER_NOT_HUMAN`); the store does not refuse it. A resolution never changes a record. Corrections happen in the system of record or on the device, and reach the log only through capture or a new statement.

A difference between records is a prompt for review, not a finding.

## 5. Spreading approvals and uncertainty [proposed]

The design does not depend on any one party running every system. It records how uncertainty is shared and where a person's decision is needed. Each operator stays accountable for its own system, and each log has one writer, which orders it; copies held by others and consistency proofs show whether a history was rewritten. Nothing is settled by counting objections. Instead:

- each party records its own view and signs it: capture what it observed, a gateway what it states, installers and reviewers what they accept or dispute;
- doubt adds a named reviewer, never a score;
- an unknown stays unknown and is shown, never treated as a pass or a fail;
- a named person decides, and the decision is recorded with what it relied on;
- a decision that needs more than one approver names the approver list and the number required as published data; the status view counts those approvals from M4, and none of them is accountability evidence until keys are bound (M5).

Actor ids are pseudonyms: `person:` or `agent:` with 32 random hex digits, `service:` or `org:` with a public name, or an opaque id for a device at someone's premises. Only an access-controlled directory kept off the log links them to people. An AI agent is one kind of actor, not only a reader. The same hooks serve an agent, a case screen and a batch job: a grant to act for someone, a verified read, a recorded statement, an approval by the accountable person, and continuity notes kept between sessions under the grant. The full design is in [approvals.md](approvals.md).

## 6. Moving records between sites [proposed]

**Transport.**

- Objects are content-addressed, attestations are signed and exports verify offline, whatever carried them.
- Entries after the last checkpoint are not yet signed, so a receiver accepts an export only up to its last checkpoint. In the checks, an export whose head was entry 20 verified `ok` with a covered size of 17.
- Any transport the operator's security policy allows: HTTPS upload, SFTP, object storage or removable media **[open]**.
- A transport's identity is not a signing identity. A gateway's signing key is bound at M5, never through the transport.

**Gateways.** A gateway that buffers through outages runs the native store (the command-line tool or the library): SQLite, a file lease on a local disk, and the store key for checkpoints. The WebAssembly module verifies, seals and signs, but has no store. Each gateway is its own log, with its own genesis and store key, and writes a checkpoint before every export. Not yet checked: an ARM build, flash wear from a sync on every commit, and keeping the store key in hardware (M5's PKCS#11 signer backend is the natural route).

**Sync [open].** A store/0 entry belongs to one log, and `store import` only builds a new store from an export. Two designs:

- **(a) Ingest.** The receiver verifies the whole export, then appends its objects and attestation lines to its own log in the sender's order, except the sender's genesis, checkpoints and their lines, which store/0 intake refuses (`S_RESERVED_SUBJECT` for the objects, then `S_ATTESTATION_TARGET` for their lines). Ids and attestation refs of everything else are unchanged; `seq` and `received_at` are the receiver's. A receipt `assert` carries the sender's genesis, last checkpoint and their lines instead, so the sender's facts stay provable. Re-running an ingest returns duplicates.
- **(b) One replica per gateway.** The receiver keeps each gateway's export as its own store. M5's replica checks apply as they are, but queries span several logs.

For both, each incoming checkpoint is compared with the last one received from that log, with a consistency proof. Two signed checkpoints of one log at the same size with different roots show that the store key signed two different histories.

**Time.** After an ingest, the receiver's `received_at` is an upper bound on when the gateway recorded an object.

## 7. Privacy

- **Synthetic or public data only until M5** ([store/0 Privacy](../spec/store-0.md#privacy)). An append-only log cannot forget.
- **Keyed digests, never bare hashes,** for device serials and installer references, under a key the data owner holds, one per link domain. If a link-domain key leaks, the digests can be guessed.
- **Pseudonymous actor ids,** never derived from a name, email address, phone number, licence or business number, or an unkeyed hash of one ([turn-reader.md](turn-reader.md), open decision 12). The id is not the key; keys are bound to ids at M5.
- **Erasure comes at M5.** By-reference bodies can then be deleted; the log keeps ids, hashes, times and pseudonyms.
- **No household attributes inline.** Objects never carry a household's address, occupants or circumstances. Device settings and measurements from equipment at a home are still information about that household, so each value is classified like any captured column: inline, keyed, by reference or left out, as in M2's mapping/0 ([M2](roadmap.md#m2-read-only-capture-from-a-synthetic-register)). A measurement summary is coarse, for example a maximum over a day or longer, never a time series. It is kept by reference unless it is classified as non-personal. Prioritising records for review, and any scoring of households or people, are out of scope.

## 8. Fit with the roadmap

| Milestone | What this profile uses or adds |
|---|---|
| [M1](roadmap.md#m1-verifiable-store-append-only-log-checkpoints-offline-verification) **[built]** | The store: two clocks, lanes and heads, checkpoints, offline export verification |
| [M2](roadmap.md#m2-read-only-capture-from-a-synthetic-register) | Capture from a synthetic register. A synthetic edge workload for the simulator **[proposed]**. Real data only after M5 |
| [M3](roadmap.md#m3-two-way-reconciliation-report-only-recorded-as-evidence) | Reconciliation reports; a reported difference adds a review step **[proposed]** |
| [M4](roadmap.md#m4-memory-reads-index-query-api-bundles-read-only-service) | The approval-status view, and the disagreement view built on it and on M4's `state` and `history` reads, with vectors **[proposed]**. M4b or early M6: an intake for asserts that runs the id-form, record-lane writer and grant checks before appending |
| [M5](roadmap.md#m5-pilot-gate-trust-time-replication-privacy-operations) | Key binding, extended **[proposed]** from people and services to organisations, gateways and agents (an agent's binding names its accountable party); approver lists bound to keys; revocation judged on the store's clock; erasure; legal review |
| [M6](roadmap.md#m6-requests-and-the-outbox-as-evidence-in-shadow-mode-no-write-code), M7 | Requests in shadow mode, then writes to the synthetic database only. Writing to any other system is a separate decision for that system's owner |
| M9 | An optional device dictionary, only if wanted |

**Edge track [proposed].** Each step sits under the milestone named:

| Step | Content | Gate |
|---|---|---|
| EG1 | The edge profile and the `approval/0` shapes (approver list, rules, position, resolution, grant), with vectors built from this document's examples; a gateway deployment profile | Alongside M2 |
| EG2 | Sync (ingest or replicas), the receipt profile, exports that end at a checkpoint, and a check for two histories signed by one key, with vectors | With M4 |
| EG3 | Gateway key binding, the transport decision and an ARM build. A trial with real devices only after M5, with legal review and the agreement of device owners and the systems' owners | After M5 |
| EG4 | Steps that need more than one listed person in the same role (a count above 1) for edge records, only if a use needs them. Before keys are bound, two names need not be two people | After M5 |

## 9. Requirements

Test names are those of the [M1 build specification](m1-store-spec.md#11-tests-and-ci-the-acceptance-criteria). "This document" means the checks in the appendix.

| ID | Requirement | Checked by today | Status |
|---|---|---|---|
| REQ-EDGE-001 | Every object parses under evidence/0's strict rules, and every export line is canonical. | `canon.json` (20 cases), `numbers.json` (4,024); object vectors 106, 107, 114; store export 112 (`S_LINE_INVALID`); T1 | Built |
| REQ-EDGE-002 | `id` is SHA-256 of the canonical object without `id` (`E_ID_MISMATCH`); `content.hash` is SHA-256 of canonical `inline` (`E_CONTENT_HASH_MISMATCH`). | Object vectors 101, 102; store export 101 | Built |
| REQ-EDGE-003 | Inclusion and consistency proofs verify, and one changed byte makes them fail. | T11 (Rust, WebAssembly, Python) | Built |
| REQ-EDGE-004 | The log tables refuse UPDATE and DELETE, and derived tables are checked for drift. | T8 | Built |
| REQ-EDGE-005 | An `at` more than 300 s after `received_at` is counted, never rejected or rewritten. | T9; store export 007 | Built |
| REQ-EDGE-006 | Export, import and export again give identical files. | T2 (exports byte-identical to the vectors), T10 (same head after import); this document | Built |
| REQ-EDGE-007 | A stale compare-and-set append is refused with `S_STALE_HEADS`, and the current heads are returned. | T4; this document | Built |
| REQ-EDGE-008 | A record-lane object by an actor other than a named capture service is flagged (`P_RECORD_LANE_WRITER`), and its status is unknown. | None | New, M4 |
| REQ-EDGE-009 | Review status is one of the five words of [3.5](#35-review-status-is-a-view), and the same log position always gives the same status. | In part: object vector 113 and T3 (an `approve` is required) | New, M4 view and approver list; keys M5 |
| REQ-EDGE-010 | The disagreement view flags each case of [section 4](#4-when-records-disagree) that it should, and nothing else. | In part: record-lane forks (store export 005, T4, T5) | New, M4 |
| REQ-EDGE-011 | An `approve` counts toward a rule only from a signer listed for the role at the line's position; on an agent's assert, only when signed as its `on_behalf_of`, which is a person or organisation. | None | New, M4; keys M5 |
| REQ-EDGE-012 | An approver list or rule set with two current versions gives unknown (`P_POLICY_AMBIGUOUS`). | None | New, M4 |
| REQ-EDGE-013 | A new version of a list or rule set counts only with `approve` lines from two administrators named in the version it replaces. | None | New, M4 |
| REQ-EDGE-014 | A position never removes an approval: every `approve` line stays held and verifiable, and only the status changes. | None | New, M4 |
| REQ-EDGE-015 | `actor.id`, `on_behalf_of` and signers follow the pseudonym form; others are refused at the assert intake (`P_ACTOR_ID_FORM`) and re-checked at verification. | None | New, M4; intake M4b or early M6 |
| REQ-EDGE-016 | An ingest keeps the ids and attestation refs of everything but the sender's genesis and checkpoints, and its receipt verifies against the sender's genesis and signed checkpoint. | In part: this document (intake refuses the sender's store objects and their lines) | New, EG2 |
| REQ-EDGE-017 | A receiver ingests an export only up to its last checkpoint. | None | New, EG2 |
| REQ-EDGE-018 | Two signed checkpoints of one log at the same size with different roots are reported. | None | New, EG2 |

## 10. Open questions

1. Who publishes which approver list, and who are its first two administrators? Is a list anchored in deployment configuration acceptable until M5?
2. For commissioning records, is the installer's approval enough unless a recorded condition adds a reviewer (as in the examples), or is a reviewer always needed? Which conditions apply?
3. For each system in a trial, which records are record changes and which are statements? Is each system's record id opaque enough to be a subject key as it is?
4. Who holds the link-domain keys for device serials and installer references?
5. Who holds a gateway's signing key, and how is it bound at M5?
6. Sync: ingest, or one replica per gateway? Which transport?

## Appendix: how the examples were checked

All data is synthetic. The `evidence` command-line tool was built from commit `a6bbb94` and run in a scratch directory. Seeds are development keys from `evidence keygen`, made for these runs only. Keyed digests use the published test key `synthetic-link-domain-test-key`. The commissioning record, the two gateway statements, the approver list, the rule set and the form check, with their attestation lines, are the same sealed files as in the examples of [approvals.md](approvals.md), so their ids match across the two documents. The form check cites a checkpoint, so its id is that of one of that note's builds. The network-portal connection record and the objects of the store cases are this document's own. Each JSON block in this document was copied out and checked: for each object, `evidence canon` gives the sealed file's bytes and `evidence verify` gives `ok` with the id shown; the attestation line verifies against the commissioning record with `{"approve":1}`.

| Command (`obj/` and `att/` are the sealed files) | Result |
|---|---|
| `evidence id obj/commissioning-v1.json` (and the gateway statements, the form check, the approver list and the rule set) | Each prints the object's own `id` |
| `evidence verify obj/commissioning-v1.json --attestations <author line> --require approve` | exit 1, `E_ATTESTATION_MISSING`, `{"author":1}` |
| `… --attestations <author + installer> --require author --require approve` | exit 0, `ok`, `{"approve":1,"author":1}` |
| `… --attestations <author + installer + reviewer> --require author --require approve` | exit 0, `ok`, `{"approve":2,"author":1}` |
| `evidence verify obj/gateway-settings-v1.json --attestations att/gateway-settings-v1.jsonl --require author` | exit 0, `ok`, `{"author":1}` |
| `evidence verify obj/gateway-settings-v2.json --store <empty folder>` | exit 1, `E_PREV_MISSING` |
| `evidence verify obj/form-check-v1.json --attestations <tool author + reviewer approve> --require author --require approve` | exit 0, `ok`, `{"approve":1,"author":1}` |
| `evidence verify obj/approver-list-v1.json --attestations att/approver-list-v1.jsonl --require author --require approve` (and the rule set) | exit 0, `ok`, `{"approve":2,"author":1}` |
| `evidence verify obj/commissioning-v1.json --attestations <installer line twice> --require approve` | exit 0, `ok`, `{"approve":2}` |
| The same with `--require approve=2` | exit 2, usage error |
| `evidence verify obj/form-check-v1.json --attestations <tool author + tool approve> --require approve` | exit 0, `ok`, `{"approve":1,"author":1}` |
| `evidence sign obj/commissioning-v1.json … --purpose reject` | exit 2, `E_USAGE` |
| `evidence store init st --name edge-demo …`, then `store append … --if-heads none` and `store attest` for the commissioning record (author and installer lines), the gateway statement and a network-portal connection record (10 kW, same device digest) | exit 0 each; the commissioning record at seq 3 |
| `evidence store verify st` at that point: record 5 kW, gateway statement 10 kW, connection record 10 kW | exit 0, `ok`, 9 entries, forks 0, no findings |
| `evidence store append st obj/gateway-settings-v2.json --if-heads none` | exit 1, `S_STALE_HEADS`, heads [version 1] |
| `evidence store append st obj/gateway-settings-v2.json --if-heads <version 1>` | exit 0 |
| `evidence store get st <commissioning id> --require author --require approve` | exit 0, `ok`, `{"approve":1,"author":1}` |
| `evidence store heads` for the two record subjects and the gateway lane | One head each; the gateway lane's is version 2 |
| `evidence store checkpoint st …`, then `evidence store verify st` | exit 0, `ok`, 13 entries, forks 0, no findings, covered size 11 |
| `evidence store export`, `store verify` of the export, `store import`, `store export` again, `diff -r` | `ok`; the two exports are identical |
| In a copy: two capture `update`s of the record with the same `prev`, then `store verify` | forks 1; record lane 2 heads |
| An `update` listing both heads, then `store verify` | forks 0 |
| A gateway statement listing versions 1 and 2, then `store heads` | 1 head |
| After a checkpoint, a second statement replacing version 1, appended without `if_heads` | gateway lane 2 heads; `store verify` `ok`, forks 0 |
| `evidence store export` of the copy, then `store verify` of the export | exit 0, `ok`, head seq 20, covered size 17 |
| In a second store: `store append` of the first export's genesis and checkpoint, then `store attest` of their author and witness lines | exit 1 for each: `S_RESERVED_SUBJECT` for the objects, `S_ATTESTATION_TARGET` for the lines |
