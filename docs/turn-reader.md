# The turn-by-turn reader (design note)

**Status:** direction from the owner, 27 November 2026, revised after review. Design input for [M4](roadmap.md#m4-memory-reads-index-query-api-bundles-read-only-service) (memory reads), [M6](roadmap.md#m6-requests-and-the-outbox-as-evidence-in-shadow-mode-no-write-code) (requests) and [M9](roadmap.md#m9-optional-centr0-profile-register-views-and-prime-coordinates) (prime coordinates). Nothing here is built. The examples are synthetic, and each evidence object in them verifies with the `evidence` command-line tool.

## In short

A tool calls the reader before it relies on evidence about a subject. The tool might be an AI agent working turn by turn, a case-management screen or a batch job.

- **Reads are passive.** The store records writes only. Getting an object, its heads or a proof adds no entry to the log ([store/0](../spec/store-0.md#log-entry)).
- **A routine read leaves a read receipt** in the tool's own log, not in the store.
- **A read that is the basis for an action is recorded in the store,** inside an `assert` about that action, appended before the action is taken.
- **S and H are recomputed on every read,** for each head, as of a named checkpoint. S and H are the state and history coordinates of [Kernel/2](kernel-2.md#32-two-coordinates-declared). The reader never takes them from a stored value.

The owner's direction stands. Review corrected the examples and filled in the store mechanics: where the basis goes, who approves, what "complete" means, and what happens when the evidence moves on between the read and the action.

## What review changed

What the proposal got right, and this note keeps:

- Reads do not become appends. store/0 gives `seq` and `received_at` to entries, and an entry is only ever an object or an attestation.
- A routine verification produces no evidence object; the log would fill with noise. This is a policy default, not a rule of the spec: M3 does record every reconciliation run, because there the result of the check is the point.
- A read that informs a decision is recorded as an `assert` holding what the decision relied on and what it concluded.
- S and H are recomputed from the chain, never trusted from an index ([README](../README.md#proposed-prime-coordinates-for-faceted-queries): "derived, never authoritative").
- Whether a given read needs an assert is policy, not a mechanical consequence of the architecture.
- An append-only log that cannot explain why a decision was made is only half an audit trail.

What needed correcting:

| The proposal | Corrected to | Why |
|---|---|---|
| The decision example | A sealed evidence/0 object ([below](#example-an-agents-recommendation-and-a-persons-approval)) | As written it fails verification with `E_SHAPE` (`/: missing member "v"`). It has no `id`, `schema`, top-level `actor`, `prev`, `content.media_type` or `content.hash`. Its `subject` is a string, not the `{system, type, key}` triple, and its `at` has a `+11:00` offset where [evidence/0](../spec/evidence-0.md#the-object) requires UTC with `Z`. |
| The S and H table (14,078,483; 1,140,417,619,169; 37,258,914,771,961) | [Kernel/2 section 5](kernel-2.md#5-example-an-accu-lifecycle) values ([below](#lane-state)) | These are not Kernel/2 coordinates. Each has a prime factor above 349, the largest dictionary prime (14,078,483 = 97 × 145,139). H after E2 is not a multiple of H after E1, which a history coordinate must be. Their source is not recorded. Do not reuse them. |
| One lane head and one `state_coordinate` per turn | S and H for each head | A lane can have several heads ([store/0 Forks](../spec/store-0.md#lanes-and-heads)), and Kernel/2 defines S and H per event. |
| `"result": "approved"` and the approver's id inside `content` | The agent's recommendation in `content`; the person's approval as an `approve` attestation | Content is the writer's claim. Approval is a signature by the accountable person ([evidence/0 Attestations](../spec/evidence-0.md#attestations), [D4](decisions.md#d4-signatures-outside-the-object)). The [profile notes](centr-profile-notes.md#pitfalls-found-in-the-earlier-draft) already say: "Approval is a signed approve attestation; refusal is an assert object." |
| An email address on a real agency's domain as the approver | Pseudonymous ids such as `person:reviewer-01` | The log cannot forget, and only synthetic data goes into a store before M5 ([store/0 Privacy](../spec/store-0.md#privacy)). This repository is not affiliated with any agency. |
| A free-text `query`, "as of 2026-11-26" | A structured basis: log, checkpoint, watermark, clock, subject, lane, heads | The text names no clock and no watermark, so nobody can re-run it. The [roadmap](roadmap.md#principles) requires every query to say which clock it uses. |
| The receipt "proves the tool did its job" | The receipt is the tool's own record | It is unsigned, on the tool's clock and never appended, so it proves nothing to anyone else. Whatever must be checkable goes in the assert. |
| Decision at `14:35:00+11:00`, receipt at `14:32:01Z` | Every time in UTC with `Z`, labelled with its clock | `14:35:00+11:00` is `03:35:00Z`, so the decision was dated 10 h 57 min *before* the read receipt of the read it relied on. |
| The README's line on AI-assisted checks read as a principle | A use case ([README](../README.md#what-it-is-for)), with [vector 003](../test-vectors/objects/003-agent-assertion-approved/) as the pattern | The roadmap's principles have no rule that reads become evidence. Their rule is that what an application states itself (a request, a decision, a check) is an `assert`. |
| "A published list is an evidence object recording a query result", quoted as roadmap M4 | README's "public registers as views". Register snapshots at a watermark are M9; M4 can seal a bundle as an assert | The sentence is in the README, not the roadmap. |
| The tool may "checkpoint" its lane state | The tool may *save* its lane state | In store/0 a checkpoint is a signed tree head, and only the store writes one. "Receipt" is also taken: it is what an append returns. This note says *read receipt*. |
| Step 4 is "a policy decision by the tool owner"; inputs to decisions "must" be captured | One rule, stated once ([open decision 4](#4-when-a-basis-assert-is-required)) | Both can hold: the architecture does not force the assert, and policy should. |

## What the reader does

### The four steps

1. **Verify.** Run evidence/0 steps 1-5 and check the attestations, with the purposes the caller requires ([evidence/0 Verification](../spec/evidence-0.md#verification)). For an object read from a store, also check an inclusion proof against a checkpoint whose witness verifies under the genesis key ([store/0](../spec/store-0.md#checkpoint-and-tree)).
2. **Establish the lane and recompute S and H as of that checkpoint.**
   - Find `heads(subject, lane)` at the checkpoint's size N.
   - Take the same-lane `prev` closure from those heads. Every object in a lane is an ancestor of one of its heads, or a head itself, so these two steps give exactly the lane as of N.
   - Compute S and H for each head from the chain, as decimal strings tagged `kernel/2`.
   - If the reader cannot establish the heads at N (for example, it was handed some objects and their inclusion proofs), it reports *membership verified, completeness unknown*. It may give S and H for a named head, but never as the subject's state.
3. **Emit a read receipt** to the tool's own log ([below](#the-read-receipt)).
4. **If the read is the basis for an action, record it before acting.** The acting tool, not the reader, appends an `assert` that carries the basis ([below](#reads-that-become-evidence)). Which actions need one is policy ([open decision 4](#4-when-a-basis-assert-is-required)).

Step 2 replaces the first draft's completeness test, "the lane is closed under `prev` back to its first event". That test finds a missing middle event but never a missing tip. A lane cut short after E3 passes every check and gives S after E3, where the true state is S after E5.

The reader itself never writes: not to the store and not to any dictionary. A facet that is not in the Kernel/2 dictionary is a finding, never a new allocation.

### The rule

| Read | Recorded in the store? | Where | Why |
|---|---|---|---|
| Routine verification of an object or a lane | No. A read receipt only | The tool's own log | Reads add no entries; appending them would flood the log |
| A verification that fails | A finding in the read receipt. Whether it also becomes an `assert` is [open decision 5](#5-failed-verifications) | The tool's log, perhaps also the store | It may be a sign of tampering, and M3 records every reconciliation result |
| A read that is the basis for a decision by an agent or a service | Yes: an `assert` carrying the basis, appended before the action | The store, in the decided subject's `assert:<schema>` lane | "What the model was given, what it returned, and who approved or overrode it" ([README](../README.md#what-it-is-for)) |
| A person's approval of that decision | Yes: an `approve` attestation on the assert | The store, as an attestation entry | Approval is the accountable person's signature, not a content field |
| A person's own decision, or an override | Yes: an `assert` by a human actor, with an `approve` attestation | The store | The resolution pattern of [M3](roadmap.md#m3-two-way-reconciliation-report-only-recorded-as-evidence) |
| A register snapshot (a query result) | Yes: an `assert` whose content is the result at a recorded watermark (M9). An M4 bundle can also be sealed as an assert | The store | "What did the list show on date X?" gets a provable answer |
| Lane state (S and H) | Never as an authority. Recomputed on every read | Tool memory; a derived index column only from M4b or M9 | Coordinates are derived, never authoritative |

## The read receipt

### Shape

The owner's receipt is the starting point. The amendments:

| Owner's field | Amended | Why |
|---|---|---|
| `ok` | Kept | |
| `id` | `heads`, one entry per head | A forked lane has several heads. (The example's `sha256:7d7f…` also matches the start of the id of README's vector 003, an object about a different subject.) |
| `subject`, `"installation:aus-syd-1247"` | `subject` as the evidence/0 triple, plus `lane` | Heads are kept per subject key and lane. |
| `verified_at` | Kept, in UTC with `Z`, plus `verified_at_clock: "tool"` | Says whose clock it is. It is not the store's `received_at`. |
| `state_coordinate` | Per head: `profile`, `S` and `H` as decimal strings | Kernel/2 tags every coordinate with its profile and writes it as a string ([3.3](kernel-2.md#33-representation-declared)). H is needed to answer "was it ever in state X?". |
| (none) | `as_of`: `clock`, `log`, `checkpoint` (`id`, `size`, `root`), `watermark` (`seq`, `entry_hash`) | S and H are true only relative to a point in a particular log. |
| (none) | `complete` | Whether step 2 established the heads at the checkpoint, or only membership. |
| `findings` | Kept | |
| (none) | `v: "read-receipt/0"` | A working name, so the shape can change. |

Example (synthetic, read at the checkpoint after E4):

```json
{
  "v": "read-receipt/0",
  "ok": true,
  "verified_at": "2026-11-27T01:24:00Z",
  "verified_at_clock": "tool",
  "subject": {
    "system": "example-registry",
    "type": "accu",
    "key": "ACCU-TEST-000001"
  },
  "lane": "record",
  "as_of": {
    "clock": "recorded",
    "log": "sha256:44588380a04d80699633f4a10fb7e7d0075c7cc38463d481ba7c62e34453da3d",
    "checkpoint": {
      "id": "sha256:354cc1665affb3b07b5719fe5b16a4b9488f00eb7aa18b6d0041011aea369b55",
      "size": 6,
      "root": "sha256:9241e2acc9d129f91bf3fd8ede1477376c438d517d0bf9eca5bfa161d6c4d582"
    },
    "watermark": {
      "seq": 6,
      "entry_hash": "sha256:eed1e34a486736dc18001fbd004cb14743c30efbc4e8963471cdf16fe21b4e0d"
    }
  },
  "complete": true,
  "heads": [
    {
      "id": "sha256:97e5e21f8b826c102e6cf6e8a5ecf37b538b4e6d531992df2782dc045cabf908",
      "profile": "kernel/2",
      "S": "8321327376004027894",
      "H": "25035387438225574245358414"
    }
  ],
  "findings": []
}
```

### Rules

- **Tool-local.** The read receipt is never appended, and no verification depends on it.
- **Value-free.** No record content, no personal data, no free text copied from records. A tool's log is outside the store's erasure path (M5), so erasure cannot reach a value copied there.
- **The same blocks as the basis.** `as_of` and `heads` have exactly the shape the basis uses ([below](#the-basis-profile)). Recording a read as a basis is then a copy, not a translation.
- **Never the only record of a read that led to an action.** The assert holds the basis. It may name the read receipt by hash.
- **Save, don't trust.** A tool may save its lane state between turns, but only as a cache. On the next turn it recomputes, and it never presents a saved value as evidence.

A cheap check across turns: heads only move forward in store/0, so each head seen at turn n is an ancestor of a head at turn n+1, or still a head. H at the earlier head must therefore divide H at the later one. If it does not, the lane was rewritten or the reader was given a different log. A consistency proof between the two checkpoints is the stronger check.

## Lane state

### Per event, per head

Kernel/2 defines S and H for each event, over that event and its ancestors ([3.2](kernel-2.md#32-two-coordinates-declared)). store/0 defines heads per subject key and lane, and a lane can have more than one. So the reader reports S and H for each head. There is no single "state of the subject" unless the lane has exactly one head.

### Worked table

This is the ACCU lifecycle of [Kernel/2 section 5](kernel-2.md#5-example-an-accu-lifecycle), one event per turn. Each event lists the previous one in `prev`, and all of them are record changes in the subject's record lane. A script recomputed every value from the committed dictionary, and each equals section 5.

| After turn | The event adds | Record-lane heads | S (`kernel/2`) | H (`kernel/2`) |
|---|---|---|---|---|
| E1 Methodology approved | `universal.entry`, `registry.entry`, `registry.locked` | E1 | `"179422"` | `"179422"` |
| E2 Project registered | `infrastructure.entry`, `infrastructure.quantified`, `infrastructure.committed` | E2 | `"2822653806194"` | `"2822653806194"` |
| E3 Measured and issued | `measurement.entry`, `measurement.quantified`, `measurement.unit` | E3 | `"229518448943052722"` | `"229518448943052722"` |
| E4 Reported | `measurement.exit`, `reporting.entry`, `reporting.quantified`, `reporting.linked` | E4 | `"8321327376004027894"` | `"25035387438225574245358414"` |
| E5 Retired | `reporting.exit`, `registry.executed` | E5 | `"985106178361706"` | `"1144592878288235028923541329666"` |

- S and H agree until the first exit, at E4. After that S drops the measurement facets and H keeps them, with the exit.
- From E3 on, every value except S after E5 passes 2^53 − 1. Written as a JSON number, such a value is rejected by evidence/0 (`E_NUMBER_RANGE`, "use a string"). S after E5 is back below 2^53 while H after E5 is 100 bits, so the rule is "always a string", whatever the size.
- Every value factorises into dictionary primes, 2 to 349. A value with any other factor is a finding, not a coordinate.

### Forks

Suppose an event E4b lists E3 in `prev`, adds `measurement.linked`, and lands after E4. The record lane then has two heads:

| Head | S (`kernel/2`) | H (`kernel/2`) |
|---|---|---|
| E4 | `"8321327376004027894"` | `"25035387438225574245358414"` |
| E4b | `"15377736079184532374"` | `"15377736079184532374"` |

The reader reports both. A tool must not treat a forked subject as having one state. store/0 counts this fork in `counts.forks` only because the events are in the record lane; it does not count heads in assert lanes.

### Merges are not yet defined

An event that lists both heads in `prev` merges the fork. Say it adds `registry.linked`. H is then well defined: `"582047722551306375630337767086"`, in either order. S is not. Kernel/2 applies events "oldest first", but names no clock and no tie-break for concurrent branches. Applying E4 first gives S = `"193462540164717644507606"`. Applying E4b first gives `"2887500599473397679218"`. If E4b was dated before E4 but appended after it, log order picks the first and `at` order the second. They differ by a factor of 67, `measurement.linked`, which E4's exit removes in one order and not in the other.

Until Kernel/2 fixes the order ([open decision 9](#9-kernel2-rules-the-reader-depends-on)), the reader reports H at a merge and marks S as order-dependent whenever the two orders disagree.

### Kernel/2 rules the reader depends on

The reader cannot be specified fully until Kernel/2 settles these ([open decision 9](#9-kernel2-rules-the-reader-depends-on)):

- **Which lane its events use.** Kernel/2 names no `op`. As record changes, its events share the record lane, as above. As asserts, they land in `assert:<schema>` lanes. A chain that alternates between the two shows up in store/0 as a record-lane fork, and a fork inside an assert lane is not counted at all.
- **Order within one event.** Whether exits apply before entries changes S when one event exits and re-enters a block, which is how [Kernel/2 section 4](kernel-2.md#4-rules-verifier-gates) suggests changing a state.
- **Scope.** Kernel/2's first use is certificate lifecycles. Whether it, or a separate CENTR dictionary, applies to installations (the proposal's subject) is undecided.
- **Size.** Decided (27 November 2026): coordinates are decimal strings, and M9's acceptance test and the README now say so.
- **Draft status.** Kernel/2's own open questions (a `remove` list, `universal.exit` at retirement) change S. Until it ships at M9, coordinates are computed against a draft, and a receipt or basis that keeps them should say which.

## Reads that become evidence

### The pattern

[Vector 003](../test-vectors/objects/003-agent-assertion-approved/) already shows how this repository records an AI-assisted check. The agent's `assert` holds what the model was given (by hash) and what it returned. `actor.kind` is `agent`, with `on_behalf_of` naming the accountable person. That person's acceptance is an `approve` attestation. This note adds one thing to the pattern: the reads from the store that the decision relied on.

- **Domain inputs stay in `content`,** as in vector 003: the rule set and prompt by hash, the model, the result, and the objects the model itself cited.
- **The store-read basis goes in one fixed `ext` profile,** working name `read-basis/0`. It names the log, the checkpoint, the watermark and clock, and for each subject read, its lane, its heads and their S and H ([open decisions 1 and 2](#open-decisions)).
- **`prev` keeps its meaning,** "ids this object replaces": for example, the previous decision in the same lane ([open decision 3](#3-the-decisions-subject)).
- **Approval is an attestation.** The accountable person signs `approve` on the assert's id.
- **A refusal or an override is a new `assert` by a human actor,** with the disposition in its content and an `approve` attestation. If it is in the same lane as the agent's assert, it lists that assert in `prev`, because it replaces it.

### Example: an agent's recommendation and a person's approval

An agent read the unit's record lane at the checkpoint covering E1 to E4 (head E4 in the table above) and recommended that the unit is eligible for retirement. A reviewer approved. All data is synthetic.

```json
{
  "v": "evidence/0",
  "id": "sha256:dcd3790d25c43dbc3e118678cb9c6ab448a2505846e25dd7e4dd46299a872779",
  "op": "assert",
  "schema": {
    "ref": "example-case/eligibility-recommendation@1",
    "hash": "sha256:9475a043aec5b08cd245a8f9e1a813b21f26adca16367069b740d2ed4359b0ae"
  },
  "subject": {
    "system": "example-registry",
    "type": "accu",
    "key": "ACCU-TEST-000001"
  },
  "content": {
    "media_type": "application/json",
    "hash": "sha256:f5cb3e7200095c4e9b83955db20d578641676489f6e96443dac0f4d990f20721",
    "inline": {
      "check": "retirement-eligibility",
      "rules": "sha256:cf03bed64b9562a7b1d6e3fd294e12592aa9cbe816082dc13345debb05efc874",
      "model": "example-model-2026-10",
      "prompt": "sha256:ded01c7397f7c7d3ccd445c83534921ff420bbfed55bc4789db9e2f9486ee38d",
      "cited": [
        "sha256:97e5e21f8b826c102e6cf6e8a5ecf37b538b4e6d531992df2782dc045cabf908"
      ],
      "result": "recommend-approve"
    }
  },
  "actor": {
    "id": "agent:eligibility-assistant",
    "kind": "agent",
    "on_behalf_of": "person:reviewer-01"
  },
  "at": "2026-11-27T01:25:00Z",
  "prev": [],
  "ext": {
    "read-basis/0": {
      "as_of": {
        "clock": "recorded",
        "log": "sha256:44588380a04d80699633f4a10fb7e7d0075c7cc38463d481ba7c62e34453da3d",
        "checkpoint": {
          "id": "sha256:354cc1665affb3b07b5719fe5b16a4b9488f00eb7aa18b6d0041011aea369b55",
          "size": 6,
          "root": "sha256:9241e2acc9d129f91bf3fd8ede1477376c438d517d0bf9eca5bfa161d6c4d582"
        },
        "watermark": {
          "seq": 6,
          "entry_hash": "sha256:eed1e34a486736dc18001fbd004cb14743c30efbc4e8963471cdf16fe21b4e0d"
        }
      },
      "reads": [
        {
          "subject": {
            "system": "example-registry",
            "type": "accu",
            "key": "ACCU-TEST-000001"
          },
          "lane": "record",
          "complete": true,
          "heads": [
            {
              "id": "sha256:97e5e21f8b826c102e6cf6e8a5ecf37b538b4e6d531992df2782dc045cabf908",
              "profile": "kernel/2",
              "S": "8321327376004027894",
              "H": "25035387438225574245358414"
            }
          ]
        }
      ],
      "left_out": [],
      "truncated": false,
      "receipt": "sha256:325e81c5b35f4780b1ec4a190a1f7ce245569c0029e2d893e077b67acaddb711"
    }
  }
}
```

The two attestation lines kept beside it ([D4](decisions.md#d4-signatures-outside-the-object)): the agent's `author` line and the reviewer's `approve` line.

```json
{"at":"2026-11-27T01:25:00Z","key":"ed25519:le29uSs1Wptjx0Va7lT2l45JrD_44rdQa64S2NAHAgo","object":"sha256:dcd3790d25c43dbc3e118678cb9c6ab448a2505846e25dd7e4dd46299a872779","purpose":"author","sig":"uLodKn2Sq0QhuDYbQvnHmU0l6-IPe-JvAn2bbi4rHxmvz_TppuG0xaAuURgEwz5aOjpMI7yE9rvqbPBpal8FCw","signer":"agent:eligibility-assistant","v":"attestation/0"}
{"at":"2026-11-27T01:27:00Z","key":"ed25519:YQRBJCBd57PjtLAXSbo4riApK1EbmtuuqwDiWxX4q6E","object":"sha256:dcd3790d25c43dbc3e118678cb9c6ab448a2505846e25dd7e4dd46299a872779","purpose":"approve","sig":"8VkZBSW1sPVXzdN5XdWtp_ccOcdH_kHS7yilxjOH6aebCD8Lp9p2tHEni0ZOJ97_6bF2Kp2XB9PTWiQ4QGXSBw","signer":"person:reviewer-01","v":"attestation/0"}
```

```sh
evidence verify decision.json --attestations decision.jsonl --require author --require approve
# {"attestations":{"approve":1,"author":1},"content_checked":true,"id":"sha256:dcd3790d…","ok":true}

evidence verify decision.json --attestations author-only.jsonl --require approve
# {"attestations":{"author":1},"code":"E_ATTESTATION_MISSING",…,"ok":false}   exit status 1
```

What the example shows:

- **Subject.** The subject is the unit decided about, so the decision appears in the unit's history and in M4's "latest assert per schema".
- **Lane.** The decision lands in lane `assert:example-case/eligibility-recommendation`. Appending it to a store/0 log left the unit's record-lane heads at E4.
- **Chaining.** It is the first decision in its lane, so `prev` is empty and it was appended with `if_heads` set to none.
- **Citations.** `content.cited` is the model's own list of what it relied on. The basis is what the reader verified. The two are kept apart, and a checker can require every cited id to be in the basis.
- **Coordinates.** S and H are strings tagged `kernel/2`. Here S and H differ, because E4 exited the measurement block.
- **Read receipt.** `receipt` names the read receipt above by its content hash. It is optional.
- **Privacy.** The actor and the approver are pseudonymous ids, and nothing personal is inline.
- **Checkability.** The ids under `ext` point into a throwaway synthetic store. They become checkable by others once the example is published as a vector together with its log.

### Where the basis goes

| Placement | What checks it | Cost |
|---|---|---|
| In `content`, as the proposal had it | Nothing but the content hash. An id that nobody holds is accepted. | Unchecked, and every tool invents its own shape |
| In `prev`, as the first draft suggested | Intake: every id must be held and not dated after the decision (`E_PREV_MISSING`, `E_PREV_ORDER`). A `prev` into another lane leaves heads alone. | evidence/0 defines `prev` as "ids this object replaces", and status is a view over `prev` links, so a naive status view would read E4 as superseded by the decision. A basis held in another store or in a bundle fails `E_PREV_MISSING`. M4's history view would list every decision as a descendant of the records it read. |
| In an `ext` profile (recommended) | A profile verifier, with `P_` codes. The store keeps an unknown profile unchecked ([roadmap decision 10](roadmap.md#decisions-confirmed-26-november-2026)); once the profile is known, it is checked. | One new profile, with vectors and a JavaScript reference |

The recommendation is the `ext` profile, with `prev` kept for what the decision replaces ([open decision 1](#1-where-the-basis-goes)).

### The basis profile

A sketch, not yet a spec:

| Member | Meaning |
|---|---|
| `as_of.clock` | `recorded` (the store's `seq` and `received_at`) or `claimed` (writers' `at`) |
| `as_of.log` | The log id |
| `as_of.checkpoint` | `id`, `size` and `root` of the checkpoint the read was made against |
| `as_of.watermark` | `seq` and `entry_hash` of the last entry the reader considered. Equal to the checkpoint's `size` and `head` when the read was made exactly at a checkpoint, as in the example. Later when the reader read past the latest checkpoint |
| `reads[]` | For each subject read: `subject`, `lane`, `complete`, and `heads[]` with `id`, `profile`, `S`, `H` |
| `left_out[]` | Scopes the reader did not or could not read, each with a reason code such as `not_permitted`, `out_of_scope` or `rejected`. A scope is a system or a lane. Never list refused subject keys: that would reveal that they exist |
| `truncated` | `true` when a limit cut the read short (M4 computes it by fetching one more than the limit) |
| `receipt` | Optional: the hash of the read receipt |

Proposed checks for its verifier, all computable from the log:

| Code (proposed) | Meaning |
|---|---|
| `P_BASIS_CHECKPOINT_UNKNOWN` | The cited checkpoint (`id`, `size`, `root`) is not one of this log's checkpoints. |
| `P_BASIS_HEADS_MISMATCH` | The claimed heads are not `heads(subject, lane)` at the watermark. The reader missed something that was there. |
| `P_BASIS_STALE` | Informational. The lane moved between the watermark and the assert's own `seq`. |
| `P_BASIS_COORDINATE_DRIFT` | A claimed S or H differs from the recompute (Kernel/2 REQ-KRN-013). At a merge, H only, until [open decision 9](#9-kernel2-rules-the-reader-depends-on) fixes S. |

A prototype of these checks, run over the synthetic log, found nothing on the example decision. It flagged `P_BASIS_STALE` on a later revision that cited the same head E4 after E5 had landed.

### What an approval does and does not prove

- **Who signed is not checked.** The core checks that a valid `approve` attestation exists, not who signed it. An `approve` signed with the agent's own key, under the agent's own name, also verifies with `ok: true`. A profile rule should require that an `approve` on an agent's assert is signed as its `on_behalf_of` (compare M3's `P_RESOLVER_NOT_HUMAN`); see [open decision 8](#8-who-may-approve).
- **Before key binding (M5), an attestation proves only that some key holder signed** ([evidence/0 Not in v0](../spec/evidence-0.md#not-in-v0), [roadmap decision 4](roadmap.md#decisions-confirmed-26-november-2026)). So before M5, anyone with the log can check *what informed the decision*: the basis ids, the checkpoint and the heads. Nobody can yet check *who decided*. Approvals made with development keys are not accountability evidence.

## Store mechanics

### Who emits

- **The acting tool.** The reader never writes. The tool that takes the action appends the assert.
- **Persist before acting.** Where policy says an action needs a basis assert, the action goes ahead only after the store has returned an append receipt for it. Failing to record means not acting.
- **Policy as data.** The list of action types that need a basis assert is published data, an `assert` like M6's `register/0`. It is not a per-request flag or a deployment switch.
- **The channel does not exist yet.** Appending needs the store's single-writer lease; without it the store answers `S_LOCKED`. From M4 the service holds the lease and its API is read-only. M6 adds intake only for `sync/0` requests. Until there is an intake for asserts, a tool that records decisions must run inside the writer process ([open decision 7](#7-an-intake-for-asserts)). M2's mapping asserts and M3's resolutions have the same gap.

### Signing

- **Who signs what.** The agent signs `author`. The accountable person signs `approve`.
- **One batch.** Send the object and its attestation lines as one batch, so the store never holds a decision without the approval it was sent with. store/0 batches are all or nothing. The library supports them; the command-line tool does not, because it commits each file in its own transaction. Between the two appends, `store get … --require approve` fails with `E_ATTESTATION_MISSING`.
- **Coverage.** Do not rely on a decision outside the store until a checkpoint covers the entries of both the assert and its approval.
- **Order.** A verifier policy may flag an `approve` dated before the object's own `at`.

### The staleness window

`if_heads` compares the heads of the object's own subject and lane ([store/0 Intake](../spec/store-0.md#intake)). A decision lives in `assert:<schema>`, so its `if_heads` cannot see the record lane it read. In the synthetic store, after the record lane moved from E4 to E5, a decision citing E4 was still accepted, and only the order of `seq` showed that its basis was stale.

- **Default: record and detect.** The basis carries the watermark and the heads, and `P_BASIS_STALE` flags a decision whose lane moved before the decision landed.
- **Option: a precondition.** An optional check of other lanes' heads, made in the same transaction as the append, chosen per decision schema so that busy capture cannot starve approvals. Decide in M6 ([open decision 6](#6-a-basis-that-moved-on)).
- **Actions that change a record.** For an action that changes a record in the system of record, the check that matters is where the change lands. M6 requests carry the base version, M7 delivers with the system of record's row version, and a stale result is held, never merged. Record lanes trail the system of record by the capture lag, so no check in the store can catch an edit that capture has not yet seen. A store precondition helps only decisions that authorise something without a write to the system of record.

### Idempotency

- **Same bytes on retry.** The tool builds the whole object once, with `at` fixed, and keeps it durably before the first send. A retry resends the same bytes, and the store answers `duplicate: true`.
- **One decision per lane position.** Decisions of one schema about one subject chain in their lane. Each lists the previous decision in `prev` and is appended with `if_heads` set to the lane's heads as the tool last saw them; the first uses `if_heads` none. A retry that was rebuilt (new `at`, so a new id) is then refused with `S_STALE_HEADS`, and the heads returned name the decision that already landed. The tool reads that decision back instead of deciding twice. The cost is that decisions of one schema about one subject are serialised.
- **Unchained decisions pile up.** They become extra heads of their assert lane, and `counts.forks` counts only record lanes, so no report shows them.
- **From M6,** fold this into the change-key de-duplication.

### Completeness and freshness

- **Valid is not complete** (step 2). Against a local store today, `evidence store heads` reads a projection at the current head. It has no "as of" option, and the projection is trustworthy only after a whole-log verify. A reader should fold log entries 1 to N itself, or read the heads inside the same snapshot it verified, and record the watermark it used. M4's index should offer heads as of a watermark.
- **Remote readers and bundles.** An M4 bundle (objects, attestations, entries, a covering checkpoint and inclusion proofs) proves membership, not completeness. To prove "these are all the heads at N" without shipping the whole prefix, store/0 needs a commitment it does not have: checkpoint content is a closed set of members, and entries carry no subject or lane. That is a store/0 format change ([open decision 10](#10-proving-completeness-to-a-remote-reader)).
- **Freshness.** Nothing tells a reader that its checkpoint is the latest. Entries after the latest checkpoint, the decision itself included, are covered by no signature until the store writes another, and only the store can write one. The basis records the watermark so that an inclusion proof in a later checkpoint can confirm it. How often checkpoints are written is [open decision 11](#11-checkpoint-cadence).

## Lessons from earlier systems

Two earlier systems informed this note. Their lessons, stated generically:

1. **A local read audit worked, but could not say what was read.** An earlier system audited every read in a local table kept apart from its authoritative records, and wrote the audit row before answering: no audit, no answer. The rows held no keys or values by rule, so they could never say which version of which record was read. *Here:* the read receipt stays tool-local and value-free, but names the checkpoint, the heads and the ids.
2. **The read-to-action link that held was a version check where the change landed.** A write carried the version it was based on, and the system of record refused it if the record had moved; the stale change was held, never merged. A fallback mode that re-read and then wrote would overwrite a person's edit made in between. *Here:* for record changes the check stays at the system of record (M6, M7).
3. **Only persist-first records held.** Where a system wrote its own record before acting (an outbox), the trail was complete. Mirrors that were never wired up, copies sent fire-and-forget, recording that a caller could switch off per request, and failures that were only logged gave sparse trails. *Here:* persist the basis assert before acting, and publish the policy as data.
4. **A permission check had side effects.** Evaluating a permit changed the caller's lifecycle state, so later permits were granted without being logged. One branch of the check could not deny, an audience check did nothing, and an evidence check tested presence rather than integrity. *Here:* whatever lets a tool emit an assert has no side effects, fails closed, is bound to the exact subject, lane and schema, and checks content by hash.
5. **Bases were recorded by mutable reference.** An AI turn recorded what it read as keys whose current value was overwritten on every write, or as a ledger name and a range of positions, so the record could not say which version or which entries were read. *Here:* the basis names content-addressed ids, the heads of each lane read and a covering checkpoint.
6. **The model could rewrite its own basis.** When the model's metadata parsed, its own list of sources replaced the system's retrieval list. A silent cap dropped the system's items first, and the provider's name was stored as the model id. *Here:* the reader writes the basis from what it verified, the model's citations are kept apart and checked against it, and the exact model and the prompt hash are recorded.
7. **Recording what was withheld helped.** One path stored what was read, what was left out and why, and whether the context was truncated. *Here:* `left_out` and `truncated` in the basis, at the level of scopes only.
8. **Who acted was kept apart from who is accountable.** A service account could recommend but not approve. A model answered on a person's behalf only while a grant to act for that person was in force. *Here:* an agent's assert names `on_behalf_of`, and approval is the accountable person's attestation.
9. **Reads wrote.** A search allocated new prime numbers to query terms it had not seen, so the numbering depended on the order of reads. *Here:* the reader writes nothing, and an unknown facet is a finding.
10. **Agent output came back as input.** Stored agent answers were recalled in later turns as context, alongside observed records, with only a mild penalty. *Here:* S and H come from record lanes only. A basis that includes an agent's assert says so, and a checker can flag reliance on agent asserts that carry no approval.
11. **"Memory" meant recalling the agent's own turns, by default.** Every turn was written and recalled, the permission defaults were permissive, and a per-identity "memory key" protected nothing. *Here:* M4 memory reads are verified queries over observed evidence, not recall of an agent's turns. Continuity between turns, if a tool needs it, is explicit asserts under a recorded grant, and forgetting is M5 erasure.
12. **Routing by actor kind split decisions from their bases.** Objects went to different logs depending on whether the actor was a person. *Here:* a decision goes in the same store as its basis, separated by lane. A basis in another store names that store's log and checkpoint.
13. **A recommendation was kept without its rules or its inputs' versions.** It was computed "as of" a date, but neither that date, the rule version nor the version of the record read was stored. *Here:* the decision carries its rule set by hash, the profile of its coordinates, the clock and the basis.
14. **Two clocks.** One system stored its own receive time and kept the writer's time only after a bounds check. *Here:* staleness is judged on `seq` and `received_at`, not on writers' claimed times ([D13](decisions.md#d13-two-clocks)).

No incident record of an unrecorded read was found in either system. These are design lessons, not measured failure rates.

## Open decisions

### 1. Where the basis goes

- Options: (a) in `content`, unchecked; (b) in `prev`; (c) in one `ext` profile; (d) the heads in `prev` and the full basis in `ext`.
- Recommended: (c). Keep `prev` for what the decision replaces, and keep domain inputs (rule set, prompt, photos) in `content` as vector 003 does.
- Why: (b) stretches "ids this object replaces" to mean "based on", risks status and history views reading records as superseded by decisions, and cannot cite a basis held in another store or a bundle. (d) is the fallback if intake checks are wanted before the store knows the profile. store/0 would then have to say that a `prev` into another lane means "based on".

### 2. The basis profile

- Options: (a) adopt the sketch above, name it, and give it vectors and a JavaScript reference ([D7](decisions.md#d7-the-vectors-are-the-contract)); (b) leave the shape to each tool.
- Recommended: (a), at M4, under a name such as `read-basis/0`.
- Why: third parties must recompute it, and only one fixed shape keeps every tool's decisions comparable.

### 3. The decision's subject

- Options: (a) the subject decided about, with decisions of one schema chained in their lane and appended with `if_heads`; (b) the decision's own record in a case system, where one exists; (c) a separate subject per decision, like M6's `evidence-sync/intent/<id>`.
- Recommended: (a) by default, and (b) where a case record already exists.
- Why: (a) puts the decision in the subject's history and state views and gives idempotency through `if_heads`. (c) takes the decision out of those views, and the de-duplication it offers is also available under (a).

### 4. When a basis assert is required

- Options: (a) the tool owner decides case by case; (b) a published list of action types that require one, enforced fail-closed; (c) every read.
- Recommended: (b). "Must" for listed action types, by default every decision by an agent or a service that recommends or authorises an action on a subject. "May" otherwise.
- Why: it keeps the owner's point that this is policy, not architecture, while making the policy itself checkable.

### 5. Failed verifications

- Options: (a) a finding in the read receipt only; (b) always an `assert`; (c) an `assert` when the failure blocks or changes an action.
- Recommended: (c). Store-wide integrity stays with `store verify` and M3.
- Why: a failure that changes an action is a basis for it. Recording every failure would let a noisy source flood the log.

### 6. A basis that moved on

- Options: (a) record and detect, with `P_BASIS_STALE`; (b) an optional same-transaction check of other lanes' heads, chosen per decision schema; (c) both.
- Recommended: (a) now. Decide (b) in M6, and only for decisions that authorise something without a write to the system of record.
- Why: for record changes the system of record's own version check is the one that works.

### 7. An intake for asserts

- Options: (a) only tools inside the writer process can record decisions until M6; (b) an assert intake in M4b; (c) one in M6 beside `POST /intents`.
- Recommended: (b), or early M6 at the latest. One request is one batch: the object and its attestation lines, with optional `if_heads`, write scopes per lane, and store receipts in the answer.
- Why: M2 mapping asserts and M3 resolutions need the same path, and the command-line tool cannot send an object and its approval together.

### 8. Who may approve

- Options: (a) any valid `approve` attestation, as the core checks today; (b) a profile rule that an `approve` on an agent's assert must be signed as its `on_behalf_of`; (c) wait for M5 key binding.
- Recommended: (b) now and (c) at M5. Label approvals made before M5 as not accountability evidence.
- Why: today an agent can approve its own recommendation and the core accepts it.

### 9. Kernel/2 rules the reader depends on

These belong in [Kernel/2's open questions](kernel-2.md#8-open-questions-for-the-owner).

- **Lane of events.** Options: the record lane, with events as lifecycle changes that capture observes in the registry; assert lanes under one schema family; either. Recommended: the record lane, so that one fold and store/0's fork count both cover the whole chain.
- **Order at a merge.** Options: log order (`seq`); `at` order; a topological order with ties broken by (`at`, id); S defined only on linear chains, with a merge event restating the state. Recommended: topological order with (`at`, id) ties, because it depends on the objects alone and not on one store's positions. Run the gates per event over that event's own ancestors, and add merge vectors.
- **Order within an event.** Options: exits first; exits last; forbid an exit and an entry of the same block in one event. Recommended: exits first, which is the only reading under which exiting and re-entering a block in one event changes a state.
- **Scope.** Options: Kernel/2 also covers installations; installations get their own dictionary and profile tag at M9. Recommended: their own, with Kernel/2 kept for certificate lifecycles.
- **Size.** Decided (27 November 2026): decimal strings. M9's acceptance test and the README were amended to match.

### 10. Proving completeness to a remote reader

- Options: (a) ship the log prefix up to the checkpoint; (b) a commitment to all heads in a new checkpoint version, for example a sparse Merkle tree keyed by a digest of subject and lane; (c) a keyed digest of subject and lane in a new log-entry version, so a reader can scan entries against the root without seeing other subjects' objects; (d) the M4 service signs a completeness assert.
- Recommended: (a) or a local store until M4, then decide between (b) and (c) in M4's design. Either is a store/0 version change; evidence/0 is unchanged.
- Why: (b) is one commitment per checkpoint that `store verify` can recheck, but it is a store-signed claim that only the full log can audit. (c) is provable from the root but costs a scan. (d) is the weakest: attested, not proven.

### 11. Checkpoint cadence

- Options: (a) on a timer; (b) after each batch that others will rely on; (c) on request from a tool, through the service.
- Recommended: (a) and (b), written by the store or the M4 service. External publication of checkpoints comes with M5.
- Why: a decision becomes checkable outside the store only once a signed checkpoint covers it.

### 12. Privacy of actor ids

- Options: (a) no rule; (b) a rule in store/0 Privacy that `actor.id`, `on_behalf_of` and attestation signers are pseudonymous, resolved to people only through an access-controlled directory or M5 key bindings.
- Recommended: (b). evidence/0 stays frozen.
- Why: today's privacy rules cover only inline content and guessable hashes, while actor ids are written into the log forever.

### 13. Agent memory

- Options: (a) M4 also serves an agent's recall of its own earlier turns; (b) out of scope for M4: continuity between turns is explicit asserts under a recorded grant.
- Recommended: (b).
- Why: memory reads are verified queries over observed evidence. An earlier system's recall of its own outputs fed them back as input (lesson 10).
