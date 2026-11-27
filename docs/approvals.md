# Approvals, disagreement and tool hooks (design note)

**Status:** draft for discussion, 27 November 2026. Design input for [M4](roadmap.md#m4-memory-reads-index-query-api-bundles-read-only-service) (reads and views), [M5](roadmap.md#m5-pilot-gate-trust-time-replication-privacy-operations) (key binding), [M6](roadmap.md#m6-requests-and-the-outbox-as-evidence-in-shadow-mode-no-write-code) (requests) and the proposed edge track, whose profile is in [cer-edge.md](cer-edge.md). Nothing here is built. All data is synthetic, and every evidence object in the examples verifies with the `evidence` command-line tool ([Appendix](#appendix-how-the-examples-were-checked)). This repository is not affiliated with any agency, and this note does not describe any agency's process.

**Labels.** **[built]** is in the code today. **[proposed]** is a design for a later milestone. **[open]** is a decision still to be made.

## Terms

Each term in one line; [evidence/0](../spec/evidence-0.md) and [store/0](../spec/store-0.md) give the detail.

- **Object:** an evidence/0 record whose id is the hash of its content, so it never changes ([evidence/0](../spec/evidence-0.md#the-object)).
- **Assert:** an object that states something (`op: assert`), such as a gateway's settings, a check result or an approver list.
- **Attestation:** a signed line kept beside an object. Its purpose is `author` (I made this), `approve` (I accept this as the accountable person or body) or `witness` (I saw this at this time) ([evidence/0](../spec/evidence-0.md#attestations)).
- **Log:** a store/0 store. One writer appends objects and attestations in order and never changes or removes them.
- **Lane:** the objects about one subject are kept in lanes: `record` for record changes, and one lane for each kind of assert ([store/0](../spec/store-0.md#lanes-and-heads)).
- **Head:** a current version: an object in a lane that no later object in that lane replaces. Two heads in one lane are two unchained versions.
- **Watermark:** a log position. Every status here is computed from the log up to a stated watermark.
- **Checkpoint:** the store's signed summary of the log up to a position, which a reader can cite and check ([store/0](../spec/store-0.md#checkpoint-and-tree)).
- **Capture service:** the service that observes changes in another system and records them in the record lane.
- **Intake:** the step where a store accepts or refuses an item before appending it. store/0 has its own; this note proposes an assert intake above it.
- **`on_behalf_of`:** the person or organisation accountable for an object that a tool made.
- **Grant:** a published object by which a person lets a named tool act for them, within stated limits.
- **Profile finding (`P_` code):** a code a profile check reports about the log, such as an approval line that does not count. It is not a judgement about any person.

## In short

- **No format change.** evidence/0 stays frozen and store/0 is unchanged. Everything here is one of three things: data published as `assert` objects, profile checks that report `P_` codes, or views computed from the log at a stated position.
- **Who may approve is published data.** An approver list says who holds which role. Approval rules say, for each kind of record, which approvals are enough. Changing either needs approvals from two named administrators.
- **Whether something is approved is a view, not a field.** Anyone holding the log can recompute it and get the same answer. The answer is one of five words: approved, awaiting approval, contested, superseded, unknown.
- **Any party can record doubt; only accountable parties settle it.** Named, recorded facts add a review step: a gateway statement that differs from the record, or a check whose result the rules list as needing review, made by a listed party or by a tool acting for one. A listed reviewer who disagrees records a position, and the object shows as contested until a person resolves it. Anyone else's position or check is shown as noted and changes nothing.
- **Tools inform; people decide.** On an object already approved, a tool's check changes nothing by itself: it is routed to the person the tool acts for, and changes the status only once that person accepts it. Only a listed person or organisation fills a review step, and only a person, with a listed reviewer's approval, settles a disagreement. Nothing is decided by counting positions, and no score decides anything.
- **The view says what would move a status on.** Each status lists the approvals or the resolution still needed, and who can still give them. The layer enables a decision without making one.
- **One set of hooks for every kind of tool.** The core and the hooks do not know what a tool is. A person at a case screen, a batch job and an AI agent use the same objects: a grant to act for someone, a verified read, a recorded statement, an approval by the accountable person, and continuity notes kept between sessions. An AI agent may read, record checks, raise doubt, witness and keep continuity notes; under this profile it never fills an approval step or settles a disagreement.
- **Ids are pseudonyms.** An actor id names nobody, and linking it to a person needs the directory, which is kept off the log. A pseudonym is still handled as personal information, because the records linked to it can help identify someone. An id is never a key and never a coordinate.
- **Before key binding (M5), none of this is accountability evidence.** It shows what was recorded and in what order, not who stood behind a key.

## 1. What the core shows today [built]

evidence/0 deliberately leaves trust policy and status to profiles and views ([evidence/0, Not in v0](../spec/evidence-0.md#not-in-v0)). These results, from the tool at commit a6bbb94, show what a profile has to add:

| Case | Core result | Why a profile is needed |
|---|---|---|
| The same `approve` line twice in one file | `ok`, `approve: 2` | Counts are lines, not people |
| One key signs `approve` under two names | `ok`, `approve: 2` | Two names are not two people |
| `--require approve=2` | usage error, exit 2 | `--require` checks presence only |
| An agent signs `approve` on its own output | `ok`, `approve: 1` | Nothing ties an approval to the accountable person |
| `on_behalf_of` names another agent | `ok` | The accountable party can be another tool |
| An `author` line signed under a name other than `actor.id` | `ok` | The signer is free text |
| An actor id that is an email address | `ok` | Actor ids have no privacy rule |
| A second first version of a rule set appended without `if_heads` | appended; `store verify` gives `ok`, forks 0 | store/0 counts forks only in record lanes ([store/0](../spec/store-0.md#lanes-and-heads)) |
| A person's `update` in a record lane | appended; it replaces the captured record as the lane's head | The record lane is reserved for capture by policy ([D11](decisions.md#d11-observe-before-writing)), not by store/0 intake |

None of these is a defect of the core. They are the gaps this note fills, without changing the core.

## 2. Principles

1. **No format change.** New things are published `assert` objects, profile checks and views.
2. **The writer orders the log; signers decide.** The store appends and checkpoints but approves nothing.
3. **Rules are data.** Approver lists and approval rules are versioned objects, approved before use. An object is judged under the rules in force when it was recorded.
4. **Every status comes from a fixed list.** Unknown is never read as approved or as refused.
5. **Missing or ambiguous inputs fail closed.** No rule, a list with two current versions, or an unapproved rule set gives `unknown`.
6. **Any party can record doubt; only accountable parties settle it.** Doubt adds a review step, never a score, and only when it comes from a listed party or from a named, recorded fact. Anyone else's doubt is shown as noted.
7. **Disagreement informs and never blocks.** It changes a status to contested and waits for a person. It never removes an approval, and it has no effect of its own outside this layer.
8. **Only accountable parties approve.** An approval that counts comes from a person or an organisation on a list, or from the accountable person named on a tool's output. Tools sign `author` and `witness`.
9. **The same objects for every actor.** The core knows only `actor.kind` and `on_behalf_of`, and so do the hooks.
10. **The layer enables decisions and makes none.** A view says what is missing and who can supply it. A person decides. Tool output that no person has accepted never changes an approved object's status, and never makes a person's approval stop counting.
11. **Checks are pure.** They read the log and write nothing. Reads never write.
12. **Nothing here is accountability evidence before M5.** Every status output says so.
13. **An id is not a key, and a pseudonym is not derived from personal data.**

## 3. Parties and ids [proposed]

**Form.** Every `actor.id`, `on_behalf_of` value and attestation signer is `<kind>:<id>`:

- `person:` and `agent:` ids are 128 random bits written as 32 lowercase hex digits, assigned at enrolment by the body that keeps the directory. A deployment's configuration names that body; this note names no such body. Example: `person:9cd0281eb4bd58de76acdefcb2125ab2`.
- `service:` and `org:` ids may be public names (`service:capture-installer-app-test`), except a service located at someone's premises, such as a gateway, which gets an opaque id like a person. An `org:` id is a public name only for a body whose name does not identify an individual. A sole trader, or any organisation named after a person, gets an opaque id like a person.
- An id is never derived from a name, email address, phone number, licence number or business number, and never an unkeyed hash of one: anyone could hash candidates and compare ([evidence/0 Privacy](../spec/evidence-0.md#privacy)).
- The prefix matches `actor.kind`: `person:` for `human`, `agent:` for `agent`, `service:` or `org:` for `service`.

**One pseudonym per party per directory,** so a check can count each party once and an authorised audit can confirm whose approvals counted. Linking across directories goes only through the directories.

**The id is not the key.** M5 key-binding objects bind keys to ids, with validity and revocation, so a key can be rotated or revoked without the id changing.

**Who can resolve an id:**

| Access | Sees |
|---|---|
| Public | Published approver lists (pseudonyms and roles), and status views that leave out signers, position authors and the `from` and `not` lists |
| Parties entitled to see approval history | Logs and exports that hold approval lines, under M4's closed-by-default read scopes ([roadmap](roadmap.md#m4-memory-reads-index-query-api-bundles-read-only-service)): who approved what, and whether the signatures verify |
| Auditor | Also key bindings and grants |
| Only the body that keeps the directory | The directory that maps pseudonyms to people. It is access-controlled, keeps its own access log, and is never appended to the store |

Existence checks for public access take opaque ids only, never values derived from content or identity, so they cannot be used to confirm a guess.

**Never coordinates.** Actor ids and approval status are never facets of a derived index. A coordinate is decodable by design, and approvals arrive after the object they approve.

**Where the rule is enforced.** The assert intake ([turn-reader open decision 7](turn-reader.md#7-an-intake-for-asserts)) refuses an object whose actor id breaks the form (`P_ACTOR_ID_FORM`), because the log cannot forget. Verification repeats the check, because a verifier never relies on intake. In the checks for this note, the core accepted an email-style actor id, and store/0 keeps whatever is appended in every later export and copy ([store/0 Privacy](../spec/store-0.md#privacy)).

**The examples use these synthetic parties:**

| Id | Role in the examples |
|---|---|
| `person:be38edba8ffe69d727480c01fb563a27` | Administrator A |
| `person:4411e8ec9efece29df272d7a3ffd5f7a` | Administrator B |
| `person:b0323effae5bd18bd0e5632e2b6cf284` | Installer |
| `person:9cd0281eb4bd58de76acdefcb2125ab2` | Reviewer 1 |
| `person:722113fd222cf1277ef6c48ba08790cc` | Reviewer 2 |
| `agent:689db15aeff6b2425b9b1e2e249f7934` | A form-checking tool acting for reviewer 1 |
| `service:4275021583e8e644a10f5e5565e09600` | A gateway |
| `service:capture-installer-app-test` | The capture service |

## 4. Who signs what [proposed]

| Who | Records | Signs |
|---|---|---|
| Capture | Record changes it observed, in the record lane | `author` |
| A gateway | Its own settings or measurements, as an `assert` | `author` |
| An automated check or an AI agent | Its output, as an `assert` with `on_behalf_of` | `author` |
| The installer | Nothing new: accepts the captured record | `approve` |
| A listed reviewer | Accepts an object, or records a position or a resolution | `approve`, or `author` on their own assert |
| An administrator | Approver lists and approval rules | `author`, `approve` |
| Anyone | That they saw an object at a time | `witness` |

Profile rules, all checked from the log:

- `P_AUTHOR_NOT_ACTOR`: an `author` line's signer must equal `actor.id`.
- `P_SPONSOR_NOT_ACCOUNTABLE`: `on_behalf_of` must be a `person:` or `org:` id.
- `P_APPROVER_NOT_ACCOUNTABLE`: an `approve` signed as an `agent:` or `service:` id never counts toward a rule.
- `P_SELF_APPROVAL`: an approval by the object's own actor or author does not count, unless the step or the policy-change rule ([5.3](#53-changing-a-list-or-a-rule-set)) says so (a person signing their own resolution, as in M3).
- `P_KEY_SHARED`: a key that appears under more than one signer name in the log up to the watermark; none of its lines count. Before M5 this is the only check of keys against names. It does not detect a new key signing under someone else's name; only M5 key binding does.
- `P_DUPLICATE_SIGNER` (informational): one signer counts once, toward one step.

## 5. Approver lists and approval rules [proposed]

### 5.1 The approver list

The approver list is published data saying who may approve in which role. It is an `assert` with schema `approval/approver-list@0`, in lane `assert:approval/approver-list` of its own subject. Entries carry no keys; keys are bound at M5. An entry may carry keyed references to the person as the system of record knows them (`record_refs`), so a check can confirm that the installer who approved is the one the record names, without the system of record's id appearing in the log.

```json
{
  "v": "evidence/0",
  "id": "sha256:0cdc21d0d8fabb8e52f27f3918f75076e8dfee5a1364ec7a373a6a074b6d3464",
  "op": "assert",
  "schema": { "ref": "approval/approver-list@0", "hash": "sha256:de59f3e09082863b6e5a4485e7de00949efe777e0f199b7d8b565e076ca3db8e" },
  "subject": { "system": "edge-demo-policy", "type": "approver-list", "key": "commissioning-review" },
  "content": {
    "media_type": "application/json",
    "hash": "sha256:63fcc3bcfe8d6757ae0b06c3dfba79e0c37d775a103f7b2ce9b8a644c79083fb",
    "inline": {
      "publisher": "org:publisher-test-01",
      "administrators": ["person:be38edba8ffe69d727480c01fb563a27", "person:4411e8ec9efece29df272d7a3ffd5f7a"],
      "changes_need": 2,
      "valid_from": "2026-11-26T00:00:00Z",
      "valid_until": "2027-11-26T00:00:00Z",
      "entries": [
        { "id": "person:b0323effae5bd18bd0e5632e2b6cf284", "role": "installer",
          "record_refs": ["hmac-sha256:04d42d217df7830a613ded2960b601d244f8690c1772793fd0ff49d4a02f884d"] },
        { "id": "person:9cd0281eb4bd58de76acdefcb2125ab2", "role": "reviewer" },
        { "id": "person:722113fd222cf1277ef6c48ba08790cc", "role": "reviewer" }
      ],
      "keys": "unbound"
    }
  },
  "actor": { "id": "person:be38edba8ffe69d727480c01fb563a27", "kind": "human" },
  "at": "2026-11-26T09:00:00Z",
  "prev": []
}
```

Administrator A signs `author`; administrators A and B sign `approve`. `evidence verify --require author --require approve` gives `ok` with `{"approve":2,"author":1}`.

Different bodies can publish their own lists for their own roles, for example installers and reviewers. The approval rules say which list supplies which role.

### 5.2 Approval rules

The rule set is an `assert` with schema `approval/rules@0`, one per deployment, versioned and approved like the list. For each kind of object it says:

- `steps`: which roles must approve, and how many distinct people in each;
- `escalate`: which recorded facts add a step ([7.1](#71-recorded-facts-that-add-a-review-step)); for a check, which kinds of check, which results and whose checks count; for a reconciliation report, which classes;
- `compare`: which fields to compare with another record type, and on which keyed link;
- `positions_from`: whose recorded disagreement makes the object contested.

The rule for commissioning records in the examples (rule set id `sha256:6c36f811…`, `{"approve":2,"author":1}`):

```json
{
  "applies_to": { "schema": "installer-app-demo/commissioning", "lane": "record" },
  "steps": [ { "name": "installer-sign-off", "role": "installer", "count": 1, "named_in_record": "installer_ref" } ],
  "escalate": [
    { "when": "record-fork",          "add": { "name": "review", "role": "reviewer", "count": 1 } },
    { "when": "statement-differs",    "add": { "name": "review", "role": "reviewer", "count": 1 } },
    { "when": "open-position",        "add": { "name": "review", "role": "reviewer", "count": 1 } },
    { "when": "reconcile-difference", "add": { "name": "review", "role": "reviewer", "count": 1 },
      "classes": ["untracked_edit", "differs", "missing", "sor_regressed", "resurrected"] },
    { "when": "check-flagged",        "add": { "name": "review", "role": "reviewer", "count": 1 },
      "checks": ["edge-demo/form-check"], "flag_results": ["fields-missing", "unable-to-check"],
      "checks_from": ["reviewer"] }
  ],
  "compare": [ { "field": "export_limit_kw", "link": "device_serial_digest",
                 "with": { "schema": "edge-demo/gateway-settings", "field": "export_limit_kw" } } ],
  "positions_from": ["installer", "reviewer"]
}
```

The same rule set also says that a resolution needs one listed reviewer, who may be its author; that a grant needs its grantor's approval; and that any agent's output needs the approval of its `on_behalf_of`. The reason codes a position may give are listed in the rule set too, so positions carry codes, not free text. M6's rules (`P_APPROVAL_REQUIRED`, and two people for an overwrite on a stale base, [roadmap](roadmap.md#m6-requests-and-the-outbox-as-evidence-in-shadow-mode-no-write-code)) can be written as rules of this kind.

### 5.3 Changing a list or a rule set

- **Two administrators.** A version counts only with `approve` lines from `changes_need` (at least 2) distinct administrators named in the version it replaces. The author line itself never counts. The author's own `approve` line counts toward `changes_need`: `P_SELF_APPROVAL` does not apply to approver lists and rule sets. So a change needs approvals from two distinct administrators, who may include its author.
- **The first version** is anchored by deployment configuration, which names its administrators. Until M5 binds keys, whoever controls that configuration controls the policy; the status output says so.
- **One current version.** A new version lists the old one in `prev` and is appended with `if_heads`. If the rule set in force at an object's own position has two current versions, the object's status is `unknown` (`P_POLICY_AMBIGUOUS`). So is the status of an object with an approval line recorded while the approver list in force at that line's position has two current versions; that line is not reported as `P_APPROVER_NOT_LISTED`. A competing version that lands after the object leaves the object's rule set in force, and the view reports it for information (`P_POLICY_AMBIGUOUS`, marked informational). In the checks, a competing first version appended with `--if-heads none` was refused with `S_STALE_HEADS`; appended without `if_heads` to a copy of the store, it landed, `store verify` still reported forks 0, and only the profile check caught it.
- **Judged by the rules in force when the object was recorded.** The rule set that governs an object is the approved, single current version at the object's own log position. A later version does not change past statuses; if it would give a different answer, the view reports `P_RULES_CHANGED` for information. That rule set also says which list supplies each role, and list membership is judged at each approval line's own log position. Removing someone from a list stops their later approvals counting and leaves their earlier ones in place.

## 6. Approval status [proposed]

For one object at a watermark (a log position), the view:

1. finds the rule set in force at the object's position, and the rule for its kind (none: `unknown`, `P_RULE_MISSING`);
2. adds a step for each escalation condition that holds at the watermark;
3. fills the steps from the object's `approve` lines in log order, applying the rules of [section 4](#4-who-signs-what-proposed) and the lists in force at each line;
4. gives one status, in this order of precedence;
5. says what would move the status on (`next`).

| Status | When |
|---|---|
| `unknown` | The rule set or a list is missing, unapproved or has two current versions, or a record-lane object was not written by a capture actor (`P_RECORD_LANE_WRITER`) |
| `superseded` | The object is no longer a head of its lane |
| `contested` | An open disagreement from a listed party, or two unchained heads in the object's own assert lane, for a schema that keeps one current version per subject (in the examples: approver lists, rule sets, grants, resolutions and gateway settings), counting only heads with no form or grant finding |
| `approved` | Every step is met |
| `awaiting-approval` | Otherwise |

Prose writes `awaiting-approval` as "awaiting approval". Positions and checks are independent statements, so two of them on one object never make each other contested.

The output always names the rule set, each step with who filled it, each line that did not count and why, the escalations, the positions, the checks it considered, what would move the status on, and `"accountability": "not-established-before-key-binding"` until M5. Here is the commissioning record at log position 19 of the examples (ids shortened, some fields left out):

```json
{
  "status": "contested",
  "watermark": 19,
  "rules": "sha256:6c36f811…",
  "escalations": ["open-position", "statement-differs"],
  "disagreement": "open",
  "steps": [
    { "name": "installer-sign-off", "role": "installer", "needed": 1, "counted": [ { "seq": 13, "signer": "person:b0323eff…" } ] },
    { "name": "review", "role": "reviewer", "needed": 1, "after": 17, "counted": [] }
  ],
  "positions": [
    { "id": "sha256:ffe43ac8…", "by": "person:722113fd…", "position": "disagree", "reason": "device-setting-differs", "counts": true, "settled": false },
    { "id": "sha256:a5204340…", "by": "person:9cd0281e…", "position": "unable-to-assess", "reason": "cannot-inspect", "counts": true, "settled": false }
  ],
  "next": [
    { "need": "approve", "step": "review", "role": "reviewer", "count": 1, "from": ["person:722113fd…"], "not": ["person:9cd0281e…"] },
    { "need": "resolution", "positions": ["sha256:ffe43ac8…"] }
  ],
  "not_counted": [],
  "findings": [],
  "accountability": "not-established-before-key-binding"
}
```

The status is a review status. It is not a finding about the device, the installer or anyone else, and it has no effect outside this layer. Any follow-up happens in the system of record or on the device. The status `approved` means only that the approvals this deployment's published rules ask for are present. An `approve` line records that a key holder accepted an object in this layer. Neither is an approval, declaration, certificate or signature under any scheme, licence or law, before or after M5.

### What would move a status on

`next` is derived from the same log position as the status. It lists what is still needed and who can still supply it. It routes the question; it never decides, gates or assigns anything.

| `need` | When | It names |
|---|---|---|
| `approve` | A step is not yet met | The step, its role, how many approvals are still needed, `from` (who can still fill it) and `not` (listed parties whose latest act on the object is unable to assess) |
| `resolution` | A listed party's disagreement is open | Every open position a resolution must name |
| `new-version` | The object's own assert lane has two unchained heads, for a schema that keeps one current version | The heads a new version would list in `prev` |
| `review-prompt` | A tool's flagged check on an object that was approved when the check was recorded, not yet accepted by the tool's `on_behalf_of` ([7.2](#72-doubt-from-a-tool)) | The check, and `from`: the `on_behalf_of` who can accept it |
| `review-findings` | The status is `unknown` | The codes that made it unknown |

`from` holds the parties listed for the step's role at the watermark (or the `on_behalf_of` or grantor the step names), leaving out anyone already counted, the object's own actor and authors unless the step allows them, and anyone whose latest act on the object is unable to assess. Those last are listed under `not`. An approved object's `next` is empty apart from any `review-prompt`, and a superseded object's `next` is empty.

### The examples, step by step

A prototype of the view, run read-only over the example store, gave these results. Each depends only on the log up to the position shown, and two separate builds gave the same results.

| Position | What was added | Status | Escalations | Disagreement | Next |
|---|---|---|---|---|---|
| 12 | Commissioning record observed by capture, with capture's `author` line | awaiting approval | none | none | approve: installer, 1, from the installer the record names |
| 13 | Installer's `approve` | approved | none | none | nothing |
| 15 | Gateway statement: export limit 10 kW, against 5 kW in the record | awaiting approval (a review step was added) | statement-differs | none | approve: reviewer, 1, from reviewer 1 or 2 |
| 19 | Reviewer 2 disagrees; reviewer 1 records unable to assess | contested | open-position, statement-differs | open | approve: reviewer, 1, from reviewer 2, not reviewer 1; a resolution of reviewer 2's position |
| 32 | Reviewer 1's resolution, approved by reviewer 1 | awaiting approval (the review step is still unmet) | statement-differs | settled | approve: reviewer, 1, from reviewer 2, not reviewer 1 |
| 33 | Reviewer 1's `approve` on the record | approved | statement-differs | settled | nothing |
| 35 | Gateway statement corrected to 5 kW | approved; reviewer 1's approval reported as not needed | none | settled | nothing |

At position 32, reviewer 1 is still listed under `not`: their latest act on the record is unable to assess, and the resolution is a separate object. The hint routes and never gates. Reviewer 1's approval at 33 counts, and it replaces their earlier unable-to-assess.

## 7. Doubt, disagreement and uncertainty [proposed]

Each log has one writer, which orders it. Nothing is settled by counting positions. Uncertainty is spread by recording each party's own view separately and passing the question to a named person. Any party can record doubt. Doubt adds a review step only when it comes from a listed party or from a recorded fact the rules name, and only accountable parties settle it.

| Who | Records | What it changes |
|---|---|---|
| A person listed in a role the rule names in `checks_from`, or a tool under a valid grant from one | A check whose result the rule lists as needing review | Adds a review step ([7.2](#72-doubt-from-a-tool)). On an object already approved, a tool's check goes first to its `on_behalf_of`, and adds the step only once they accept it |
| A party listed in a role the rule names in `positions_from` | A `disagree` position | The object is contested until a person resolves it, and the review step is added |
| A listed party | An `unable-to-assess` position | No count changes; the question is routed to others |
| Anyone else | A position or a check | Shown as noted (`P_POSITION_NOTED`, `P_CHECK_NOTED`); changes nothing |
| A listed reviewer | An `approve` | Fills a review step |
| A person, approved by a listed reviewer | A resolution | Settles the positions it names |

### 7.1 Recorded facts that add a review step

Each condition is a fixed, named test over the log at the watermark. When it holds, the rule adds the step it names; when it stops holding, the step goes away and approvals already given are reported as not needed.

| Condition | Holds when | Available |
|---|---|---|
| `record-fork` | The subject's record lane has more than one head | M4 |
| `statement-differs` | A current statement of a declared kind, linked by an equal keyed digest, has a different value for a declared field. The statement carries its `author` line signed as `actor.id`, with no `P_ACTOR_ID_FORM` or `P_AUTHOR_NOT_ACTOR` finding | M4 |
| `check-flagged` | A current check (a head of its lane) of a kind the rule names (`checks`) has a result the rule lists as needing review (`flag_results`). The check names the object it checked and carries its `author` line. Its actor, or for a tool its `on_behalf_of`, is listed in a role the rule names in `checks_from` (otherwise `P_CHECK_NOTED`). It has no `P_ACTOR_ID_FORM`, `P_AUTHOR_NOT_ACTOR`, `P_SPONSOR_NOT_ACCOUNTABLE` or `P_GRANT_*` finding, so a tool must act under a valid grant. A tool's check on an object that was approved when the check was recorded counts only once its `on_behalf_of` has accepted it ([7.2](#72-doubt-from-a-tool)) | M4 |
| `open-position` | A listed party's `disagree` position, carrying its `author` line signed as `actor.id` and with no `P_ACTOR_ID_FORM` or `P_AUTHOR_NOT_ACTOR` finding, is not yet settled | M4 |
| `reconcile-difference` | The latest reconciliation report covering the subject lists it with a class the rule names in `classes` | M3 reports, M4 view |
| `basis-stale` | `P_BASIS_STALE` from the read-basis profile ([turn-reader](turn-reader.md#the-basis-profile)) | M4 |

The example rule names the classes `untracked_edit`, `differs`, `missing`, `sor_regressed` and `resurrected`. It leaves out `match`, the informational `coalesced`, and the classes that reflect a capture pause or a system that cannot be reached rather than a difference between records: `lagging`, `unobserved`, `unchecked` and `unreachable`.

Several conditions that add the same step add it once. An approval fills an added step only if it was recorded after the latest fact that added it. The step's `after` is the log position at which that fact first made its condition hold: for a statement, a position or a check, its `author` line. So an approval that counts was given after the doubt was recorded. An earlier approval stays in the log and is reported as `P_APPROVAL_PREDATES_STEP` ([open question 5](#15-open-questions)). A tool's check that its `on_behalf_of` has not accepted never sets `after`, so unaccepted tool output never makes a person's earlier approval stop counting.

### 7.2 Doubt from a tool

This is how an automated tool, including an AI agent, passes uncertainty to a person:

1. The tool records a check under a grant from a listed party: an `assert` naming the object it checked and a result code.
2. If the rule lists that result as needing review and the object was not approved when the check was recorded, `check-flagged` holds and the review step is added.
3. If the object was already approved, the check changes nothing by itself. The view lists it under `next` as `review-prompt`, routed to the tool's `on_behalf_of`. The status changes only once that person accepts the check with `approve`, which adds the review step, or a listed party records a `disagree` position ([7.3](#73-positions)).
4. The view's `next` names the listed reviewers who can fill the step.
5. A listed reviewer looks at the object and accepts it with `approve`, or records a position.

A passing check adds nothing. No check ever counts as an approval, whatever its result. On an object not yet approved, the flag adds the review step at once, so doubt reaches a person without anyone first accepting the tool's output. On an object already approved, a tool's check never changes the status by itself. A check by a listed person is that person's own act and needs no further acceptance. A current check with a finding, or by a party not listed in `checks_from`, is listed in the view with its codes, and adds no step.

In the examples, in a copy of the store, a flagged check by the tool on the approved record left it approved and was listed as a review prompt for reviewer 1. Once reviewer 1 accepted the check, the review step was added, and reviewer 2's later approval met it. In another copy, holding the record before the installer approved it, the same kind of check added the review step at once ([Appendix](#appendix-how-the-examples-were-checked)).

### 7.3 Positions

A position is a party's own recorded view of an object. Attestation purposes are frozen, so it is an `assert`, schema `approval/position@0`, on the object's subject. A position counts only if it carries its `author` line signed as `actor.id`, with no `P_ACTOR_ID_FORM` or `P_AUTHOR_NOT_ACTOR` finding; otherwise it is listed with its codes and changes nothing.

- `disagree`: makes the object contested, if the author is listed in a role named in `positions_from`. Anyone else's position is shown as noted (`P_POSITION_NOTED`) and changes nothing. Only listed parties can make an object contested, and a check adds a step only when a listed party, or a tool acting for one, made it ([7.1](#71-recorded-facts-that-add-a-review-step)). This avoids letting anyone at all hold up a record.
- `unable-to-assess`: routes the question on. It changes no count, but `next` lists this party under `not` and the other listed reviewers under `from`. A party's later `approve` or position replaces their earlier unable-to-assess, and the view marks it `replaced`.
- `withdraw`: names one of the author's own `approve` lines, which then stops counting. The line stays in the log.

A position never deletes or alters an approval. Reasons are codes from the rule set's published list.

```json
{
  "v": "evidence/0",
  "id": "sha256:ffe43ac8ef4fde36dff97c96206cae9f3e27040200921b0ed7215a984aeb6008",
  "op": "assert",
  "schema": { "ref": "approval/position@0", "hash": "sha256:fd30d4aa010456140891e31d42456e02cf877761f53bc03fa89406a2bb36d76b" },
  "subject": { "system": "installer-app-demo", "type": "commissioning", "key": "COM-TEST-000001" },
  "content": {
    "media_type": "application/json",
    "hash": "sha256:2646bee27173d9417f316498e3637ea0394553841be022ad25305b3dc4fa9c39",
    "inline": {
      "object": "sha256:1b58519d6e2ea7b9dc42f2429f1f2d29a703b7c1bd3c4fcba85c5a6893f38455",
      "position": "disagree",
      "reason": "device-setting-differs",
      "cites": ["sha256:a37fb277a40307c5bfe642c7a145c5c8ff0f9ae3a9cb2968a6565f5f93cf06b3"]
    }
  },
  "actor": { "id": "person:722113fd222cf1277ef6c48ba08790cc", "kind": "human" },
  "at": "2026-11-26T10:30:00Z",
  "prev": []
}
```

Reviewer 2 signs `author`; `evidence verify --require author` gives `ok` with `{"author":1}`.

### 7.4 Resolution by a person

A person settles a disagreement with an `assert`, schema `approval/resolution@0`, by a human actor, carrying its `author` line signed as `actor.id` with no `P_ACTOR_ID_FORM` or `P_AUTHOR_NOT_ACTOR` finding, and approved by a listed reviewer: the pattern of M3 resolutions ([roadmap](roadmap.md#m3-two-way-reconciliation-report-only-recorded-as-evidence)). Its content names the object, the positions it settles and a disposition: `record-stands`, `correct-at-source`, `correct-on-device` or `escalated`. Its `read-basis/0` names the checkpoint and the heads it relied on.

- A resolution must name every open position from a listed party at its basis (`P_RESOLUTION_INCOMPLETE` otherwise), so a disagreement is never settled by being left out.
- A position recorded after the basis stays open and is reported (`P_POSITION_AFTER_RESOLUTION`).
- A resolution never changes record-lane heads. Corrections happen in the system of record or on the device, and reach the log only through capture or a new statement.
- Two unchained resolutions that each meet these conditions are two heads of the resolution lane, so the object shows as contested again. Conflicting resolutions are always shown, never settled by the system.
- A resolution by an agent or a service settles nothing, and the profile check flags it (`P_RESOLVER_NOT_HUMAN`, as in M3). The store does not refuse it.

### 7.5 Re-checking later

Anyone holding the log, or an export, can recompute the status of any object at any earlier position and compare it with what was recorded. A resolution is re-checked at the checkpoint it cites: the cited heads must be the heads at that checkpoint, and the open positions it had to name must be the ones it named. A remote reader holding only a bundle can check membership, not completeness ([turn-reader open decision 10](turn-reader.md#10-proving-completeness-to-a-remote-reader)).

### 7.6 What is not proposed

- No outcome computed by counting positions or objections.
- No deadline that decides anything. A report can list objects that have awaited approval for longer than a set time; a person acts on it.
- No scores, weights or rankings of people, records or households.
- No assignment of work. `next` says who can act; it assigns nothing.
- No status that changes anything outside this layer.

## 8. Hooks for tools [proposed]

The core and the hooks do not know what a tool is. They see an `actor.kind`, an `on_behalf_of` and a grant, nothing more. The same hooks serve an AI agent, a case-management screen and a batch job; the actor's kind changes only which rules apply.

| Hook | When | Contract |
|---|---|---|
| grant | Before any tool acts for someone else | An `approval/grant@0` assert by the accountable person, approved by them. It sets what the tool may write, which systems it may read, which purposes it may sign, a notes subject and recall window, and a validity period. Changing or revoking it is a new version. |
| read | Before a tool relies on evidence | The [turn-by-turn reader](turn-reader.md): verify, establish heads at a checkpoint, keep a value-free read receipt locally. Every item is labelled `observed` (record lane, written by capture), `stated` (anything else written by a person or service, including a record-lane object not written by capture), `tool-stated` (an assert by an agent) or `note` (a continuity note). Nothing is returned as observed unless capture wrote it. Writes nothing. |
| record | When a tool states something: a check, a recommendation, a note | One batch: the assert and its `author` line, signed as `actor.id`. A delegated assert cites its grant in `ext["approval/0"].grant`. A check names the object it checked and gives a result code. If recording fails, the tool does not act. |
| approve | When the accountable party accepts an object | An `approve` line that counts only under the published rules ([section 6](#6-approval-status-proposed)). |
| position | When a listed party disagrees, cannot assess, or withdraws an approval | [7.3](#73-positions) |
| resolve | When a person settles a disagreement | [7.4](#74-resolution-by-a-person) |
| notes | When a tool needs continuity between sessions | [Section 9](#9-continuity-for-tools-proposed) |
| witness | When any actor records that it saw an object at a time | A `witness` line. Never counts toward a rule. |
| status | Whenever anyone asks whether an object is approved, or what is still needed | The view of [section 6](#6-approval-status-proposed), with `next`, at a stated watermark. |

Every check behind these hooks is a pure function of the log: no side effects, a failure to record is a refusal, and every failed check is named, not just the first.

**An AI agent is one kind of actor, not only a reader or a contributor.** Under this profile:

| An AI agent may | Within |
|---|---|
| Read and re-read evidence, labelled | Its grant's read scope |
| Record checks and recommendations | Its grant's write scope, with its own `author` line |
| Raise doubt: a check whose result the rule lists as needing review adds a review step | A valid grant from a listed party ([7.2](#72-doubt-from-a-tool)). On an object not yet approved, at once; on an approved object, only once its accountable person accepts the check |
| Witness an object | Any actor may |
| Keep continuity notes | Its grant's notes subject and recall window ([section 9](#9-continuity-for-tools-proposed)) |
| File a request to change a record (M6) | **[open]** ([open question 8](#15-open-questions)). A request changes nothing by itself: M6 records requests in shadow mode, with no write code. Any later delivery needs approval under the published rules and the agreement of the system's owner ([M7](roadmap.md#m7-write-through-to-the-synthetic-database-only-writes-opt-in)) |

| An AI agent may not | Because |
|---|---|
| Fill an approval step | An `approve` signed as an `agent:` or `service:` id never counts (`P_APPROVER_NOT_ACCOUNTABLE`) |
| Settle a disagreement | A resolution settles positions only when a person records it and a listed reviewer approves it (`P_RESOLVER_NOT_HUMAN` otherwise) |
| Change an approved object's status by itself | Its check on an approved object is only routed to the person it acts for until that person accepts it ([7.2](#72-doubt-from-a-tool)) |

These limits are rules of this profile, not of the core. The core counts an agent's `approve` like any other line ([section 1](#1-what-the-core-shows-today-built)). The same limits apply to every service, and [open question 4](#15-open-questions) asks for them to be confirmed.

**Grant checks**, judged at the delegated object's own log position and on the store's clock (`received_at`), never on the object's own `at`:

| Code | Meaning |
|---|---|
| `P_GRANT_MISSING` | The object has `on_behalf_of` but cites no grant, or the grant was not held before it |
| `P_GRANT_SUPERSEDED` | The cited grant was not the current version at the object's position |
| `P_GRANT_UNAPPROVED` | The grantor had not approved the grant |
| `P_GRANT_PARTIES` | The grantee is not `actor.id`, or the grantor is not `on_behalf_of` |
| `P_GRANT_EXPIRED` | The object was received outside the grant's validity period |
| `P_GRANT_SCOPE` | The schema or subject is outside the grant, or the object is in a record lane |

The grant in the examples (id `sha256:8ff1dd42…`, `{"approve":1,"author":1}`), content only:

```json
{
  "grantee": "agent:689db15aeff6b2425b9b1e2e249f7934",
  "for": "person:9cd0281eb4bd58de76acdefcb2125ab2",
  "write": [
    { "schema": "edge-demo/form-check", "subject": { "system": "installer-app-demo", "type": "commissioning" } },
    { "schema": "tool/continuity-note", "subject": { "system": "tool-notes-demo", "type": "notes", "key": "GRANT-TEST-0001" } }
  ],
  "sign": ["author"],
  "read": [ { "system": "installer-app-demo" }, { "system": "edge-gateway-demo" }, { "system": "tool-notes-demo" } ],
  "notes": { "subject": { "system": "tool-notes-demo", "type": "notes", "key": "GRANT-TEST-0001" }, "recall_until": "2026-12-26T00:00:00Z" },
  "valid_from": "2026-11-26T11:00:00Z",
  "valid_until": "2026-12-26T00:00:00Z"
}
```

The tool's form check cites this grant and a `read-basis/0` naming the checkpoint it read. Its content names the record it checked, the hash of the check's rule set, a hash naming the tool and its configuration, and the result. That hash names a description of the tool, its version and its configuration, which the deployment keeps and makes available to the reviewers and auditors who see the check. Reviewer 1's `approve` on it counts; the tool's own `approve` on its own output does not (`P_APPROVER_NOT_ACCOUNTABLE`), although the core counts it.

## 9. Continuity for tools [proposed]

Some tools need to pick up where they left off, between sessions or between versions of the tool. Here a tool's continuity has two parts, and both come from the log:

1. **What it can re-read.** Under its grant, the tool re-reads the evidence it may see, through the verified reader. Each item comes back labelled `observed`, `stated`, `tool-stated` or `note`, as in the read hook.
2. **Its own continuity notes.** Small governed asserts the tool records under its grant: what is open, what it has done, what it expects next.

Recall is a verified read. A tool cannot recall something the log does not hold, and what it recalls carries its provenance: who recorded it, when, under which grant, and with which label. This is what is sometimes called persistent memory for tools. It lives in the same verified log that people read, not in a tool's private cache, and it works the same for a person's saved working notes.

- **Under a grant.** A note is an `assert`, schema `tool/continuity-note@1`, in the notes subject the grant names. The subject's system is a dedicated one, so M4's closed-by-default read scopes ([roadmap](roadmap.md#m4-memory-reads-index-query-api-bundles-read-only-service)) keep notes private unless a scope allows them.
- **By reference.** `content.hash` is in the log; the body, with a random salt, is in the erasable blob store (M2). The body holds ids, codes and hashes only, never personal data or free text copied from records. The salt stops anyone confirming a guessed body against the hash. In the example, `evidence verify` reports `content_checked: false`, and `evidence content-hash` of the stored body equals the note's `content.hash`.
- **Recall.** The reader returns current, unforgotten notes to the grantee only while the grant's recall window is open on the store's clock, and to the grantor at any time; it declines otherwise. Notes are labelled `note`, never `observed`, and never count as approvals. An approved note is still a note.
- **No loop back.** A read basis that relies on notes or tool statements says so, and a check can flag reliance on ones nobody approved (`P_BASIS_RELIES_ON_CLAIM`).
- **Forgetting.** A new version with `"state": "forgotten"` stops recall. At M5, erasure deletes the body and appends a redaction record; verification then reports the body as erased. The log keeps ids, hashes, times and pseudonyms. Nothing here claims that the log forgot.
- **Scope.** This is consistent with [turn-reader open decision 13](turn-reader.md#13-continuity-for-tools) (b): M4 serves no recall endpoint, ranking or similarity search. Notes are built with the assert intake, after M5's erasure exists.

In the examples, the tool's re-read under its grant at position 35 returned one `observed` item (the commissioning record), four `stated` (the corrected gateway statement, two positions and the resolution), one `tool-stated` (its form check) and one `note`. After the note was forgotten, the same re-read returned no note. Reviewer 2, who is not a party to the grant, was declined. In a copy of the store holding a person's record-lane update, that update came back labelled `stated` with `P_RECORD_LANE_WRITER`, not `observed`.

## 10. Edge records [proposed]

The edge profile ([cer-edge.md](cer-edge.md)) applies this note to records about consumer energy resources, such as inverters and gateways. By record type, an edge object is either a record change observed by capture or a statement by a gateway:

| Record | In the log | Lane | Approved by |
|---|---|---|---|
| A commissioning record in an installer app | `create`, `update` or `delete` by the capture service | `record`, subject in the installer app | The installer named in the record, by `approve` |
| A connection record in a network portal | The same pattern, under its own subject | `record` | As the rules say |
| A gateway's settings or measurement summary | `assert` by the gateway, `kind: "service"`, opaque id | `assert:edge-demo/gateway-settings` | Not approved; compared |
| An automated check of a record | `assert` with `on_behalf_of`, citing a grant | `assert:edge-demo/form-check` | Its `on_behalf_of` |

The same device held in different systems is linked by an equal keyed digest of its serial number, under a key held by the data owner, one key per link domain. A difference between a gateway statement and a record is a prompt for review, not a finding.

Only the capture actors named in the deployment may write a record lane. Any other writer is flagged (`P_RECORD_LANE_WRITER`) and the status is `unknown`. In the checks, store/0's own intake accepted a person's `update` in a record lane, which then replaced the captured record as the lane's head. The proposed assert intake (M4b or early M6) refuses such an object, and verification flags it; making store/0 itself refuse it would need a store/0 version change, which is not proposed now.

## 11. Advisory reports [proposed]

- **Awaiting approval:** objects that have awaited approval or stayed contested for longer than a set time, each with its `next`.
- **Approval spread:** for a period on the store's clock and for each rule, how many distinct listed approvers filled its steps. It is an aggregate that names no one: it shows whether approval rests with too few people without describing any one person's work. Counts below a set threshold are shown only as "fewer than N". Advisory only; it gates nothing, and it means little before keys are bound at M5.

## 12. Where this lands

| Milestone | Work |
|---|---|
| Now (documents only) | This note; the edge profile ([cer-edge.md](cer-edge.md)); turn-reader decisions 8, 12 and 13 |
| Edge track, alongside M2 | Shapes and vectors for the edge record types and for `approval/0`: approver list, rules, position, resolution, grant |
| M3 | Reconciliation reports feed the `reconcile-difference` condition |
| M4 | The approval-status view with `next`, read labels, the `read-basis/0` checks, and tokens that carry actor kind. Rust implementation, an independent JavaScript reference and vectors ([D7](decisions.md#d7-the-vectors-are-the-contract)) |
| M4b or early M6 | The assert intake: one batch per request, with the pure pre-append checks (id form, lanes, grants) |
| M5 | Key binding for people, organisations, services, gateways and agents (an agent's binding names its sponsor); revocation judged on the store's clock; approver lists bound to keys; the directory; erasure |
| M6 | M6's approval rules written as `approval/rules@0`; continuity notes; whether tools may file requests under a grant **[open]** |

## 13. Proposed codes

All are profile findings (`P_`), reported by verification and the views. The assert intake refuses on the ones marked *intake*.

| Code | Meaning |
|---|---|
| `P_ACTOR_ID_FORM` *(intake)* | An actor id or signer breaks the id form or does not match `actor.kind` |
| `P_AUTHOR_NOT_ACTOR` *(intake)* | An `author` line is signed under a name other than `actor.id` |
| `P_SPONSOR_NOT_ACCOUNTABLE` *(intake)* | `on_behalf_of` is not a person or organisation |
| `P_RECORD_LANE_WRITER` *(intake)* | A record-lane object by an actor other than capture |
| `P_GRANT_*` *(intake)* | See [section 8](#8-hooks-for-tools-proposed) |
| `P_APPROVER_NOT_ACCOUNTABLE` | An `approve` by an agent or service; not counted |
| `P_SELF_APPROVAL` | An `approve` by the object's own actor or author; not counted, unless the step or the policy-change rule ([5.3](#53-changing-a-list-or-a-rule-set)) says so |
| `P_KEY_SHARED` | A key used under more than one signer name in the log up to the watermark; its lines are not counted |
| `P_APPROVER_NOT_LISTED` | The signer is not listed for any needed role at the line's position |
| `P_APPROVAL_PREDATES_STEP` | An approval recorded before the fact that added the step it would fill; not counted toward that step |
| `P_DUPLICATE_SIGNER` | Informational: a signer already counted |
| `P_APPROVAL_NOT_NEEDED` | Informational: a listed approval that no current step needs |
| `P_POLICY_AMBIGUOUS` | An approver list or rule set has more than one current version. Informational when the competing version landed after the object |
| `P_POLICY_UNAPPROVED` | A list or rule set lacks its administrators' approvals |
| `P_RULE_MISSING` | No rule applies to the object |
| `P_RULES_CHANGED` | Informational: a later rule set would give a different answer |
| `P_POSITION_NOTED` | Informational: a position by a party not listed for it |
| `P_CHECK_NOTED` | Informational: a check whose actor, or for a tool its `on_behalf_of`, is not listed in a role named in `checks_from`; it adds no step |
| `P_RESOLUTION_INCOMPLETE` | A resolution omits an open position from a listed party at its basis |
| `P_POSITION_AFTER_RESOLUTION` | Informational: a position recorded after a resolution's basis |
| `P_RESOLVER_NOT_HUMAN` | A resolution by an agent or service; it settles nothing (as in M3) |
| `P_BASIS_RELIES_ON_CLAIM` | Informational: a basis relies on unapproved tool statements or notes |

## 14. Limits and risks

- **Before M5, names are free text.** Anyone with any key can sign under a listed name. `P_KEY_SHARED` catches one key used under two names, not a new key used under someone else's. Every status says `accountability: not-established-before-key-binding`, and public text must say the same.
- **The trust anchor before M5 is configuration.** Whoever controls the deployment configuration that names the first administrators controls the policy until keys are bound and checkpoints are published outside the store.
- **The single writer can delay or leave out a position or a check.** The view is only as complete as the log. Authors should keep their append receipts, and M5's replicas and published checkpoints make omission detectable.
- **store/0 does not count heads in assert lanes.** A competing list or rule version is visible only to the profile check, which fails closed.
- **Grant windows and list periods use the store's clock,** which is the store's own claim until M5 adds independent time.
- **A status can be misread as a finding.** Public text must say that statuses are review statuses with no effect outside this layer, and that a difference between records is a prompt for review, not a finding.
- **`next` can be misread as an assignment.** It is derived, it routes, and it gates nothing. Who actually reviews is decided by people, outside this layer.
- **Escalation creates work.** Broad rules, or a tool that flags often, could flood reviewers. The rules name which check kinds and results count, and the reports make the load visible, but nothing here prevents it.
- **Pseudonyms still link one person's approvals,** which audit needs and which is personal information about staff. Anyone holding a log or an export can list every approval under one pseudonym. So logs and exports that hold approval lines are shared only with parties entitled to see that history, under M4's closed-by-default read scopes and subject to M5's legal review. A status view offered more widely leaves out signers, position authors and the `from` and `not` lists. Reports aggregate rather than describe individuals.
- **Keyed links depend on their key.** If a link-domain key leaks, serial numbers and installer references can be guessed.
- **Scope.** Two profiles (`approval/0`, an edge profile), the read-basis profile and the views each need vectors and a JavaScript reference. Build the shapes first, the views at M4, and notes last.
- **The prototype is a design aid.** It is not a specification and not conformance code.

## 15. Open questions

1. Who publishes which approver list, and who are the first administrators? Is a deployment-configured anchor acceptable until M5?
2. The default rule for commissioning records: the installer's approval only, with a reviewer added when a recorded condition holds (as in the examples), or always a reviewer? Which conditions apply, which check kinds, results and roles (`checks_from`) count, and which reconciliation classes?
3. Judge objects under the rule set in force when they were recorded (proposed), or under the current one?
4. Confirm that agents and services never fill an approval step or settle a disagreement, and that a tool's output counts as approved only through its `on_behalf_of`. This would be a rule of the profile, not of the core.
5. Should an approval count toward an added review step only when recorded after the fact that added it (proposed), or whenever it was recorded? Should a tool's check on an object already approved change nothing until the person it acts for accepts it (proposed), or add the review step at once?
6. Pseudonyms: random at enrolment (proposed) or a keyed digest held by the directory? One per party per directory? A one-sentence rule in store/0 Privacy, or the profile only?
7. Which rules should the assert intake refuse, and which should only be flagged?
8. Continuity notes: what default recall window and retention? May a tool file M6 requests under a grant?

## Appendix: how the examples were checked

- **Tool.** The `evidence` command-line tool built from commit a6bbb94, copied to a scratch directory. Seeds are development keys from `evidence keygen`, made for these runs only. Keyed digests use a published test key.
- **Objects.** Twelve objects were sealed, signed and appended to a fresh store: the approver list, the rule set, the commissioning record, two gateway statements (the second appended with `--if-heads` on the first), two positions, the grant, the form check, two continuity notes (the second a forgotten version), and the resolution, with the record's approvals. Each gives `ok` with `evidence verify --store` and its required purposes, and with `evidence store get --require`. With `--require author --require approve`, the approver list and the rule set give `{"approve":2,"author":1}`, the grant, the form check and the resolution give `{"approve":1,"author":1}`, and the commissioning record gives `{"approve":2,"author":1}` (the installer and reviewer 1). The positions, gateway statements and notes give `{"author":1}` with `--require author`. `evidence store verify` gives `ok`, no findings, forks 0.

**Doubt from a tool on an approved record,** appended only to a copy of the store. A second form check by the tool, under the same grant, with result `fields-missing`, was appended with `--if-heads` on the first check (position 40), then its `author` line (41). Reviewer 1, for whom the tool acts, then accepted the check with `approve` (42), and reviewer 2 approved the record (43). The check gives `ok` with `{"author":1}` and `--require author`, and `{"approve":1,"author":1}` with reviewer 1's line and `--require author --require approve`. `store verify` on the copy gives `ok`, no findings, forks 0.

| Position | What was added | Status | Escalations | Next |
|---|---|---|---|---|
| 41 | The flagged check and its `author` line | approved; the record was approved when the check was recorded, so the check adds no step | none | review-prompt: the check, from reviewer 1 |
| 42 | Reviewer 1's `approve` on the check | awaiting approval; reviewer 1's approval at 33 reported as `P_APPROVAL_PREDATES_STEP` | check-flagged | approve: reviewer, 1, from reviewer 1 or 2 |
| 43 | Reviewer 2's `approve` on the record | approved | check-flagged | nothing |

Between 41 and 42 the record's status did not change: the tool's output reached reviewer 1 as a prompt, and it was reviewer 1's acceptance that added the review step.

**Doubt from a tool on a record not yet approved,** in another copy, built from an export taken when the store held only the policy and the captured record (position 12). The grant and its lines (13-15), a checkpoint (16-17), and a flagged check by the tool citing that checkpoint (18) with its `author` line (19) were appended, then the installer's approval (20) and reviewer 2's (21). Nobody accepted this check. It gives `ok` with `{"author":1}` and `--require author`, and `store verify` on the copy gives `ok`, no findings, forks 0.

| Position | What was added | Status | Escalations | Next |
|---|---|---|---|---|
| 19 | The flagged check and its `author` line | awaiting approval; the review step was added at once, with no `after` | check-flagged | approve: installer, 1, from the installer the record names; approve: reviewer, 1, from reviewer 1 or 2 |
| 20 | The installer's `approve` | awaiting approval (the review step is still unmet) | check-flagged | approve: reviewer, 1, from reviewer 1 or 2 |
| 21 | Reviewer 2's `approve` on the record | approved | check-flagged | nothing |

That check itself still awaits its `on_behalf_of` approval; the doubt reached a person without it, because the record was not yet approved.

- **Negative cases,** each accepted by the core and caught by the profile prototype: the tool approving its own output (`P_APPROVER_NOT_ACCOUNTABLE`), one key signing as two people (`P_KEY_SHARED`), the same signer approving twice (in a file the core counts the same line as `approve: 2`; a store keeps one copy of an identical line but stores a re-signed one, which the view reports as `P_DUPLICATE_SIGNER`), an email-style actor id (`P_ACTOR_ID_FORM`), an agent as `on_behalf_of` (`P_SPONSOR_NOT_ACCOUNTABLE`, `P_GRANT_MISSING`, and `P_CHECK_NOTED` because no listed party stands behind it; this check's result is one the rule flags, yet it added no review step and did not make the first check contested), an `author` line under another name (`P_AUTHOR_NOT_ACTOR`; the position carrying it then counted for nothing), a competing rule set (`P_POLICY_AMBIGUOUS`: informational for objects recorded before it, and `unknown` for one recorded after it), and a person's record-lane update (`P_RECORD_LANE_WRITER`). `--require approve=2` is a usage error. The negative cases were appended only to copies of the store.
- **Ids.** Objects that do not cite a checkpoint have the same ids on every build. The form checks and the resolution cite a checkpoint, whose id depends on the store key and the time, so their ids, and those of the notes that refer to the form check, differ between builds.
- **Prototype.** A short script computes the status view, recall and the labelled re-read from `evidence store log`, read-only. To tell whether an object was approved when a check was recorded, it runs the same computation at the position before the check's `author` line. Two separate builds from the same seeds gave the same statuses, steps, `next` lists and codes at every position in the tables above, once the ids that depend on a checkpoint were matched by name. The examples do not exercise `record-fork`, `reconcile-difference`, `basis-stale`, `withdraw`, a check by a person, a competing approver list, two unchained resolutions, `P_SELF_APPROVAL`, `P_APPROVER_NOT_LISTED`, `P_POLICY_UNAPPROVED`, `P_RULES_CHANGED`, `P_RESOLUTION_INCOMPLETE`, `P_POSITION_AFTER_RESOLUTION`, `P_RESOLVER_NOT_HUMAN`, `P_BASIS_RELIES_ON_CLAIM`, or any grant code other than `P_GRANT_MISSING`.
