# Edge devices: a draft profile for consumer energy resources records

**Status:** draft for discussion, 27 November 2026. A proposed profile for the edge track in the [roadmap](roadmap.md#edge-track), which stays the only roadmap. The points decided on 27 November 2026 are listed in the [roadmap](roadmap.md#decisions-confirmed-27-november-2026); everything else is proposed unless it is labelled **[built]** or **[gated]**. The open questions were settled on 28 November 2026 ([section 10](#10-open-questions)); those that need a decision outside the project are marked gated. Not an official publication of any agency; it describes no agency's process (see the [README](../README.md) notice).

**Scope:** records about edge devices, using synthetic data only until the real-data gate ([M5](roadmap.md#m5-real-data-gate-trust-time-replication-privacy-operations)). Nothing in this profile writes to another system; the layer only reads from other systems. The roadmap's later write path ([M7](roadmap.md#m7-write-through-to-the-synthetic-database-only-writes-opt-in)) reaches only a synthetic demo database, and writing to any real system is a separate decision for that system's owner, one kind of record at a time. The layer makes no automated decisions and does not score households.

**Labels:** **[built]** is in the code at commit `a6bbb94`. **[proposed]** is a design for a later milestone. **[gated]** is a default that waits for a decision outside the project; nothing real proceeds on it until that decision is made.

**Names:** here "CER" means consumer energy resources (rooftop solar inverters, batteries, electric vehicle chargers and controllable loads). The Clean Energy Regulator is always named in full ([D1](decisions.md#d1-a-neutral-name)).

## In short

- **Records stay in their own systems.** An installer's commissioning app, a network connection portal and a gateway each keep their own records. This layer keeps verifiable evidence of what they held and said, beside them, and does not write to them.
- **Two kinds of edge object, chosen by record type.** A record held in another system enters as a record change observed by a capture service. A gateway's own settings or measurement summary is a statement (`assert`) signed by the gateway.
- **Approvals are signatures, and a published list says which count.** The installer approves the captured record. An approver list, published as data, says whose approvals count for which role. An approval counts in this layer only ([3.5](#35-review-status-is-a-view)). A network connection record is compared, not approved: the portal's own workflow stays authoritative for the connection.
- **Disagreement is shown, and a person resolves it.** When two records about one device differ, that is a prompt for review, not a conclusion. The system decides nothing: it shows what is still needed and who can supply it, and a person records the outcome, approved by a listed reviewer.
- **Exports verify offline over any transport.** Objects are content-addressed and signed, so how a file travelled does not matter. A receiver relies only on what a signed checkpoint covers.
- **Content stays in the gateway's own log.** A receiving site keeps a verified replica of each gateway's log and records a receipt for each position it accepts. Its views include only receipted positions, in one fixed order across logs ([section 6](#6-moving-records-between-sites-proposed)).
- **Built today:** the evidence format and the single-writer store (M1). **Proposed:** this profile, the approval-status view (M4), key binding (M5) and the edge track. Before M5, no signature here is accountability evidence.

## 1. Two kinds of edge object [proposed]

Records about one consumer energy device (an inverter, a battery, an electric vehicle charger) are kept in several systems: an equipment register, an installer's commissioning app, a network connection portal. Each keeps its own current state. Which kind of object enters the log depends on where the record lives:

| Record | Kept in | In the log | Lane | Signed by |
|---|---|---|---|---|
| Commissioning record | An installer's app (`installer-app-demo`) | `create`, `update` or `delete` by the capture service that observed it | `record` | Capture: `author`. Installer: `approve` |
| Connection record | A network portal (`network-portal-demo`) | The same pattern, under its own subject | `record` | Capture: `author`. Compared, not approved: an explicit rule with no approval step ([approvals.md 5.2](approvals.md#52-approval-rules)); the portal's own workflow stays authoritative ([3.5](#35-review-status-is-a-view)) |
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

- **`op` is `create`** because capture saw a new record. Later changes are `update`s that list it in `prev`. Record changes are only ever observations of a system of record ([D11](decisions.md#d11-observe-before-writing)), so only the capture services named in the deployment may write a record lane. store/0 does not enforce that. The assert intake refuses any other writer (`P_RECORD_LANE_WRITER`), and verification flags one that reached the log another way, such as through store/0's own intake. Such an object never supersedes the captured record, which keeps its status and is still read as `observed`.
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
- **Subject keys are opaque record ids.** They are in the log in the clear. A device's subject key is an opaque id assigned like the gateway's own actor id (128 random bits), never its serial number, a meter identifier or an address; `DEV-TEST-000001` here is a test placeholder. A captured record keeps its own system's record id, such as `COM-TEST-000001`, when that id is opaque. When it is not (it carries a serial number, a meter identifier, an address or a licence number), capture uses a keyed digest of it instead, as M2's `mapping/0` does for a `keyed` column, and `P_SUBJECT_KEY_MISMATCH` compares against the keyed value. Whether a real system's ids are opaque is assessed for each system before its first capture **[gated]** ([question 3](#10-open-questions)).
- **It states 10 kW where the record says 5 kW.** The two objects are about different subjects in different systems. Their only link is the equal `device_serial_digest`. [Section 4](#4-when-records-disagree) says what happens next.
- **A setting, not a measurement.** `export_limit_kw` is read from the device's configuration. A measurement summary states what was measured, for example `observed_max_export_kw` over a time window, because a measurement observes power, not a limit. Settings and measurement summaries are still information about a household; [section 7](#7-privacy) says how they are classified and kept, and the examples' inline values are synthetic.
- **Versions chain.** A later statement lists this one in `prev` and is appended with `if_heads`. In the checks, version 2 (5 kW) sent with `--if-heads none` was refused with `S_STALE_HEADS`, and sent with `--if-heads` set to version 1 it landed. A new schema version keeps the version after `@` (`edge-demo/gateway-settings@2`), so it stays in the same lane; a ref that carries the version in its name would open a new lane and leave the old statement a head.

**A faceted index for device records [proposed].** Installation and device records get a coordinate dictionary (`coordinate-dictionary/0`) and profile tag of their own; Kernel/2 stays for certificate lifecycles ([turn-reader.md decision 9](turn-reader.md#9-kernel2-rules-the-reader-depends-on)). Its facets are non-personal classes, such as a product model class, a standard or a jurisdiction, never a site, a premises or a household attribute, and approval status is never a facet. M4b builds it on synthetic data and a synthetic dictionary only. A real dictionary for device records comes only at M9 **[gated]**: an external publisher and a first consumer decide when one ships, and until then no coordinate is computed for a real device record.

## 2. Store facts and two clocks [built]

- **Two clocks.** The object holds its writer's claim (`at`). The store keeps its own position (`seq`) and time (`received_at`) in a separate log entry, so an object's id is the same in every store ([store/0 Two clocks](../spec/store-0.md#two-clocks), [D13](decisions.md#d13-two-clocks)).
- **Late claims are counted, never rejected.** An object whose `at` is more than 300 seconds after its entry's `received_at` is counted in `counts.at_after_received`. It is never rewritten.
- **Checkpoints.** A checkpoint signed by the store key covers entries 1 to its `size`. `evidence store prove` gives inclusion and consistency proofs against it.
- **Compare-and-set.** `if_heads` refuses a stale append with `S_STALE_HEADS` and returns the current heads.
- **Time is claimed until M5.** Until M5 adds independent time, a store's `received_at` is that store's own claim; for a gateway's own store **[proposed]**, that is the gateway's claim, and a receiver's receipt bounds it ([section 6](#6-moving-records-between-sites-proposed)).

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

**Where the installer signs [gated].** By default the installer signs in a separate signing tool that shows the object capture made, through M4's verified read, not the app's own record. It needs no vendor's agreement and keeps the app away from installers' keys. Each installer holds their own key: a software key in that tool, or a hardware token that signs Ed25519, which attestation/0 requires. At M5 the installer list's administrators approve the binding of an installer's key to their id, with the directory keeper confirming identity off the log, and revocation is judged on the store's clock. An `approve` line from outside the writer process reaches the log through the assert intake. Installer-app vendors and the deployment decide these before EG3 ([question 6](#10-open-questions)); the synthetic EG1 vectors are signed with the command-line tool.

### 3.2 The approver list

Purpose alone cannot tell an installer's approval from a reviewer's, or from anyone else's: any valid `approve` line passes `--require approve`. So whose approvals count is published data, and an approval counts only when it comes from a listed approver: someone named on the approver list for the role a rule needs.

- **An approver list** (`approval/approver-list@0`) is an `assert` naming, by pseudonymous id, who may approve in which role, such as installer or reviewer. An installer's entry can carry the keyed reference the installer app uses, so a check can confirm that the installer who approved is the one the record names. That reference is shown only to parties entitled to see approval history, never in a public list ([approvals.md section 3](approvals.md#3-parties-and-ids-proposed)).
- **Approval rules** (`approval/rules@0`) say, for each kind of record, which roles must approve, how many distinct people in each, which list supplies each role, and which recorded facts add a review step. A kind of record can instead be compared, not approved: EG1's example rule set gives connection records such a rule, with no steps, whose comparisons only feed the disagreement view ([section 4](#4-when-records-disagree)).
- **At least two administrators change them.** A new version of a list or rule set counts only with `approve` lines from at least two distinct administrators (its `changes_need`) named in the version it replaces. Two is the minimum; a deployment may set more. Each new version carries a reason code (`reason`), which EG1's shapes add. The example list (`sha256:0cdc21d0…`) carries one `author` and two `approve` lines (`{"approve":2,"author":1}`).
- **The first version is anchored by configuration.** Deployment configuration names its administrators. Until M5 binds keys, whoever controls that configuration controls the policy, and every status says so; M5's security review revisits the anchor. Who publishes each list, and who its first administrators are, is decided by the deployment's publishing bodies **[gated]**; the examples use a synthetic publisher and two test administrators.
- **Keys are bound at M5.** Until then, list entries carry no keys, and approvals made with development keys are not accountability evidence ([roadmap decision 4](roadmap.md#decisions-confirmed-26-november-2026)). Key binding ties a key to a pseudonymous id, nothing more. A signature never shows that the signer holds a licence or accreditation, and a role on an approver list is a role in this layer only, as the list's publisher names it.

The shapes, checks and codes are in the design note [approvals.md](approvals.md).

### 3.3 Automated checks and agents

- **A check informs; people decide.** A check result is an `assert` by the checking tool, naming the checked object by id and its rule set by hash. It never counts as an approval. Its only effect is on review status, and only when the published rules list its result as needing review and the tool acts under a grant from a party the rules list. A record not yet approved then gets a review step at once, and awaits a listed reviewer's approval. On a record already approved, the check is only routed, as a prompt, to the party the tool acts for, and adds the step once that party accepts it ([approvals.md 7.2](approvals.md#72-doubt-from-a-tool)). A check never approves, changes or removes a record or an approval, and it has no effect outside this layer.
- **An AI agent is an actor** of kind `agent`, with `on_behalf_of` naming the accountable person or organisation. Its output counts as approved only through that party's `approve` (for an organisation, the `approve` of a person listed as acting for it). The example form check carries the tool's `author` line and the reviewer's `approve` (`{"approve":1,"author":1}`).
- **Conditions about a device are analytics detectors.** A firmware hash not on a published list, a battery capacity above a published threshold or a feeder on a published constraint list is not an escalation condition of this profile. Each is an analytics detector with a published rule, and states facts only: a measure, the rule that fired and a result code, never a score ([analytics.md](analytics.md#6-routing-to-people-proposed)). Its record-level flag adds a review step only through `check-flagged`, where a deployment's rules opt in that detector; the default rule opts in none. Each needs its input in the log, classified as [section 7](#7-privacy) says.

### 3.4 What the core counts today [built]

| Case | Core result | What the profile adds |
|---|---|---|
| The same `approve` line twice in one file | `ok`, `approve: 2` | Counts distinct people, not lines |
| `--require approve=2` | Usage error, exit 2 | The number required per role, as rule data |
| An agent's `approve` on its own output | `ok`, `approve: 1` | Only the `on_behalf_of` party's approval counts |
| A disagreement signed as its own purpose (`--purpose reject`) | Refused, `E_USAGE` | A position, recorded as an `assert` |

### 3.5 Review status is a view

Whether an object is approved is never a member of the object ([evidence/0 Not in v0](../spec/evidence-0.md#not-in-v0)). A view computes it from the log at a stated position, under the rules in force when the object was recorded. The answer is one of five words: **approved**, **awaiting approval**, **contested**, **superseded** or **unknown**. With each status, the view lists what would move it on and who can still supply it (`next`); it routes the question and assigns nothing. A missing rule, or a rule set or list with two current versions at the position that governs it, gives unknown; a competing version that lands later is reported for information only. Unknown is never read as approved or as refused. A connection record's rule says it is compared, not approved: its status is unknown with `P_COMPARED_NOT_APPROVED`, never `P_RULE_MISSING`, and its `next` is empty, because nothing in this layer would move it on. The network portal's own workflow stays authoritative for the connection itself, so no organisation signs anything for EG1. Until M5, every status says that accountability is not established.

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

- **Comparisons as rule data.** The rules name which values of a record to compare with which statements, linked by an equal keyed digest. When a current statement by a source the deployment names (in the examples, the gateway) differs from the record, a review step is added. A statement by anyone else is noted and adds nothing. A rule may also compare a record with another captured record about the same device in another system (`record-differs`), linked the same way. That condition ([approvals.md 7.1](approvals.md#71-recorded-facts-that-add-a-review-step)) is off by default; each record whose rule names it gets the step, and both systems must key the link under the same link-domain key. A connection record is compared, not approved: the differences its rule's comparisons find are shown in the disagreement view and add no step. Those comparisons are with statements of another type, as for `statement-differs`. Comparing it with another captured record, such as the commissioning record, is `record-differs`, which the view shows only where a rule names it (off by default).
- **A disagreement view** for each subject, built on M4's `state` and `history` reads and the approval-status view ([M4](roadmap.md#m4-memory-reads-index-query-api-bundles-read-only-service)): more than one head in a lane, a parent with children in more than one lane, and the differences the rules' comparisons find. It is computed from `prev` links over the whole history, with vectors for cases that must be flagged and cases that must not (the third row above, and the last row under a rule that does not name `record-differs`). In a check lane such as `assert:analysis/flag`, heads are grouped by detector: separate detectors' chains are expected, and only a fork within one detector's own chain is shown as a disagreement. At a receiving site, the view reads its own log and the receipted positions of each gateway replica ([section 6](#6-moving-records-between-sites-proposed)).
- **Positions.** A listed party (in the examples, an installer or a reviewer) who disagrees records a position (`approval/position@0`), an `assert` of their own. The object shows as contested until a person resolves it. A position never removes or alters an approval. A party can withdraw only their own approval, which then stops counting; the line stays in the log.
- **Resolution by a person.** A person records an `assert`, approved by a listed reviewer, naming the positions it settles and an outcome: the record stands, check it at the source, check it on the device, or refer it to another person. An outcome is not an instruction to anyone and has no effect outside this layer. This is the pattern of M3 resolutions. A profile check flags a resolution by an agent or a service (compare M3's `P_RESOLVER_NOT_HUMAN`); the store does not refuse it. A resolution never changes a record. Any change happens in the system of record or on the device, and reaches the log only through capture or a new statement. The default rule keeps a review step after a resolution whose outcome is a referral, or a check at the source or on the device (`follow-up-pending`), until a listed reviewer approves after it.

A difference between records is a prompt for review, not a conclusion.

## 5. Spreading approvals and uncertainty [proposed]

The design is about how uncertainty is shared and how decisions are enabled, not about who runs which system. Nothing is settled by counting objections. Instead:

- each party records its own view, signs it and stays accountable for it: capture what it observed, a gateway what it states, installers and reviewers what they accept or dispute;
- doubt adds a named reviewer, never a score;
- a listed reviewer who cannot assess an object records that (`unable-to-assess`); no count changes, and the view lists the other listed reviewers who can still answer;
- every status says what would move it on and who can still supply it (`next`); this routes the question and decides, gates or assigns nothing;
- an unknown stays unknown and is shown, never treated as a pass or a fail;
- a named person decides, and the decision is recorded with what it relied on;
- a decision that needs more than one approver names the approver list and the number required as published data; the status view counts those approvals from M4, and none of them is accountability evidence until keys are bound (M5).

Actor ids are pseudonyms: `person:` or `agent:` with 32 random hex digits; `service:` or `org:` may use a public name, but a device at someone's premises, a sole trader or an organisation named after a person gets an opaque id like a person. Only an access-controlled directory kept off the log links ids to people, except that an installer whose approver-list entry carries keyed record references can also be linked by the data owner, who holds the link-domain key and the system of record. A pseudonym is still handled as personal information. An AI agent is one kind of actor, not only a reader or a contributor: under a grant it can also raise doubt that adds a review step ([3.3](#33-automated-checks-and-agents)). The same hooks serve an agent, a case screen and a batch job: a grant to act for someone, a verified read, a recorded statement, an approval by the accountable person, and continuity notes kept between sessions under the grant, which are built at M6, after M5's erasure exists ([approvals.md section 9](approvals.md#9-continuity-for-tools-proposed)). The full design is in [approvals.md](approvals.md).

## 6. Moving records between sites [proposed]

**Transport.**

- Objects are content-addressed, attestations are signed and exports verify offline, whatever carried them.
- Entries after the last checkpoint are not yet signed, so a receiver accepts an export only up to its last checkpoint. In the checks, an export whose head was entry 20 verified `ok` with a covered size of 17.
- Any transport the operator's security policy allows: HTTPS upload, SFTP, object storage, removable media or a private mesh VPN **[gated]**. The deployment's operator chooses under its security policy, before EG3, and EG3 builds and tests one or two. How often a gateway exports sets how much a failed unit can lose and how tight a receipt's time bound is.
- A transport's identity is not a signing identity. At M5 a gateway's binding names its actor id and signing key, and also the log id and store key of its log, so a receiver can tie each log's checkpoints to the gateway. Nothing is bound through the transport, and a mesh VPN's node identity is never mapped to a signing key.

**Gateways.** A gateway that buffers through outages runs the native store (the command-line tool or the library): SQLite, a file lease on a local disk, and the store key for checkpoints. The WebAssembly module verifies, seals and signs, but has no store. Each gateway is its own log, with its own genesis and store key, and writes a checkpoint before every export. Its store name becomes the store's signer id (`service:evidence-store/<name>`), which is in the genesis, is the actor of every checkpoint and the signer of its witness line, and goes with every export and receipt. So the store name is opaque like the gateway's own id (for example the same 32 hex digits), never a site, address, meter or customer name.

- **Commits.** To limit flash wear, a gateway batches its statements into one commit on a timer or a count, and acknowledges each statement only once its batch has committed. An acknowledged append stays durable, as in M1; a power cut can lose only statements not yet acknowledged. The interval is measured on target hardware in EG3, with the ARM build; neither is checked yet.
- **Keys [gated].** By default a gateway's signing key and store key are generated on the gateway and never leave it, and an administrator the deployment names approves their binding at M5, as for every binding. For any trial with real devices, both keys are kept in hardware through M5's PKCS#11 signer backend, on hardware that signs Ed25519, which attestation/0 requires. The deployment and the device owners decide custody, and the deployment's security review decides whether hardware keys are required, before EG3 ([question 5](#10-open-questions)). Development keys are enough before M5.
- **Replacing or restoring.** A gateway never resumes its log from an older backup. If its store key survives, it restores from the last export a receiver accepted (`store import`) and continues that log. On new hardware, or when the key is lost, it starts a new log whose first statement names the old log id and its last accepted checkpoint in its content, not in `prev`. With keys held in hardware, a new log is the usual case. The receiver links the two logs, and the new log needs its own M5 binding (its log id and store key).
- **Rotating, not pruning.** A gateway's log only grows: no entry is ever deleted from it. After a receiver accepts an export, a gateway may rotate: it starts a new log in the same way, and the old local log can then be retired, because the receiver keeps its full history in the replica. When to rotate is a deployment setting. store/0 is unchanged.

**Sync.** A store/0 entry belongs to one log, and an object's content is written to one log only: its origin log, here the gateway's. Other logs refer to it by log id, position and entry hash.

- **A verified replica of each gateway's log.** The receiving site keeps each gateway's log as a replica, built from the gateway's exports and checked before use: before M5, with store/0's export verification and a consistency proof from the last checkpoint it accepted; from M5, with M5's replica checks (`S_REPLICA_TRUNCATED`, `S_REPLICA_FORK`). A replica holds only positions covered by a signed checkpoint. It is the gateway's whole log up to that checkpoint, so the receiver can check that log for completeness itself; EG2 exports carry no other completeness proof ([turn-reader decision 10](turn-reader.md#10-proving-completeness-to-a-remote-reader)).
- **Receipts for accepted positions.** For the positions it accepts, the receiver records a receipt `assert` in its own log. It covers positions of one origin log, names the checkpoint they were verified against, and lists for each position the origin log id, the position and the entry hash. One receipt can cover many positions, for example one export, and a position is receipted once. Nothing of the gateway's content is written to the receiver's log.
- **Views read receipted positions only.** A receiver's views (M4's `state` and `history`, the approval-status view and the disagreement view) include a replica's position only once a receipt in the receiver's log names it. So `statement-differs` compares a record in the receiver's log with a statement in a replica. A position is a pair, a log id and a position in that log; M4's design defines it, and the views use it.
- **One order across logs.** The receiver's own entries are in its log order. A replica's positions are ordered by the receiver's own log position of the receipt that names them, then by origin log position within that receipt. Ties cannot arise, so the same receiver position always gives the same order and the same status.
- **Checkpoints compared.** Each incoming checkpoint is compared with the last one received from that log, with a consistency proof. Two signed checkpoints of one log at the same size with different roots show that the store key signed two different histories. A gateway restored from an older backup that resumes the same log gives the same result, so a report is a prompt for review, not a conclusion. A new or rotated log is followed through the link its first statement carries, so the check spans them.
- **Full ingest is not adopted.** Copying the gateway's objects and lines into the receiver's log would hold content in two logs. The checks in the [appendix](#appendix-how-the-examples-were-checked) also show what it would have to work around: store/0's intake refuses the sender's genesis and checkpoints (`S_RESERVED_SUBJECT`) and their lines (`S_ATTESTATION_TARGET`), and refuses any object whose `prev` names one of them (`E_PREV_MISSING`).

**Time.** A receipt's `received_at` in the receiver's log bounds when the receiver knew of the content: the gateway recorded it no later than that. Inside a replica, `received_at` stays the gateway's own claim until M5 adds independent time.

## 7. Privacy

- **Synthetic or public data only until M5** ([store/0 Privacy](../spec/store-0.md#privacy)). An append-only log cannot forget.
- **Keyed digests, never bare hashes,** for device serials and installer references, under a key the data owner holds, one per link domain. If a link-domain key leaks, the digests can be guessed, and an approver list's keyed record references would then tie an installer's pseudonym to the installer. For a link domain that spans systems, such as device serials, one holder serves every capture service and gateway in it, named in deployment configuration, because a device links across systems only when all of them use the same key **[gated]** ([question 4](#10-open-questions)). Digests in the log cannot be re-keyed.
- **A gateway never holds a link-domain key.** It reads each device's serial and asks the data owner's key service for the keyed digest when the device is commissioned or replaced. So no key is kept on equipment at someone's premises, and the digest still comes from what the device reports. The key service answers anyone allowed to ask, and a caller could test guessed serials, so it authenticates each caller, tied to the gateway's M5 binding, and limits the rate; it still learns which serials were asked about. A gateway that cannot reach it sends statements that cannot be linked until the digest arrives. The data owner, as the key service's owner, decides this route before EG3 **[gated]** ([question 4](#10-open-questions)).
- **Pseudonymous actor ids,** never derived from a name (a sole trader's trading name included), email address, phone number, licence or business number, or an unkeyed hash of one ([approvals.md section 3](approvals.md#3-parties-and-ids-proposed); [turn-reader.md decision 12](turn-reader.md#12-privacy-of-actor-ids)). A pseudonym is still handled as personal information. The id is not the key; keys are bound to ids at M5. Until the M5 privacy and legal review decides the form **[gated]**, `person:` and `agent:` ids are 128 random bits assigned at enrolment.
- **Erasure comes at M5.** By-reference bodies can then be deleted; the log keeps ids, hashes, times and pseudonyms. Deletion follows the data owner's records-disposal rules. Section 24 of the Archives Act 1983 applies to any Commonwealth record (a record that is the property of the Commonwealth or of a Commonwealth institution, wherever it is held) and allows it to be destroyed only as required by law, with the National Archives' permission (for example, under a records authority), or under a normal administrative practice the Archives has not disapproved ([National Archives](https://www.naa.gov.au/information-management/disposing-information/disposal-commonwealth-records-absence-agency-specific-records-authority)). The APP 11.2 duty to destroy or de-identify personal information does not apply to personal information contained in a Commonwealth record ([OAIC](https://www.oaic.gov.au/privacy/australian-privacy-principles/australian-privacy-principles-guidelines/chapter-11-app-11-security-of-personal-information)). **[gated]** Who authorises an erasure, and what is checked first, are decided by the data owner with the M5 legal review, before erasure is used on real data. Until then, the default is that a redaction counts only when approved under a published rule by a role the data owner names on an approver list (for example its records manager), citing the data owner's disposal authority by reference, and only after a check that no disposal rule or pending access request stops the deletion.
- **No household attributes inline.** Objects never carry a household's address, occupants or circumstances. Device settings and measurements from equipment at a home are still information about that household, so each value is classified like any captured column: inline, keyed, by reference or left out, as in M2's mapping/0 ([M2](roadmap.md#m2-read-only-capture-from-a-synthetic-register)). A measurement summary is coarse, for example a maximum over a day or longer, never a time series. Any value not classified as non-personal, a measurement summary included, is never inline: it is kept by reference, or as a keyed digest where it must be linked. Prioritising records for review, and any scoring of households or people, are out of scope. Analytics flags state facts against published rules and carry no score member ([analytics.md](analytics.md)); nothing scores, ranks or profiles people or households.

## 8. Fit with the roadmap

| Milestone | What this profile uses or adds |
|---|---|
| [M1](roadmap.md#m1-verifiable-store-append-only-log-checkpoints-offline-verification) **[built]** | The store: two clocks, lanes and heads, checkpoints, offline export verification |
| [M2](roadmap.md#m2-read-only-capture-from-a-synthetic-register) | Capture from a synthetic register; `mapping/0` keys a record id that is not opaque before it becomes a subject key. A synthetic edge workload for the simulator **[proposed]**. Real data only after M5 |
| [M3](roadmap.md#m3-two-way-reconciliation-report-only-recorded-as-evidence) | Reconciliation reports; a difference in one of the five classes the default rule names adds a review step **[proposed]** |
| [M4](roadmap.md#m4-memory-reads-index-query-api-bundles-read-only-service) | The approval-status view, and the disagreement view built on it and on M4's `state` and `history` reads, with vectors **[proposed]**; the position across logs (a log id and a position) that the views use for gateway replicas. M4b: a synthetic dictionary for device records **[proposed]**. Late M4 (early M6 at the latest): the assert intake, which runs the id-form, authority (the record-lane writer among them) and grant checks before appending, and refuses an object that fails one, naming every failed code |
| [M5](roadmap.md#m5-real-data-gate-trust-time-replication-privacy-operations) | Key binding, extended **[proposed]** from people and services to organisations, gateways and agents (an agent's binding names its accountable party; a gateway's also names its log id and store key); approver lists bound to keys; revocation judged on the store's clock; the replica checks that gateway replicas use; erasure; legal review |
| [M6](roadmap.md#m6-requests-and-the-outbox-as-evidence-in-shadow-mode-no-write-code), M7 | Continuity notes and their recall (M6, after M5's erasure). Requests in shadow mode, then writes to the synthetic database only. Writing to any other system is a separate decision for that system's owner |
| M9 | A real published dictionary for device records, with its own profile tag ([section 1](#1-two-kinds-of-edge-object-proposed)); Kernel/2 stays for certificate lifecycles. **[gated]** on an external publisher and a first consumer; synthetic only until then |

**Edge track [proposed].** The steps EG1-EG4 are planned only in the [roadmap's edge track](roadmap.md#edge-track). This profile supplies EG1's examples and the gateway set-up in [section 6](#6-moving-records-between-sites-proposed), and the requirements marked EG2 below. EG2's sync is the replica-and-receipt design of section 6. A trial with real devices (EG3) comes only after M5 **[gated]**: the device owners, the systems' owners and the legal review decide whether and when. The record classification, subject keys, link-domain keys, gateway keys, digest route and transport above come first.

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
| REQ-EDGE-008 | A record-lane object by an actor other than a named capture service is refused at the assert intake (`P_RECORD_LANE_WRITER`). One that reaches the log another way is flagged with that code, and its status is unknown. It never supersedes the captured record, which keeps its status and is still read as `observed`. | None | New, M4; intake late M4 (early M6 at the latest) |
| REQ-EDGE-009 | Review status is one of the five words of [3.5](#35-review-status-is-a-view), and the same position always gives the same status; at a receiving site, a position covers its own log and the receipted positions of each replica ([section 6](#6-moving-records-between-sites-proposed)). A connection record's status is unknown with `P_COMPARED_NOT_APPROVED`, never `P_RULE_MISSING`. | In part: object vector 113 and T3 (an `approve` is required) | New, M4 view and approver list; keys M5 |
| REQ-EDGE-010 | The disagreement view flags each case of [section 4](#4-when-records-disagree) that it should under the rules in force, and nothing else: the same device in two systems only under a rule that names `record-differs`, and separate detectors' chains in one check lane never. | In part: record-lane forks (store export 005, T4, T5) | New, M4 |
| REQ-EDGE-011 | An `approve` counts toward a rule only from a person listed for the role at the line's position, in their own name or for an organisation they are listed as acting for; on an agent's assert, only when signed as its `on_behalf_of`, or, where that is an organisation, by a person listed as acting for it. An `approve` signed as an agent, a service or an `org:` id never counts. | None | New, M4; keys M5 |
| REQ-EDGE-012 | An object's status is unknown (`P_POLICY_AMBIGUOUS`) when the rule set in force at the object's position, or the approver list in force at one of its approval lines' positions, has two current versions. A competing version that lands later leaves the status unchanged and is reported for information only. | None | New, M4 |
| REQ-EDGE-013 | A new version of a list or rule set counts only with `approve` lines from at least two distinct administrators (its `changes_need`) named in the version it replaces. | None | New, M4 |
| REQ-EDGE-014 | A position never removes an approval: every `approve` line stays held and verifiable, and only the status changes. | None | New, M4 |
| REQ-EDGE-015 | `actor.id`, `on_behalf_of` and signers follow the pseudonym form; others are refused at the assert intake (`P_ACTOR_ID_FORM`), naming every failed code, and re-checked at verification. | None | New, M4; intake late M4 (early M6 at the latest) |
| REQ-EDGE-016 | A receiver keeps each gateway's log as a verified replica and writes none of its content into the receiver's own log. Each receipt covers positions of one origin log, lists for each the origin log id, the position and the entry hash, and verifies against the replica and the origin log's signed checkpoint that it names. | In part: this document (an export verifies offline, which a replica's check builds on) | New, EG2 with M4; replica checks M5 |
| REQ-EDGE-017 | A receiver accepts positions only up to the origin log's last checkpoint, and its views include only receipted positions, ordered by the receiver's log position of the receipt and then by origin log position within it. The same receiver position always gives the same order. | In part: this document (an export whose head was entry 20 verified with a covered size of 17) | New, EG2 with M4 |
| REQ-EDGE-018 | Two signed checkpoints of one log at the same size with different roots are reported. The check follows a gateway's new or rotated logs through the link each one's first statement carries. | None | New, EG2 |

## 10. Open questions

1. Who publishes which approver list, and who are its first administrators (at least two)? Is a list anchored in deployment configuration acceptable until M5?
   - **Decided (28 November 2026):** a new version of a list or rule set needs `approve` lines from `changes_need` distinct administrators named in the version it replaces: at least 2, and a deployment may set more. Until M5, the first version is anchored by deployment configuration that names its administrators, every status says so, and M5's security review revisits the anchor. The rule set names which list supplies each role ([3.2](#32-the-approver-list)).
   - **Gated:** the deployment's publishing bodies decide who publishes each list and who its first administrators are, before any real deployment (M5). Until then, the examples and vectors use a synthetic publisher and two test administrators, and this profile names no body.
2. For commissioning records, is the installer's approval enough unless a recorded condition adds a reviewer (as in the examples), or is a reviewer always needed? Which conditions apply?
   - **Decided (28 November 2026):** the installer's approval is enough unless a named recorded condition adds a reviewer. The default rule names `record-fork`, `statement-differs`, `open-position`, `reconcile-difference` (the classes `untracked_edit`, `differs`, `missing`, `sor_regressed` and `resurrected`), `check-flagged` (the form check's `fields-missing` and `unable-to-check`, from the reviewer role or tools under a reviewer's grant) and `follow-up-pending`. It leaves `basis-stale` and `record-differs`, the comparison of two captured records about one device, to deployments, and opts in no analytics detector. Conditions about a device itself are analytics detectors with published rules, never scores ([3.3](#33-automated-checks-and-agents)). Connection records get an explicit rule: compared, not approved ([3.5](#35-review-status-is-a-view)). Each deployment publishes its own rules ([approvals.md 5.2](approvals.md#52-approval-rules)).
3. Under the record-type rule (a record kept in another system enters as a captured record change; a gateway's own settings or measurement summaries are its own statements), in any trial, which records of each system fall under which? Is each system's record id opaque enough to be a subject key as it is?
   - **Decided (28 November 2026):** a record id that is not opaque is captured as a keyed digest, as M2's `mapping/0` keys a column, never in the clear ([section 1](#1-two-kinds-of-edge-object-proposed)).
   - **Gated:** the deployment, with each system's owner and data owner, classifies that system's records under the rule, and each data owner, with the M5 privacy and legal review, assesses whether its ids are opaque, at trial or capture set-up after M5. A trial with real devices itself waits for the device owners, the systems' owners and the legal review (EG3). Until then, only the demo systems are classified, as in section 1, and every subject key is synthetic.
4. For each link domain (device serials, installer references), which data owner holds the key ([roadmap decision 8](roadmap.md#decisions-confirmed-26-november-2026))? How does a gateway's statement get its keyed digest without the key leaving that owner's key service? (Proposed: the gateway asks the key service when a device is commissioned or replaced.)
   - **Gated:** the data owners of the participating systems, with the deployment and the M5 legal review, decide who holds each key; the data owner, as the key service's owner, decides the digest route; and each system's owner agrees read access and how each captured column is classified. All of this comes before any real data (M5) and before EG3. Until then, one holder per cross-system link domain is named in deployment configuration, the examples use the published test key, and a gateway asks the key service for each digest at commissioning or replacement, as an authenticated, rate-limited caller ([section 7](#7-privacy)).
5. Who holds a gateway's signing key and store key, and how are they bound at M5?
   - **Gated:** the deployment and the device owners decide who holds a gateway's keys, and the deployment's security review decides whether they must be in hardware, before M5 binds gateway keys and before EG3. Until then, development keys only. The default is that both keys are generated on the gateway and never leave it, an administrator the deployment names approves the binding (actor id and signing key, log id and store key), and for a trial with real devices both keys are kept in hardware through M5's PKCS#11 signer backend ([section 6](#6-moving-records-between-sites-proposed)).
6. Where does an installer sign `approve`: in the installer app, if its vendor agrees to build that, or in a separate tool? The installer signs the object capture made, not the app's record, so whichever tool they use must show them that object. Does each installer hold their own key, or would the app hold keys for them? Who binds an installer's key to their id at M5, and who revokes it? How is one job kept from needing two sign-offs?
   - **Gated:** installer-app vendors and the deployment decide where installers sign and who holds their keys, and the deployment decides who binds and revokes them, before EG3. Until then, the default is a separate signing tool that shows the captured object through M4's verified read, a key each installer holds themselves, and a binding approved by the installer list's administrators, with the directory keeper confirming identity off the log ([3.1](#31-who-signs-what)). The synthetic EG1 vectors are signed with the command-line tool.
   - **Open:** how one job is kept from needing two sign-offs. Which duplication is meant must be settled first: the app's own completion step plus an `approve` here, or an `approve` on each captured version of one job's record.
7. How is a gateway replaced or restored: from the last export a receiver accepted, or as a new log that names the old one?
   - **Decided (28 November 2026):** by case. A gateway never resumes its log from an older backup. If its store key survives, it restores from the last export a receiver accepted and continues that log; on new hardware, or with a lost key, it starts a new log whose first statement names the old log id and its last accepted checkpoint in its content, not in `prev`. A gateway's log is never pruned: it may rotate to a new log in the same way after an accepted export, and the receiver keeps the full history ([section 6](#6-moving-records-between-sites-proposed)).
8. Sync: ingest, or one replica per gateway? A candidate rule: an object's content is written to one log only, and other logs refer to it by log id, position and hash. That favours one replica per gateway, or an ingest that keeps only a receipt. Which transport?
   - **Decided (28 November 2026):** content stays in its origin log. The receiver keeps a verified replica of each gateway's log and records a receipt `assert` in its own log for the positions it accepts (origin log id, position and entry hash). Its views include only receipted positions, ordered by the receipt's position in the receiver's log and then by origin log position. M4's design defines the position across logs, and the views use it. Full ingest is not adopted ([section 6](#6-moving-records-between-sites-proposed), REQ-EDGE-016 to -018).
   - **Gated:** the deployment's operator decides the transport under its security policy, before EG3. Until then, any transport that policy allows (HTTPS upload, SFTP, object storage, removable media or a private mesh VPN), with no transport identity mapped to a signing key; EG3 builds and tests one or two.

Open questions 3 to 9 of [approvals.md](approvals.md#15-open-questions) apply here too, and are settled there. EG1 needs no organisation signing (question 9), because connection records are compared, not approved.

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
