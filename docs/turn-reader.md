# The turn-by-turn reader (design note)

**Status:** design note, 27 November 2026, revised after review. Design input for [M4](roadmap.md#m4-memory-reads-index-query-api-bundles-read-only-service) (memory reads, and the coordinate column in M4b), [M6](roadmap.md#m6-requests-and-the-outbox-as-evidence-in-shadow-mode-no-write-code) (requests and continuity notes) and [M9](roadmap.md#m9-optional-centr0-profile-register-views-and-a-published-dictionary) (Kernel/2 and a published dictionary). Decisions 8, 12 and 13 follow the design in [approvals.md](approvals.md). The open decisions were settled on 28 November 2026 ([Open decisions](#open-decisions)); those that need a decision outside the project are marked gated. Nothing here is built. The examples are synthetic, and each evidence object in them verifies with the `evidence` command-line tool. Not an official publication of any agency; it describes no agency's process (see the [README](../README.md) notice).

## In short

A tool calls the reader before it relies on evidence about a subject. The tool might be an AI agent working turn by turn, a case-management screen or a batch job.

- **Reads are passive.** The store records writes only. Getting an object, its heads or a proof adds no entry to the log ([store/0](../spec/store-0.md#log-entry)).
- **A routine read leaves a read receipt** in the tool's own log, not in the store.
- **A read that is the basis for an action is recorded in the store,** inside an `assert` about that action, appended before the action is taken.

The sections below fill in the store mechanics: where the basis goes, who approves, what "complete" means, and what happens when the evidence moves on between the read and the action.

## Starting points

- **Reads do not become appends.** store/0 gives `seq` and `received_at` to entries, and an entry is only ever an object or an attestation.
- **A routine verification produces no evidence object,** or the log would fill with noise. This is a policy default, not a rule of the spec: M3 does record every reconciliation run, because there the result of the check is the point.
- **A read that informs a decision is recorded** as an `assert` holding what the decision relied on and what it concluded. An append-only log that cannot explain why a decision was made is only half an audit trail. Here a *decision* is a recorded `assert` that an action relies on, together with its approvals.
- **Whether a given read needs an assert is policy,** set by each tool's owner as published data, not a mechanical consequence of the architecture ([decision 4](#4-when-a-basis-assert-is-required)).
- **A basis is structured, not free text:** log, checkpoint, watermark, clock, subject, lane and heads, so that anyone can re-run it. Every query says which clock it uses ([roadmap principles](roadmap.md#principles)).
- **Every time is in UTC with `Z`,** and labelled with whose clock it is.
- **Approval is a signature, not a content field.** Content is the writer's claim. Approval is an `approve` attestation by the accountable person ([evidence/0 Attestations](../spec/evidence-0.md#attestations), [D4](decisions.md#d4-signatures-outside-the-object)).
- **Tools do not decide.** A tool's output authorises nothing by itself. It counts only once the accountable person signs `approve` on it ([decision 8](#8-who-may-approve), [approvals.md](approvals.md)).
- **Derived values are never authoritative.** They are recomputed from the chain, never trusted from an index ([README](../README.md#prime-coordinates-for-faceted-queries): "derived, never authoritative").

## What the reader does

### The four steps

1. **Verify.** Run evidence/0 steps 1-5 and check the attestations, with the purposes the caller requires ([evidence/0 Verification](../spec/evidence-0.md#verification)). For an object read from a store, also check an inclusion proof against a checkpoint whose witness verifies under the genesis key ([store/0](../spec/store-0.md#checkpoint-and-tree)).
2. **Establish the lane as of that checkpoint.**
   - Find `heads(subject, lane)` at the checkpoint's size N.
   - Take the same-lane `prev` closure from those heads. Every object in a lane is an ancestor of one of its heads, or a head itself, so these two steps give exactly the lane as of N.
   - If the lane's profile defines derived values, compute them for each head from the chain: coordinate/0 coordinates, for example, or the Kernel/2 draft's S and H. Each value is carried in coordinate/0's tagged form (profile, dictionary id and coordinate) under its coordinate name ([coordinate/0 Tagged form](../spec/coordinate-0.md#tagged-form)). A lane whose profile defines none gets heads and ids only.
   - If the reader cannot establish the heads at N (for example, it was handed some objects and their inclusion proofs), it reports *membership verified, completeness unknown*. It may give derived values for a named head, but never as the subject's state.
3. **Emit a read receipt** to the tool's own log ([below](#the-read-receipt)).
4. **If the read is the basis for an action, record it before acting.** The acting tool, not the reader, appends an `assert` that carries the basis ([below](#reads-that-become-evidence)). Which actions need one is the tool owner's published policy ([decision 4](#4-when-a-basis-assert-is-required)).

Step 2 replaces a weaker completeness test, "the lane is closed under `prev` back to its first event". That test finds a missing middle event but never a missing tip. A lane cut short after E3 passes every check and reports the lane as of E3, where the true state is as of E5.

The reader itself never writes: not to the store and not to any dictionary. A facet that is not in the profile's dictionary is a finding, never a new allocation.

### The rule

| Read | Recorded in the store? | Where | Why |
|---|---|---|---|
| Routine verification of an object or a lane | No. A read receipt only | The tool's own log | Reads add no entries; appending them would flood the log |
| A verification that fails | A finding in the read receipt. Also an `assert` when the failure blocks or changes an action ([decision 5](#5-failed-verifications)) | The tool's log; also the store when it changes an action | It may be a sign of tampering, and M3 records every reconciliation result. Recording every failure would let a noisy source flood the log |
| A read that is the basis for an agent's or a service's output that an action relies on | Yes: an `assert` carrying the basis, appended before the action | The store, in the subject's `assert:<schema>` lane | "What the model was given, what it returned, and who approved or overrode it" ([README](../README.md#what-it-is-for)) |
| A person's approval of that output | Yes: an `approve` attestation on the assert | The store, as an attestation entry | Approval is the accountable person's signature, not a content field |
| A person's own decision, or an override | Yes: an `assert` by a human actor, with an `approve` attestation | The store | The resolution pattern of [M3](roadmap.md#m3-two-way-reconciliation-report-only-recorded-as-evidence) |
| A register snapshot (a query result) | Yes: an `assert` whose content is the result at a recorded watermark (M9). An M4 bundle can also be sealed as an assert | The store | "What did the list show on date X?" gets a provable answer |
| A derived index (such as S and H), if the lane's profile defines one | Never as an authority. Recomputed on every read | Tool memory; a derived index column only from M4b or M9 | Derived values are never authoritative |

## The read receipt

### Shape

| Member | Meaning |
|---|---|
| `v` | `"read-receipt/0"`. Its JSON Schema, planned for M4, is built from the basis's `as_of` and `heads` blocks ([decision 2](#2-the-basis-profile)). |
| `ok` | Whether every check passed. |
| `verified_at`, `verified_at_clock` | When the tool verified, in UTC with `Z`, and whose clock that is (`"tool"`). It is not the store's `received_at`. |
| `subject`, `lane` | The evidence/0 subject triple and the lane read. Heads are kept per subject key and lane. |
| `as_of` | `clock`, `log`, `checkpoint` (`id`, `size`, `root`) and `watermark` (`seq`, `entry_hash`). Heads, and any derived values, are true only relative to a point in a particular log. |
| `complete` | Whether step 2 established the heads at the checkpoint, or only membership. |
| `heads` | One entry per head, because a forked lane has several. Each has its `id` and, if the lane's profile defines derived values, each value in coordinate/0's tagged form under its coordinate name: here `S` and `H`, each with `profile`, `dictionary` and `coordinate` ([coordinate/0 Tagged form](../spec/coordinate-0.md#tagged-form)). The coordinate is a decimal string ([Kernel/2 3.3](kernel-2.md#33-representation-declared)). H answers "was it ever in state X?". |
| `findings` | Anything that failed or could not be checked. |

Two terms are already taken in store/0. A *checkpoint* is a signed tree head, and only the store writes one; a *receipt* is what an append returns. So a tool *saves* its lane state, and this note says *read receipt*.

Example (synthetic, read at the checkpoint after E4, in a lane whose profile defines the Kernel/2 coordinates):

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
      "S": {
        "profile": "kernel/2",
        "dictionary": "sha256:28ec5d10ee5e6e5fd8effc7515e9adec7f049f75beeccce5064d738d922cac71",
        "coordinate": "8321327376004027894"
      },
      "H": {
        "profile": "kernel/2",
        "dictionary": "sha256:28ec5d10ee5e6e5fd8effc7515e9adec7f049f75beeccce5064d738d922cac71",
        "coordinate": "25035387438225574245358414"
      }
    }
  ],
  "findings": []
}
```

### Rules

- **Tool-local.** The read receipt is never appended, and no verification depends on it. So its JSON Schema has no JavaScript reference: those are kept for formats that others must reproduce ([roadmap decision 5](roadmap.md#decisions-confirmed-26-november-2026)).
- **Value-free.** No record content, no personal data, no free text copied from records. A tool's log is outside the store's erasure path (M5), so erasure cannot reach a value copied there.
- **The same blocks as the basis.** `as_of` and `heads` have exactly the shape the basis uses ([below](#the-basis-profile)). Recording a read as a basis is then a copy, not a translation.
- **Never the only record of a read that led to an action.** The assert holds the basis. It may name the read receipt by hash.
- **Save, don't trust.** A tool may save its lane state between turns, but only as a cache. On the next turn it recomputes, and it never presents a saved value as evidence.

A cheap check across turns: heads only move forward in store/0, so each head seen at turn n is an ancestor of a head at turn n+1, or still a head. Where the lane's profile defines H, H at the earlier head must therefore divide H at the later one. If it does not, the lane was rewritten or the reader was given a different log. A consistency proof between the two checkpoints is the stronger check, and it needs no derived index.

## Lane state

This section applies only when a lane's profile defines derived values. The worked example uses the [Kernel/2 draft](kernel-2.md), an M9 deliverable that is not built. Installations and devices get a dictionary and profile tag of their own ([decision 9](#9-kernel2-rules-the-reader-depends-on)). A lane with no such profile has heads and ids, and no S or H.

### Per event, per head

Kernel/2 defines S and H for each event, over that event and its ancestors ([3.2](kernel-2.md#32-two-coordinates-declared)). store/0 defines heads per subject key and lane, and a lane can have more than one. So the reader reports S and H for each head. There is no single "state of the subject" unless the lane has exactly one head.

### Worked table

This is the illustrative ACCU-style lifecycle of [Kernel/2 section 5](kernel-2.md#5-example-an-accu-lifecycle), one event per turn. Each event lists the previous one in `prev`, and all of them are record changes in the subject's record lane. A script recomputed every value from the committed dictionary, and each equals section 5.

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
- E5 adds `registry.executed` only, not `universal.exit`, so S after E5 keeps the universal entry ([decision 9](#9-kernel2-rules-the-reader-depends-on)). No event here exits and enters the same block, so the Kernel/2 rules settled on 28 November 2026 change no value in the table.

### Forks

Suppose an event E4b lists E3 in `prev`, adds `measurement.linked`, and lands after E4. The record lane then has two heads:

| Head | S (`kernel/2`) | H (`kernel/2`) |
|---|---|---|
| E4 | `"8321327376004027894"` | `"25035387438225574245358414"` |
| E4b | `"15377736079184532374"` | `"15377736079184532374"` |

The reader reports both. A tool must not treat a forked subject as having one state. store/0 counts this fork in `counts.forks` only because the events are in the record lane; it does not count heads in assert lanes.

<a id="merges-are-not-yet-defined"></a>

### Merges

An event that lists both heads in `prev` merges the fork. Say it adds `registry.linked`. H is then `"582047722551306375630337767086"` in either order. S depends on the order in which the two branches are applied. Applying E4 first gives S = `"193462540164717644507606"`. Applying E4b first gives `"2887500599473397679218"`. They differ by a factor of 67, `measurement.linked`, which E4's exit removes in one order and not in the other.

Kernel/2 fixes the order ([decision 9](#9-kernel2-rules-the-reader-depends-on)): the merge event's ancestors are applied in topological order, with ties broken by (`at`, id). E4 and E4b both follow E3, so the one with the earlier `at` goes first, and the smaller id if their `at` are equal. If E4b was dated before E4 but appended after it, S at the merge is `"2887500599473397679218"` in every store, whatever the log order. The order depends on the objects alone, so stores and replicas that hold the same objects agree. The gates run per event over that event's own ancestors, so E4b's `measurement.linked` is valid although E4 exits the block on the other branch. Merge vectors come with Kernel/2 at M9.

### Kernel/2 rules the reader depends on

The reader for a Kernel/2 lane depends on these rules, settled on 28 November 2026 ([decision 9](#9-kernel2-rules-the-reader-depends-on)). Kernel/2 states them as profile rules before it ships at M9.

- **Which lane its events use.** The record lane. Kernel/2's events are record changes that capture observes in the registry, as above, so one fold and store/0's fork count cover the whole chain. A chain that alternated between the record lane and `assert:<schema>` lanes would show up in store/0 as a record-lane fork, and a fork inside an assert lane is not counted at all.
- **Order at a merge.** Topological, with ties broken by (`at`, id) ([above](#merges)).
- **Order within one event.** An event that exits a block names no other facet of that block, so it never exits and enters one block. To change a state, it names the old state in an optional `remove` list instead ([Kernel/2 section 4](kernel-2.md#4-rules-verifier-gates)): only state facets of a block the unit has entered and not exited, applied before `add`, changing S only. H only grows. So no value depends on the order of the facets inside one event.
- **Re-adding a held facet.** S is a set. An event that adds a facet already in S, a state or the entry of a block already entered, leaves S unchanged, and H still counts it.
- **Retirement.** A retirement adds `registry.executed` only, not `universal.exit`, so universal state that stays true of a retired unit stays in S. The worked table already does this.
- **Scope.** Kernel/2 is for certificate lifecycles. Installations and devices get a dictionary and profile tag of their own: synthetic through M4b, and a real one only at M9 **[gated]**.
- **Size.** Decided (27 November 2026): coordinates are decimal strings, and M4b's acceptance test (M9's until 28 November 2026) and the README now say so.
- **Draft status.** Each value is in coordinate/0's tagged form, so a receipt or basis names the dictionary it was computed with. Until Kernel/2 ships at M9, a rule change could still alter S without changing the dictionary. Values computed before then, such as those in this note, are illustrative, and the `read-basis/0` vectors at M4 use a synthetic coordinate/0 profile, not Kernel/2 values.

## Reads that become evidence

### The pattern

[Vector 003](../test-vectors/objects/003-agent-assertion-approved/) already shows how this repository records an AI-assisted check (its ids are readable synthetic names; under [decision 12](#12-privacy-of-actor-ids) they would be opaque). The agent's `assert` holds what the model was given (by hash) and what it returned. `actor.kind` is `agent`, with `on_behalf_of` naming the accountable person. That person's acceptance is an `approve` attestation. This note adds one thing to the pattern: the reads from the store that the agent's output relied on.

- **Domain inputs stay in `content`,** as in vector 003: the rule set and prompt by hash, the model, the result, and the objects the model itself cited.
- **The store-read basis goes in one fixed `ext` profile,** `read-basis/0`. It names the log, the checkpoint, the watermark and clock, and for each subject read, its lane, its heads and, where the lane's profile defines them, their derived values ([decisions 1 and 2](#open-decisions)).
- **`prev` keeps its meaning,** "ids this object replaces": for example, the previous decision in the same lane ([decision 3](#3-the-decisions-subject)).
- **Approval is an attestation.** The accountable person signs `approve` on the assert's id.
- **A refusal or an override is a new `assert` by a human actor,** with the disposition in its content and an `approve` attestation. If it is in the same lane as the agent's assert, it lists that assert in `prev`, because it replaces it.

### Example: a tool's completeness check and a person's approval

A tool acting for a person read the unit's record lane at the checkpoint covering E1 to E4 (head E4 in the table above). It checked that the record is complete under a synthetic rule set: the events name the blocks the rule set requires, and each event after the first lists the previous one in `prev`. The person it acts for approved its output. This is illustrative only: no scheme decision is automated, and a person decides. All data is synthetic.

```json
{
  "v": "evidence/0",
  "id": "sha256:cccfa947ae909dab63807af189431a62eb3771b1c6a9179803c0fbc933b4b7dd",
  "op": "assert",
  "schema": {
    "ref": "example-case/completeness-check@1",
    "hash": "sha256:7271c95c29e77c0e8ebee89dd59ffc76258a7fdc37f11d86f6edae3dadb66cd0"
  },
  "subject": {
    "system": "example-registry",
    "type": "accu",
    "key": "ACCU-TEST-000001"
  },
  "content": {
    "media_type": "application/json",
    "hash": "sha256:8c09e5a5da5b4ced110333fddb1b801f6874e3e630fe46e0b84cc33bd325f813",
    "inline": {
      "check": "record-completeness",
      "rules": "sha256:6c9b617056be6a91604cb878b520fbe553120ab6f2de8c8821a489466372c5a2",
      "model": "example-model-1",
      "prompt": "sha256:6345213c4bc941a42626a2cd30356a4980d1073cbb0fb4780b909239ca9cfb46",
      "cited": [
        "sha256:97e5e21f8b826c102e6cf6e8a5ecf37b538b4e6d531992df2782dc045cabf908"
      ],
      "result": "complete"
    }
  },
  "actor": {
    "id": "agent:e7cc8fb8c7b86ecc223efea79fe7547f",
    "kind": "agent",
    "on_behalf_of": "person:f10c170c61109ad83fa7b99231f23fa0"
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
              "S": {
                "profile": "kernel/2",
                "dictionary": "sha256:28ec5d10ee5e6e5fd8effc7515e9adec7f049f75beeccce5064d738d922cac71",
                "coordinate": "8321327376004027894"
              },
              "H": {
                "profile": "kernel/2",
                "dictionary": "sha256:28ec5d10ee5e6e5fd8effc7515e9adec7f049f75beeccce5064d738d922cac71",
                "coordinate": "25035387438225574245358414"
              }
            }
          ]
        }
      ],
      "left_out": [],
      "truncated": false,
      "receipt": "sha256:ce8d8358efede317b66f71a7bdd41c0324b6f155eeee63797a7ac4347f93aff0"
    }
  }
}
```

The two attestation lines kept beside it ([D4](decisions.md#d4-signatures-outside-the-object)): the tool's `author` line and the accountable person's `approve` line.

```json
{"at":"2026-11-27T01:25:00Z","key":"ed25519:64cw5Nd6QOXqUuQQgBXW6x-vF5qZQ65wJHhCuRglJN0","object":"sha256:cccfa947ae909dab63807af189431a62eb3771b1c6a9179803c0fbc933b4b7dd","purpose":"author","sig":"GOoBq9n95F10XE3CzXaCcqKzQX5YXPvdhmZ-OJDU2lGdBMAFTxGgIWEWjmTdcAA6o1saHGNy4-i2V30nnVMjBQ","signer":"agent:e7cc8fb8c7b86ecc223efea79fe7547f","v":"attestation/0"}
{"at":"2026-11-27T01:27:00Z","key":"ed25519:x0snGg5MPjHpo4l4h-BtEslM5OQTqQtnYkcFiWtORxA","object":"sha256:cccfa947ae909dab63807af189431a62eb3771b1c6a9179803c0fbc933b4b7dd","purpose":"approve","sig":"W4UeBWTXWOQ4K9y5eHoikLdXmS81if_2jFYXUgJ3j7Eh4JlGLD_dcqgA6U4xyS_VV4fxjBatZzjAWZkgfnWPCQ","signer":"person:f10c170c61109ad83fa7b99231f23fa0","v":"attestation/0"}
```

```sh
evidence verify check.json --attestations check.jsonl --require author --require approve
# {"attestations":{"approve":1,"author":1},"content_checked":true,"id":"sha256:cccfa947…","ok":true}

evidence verify check.json --attestations author-only.jsonl --require approve
# {"attestations":{"author":1},"code":"E_ATTESTATION_MISSING",…,"ok":false}   exit status 1
```

What the example shows:

- **Subject.** The subject is the unit checked, so the check result appears in the unit's history and in M4's "latest assert per schema".
- **Lane.** The check result lands in lane `assert:example-case/completeness-check`. Appending it to a store/0 log left the unit's record-lane heads at E4.
- **Chaining.** It is the first check result in its lane, so `prev` is empty and it was appended with `if_heads` set to none.
- **Citations.** `content.cited` is the model's own list of what it relied on. The basis is what the reader verified. The two are kept apart, and the basis checks flag a cited id that is not in the basis (`P_BASIS_CITED_NOT_READ`).
- **Derived values.** This lane's profile defines the Kernel/2 coordinates, so the basis carries S and H, each in coordinate/0's tagged form with the Kernel/2 dictionary's id. Here S and H differ, because E4 exited the measurement block. In a lane with no derived values, each head carries its id only. The values are computed against the Kernel/2 draft, so they are illustrative.
- **Read receipt.** `receipt` names the read receipt above by its content hash (`evidence content-hash`). It is optional.
- **Privacy.** The tool and the person are pseudonymous ids of the form [decision 12](#12-privacy-of-actor-ids) sets, and nothing personal is inline.
- **Grant.** Under the design in [approvals.md](approvals.md), a tool that acts for a person also cites the grant it acts under ([decision 13](#13-continuity-for-tools)). This example leaves the grant out, to keep to the read basis.
- **Checkability.** The ids under `ext` point into a throwaway synthetic store. They become checkable by others once the example is published as a vector together with its log.

### Where the basis goes

| Placement | What checks it | Cost |
|---|---|---|
| In `content` | Nothing but the content hash. An id that nobody holds is accepted. | Unchecked, and every tool invents its own shape |
| In `prev` | Intake: every id must be held and not dated after the decision (`E_PREV_MISSING`, `E_PREV_ORDER`). A `prev` into another lane leaves heads alone. | evidence/0 defines `prev` as "ids this object replaces", and status is a view over `prev` links, so a naive status view would read E4 as superseded by the decision. A basis held in another store or in a bundle fails `E_PREV_MISSING`. M4's history view would list every decision as a descendant of the records it read. |
| In an `ext` profile (decided) | A profile verifier, with `P_` codes. The store keeps an unknown profile unchecked ([roadmap decision 10](roadmap.md#decisions-confirmed-26-november-2026)); once the profile is known, it is checked. | One new profile, with vectors and a JavaScript reference |

The basis goes in the `ext` profile, with `prev` kept for what the decision replaces ([decision 1](#1-where-the-basis-goes)).

### The basis profile

A sketch, not yet a spec. `read-basis/0` is specified at M4, with vectors and a JavaScript reference; its name is fixed before any EG1 vector cites it ([decision 2](#2-the-basis-profile)).

| Member | Meaning |
|---|---|
| `as_of.clock` | `recorded` (the store's `seq` and `received_at`) or `claimed` (writers' `at`) |
| `as_of.log` | The log id |
| `as_of.checkpoint` | `id`, `size` and `root` of the checkpoint the read was made against |
| `as_of.watermark` | `seq` and `entry_hash` of the last entry the reader considered. Equal to the checkpoint's `size` and `head` when the read was made exactly at a checkpoint, as in the example. Later when the reader read past the latest checkpoint |
| `reads[]` | For each subject read: `subject`, `lane`, `complete`, and `heads[]` with `id`, plus, where the lane's profile defines derived values, each value in coordinate/0's tagged form under its coordinate name (here `S` and `H`) |
| `scopes[]` | Optional. Whole scopes read, for a reader that reads a scope rather than a list of subjects, such as an aggregate detector, a reconciliation run or a register snapshot: each with `system`, `type`, `lane`, `filter` and `complete`. It names the scope, as `left_out` does, not each subject in it |
| `left_out[]` | Scopes the reader did not or could not read, each with a reason code from a closed list: `not_permitted`, `out_of_scope` or `rejected` (it failed verification). A scope is a system or a lane. Never list refused subject keys: that would reveal that they exist |
| `truncated` | `true` when a limit cut the read short (M4 computes it by fetching one more than the limit) |
| `receipt` | Optional: the hash of the read receipt |

Checks for its verifier, planned for M4, all computable from the log:

| Code | Meaning |
|---|---|
| `P_BASIS_CHECKPOINT_UNKNOWN` | The cited checkpoint (`id`, `size`, `root`) is not one of this log's checkpoints. |
| `P_BASIS_HEADS_MISMATCH` | The claimed heads are not `heads(subject, lane)` at the watermark. The reader missed something that was there. |
| `P_BASIS_STALE` | Informational. The lane moved between the watermark and the assert's own `seq`. |
| `P_BASIS_COORDINATE_DRIFT` | A claimed derived value differs from the recompute (for Kernel/2, REQ-KRN-013). At a merge, S is recomputed in Kernel/2's merge order ([decision 9](#9-kernel2-rules-the-reader-depends-on)). |
| `P_BASIS_CITED_NOT_READ` | An id the object's content cites (in the example, `content.cited`) is neither a head named in the basis nor an ancestor of one in the lane read. |

A prototype of the first four checks, run over the synthetic log, found nothing on the example check (in its earlier form, with S and H as bare strings). It flagged `P_BASIS_STALE` on a later revision that cited the same head E4 after E5 had landed.

### What an approval does and does not prove

- **The core does not check who signed.** It checks that a valid `approve` attestation exists, not who signed it. An `approve` signed with the tool's own key, under the tool's own name, also verifies with `ok: true`. Under the profile rules of [decision 8](#8-who-may-approve), an approval counts only under published rules; the approval-status view that applies them ([approvals.md](approvals.md)) is planned for M4.
- **Before key binding (M5), an attestation proves only that some key holder signed** ([evidence/0 Not in v0](../spec/evidence-0.md#not-in-v0), [roadmap decision 4](roadmap.md#decisions-confirmed-26-november-2026)). So before M5, anyone with the log can check *what informed the decision*: the basis ids, the checkpoint and the heads. Nobody can yet check *who decided*. Approvals made with development keys are not accountability evidence, and the planned approval-status view says so on every status (`"accountability": "not-established-before-key-binding"`).

## Store mechanics

### Who emits

- **The acting tool.** The reader never writes. The tool that takes the action appends the assert.
- **Persist before acting.** Where policy says an action needs a basis assert, the action goes ahead only after the store has returned an append receipt for it. Failing to record means not acting.
- **Policy as data.** The owner of each tool picks and publishes the list of action types that need a basis assert for that tool. The list is published data: an `assert` like M6's `register/0`. The deployment's approval rules may cite it; they do not choose it. A tool whose owner publishes no list falls back to the default: every action on a subject that relies on an agent's or a service's output needs a basis assert ([decision 4](#4-when-a-basis-assert-is-required)). The list is not a per-request flag or a deployment switch.
- **The channel does not exist yet.** Appending needs the store's single-writer lease; without it the store answers `S_LOCKED`. From M4 the service holds the lease and its API is read-only. M6 adds intake only for `sync/0` requests. Until there is an intake for asserts, planned for late M4 or early M6 at the latest, a tool that records asserts must run inside the writer process ([decision 7](#7-an-intake-for-asserts)). M2's mapping asserts and M3's resolutions have the same gap.

### Signing

- **Who signs what.** The tool signs `author`. The accountable person signs `approve`.
- **One batch.** Send the object and its attestation lines as one batch, so the store never holds a decision without the approval it was sent with. store/0 batches are all or nothing. The library supports them; the command-line tool does not, because it commits each file in its own transaction. Between the two appends, `store get … --require approve` fails with `E_ATTESTATION_MISSING`.
- **Coverage.** Do not rely on a decision outside the store until a checkpoint covers the entries of both the assert and its approval.
- **Order.** A verifier flags an `approve` dated before the object's own `at` with `P_APPROVAL_PREDATES_OBJECT`. It is informational and the approval still counts, because both times are signers' claims ([D13](decisions.md#d13-two-clocks)). Where order really matters, [approvals.md](approvals.md) judges it by log position (`P_APPROVAL_PREDATES_STEP`).

### The staleness window

`if_heads` compares the heads of the object's own subject and lane ([store/0 Intake](../spec/store-0.md#intake)). A decision lives in `assert:<schema>`, so its `if_heads` cannot see the record lane it read. In the synthetic store, after the record lane moved from E4 to E5, a decision citing E4 was still accepted, and only the order of `seq` showed that its basis was stale.

- **From M4: record and detect.** The basis carries the watermark and the heads, and `P_BASIS_STALE` flags a decision whose lane moved before the decision landed.
- **Later option: a precondition.** An optional check of other lanes' heads, made in the same transaction as the append, chosen per decision schema so that busy capture cannot starve approvals. It is decided in M6's design, and only for actions that write nothing to the system of record. The assert intake's request shape leaves a place for other lanes' heads, so adding the check then changes no shape ([decision 6](#6-a-basis-that-moved-on)).
- **Actions that change a record.** For an action that changes a record in the system of record, the check that matters is where the change lands. M6 requests carry the base version, M7 delivers with the system of record's row version, and a stale result is held, never merged. Record lanes trail the system of record by the capture lag, so no check in the store can catch an edit that capture has not yet seen. A store precondition helps only actions that write nothing to the system of record.

### Idempotency

- **Same bytes on retry.** The tool builds the whole object once, with `at` fixed, and keeps it durably before the first send. A retry resends the same bytes, and the store answers `duplicate: true`.
- **One decision per lane position.** Decisions of one schema about one subject chain in their lane. Each lists the previous decision in `prev` and is appended with `if_heads` set to the lane's heads as the tool last saw them; the first uses `if_heads` none. A retry that was rebuilt (new `at`, so a new id) is then refused with `S_STALE_HEADS`, and the heads returned name the decision that already landed. The tool reads that decision back instead of recording a second one. The cost is that decisions of one schema about one subject are serialised.
- **Unchained decisions pile up.** They become extra heads of their assert lane, and `counts.forks` counts only record lanes, so no report shows them.
- **From M6,** fold this into the change-key de-duplication.

### Completeness and freshness

- **Valid is not complete** (step 2). Against a local store today, `evidence store heads` reads a projection at the current head. It has no "as of" option, and the projection is trustworthy only after a whole-log verify. A reader should fold log entries 1 to N itself, or read the heads inside the same snapshot it verified, and record the watermark it used. M4's index should offer heads as of a watermark.
- **Remote readers and bundles.** An M4 bundle (objects, attestations, entries, a covering checkpoint and inclusion proofs) proves membership, not completeness. To prove "these are all the heads at N" without shipping the whole prefix, store/0 needs a commitment it does not have: checkpoint content is a closed set of members, and entries carry no subject or lane. That is a store/0 format change, and the choice between the two constructions in [decision 10](#10-proving-completeness-to-a-remote-reader) is deferred to M4's bundle design. Until then, a reader that does not hold the whole log reports *membership verified, completeness unknown*, and M4 bundles and EG2 exports carry no completeness proof. Shipping the log prefix up to a checkpoint, or holding a local store, is the way to get completeness until then.
- **Freshness.** Nothing tells a reader that its checkpoint is the latest. Entries after the latest checkpoint, the decision itself included, are covered by no signature until the store writes another, and only the store can write one. The basis records the watermark so that an inclusion proof in a later checkpoint can confirm it. Checkpoints are written on a timer and after each batch that others will rely on ([decision 11](#11-checkpoint-cadence)).

## Design rules

1. **A read receipt names what was read.** A read audit that holds no keys or ids cannot say which version of which record was read. The read receipt stays tool-local and value-free, but names the checkpoint, the heads and the ids.
2. **For record changes, the version check is where the change lands.** A write carries the version it was based on, and the system of record refuses it if the record has moved; the stale change is held, never merged. Re-reading and then writing could overwrite a person's edit made in between. So for record changes the check stays at the system of record (M6, M7).
3. **Persist first.** A record written before the action (an outbox) gives a complete trail. Mirrors, copies sent without confirmation, recording that a caller can switch off, and failures that are only logged do not. Persist the basis assert before acting, and publish the policy as data.
4. **Checks have no side effects.** Whatever lets a tool emit an assert changes no state, fails closed, is bound to the exact subject, lane and schema, and checks content by hash rather than by presence.
5. **A basis names immutable things.** A key whose value is later overwritten, or a name and a range of positions, cannot say which version was read. The basis names content-addressed ids, the heads of each lane read and a covering checkpoint.
6. **A tool cannot rewrite its basis.** The reader writes the basis from what it verified. A model's own citations are kept apart and checked against it, no limit silently drops verified items, and the exact model and the prompt hash are recorded.
7. **What was withheld is recorded.** `left_out` and `truncated` in the basis, at the level of scopes only.
8. **Who acted is kept apart from who is accountable.** An agent's assert names `on_behalf_of`, a tool acts for a person only under a grant in force, and approval is the accountable person's attestation ([approvals.md](approvals.md)).
9. **Reads never write.** Numbering that depends on the order of reads cannot be reproduced. The reader writes nothing, and an unknown facet is a finding.
10. **Tool output is not observation.** A tool's earlier output never comes back as if it were an observed record. Derived values come from record lanes only. A basis that includes a tool's assert (one with `on_behalf_of`) says so, and a checker can flag reliance on ones that their `on_behalf_of` has not approved.
11. **Continuity is explicit.** M4 memory reads are verified queries over the log, with each item labelled `observed`, `stated`, `tool-stated` or `note`, and nothing labelled `observed` unless capture wrote it ([approvals.md](approvals.md#8-hooks-for-tools-proposed)). They are not recall of a tool's earlier turns, and M4 serves no recall endpoint. Continuity between turns, where a tool needs it, is explicit notes under a recorded grant, built at M6 on the assert intake once M5's erasure exists: a forgotten version stops recall, and M5 erasure deletes the body ([decision 13](#13-continuity-for-tools)).
12. **A decision stays with its basis.** A decision goes in the same store as its basis, separated by lane, whatever kind of actor wrote it. A basis in another store names that store's log and checkpoint.
13. **A decision carries its rules and its inputs' versions.** It carries its rule set by hash, the profile and dictionary id of any derived values, the clock and the basis.
14. **Two clocks.** Staleness is judged on `seq` and `received_at`, not on writers' claimed times ([D13](decisions.md#d13-two-clocks)).

## Open decisions

Settled on 28 November 2026. Each item keeps its options. Its status line gives the decision, or, where a party outside the project decides, says who decides and what waits for it.

### 1. Where the basis goes

- Options: (a) in `content`, unchecked; (b) in `prev`; (c) in one `ext` profile; (d) the heads in `prev` and the full basis in `ext`.
- **Decided (28 November 2026):** (c). `prev` is kept for what the decision replaces, and domain inputs (rule set, prompt, photos) stay in `content`, as vector 003 does.
- Why: (b) stretches "ids this object replaces" to mean "based on", risks status and history views reading records as superseded by decisions, and cannot cite a basis held in another store or a bundle. (d) would make store/0 say that a `prev` into another lane means "based on", a change to store/0 after M1 is built.

### 2. The basis profile

- Options: (a) adopt the sketch above, name it, and give it vectors and a JavaScript reference ([D7](decisions.md#d7-the-vectors-are-the-contract)); (b) leave the shape to each tool.
- **Decided (28 November 2026):** (a), at M4, under the name `read-basis/0`, which is fixed before any EG1 vector cites it. With it ([The basis profile](#the-basis-profile)):
  - The `left_out` reason codes are a closed list: `not_permitted`, `out_of_scope` and `rejected`.
  - A whole scope read is named in an optional `scopes[]` member, parallel to `left_out`.
  - A cited id that is not in the basis is a named finding, `P_BASIS_CITED_NOT_READ`.
  - Derived values appear only where the lane's profile defines them (step 2), each in coordinate/0's tagged form under its coordinate name ([decision 9](#9-kernel2-rules-the-reader-depends-on)).
  - The read receipt gets a JSON Schema built from the basis's `as_of` and `heads` blocks, with no JavaScript reference, because no verification depends on it.
- Why: third parties must recompute it, and only one fixed shape keeps every tool's recorded basis comparable. The name is the key under `ext`, so renaming it after vectors exist would change their ids. Design rule 6 says a model's citations are checked against the basis, and a check needs a code to report. An open list of reasons would let two tools record the same withheld scope differently.

### 3. The decision's subject

- Options: (a) the subject decided about, with decisions of one schema chained in their lane and appended with `if_heads`; (b) the decision's own record in a case system, where one exists; (c) a separate subject per decision, like M6's `evidence-sync/intent/<id>`.
- **Decided (28 November 2026):** (a) by default, and (b) where a case record already exists.
- Why: (a) puts the decision in the subject's history and state views and gives idempotency through `if_heads`. (c) takes the decision out of those views, and the de-duplication it offers is also available under (a).

### 4. When a basis assert is required

- Options: (a) the tool owner decides case by case; (b) a published list of action types that require one, enforced fail-closed; (c) every read.
- **Decided (28 November 2026):** (b). The owner of each tool picks and publishes the list for that tool, as an `assert` like M6's `register/0`. The deployment's approval rules ([approvals.md](approvals.md#52-approval-rules)) may cite the list; they do not choose it.
  - "Must" for listed action types, and "may" otherwise.
  - The default list, which also applies to a tool whose owner publishes none, is every action on a subject that relies on an agent's or a service's output, such as a recommendation or a check result.
- Why: it keeps this a matter of policy, not architecture, while making the policy itself checkable, and the decision stays with the tool owner. The fallback keeps "fail-closed" meaningful for a tool whose owner publishes nothing, as missing policy fails closed in [approvals.md](approvals.md#2-principles).

### 5. Failed verifications

- Options: (a) a finding in the read receipt only; (b) always an `assert`; (c) an `assert` when the failure blocks or changes an action.
- **Decided (28 November 2026):** (c). Store-wide integrity stays with `store verify` and M3. A failed read that changes an action is recorded in that action's basis, as a `left_out` scope with reason `rejected` or as a check result code. Which failures change an action follows the tool owner's list ([decision 4](#4-when-a-basis-assert-is-required)).
- Why: a failure that changes an action is a basis for it. Recording every failure would let a noisy source flood the log.

### 6. A basis that moved on

- Options: (a) record and detect, with `P_BASIS_STALE`; (b) an optional same-transaction check of other lanes' heads, chosen per decision schema; (c) both.
- **Decided (28 November 2026):** (a), from M4. Whether to add (b) is decided in M6's design, and only for actions that write nothing to the system of record. The assert intake's request shape leaves a place for other lanes' heads, so adding (b) then changes no shape.
- Why: for record changes the system of record's own version check is the one that works.

### 7. An intake for asserts

- Options: (a) only tools inside the writer process can record asserts until M6; (b) an assert intake late in M4; (c) one in M6 beside `POST /intents`.
- **Decided (28 November 2026):** (b), or early M6 at the latest. One request is one batch: the object and its attestation lines, with optional `if_heads`, write scopes per lane, and store receipts in the answer. The intake also runs the pure pre-append checks of [approvals.md](approvals.md): the actor id form, an `author` signer equal to `actor.id`, the kind of `on_behalf_of`, the record-lane writer, and grants. It refuses with every failed `P_` code, not only the first. Verification repeats the same checks, because a verifier never relies on intake.
- **Decided (28 November 2026):** what the intake refuses and what it records:
  - The intake refuses a malformed identity (`P_ACTOR_ID_FORM`).
  - It refuses a write that fails an authority check that applies to it (`P_AUTHOR_NOT_ACTOR`, `P_SPONSOR_NOT_ACCOUNTABLE`, `P_RECORD_LANE_WRITER`, `P_GRANT_*`) or an integrity check (a signature, hash or link that does not verify), naming every failed `P_` code; a failed integrity check gives the single `E_` code of store/0's intake.
  - Doubts about content quality or meaning are recorded as findings, not refusals: a value out of an expected range, records that disagree, a missing optional field, a doubtful reading.
  - Verification repeats every check either way.
- Why: M2 mapping asserts and M3 resolutions need the same path, and the command-line tool cannot send an object and its approval together. A check made before the append matters because the log cannot forget: an id that carries personal data, or a write made without authority, stays in it for good, while a doubt about content can be recorded and answered by a later entry. A refused write leaves no entry, so a tool that persists first does not act.

### 8. Who may approve

- Options: (a) any valid `approve` attestation, as the core checks today; (b) a profile rule on whose approval counts; (c) wait for M5 key binding.
- **Decided (28 November 2026):** (b), extended as below, now, and (c) at M5. Approvals made before M5 are labelled as not accountability evidence. The design is in [approvals.md](approvals.md).
  - An `approve` counts toward an approval rule only when it comes from a person listed for the role (in their own name or for an organisation they are listed as acting for) in the approver list in force at the line's log position, or from the object's `on_behalf_of`. A person acting for an organisation signs under their own id.
  - Agents, services and `org:` ids never fill an approval step. Agents and services sign `author` and `witness`.
  - `on_behalf_of` must be a `person:` or `org:` id (`P_SPONSOR_NOT_ACCOUNTABLE`). An `org:` one counts only under a grant approved by a person listed as acting for that organisation.
  - An `author` line's signer must equal `actor.id` (`P_AUTHOR_NOT_ACTOR`).
  - Until M5, a key seen under two signer names counts for neither (`P_KEY_SHARED`).
- Why: today an agent can approve its own recommendation and the core accepts it. Published lists and rules let anyone holding the log check whose approval counted.
- **Decided (28 November 2026):** agents and services never fill an approval step or settle a disagreement, and a tool's output counts as approved only through its `on_behalf_of`. This is a rule of the `approval/0` profile, not of the core.

### 9. Kernel/2 rules the reader depends on

Kernel/2 states these as its own profile rules ([Kernel/2 section 8](kernel-2.md#8-open-questions)). They are listed here because the reader depends on them.

- **Lane of events.** Options: the record lane, with events as lifecycle changes that capture observes in the registry; assert lanes under one schema family; either. **Decided (28 November 2026):** the record lane, so that one fold and store/0's fork count both cover the whole chain. Only capture writes a record lane, so Kernel/2 facets come from capture's mapping (M2's `mapping/0`) or a capture baseline.
- **Order at a merge.** Options: log order (`seq`); `at` order; a topological order with ties broken by (`at`, id); S defined only on linear chains, with a merge event restating the state. **Decided (28 November 2026):** topological order with (`at`, id) ties, because it depends on the objects alone and not on one store's positions. The gates run per event over that event's own ancestors, and Kernel/2 adds merge vectors at M9.
- **Order within an event.** Options: exits first; exits last; forbid an exit and an entry of the same block in one event. **Decided (28 November 2026):** forbid it, and give events a constrained `remove` list for changing a state ([Kernel/2 section 4](kernel-2.md#4-rules-verifier-gates)). An event that exits a block and names any other facet of it (its entry, or one of its states in `add` or `remove`), or names one facet in both `add` and `remove`, is invalid ([Kernel/2 REQ-KRN-021 and REQ-KRN-022](kernel-2.md#4-rules-verifier-gates)). `remove` names only state facets of a block the unit has entered and not exited, never an entry or exit facet. It applies before `add` and changes S only, never H. Exits-first is not adopted. An exit needs the block entered by an earlier event (REQ-KRN-009 as written).
- **Re-adding a held facet.** Options: S is a set, so the re-add changes nothing in S and is still counted in H; the event is refused; S counts repeats, as H does. **Decided (28 November 2026):** S is a set.
- **Retirement.** Options: `registry.executed` only; also `universal.exit`. **Decided (28 November 2026):** `registry.executed` only. Universal state that stays true of a retired unit stays in S, and no value in the worked table changes.
- **Scope.** Options: Kernel/2 also covers installations and devices; they get their own dictionary and profile tag. **Decided (28 November 2026):** their own coordinate dictionary and profile tag, with Kernel/2 kept for certificate lifecycles, on a synthetic dictionary through M4b. **Gated:** an external publisher and a first consumer decide when a real dictionary ships, before M9. Until then, coordinates for installations and devices are synthetic only, and none is computed for a real record.
- **Size.** Decided (27 November 2026): decimal strings. The acceptance test, now M4b's (M9's until 28 November 2026), and the README were amended to match.
- **Naming the dictionary.** Options: a profile tag only; coordinate/0's tagged form per value, under its coordinate name; a draft marker until M9; no derived values in a basis until the profile ships. **Decided (28 November 2026):** coordinate/0's tagged form (`profile`, `dictionary`, `coordinate`) per value, under its coordinate name. The `read-basis/0` vectors at M4 use a synthetic coordinate/0 profile, not Kernel/2 values, because Kernel/2 is not built before M9.
- **Where Kernel/2 is built.** Options: at M9, listed as one of its deliverables; with coordinate/0 or M4b; unscheduled until a consumer for certificate lifecycles exists. **Decided (28 November 2026):** at M9, listed among M9's deliverables in the [roadmap](roadmap.md#m9-optional-centr0-profile-register-views-and-a-published-dictionary).
- **Not coordinates.** Recorded here for any derived index: actor ids and approval status are never coordinates. A coordinate is decodable by design, so an actor id in one would publish who acted on what. An approval arrives after the object it approves, so approval status is a view at a log position ([approvals.md](approvals.md)), not a facet of the object.

### 10. Proving completeness to a remote reader

- Options: (a) ship the log prefix up to the checkpoint; (b) a commitment to all heads in a new checkpoint version, for example a sparse Merkle tree keyed by a digest of subject and lane; (c) a keyed digest of subject and lane in a new log-entry version, so a reader can scan entries against the root without seeing other subjects' objects; (d) the M4 service signs a completeness assert.
- **Decided (28 November 2026):** the choice between (b) and (c) is deferred, deliberately, to M4's bundle design. Either is a store/0 version change; evidence/0 is unchanged. Until then, a reader that does not hold the whole log reports *membership verified, completeness unknown*, and M4 bundles and EG2 exports carry no completeness proof. Shipping the log prefix up to a checkpoint (a), or holding a local store, is the way to get completeness until then.
- Why: (b) is one commitment per checkpoint that `store verify` can recheck, but it is a store-signed claim that only the full log can audit. (c) is provable from the root but costs a scan. (d) is the weakest: attested, not proven.

### 11. Checkpoint cadence

- Options: (a) on a timer; (b) after each batch that others will rely on; (c) on request from a tool, through the service.
- **Decided (28 November 2026):** (a) and (b), written by the store or the M4 service. External publication of checkpoints comes with M5. Each deployment sets the timer's interval as published data; this note proposes none.
- Why: a decision becomes checkable outside the store only once a signed checkpoint covers it. (c) would add a write-like endpoint to a read-only API.

### 12. Privacy of actor ids

- Options: (a) no rule; (b) `actor.id`, `on_behalf_of` and attestation signers are pseudonymous, resolved to people only through an access-controlled directory or M5 key bindings, or, for keyed record references on an approver list, through the data owner's link-domain key and system of record.
- **Decided (28 November 2026):** (b), spelled out as below. evidence/0 stays frozen. The design is in [approvals.md](approvals.md).
  - `person:` and `agent:` ids are 128 random bits written as 32 hex digits, assigned at enrolment. `service:` and `org:` ids may be public names, but a gateway or other service at someone's premises, a sole trader, or an organisation named after a person gets an opaque id.
  - An id is never derived from personal data, and never an unkeyed hash of it.
  - One pseudonym per party per directory.
  - The id is not the key. M5 key-binding objects bind keys to ids, with validity and revocation.
  - Who can resolve an id depends on access; the full table is in [approvals.md section 3](approvals.md#3-parties-and-ids-proposed). By default the public sees no pseudonyms: a deployment may publish the roles on an approver list, not who holds them. Parties with a status-only scope see status views that leave out signers. Parties entitled to see approval history also see approval lines, under M4's closed-by-default read scopes. A data owner can recognise its own users on an approver list that carries `record_refs`. An auditor also sees key bindings and grants. Only the body that keeps the directory sees the directory that maps pseudonyms to people. A deployment's configuration names that body; this note names none. The directory is access-controlled and kept off the log.
  - An actor id is never a coordinate ([decision 9](#9-kernel2-rules-the-reader-depends-on)).
  - The assert intake refuses an id that breaks the form (`P_ACTOR_ID_FORM`, [decision 7](#7-an-intake-for-asserts)), and verification checks it again.
  - store/0 Privacy gains one informative bullet: `actor.id`, `on_behalf_of` and attestation signers are never derived from personal data, and ids for people and agents are pseudonyms. It adds no check, code or vector; the check stays in the profile.
- Why: today's privacy rules cover only inline content and guessable hashes, while actor ids are written into the log forever. One pseudonym per party lets a check count each party once.
- **Gated:** the M5 privacy and legal review decides, before M5, whether ids stay random at enrolment or become a keyed digest held by the directory. Until then, random at enrolment, as above.
- **Gated:** each deployment decides which body keeps its directory, with the M5 legal review, before M5 and any real deployment. Until then, a deployment's configuration names that body, this note names none, and every id is synthetic.

<a id="13-agent-memory"></a>

### 13. Continuity for tools

- Options: (a) M4 also serves a tool's recall of its own earlier turns; (b) out of scope for M4: continuity between turns is explicit asserts under a recorded grant.
- **Decided (28 November 2026):** (b), with these definitions. The design is in [approvals.md](approvals.md).
  - **The recorded grant** is an `approval/grant@0` assert by the accountable person, approved by them. It names the tool, what it may write and read, which purposes it may sign, a notes subject, a `recall_until` time and a validity period. Its periods are judged on the store's clock (`received_at`), never on the tool's own `at`. Changing or revoking a grant is a new version.
  - **Continuity notes** are asserts by reference in a dedicated notes system, under the grant. The hash is in the log; the body, with a random salt, is in the erasable blob store (M2). The reader labels them `note`, never `observed`, and returns them to the grantee only while the grant is in force and its recall window is open on the store's clock, and to the grantor at any time. A new version marked forgotten stops recall, and M5 erasure deletes the body. Revoking a grant ends the grantee's recall, and either party to the grant may record the forgotten version.
  - **Timing.** Continuity notes and their recall are built at M6, on the assert intake, once M5's erasure exists. M4 serves no recall endpoint, and no ranking or similarity search, and does not become a general memory service. The grant already carries the notes subject and `recall_until`, so EG1 is not held up, and notes stay in scope through the hooks.
- Why: memory reads are verified queries over the log, and nothing comes back as `observed` unless capture wrote it. A tool's own earlier output must not come back as if it were observed ([design rule 10](#design-rules)). Note bodies are written only once they can be erased.
- **Decided (28 November 2026):** by default a grant's `recall_until` is no later than its `valid_until`, as in the approvals.md example grant.
- **Decided (28 November 2026):** a tool files an M6 request only when a person starts it, for a change that person could request themselves, with that person in `on_behalf_of`. Delivery stays M7's separate decision.
- **Gated:** the M5 legal review decides what a note body may hold, before notes are built at M6. Until then, ids, codes and hashes only: no names, contact details, record values or free text copied from records.
- **Gated:** the data owner and each deployment's records authority, with the M5 legal review, decide how long note bodies are kept and who authorises their erasure, before notes hold real data. Until then, no retention period is set, and every note is synthetic.
