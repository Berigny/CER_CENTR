# Edge devices: a draft profile for consumer energy resources records

**Status:** draft for discussion, 27 November 2026. A proposed profile for the edge track in the [roadmap](roadmap.md#edge-track), which stays the only roadmap. The points decided on 27 November 2026 are listed in the [roadmap](roadmap.md#decisions-confirmed-27-november-2026); everything else is proposed or open unless it is labelled **[built]**. Not an official publication of any agency; it describes no agency's process (see the [README](../README.md) notice).

**Scope:** records about edge devices, using synthetic data only until the real-data gate ([M5](roadmap.md#m5-real-data-gate-trust-time-replication-privacy-operations)). Nothing in this profile writes to another system; the layer only reads from other systems. The roadmap's later write path ([M7](roadmap.md#m7-write-through-to-the-synthetic-database-only-writes-opt-in)) reaches only a synthetic demo database, and writing to any real system is a separate decision for that system's owner, one kind of record at a time. The layer makes no automated decisions and does not score households.

**Labels:** **[built]** is in the code at commit `a6bbb94`. **[proposed]** is a design for a later milestone. **[open]** is a decision still to be made.

**Names:** here "CER" means consumer energy resources (rooftop solar inverters, batteries, electric vehicle chargers and controllable loads). The Clean Energy Regulator is always named in full ([D1](decisions.md#d1-a-neutral-name)).

## In short

- **Records stay in their own systems.** An installer's commissioning app, a network connection portal and a gateway each keep their own records. This layer keeps verifiable evidence of what they held and said, beside them, and does not write to them.
- **Two kinds of edge object, chosen by record type.** A record held in another system enters as a record change observed by a capture service. A gateway's own settings or measurement summary is a statement (`assert`) signed by the gateway.
- **Approvals are signatures, and a published list says which count.** The installer approves the captured record. An approver list, published as data, says whose approvals count for which role. An approval counts in this layer only ([3.5](#35-review-status-is-a-view)).
- **Disagreement is shown, and a person resolves it.** When two records about one device differ, that is a prompt for review, not a finding. The system decides nothing: it shows what is still needed and who can supply it, and a person records the outcome, approved by a listed reviewer.
- **Exports verify offline over any transport.** Objects are content-addressed and signed, so how a file travelled does not matter. A receiver relies only on what a signed checkpoint covers.
- **Built today:** the evidence format and the single-writer store (M1). **Proposed:** this profile, the approval-status view (M4), key binding (M5) and the edge track. Before M5, no signature here is accountability evidence.

## 1. Two kinds of edge object [proposed]

Records about one consumer energy device (an inverter, a battery, an electric vehicle charger) are kept in several systems: an equipment register, an installer's commissioning app, a network connection portal. Each keeps its own current state. Which kind of object enters the log depends on where the record lives:

| Record | Kept in | In the log | Lane | Signed by |
|---|---|---|---|---|
| Commissioning record | An installer's app (`installer-app-demo`) | `create`, `update` or `delete` by the capture service that observed it | `record` | Capture: `author`. Installer: `approve` |
| Connection record | A network portal (`network-portal-demo`) | The same pattern, under its own subject | `record` | Capture: `author`. Approvals as the published rules say |
| Gateway settings or measurement summary | The gateway (`edge-gateway-demo`) | `assert` by the gateway, `kind: "service"`, opaque id | `assert:edge-demo/gateway-settings` for settings; a measurement summary in the lane of its own schema | Gateway: `author`. Compared, not approved |
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

- **`op` is `create`** because capture saw a new record. Later changes are `update`s that list it in `prev`. Record changes are only ever observations of a system of record ([D11](decisions.md#d11-observe-before-writing)), so only the capture services named in the deployment may write a record lane. store/0 does not enforce that, so a profile check flags any other writer (`P_RECORD_LANE_WRITER`). Such an object never supersedes the captured record, which keeps its status and is still read as `observed`.
- **No `ext` member here.** M2's capture/0 profile **[proposed]** adds an `ext` member to each captured object, keeping for example the raw source time ([M2](roadmap.md#m2-read-only-capture-from-a-synthetic-register)). It is left out of this example.
- **No personal identifiers are inline.** The device serial and the installer appear only as keyed digests (HMAC under a key the data owner holds, one key per link domain). A bare hash of a serial number can be reversed by guessing ([evidence/0 Privacy](../spec/evidence-0.md#privacy)). These values use a published test key. The other inline values in these examples (times, export limits, the inverter model and the firmware hash) are synthetic, and are inline so the examples can be checked in full. In a deployment each value is classified first, as [section 7](#7-privacy) says, and is inline only if it is classified as non-personal.
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
- **Subject keys are opaque record ids.** They are in the log in the clear. A device's subject key is an opaque id assigned like the gateway's own actor id (128 random bits), never its serial number, a meter identifier or an address; `DEV-TEST-000001` here is a test placeholder. A captured record keeps its own system's record id, such as `COM-TEST-000001`. Whether each system's record id is fit to be a subject key is [open question 3](#10-open-questions).
- **It states 10 kW where the record says 5 kW.** The two objects are about different subjects in different systems. Their only link is the equal `device_serial_digest`. [Section 4](#4-when-records-disagree) says what happens next.
- **A setting, not a measurement.** `export_limit_kw` is read from the device's configuration. A measurement summary states what was measured, for example `observed_max_export_kw` over a time window, because a measurement observes power, not a limit. Settings and measurement summaries are still information about a household; [section 7](#7-privacy) says how they are classified and kept, and the examples' inline values are synthetic.
- **Versions chain.** A later statement lists this one in `prev` and is appended with `if_heads`. In the checks, version 2 (5 kW) sent with `--if-heads none` was refused with `S_STALE_HEADS`, and sent with `--if-heads` set to version 1 it landed. A new schema version keeps the version after `@` (`edge-demo/gateway-settings@2`), so it stays in the same lane; a ref that carries the version in its name would open a new lane and leave the old statement a head.

A faceted index for device records is not part of this profile. Whether device records get one, and whether it would use Kernel/2's dictionary or a separate one with its own profile tag, is **[open]** ([turn-reader.md open decision 9](turn-reader.md#9-kernel2-rules-the-reader-depends-on)).

## 2. Store facts and two clocks [built]

- **Two clocks.** The object holds its writer's claim (`at`). The store keeps its own position (`seq`) and time (`received_at`) in a separate log entry, so an object's id is the same in every store ([store/0 Two clocks](../spec/store-0.md#two-clocks), [D13](decisions.md#d13-two-clocks)).
- **Late claims are counted, never rejected.** An object whose `at` is more than 300 seconds after its entry's `received_at` is counted in `counts.at_after_received`. It is never rewritten.
- **Checkpoints.** A checkpoint signed by the store key covers entries 1 to its `size`. `evidence store prove` gives inclusion and consistency proofs against it.
- **Compare-and-set.** `if_heads` refuses a stale append with `S_STALE_HEADS` and returns the current heads.
- **Time is claimed until M5.** Until M5 adds independent time, a store's `received_at` is that store's own claim; for a gateway's own store **[proposed]**, that is the gateway's claim.

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

- **An approver list** (`approval/approver-list@0`) is an `assert` naming, by pseudonymous id, who may approve in which role, such as installer or reviewer. An installer's entry can carry the keyed reference the installer app uses, so a check can confirm that the installer who approved is the one the record names. That reference is shown only to parties entitled to see approval history, never in a public list ([approvals.md section 3](approvals.md#3-parties-and-ids-proposed)).
- **Approval rules** (`approval/rules@0`) say, for each kind of record, which roles must approve, how many distinct people in each, and which recorded facts add a review step.
- **At least two administrators change them.** A new version of a list or rule set counts only with `approve` lines from at least two distinct administrators (its `changes_need`) named in the version it replaces. The example list (`sha256:0cdc21d0…`) carries one `author` and two `approve` lines (`{"approve":2,"author":1}`).
- **Keys are bound at M5.** Until then, list entries carry no keys, and approvals made with development keys are not accountability evidence ([roadmap decision 4](roadmap.md#decisions-confirmed-26-november-2026)). Key binding ties a key to a pseudonymous id, nothing more. A signature never shows that the signer holds a licence or accreditation, and a role on an approver list is a role in this layer only, as the list's publisher names it.

The shapes, checks and codes are in the design note [approvals.md](approvals.md).

### 3.3 Automated checks and agents

- **A check informs; people decide.** A check result is an `assert` by the checking tool, naming the checked object by id and its rule set by hash. It never counts as an approval. Its only effect is on review status, and only when the published rules list its result as needing review and the tool acts under a grant from a party the rules list. A record not yet approved then gets a review step at once, and awaits a listed reviewer's approval. On a record already approved, the check is only routed, as a prompt, to the party the tool acts for, and adds the step once that party accepts it ([approvals.md 7.2](approvals.md#72-doubt-from-a-tool)). A check never approves, changes or removes a record or an approval, and it has no effect outside this layer.
- **An AI agent is an actor** of kind `agent`, with `on_behalf_of` naming the accountable person or organisation. Its output counts as approved only through that party's `approve`. The example form check carries the tool's `author` line and the reviewer's `approve` (`{"approve":1,"author":1}`).

### 3.4 What the core counts today [built]

| Case | Core result | What the profile adds |
|---|---|---|
| The same `approve` line twice in one file | `ok`, `approve: 2` | Counts distinct people, not lines |
| `--require approve=2` | Usage error, exit 2 | The number required per role, as rule data |
| An agent's `approve` on its own output | `ok`, `approve: 1` | Only the `on_behalf_of` party's approval counts |
| A disagreement signed as its own purpose (`--purpose reject`) | Refused, `E_USAGE` | A position, recorded as an `assert` |

### 3.5 Review status is a view

Whether an object is approved is never a member of the object ([evidence/0 Not in v0](../spec/evidence-0.md#not-in-v0)). A view computes it from the log at a stated position, under the rules in force when the object was recorded. The answer is one of five words: **approved**, **awaiting approval**, **contested**, **superseded** or **unknown**. With each status, the view lists what would move it on and who can still supply it (`next`); it routes the question and assigns nothing. A missing rule, or a rule set or list with two current versions at the position that governs it, gives unknown; a competing version that lands later is reported for information only. Unknown is never read as approved or as refused. Until M5, every status says that accountability is not established.

A status is a review status. It is not a finding about a device, an installer or a household, and it has no effect outside this layer. The status `approved` means only that the approvals the published rules ask for are present. An `approve` line, an installer's included, records that a key holder accepted the object in this layer. Neither is an approval, declaration, certificate or signature under any scheme, licence or law, before or after M5.

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

- **Comparisons as rule data.** The rules name which values of a record to compare with which statements, linked by an equal keyed digest. When a current statement by a source the deployment names (in the examples, the gateway) differs from the record, a review step is added. A statement by anyone else is noted and adds nothing.
- **A disagreement view** for each subject, built on M4's `state` and `history` reads and the approval-status view ([M4](roadmap.md#m4-memory-reads-index-query-api-bundles-read-only-service)): more than one head in a lane, a parent with children in more than one lane, and the differences the rules' comparisons find. It is computed from `prev` links over the whole history, with vectors for cases that must be flagged and cases that must not (the third row above).
- **Positions.** A listed party (in the examples, an installer or a reviewer) who disagrees records a position (`approval/position@0`), an `assert` of their own. The object shows as contested until a person resolves it. A position never removes or alters an approval. A party can withdraw only their own approval, which then stops counting; the line stays in the log.
- **Resolution by a person.** A person records an `assert`, approved by a listed reviewer, naming the positions it settles and an outcome: the record stands, check it at the source, check it on the device, or refer it to another person. An outcome is not an instruction to anyone and has no effect outside this layer. This is the pattern of M3 resolutions. A profile check flags a resolution by an agent or a service (compare M3's `P_RESOLVER_NOT_HUMAN`); the store does not refuse it. A resolution never changes a record. Any change happens in the system of record or on the device, and reach the log only through capture or a new statement.

A difference between records is a prompt for review, not a finding.

## 5. Spreading approvals and uncertainty [proposed]

The design is about how uncertainty is shared and how decisions are enabled, not about who runs which system. Nothing is settled by counting objections. Instead:

- each party records its own view, signs it and stays accountable for it: capture what it observed, a gateway what it states, installers and reviewers what they accept or dispute;
- doubt adds a named reviewer, never a score;
- a listed reviewer who cannot assess an object records that (`unable-to-assess`); no count changes, and the view lists the other listed reviewers who can still answer;
- every status says what would move it on and who can still supply it (`next`); this routes the question and decides, gates or assigns nothing;
- an unknown stays unknown and is shown, never treated as a pass or a fail;
- a named person decides, and the decision is recorded with what it relied on;
- a decision that needs more than one approver names the approver list and the number required as published data; the status view counts those approvals from M4, and none of them is accountability evidence until keys are bound (M5).

Actor ids are pseudonyms: `person:` or `agent:` with 32 random hex digits; `service:` or `org:` may use a public name, but a device at someone's premises, a sole trader or an organisation named after a person gets an opaque id like a person. Only an access-controlled directory kept off the log links ids to people, except that an installer whose approver-list entry carries keyed record references can also be linked by the data owner, who holds the link-domain key and the system of record. A pseudonym is still handled as personal information. An AI agent is one kind of actor, not only a reader or a contributor: under a grant it can also raise doubt that adds a review step ([3.3](#33-automated-checks-and-agents)). The same hooks serve an agent, a case screen and a batch job: a grant to act for someone, a verified read, a recorded statement, an approval by the accountable person, and continuity notes kept between sessions under the grant. The full design is in [approvals.md](approvals.md).

## 6. Moving records between sites [proposed]

**Transport.**

- Objects are content-addressed, attestations are signed and exports verify offline, whatever carried them.
- Entries after the last checkpoint are not yet signed, so a receiver accepts an export only up to its last checkpoint. In the checks, an export whose head was entry 20 verified `ok` with a covered size of 17.
- Any transport the operator's security policy allows: HTTPS upload, SFTP, object storage or removable media **[open]**.
- A transport's identity is not a signing identity. At M5 a gateway's binding names its actor id and signing key, and also the log id and store key of its log, so a receiver can tie each log's checkpoints to the gateway. Nothing is bound through the transport.

**Gateways.** A gateway that buffers through outages runs the native store (the command-line tool or the library): SQLite, a file lease on a local disk, and the store key for checkpoints. The WebAssembly module verifies, seals and signs, but has no store. Each gateway is its own log, with its own genesis and store key, and writes a checkpoint before every export. Its store name becomes the store's signer id (`service:evidence-store/<name>`), which is in the genesis, is the actor of every checkpoint and the signer of its witness line, and goes with every export and receipt. So the store name is opaque like the gateway's own id (for example the same 32 hex digits), never a site, address, meter or customer name. A replaced or restored gateway never resumes its log from an older backup. It restores from the last export a receiver accepted (`store import`), or starts a new log whose first statement names the old log id and its last accepted checkpoint in its content, not in `prev` **[open]**. Not yet checked: an ARM build, flash wear from a sync on every commit, and keeping the store key in hardware (M5's PKCS#11 signer backend is the natural route).

**Sync [open].** A store/0 entry belongs to one log, and `store import` only builds a new store from an export. Two designs:

- **(a) Ingest.** The receiver verifies the whole export, then appends its objects and attestation lines to its own log in the sender's order, except the sender's genesis, checkpoints and their lines, which store/0 intake refuses (`S_RESERVED_SUBJECT` for the objects, then `S_ATTESTATION_TARGET` for their lines). Ids and attestation refs of everything else are unchanged; `seq` and `received_at` are the receiver's. An object whose `prev` names the sender's genesis or a checkpoint is accepted by the sender's store, but the receiver refuses it (`E_PREV_MISSING`), its attestation lines and anything that lists it in `prev`, because the genesis and checkpoints are never appended there. So the edge profile does not allow a genesis or checkpoint in `prev` (a [`read-basis/0`](turn-reader.md#the-basis-profile) cites its checkpoint in `ext`), and an ingest checks the whole export for such a `prev` and refuses the export before appending anything. A receipt `assert` carries the sender's genesis, last checkpoint and their lines instead, so the sender's facts stay provable. Re-running an ingest returns duplicates.
- **(b) One replica per gateway.** The receiver keeps each gateway's export as its own store. M5's replica checks apply as they are, but queries span several logs.

For both, each incoming checkpoint is compared with the last one received from that log, with a consistency proof. Two signed checkpoints of one log at the same size with different roots show that the store key signed two different histories. A gateway restored from an older backup that resumes the same log gives the same result, so a report is a prompt for review, not a finding.

**Time.** After an ingest, the receiver's `received_at` is an upper bound on when the gateway recorded an object.

## 7. Privacy

- **Synthetic or public data only until M5** ([store/0 Privacy](../spec/store-0.md#privacy)). An append-only log cannot forget.
- **Keyed digests, never bare hashes,** for device serials and installer references, under a key the data owner holds, one per link domain. If a link-domain key leaks, the digests can be guessed, and an approver list's keyed record references would then tie an installer's pseudonym to the installer.
- **A gateway never holds a link-domain key.** It reads each device's serial and asks the data owner's key service for the keyed digest when the device is commissioned or replaced. So no key is kept on equipment at someone's premises, and the digest still comes from what the device reports ([open question 4](#10-open-questions)).
- **Pseudonymous actor ids,** never derived from a name (a sole trader's trading name included), email address, phone number, licence or business number, or an unkeyed hash of one ([approvals.md section 3](approvals.md#3-parties-and-ids-proposed); [turn-reader.md open decision 12](turn-reader.md#12-privacy-of-actor-ids)). A pseudonym is still handled as personal information. The id is not the key; keys are bound to ids at M5.
- **Erasure comes at M5.** By-reference bodies can then be deleted; the log keeps ids, hashes, times and pseudonyms. Deletion follows the data owner's records-disposal rules. Section 24 of the Archives Act 1983 applies to any Commonwealth record (a record that is the property of the Commonwealth or of a Commonwealth institution, wherever it is held) and allows it to be destroyed only as required by law, with the National Archives' permission (for example, under a records authority), or under a normal administrative practice the Archives has not disapproved ([National Archives](https://www.naa.gov.au/information-management/disposing-information/disposal-commonwealth-records-absence-agency-specific-records-authority)). The APP 11.2 duty to destroy or de-identify personal information does not apply to personal information contained in a Commonwealth record ([OAIC](https://www.oaic.gov.au/privacy/australian-privacy-principles/australian-privacy-principles-guidelines/chapter-11-app-11-security-of-personal-information)).
- **No household attributes inline.** Objects never carry a household's address, occupants or circumstances. Device settings and measurements from equipment at a home are still information about that household, so each value is classified like any captured column: inline, keyed, by reference or left out, as in M2's mapping/0 ([M2](roadmap.md#m2-read-only-capture-from-a-synthetic-register)). A measurement summary is coarse, for example a maximum over a day or longer, never a time series. Any value not classified as non-personal, a measurement summary included, is never inline: it is kept by reference, or as a keyed digest where it must be linked. Prioritising records for review, and any scoring of households or people, are out of scope.

## 8. Fit with the roadmap

| Milestone | What this profile uses or adds |
|---|---|
| [M1](roadmap.md#m1-verifiable-store-append-only-log-checkpoints-offline-verification) **[built]** | The store: two clocks, lanes and heads, checkpoints, offline export verification |
| [M2](roadmap.md#m2-read-only-capture-from-a-synthetic-register) | Capture from a synthetic register. A synthetic edge workload for the simulator **[proposed]**. Real data only after M5 |
| [M3](roadmap.md#m3-two-way-reconciliation-report-only-recorded-as-evidence) | Reconciliation reports; a reported difference adds a review step **[proposed]** |
| [M4](roadmap.md#m4-memory-reads-index-query-api-bundles-read-only-service) | The approval-status view, and the disagreement view built on it and on M4's `state` and `history` reads, with vectors **[proposed]**. Late M4 or early M6: an intake for asserts that runs the id-form, record-lane writer and grant checks before appending |
| [M5](roadmap.md#m5-real-data-gate-trust-time-replication-privacy-operations) | Key binding, extended **[proposed]** from people and services to organisations, gateways and agents (an agent's binding names its accountable party; a gateway's also names its log id and store key); approver lists bound to keys; revocation judged on the store's clock; erasure; legal review |
| [M6](roadmap.md#m6-requests-and-the-outbox-as-evidence-in-shadow-mode-no-write-code), M7 | Requests in shadow mode, then writes to the synthetic database only. Writing to any other system is a separate decision for that system's owner |
| M9 | A faceted index for device records, only if one is wanted; whether it uses Kernel/2's dictionary or a separate one is **[open]** ([section 1](#1-two-kinds-of-edge-object-proposed)) |

**Edge track [proposed].** The steps EG1-EG4 are planned only in the [roadmap's edge track](roadmap.md#edge-track). This profile supplies EG1's examples and the gateway set-up in [section 6](#6-moving-records-between-sites-proposed), and the requirements marked EG2 below.

## 9. Requirements

Test names are those of the [M1 build specification](m1-store-spec.md#11-tests-and-ci-the-acceptance-criteria). "This document" means the checks in the appendix.

| ID | Requirement | Checked by today | Status |
|---|---|---|---|
| REQ-EDGE-001 | Every object parses under evidence/0's strict rules, and every export line is canonical. | `canon.json` (20 cases), `numbers.json` (4,024); object vectors 106, 107, 114; store export 112 (`S_LINE_INVALID`); T1 | Built |
| REQ-EDGE-002 | `id` is SHA-256 of the canonical object without `id` (`E_ID_MISMATCH`); `content.hash` is SHA-256 of canonical `inline` (`E_CONTENT_HASH_MISMATCH`). | Object vectors 101, 102; store export 101 | Built |
| REQ-EDGE-003 | Inclusion and consistency proofs verify, and one changed byte makes them fail. | T11 (Rust, WebAssembly, Python) | Built |
| REQ-EDGE-004 | The log tables refuse UPDATE and DELETE, and derived tables are checked for drift. | T8 | Built |
| REQ-EDGE-005 | An `at` more than 300 s after `received_at` is counted, never rejected or rewritten. | T9; store export 007 | Built |
| REQ-EDGE-006 | Export, import and export again give identical files. | T4 (the imported store exports the same bytes), T10 (same head and log after import); this document | Built |
| REQ-EDGE-007 | A stale compare-and-set append is refused with `S_STALE_HEADS`, and the current heads are returned. | T4; this document | Built |
| REQ-EDGE-008 | A record-lane object by an actor other than a named capture service is flagged (`P_RECORD_LANE_WRITER`), and its status is unknown. It never supersedes the captured record, which keeps its status and is still read as `observed`. | None | New, M4 |
| REQ-EDGE-009 | Review status is one of the five words of [3.5](#35-review-status-is-a-view), and the same log position always gives the same status. | In part: object vector 113 and T3 (an `approve` is required) | New, M4 view and approver list; keys M5 |
| REQ-EDGE-010 | The disagreement view flags each case of [section 4](#4-when-records-disagree) that it should, and nothing else. | In part: record-lane forks (store export 005, T4, T5) | New, M4 |
| REQ-EDGE-011 | An `approve` counts toward a rule only from a signer listed for the role at the line's position; on an agent's assert, only when signed as its `on_behalf_of`, which is a person or organisation. | None | New, M4; keys M5 |
| REQ-EDGE-012 | An object's status is unknown (`P_POLICY_AMBIGUOUS`) when the rule set in force at the object's position, or the approver list in force at one of its approval lines' positions, has two current versions. A competing version that lands later leaves the status unchanged and is reported for information only. | None | New, M4 |
| REQ-EDGE-013 | A new version of a list or rule set counts only with `approve` lines from at least two distinct administrators (its `changes_need`) named in the version it replaces. | None | New, M4 |
| REQ-EDGE-014 | A position never removes an approval: every `approve` line stays held and verifiable, and only the status changes. | None | New, M4 |
| REQ-EDGE-015 | `actor.id`, `on_behalf_of` and signers follow the pseudonym form; others are refused at the assert intake (`P_ACTOR_ID_FORM`) and re-checked at verification. | None | New, M4; intake late M4 or early M6 |
| REQ-EDGE-016 | An ingest keeps the ids and attestation refs of everything but the sender's genesis and checkpoints. It refuses, before appending anything, an export in which another object lists one of them in `prev`. Its receipt verifies against the sender's genesis and signed checkpoint. | In part: this document (intake refuses the sender's store objects, their lines, and objects whose `prev` names them) | New, EG2 |
| REQ-EDGE-017 | A receiver ingests an export only up to its last checkpoint. | None | New, EG2 |
| REQ-EDGE-018 | Two signed checkpoints of one log at the same size with different roots are reported. | None | New, EG2 |

## 10. Open questions

1. Who publishes which approver list, and who are its first administrators (at least two)? Is a list anchored in deployment configuration acceptable until M5?
2. For commissioning records, is the installer's approval enough unless a recorded condition adds a reviewer (as in the examples), or is a reviewer always needed? Which conditions apply?
3. Under the record-type rule (a record kept in another system enters as a captured record change; a gateway's own settings or measurement summaries are its own statements), in any trial, which records of each system fall under which? Is each system's record id opaque enough to be a subject key as it is?
4. For each link domain (device serials, installer references), which data owner holds the key ([roadmap decision 8](roadmap.md#decisions-confirmed-26-november-2026))? How does a gateway's statement get its keyed digest without the key leaving that owner's key service? (Proposed: the gateway asks the key service when a device is commissioned or replaced.)
5. Who holds a gateway's signing key and store key, and how are they bound at M5?
6. Where does an installer sign `approve`: in the installer app, if its vendor agrees to build that, or in a separate tool? The installer signs the object capture made, not the app's record, so whichever tool they use must show them that object. Does each installer hold their own key, or would the app hold keys for them? Who binds an installer's key to their id at M5, and who revokes it? How is one job kept from needing two sign-offs?
7. How is a gateway replaced or restored: from the last export a receiver accepted, or as a new log that names the old one?
8. Sync: ingest, or one replica per gateway? A candidate rule: an object's content is written to one log only, and other logs refer to it by log id, position and hash. That favours one replica per gateway, or an ingest that keeps only a receipt. Which transport?

Open questions 3 to 9 of [approvals.md](approvals.md#15-open-questions) apply here too: which rule set an object is judged under (3), confirming that tools and services never fill an approval step or settle a disagreement (4), when an approval counts toward an added review step (5), the pseudonym form (6), which checks the assert intake refuses (7), continuity notes (8), and who signs when an organisation approves, for example a network operator approving a connection record (9).

## Appendix: how the examples were checked

All data is synthetic. The `evidence` command-line tool was built from commit `a6bbb94` and run in a scratch directory. Seeds are development keys from `evidence keygen`, made for these runs only. Keyed digests are HMAC-SHA256 under the published test key `synthetic-link-domain-test-key`, of `SN-TEST-000001` (the device serial) and `installer-app-demo/user/U-TEST-0042` (the installer reference). The commissioning record, the two gateway statements, the approver list, the rule set and the form check, with their attestation lines, are the same sealed files as in the examples of [approvals.md](approvals.md), so their ids match across the two documents. The form check cites a checkpoint, so its id is that of one of that note's builds. The network-portal connection record and the objects of the store cases are this document's own. Each JSON block in this document was copied out and checked: for each object, `evidence canon` gives the sealed file's bytes and `evidence verify` gives `ok` with the id shown; the attestation line verifies against the commissioning record with `{"approve":1}`.

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
| In a copy of the first store, built from its export: `store append` of a gateway statement whose `prev` names the store's genesis, and of one naming its last checkpoint; then `store checkpoint` with the same store key and `store verify` | exit 0 each; `ok` |
| The same two statements appended to the second store, then the first one's `author` line | exit 1 for each statement, `E_PREV_MISSING`; exit 1 for the line, `S_ATTESTATION_TARGET` |
