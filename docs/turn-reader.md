# The turn-by-turn reader (design note)

**Status:** design note, 27 November 2026, revised after review. Design input for [M4](roadmap.md#m4-memory-reads-index-query-api-bundles-read-only-service) (memory reads), [M6](roadmap.md#m6-requests-and-the-outbox-as-evidence-in-shadow-mode-no-write-code) (requests) and [M9](roadmap.md#m9-optional-centr0-profile-register-views-and-prime-coordinates) (prime coordinates). Open decisions 8, 12 and 13 have a recommended design in [approvals.md](approvals.md). Nothing here is built. The examples are synthetic, and each evidence object in them verifies with the `evidence` command-line tool. Not an official publication of any agency; it describes no agency's process (see the [README](../README.md) notice).

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
- **Whether a given read needs an assert is policy,** not a mechanical consequence of the architecture ([open decision 4](#4-when-a-basis-assert-is-required)).
- **A basis is structured, not free text:** log, checkpoint, watermark, clock, subject, lane and heads, so that anyone can re-run it. Every query says which clock it uses ([roadmap principles](roadmap.md#principles)).
- **Every time is in UTC with `Z`,** and labelled with whose clock it is.
- **Approval is a signature, not a content field.** Content is the writer's claim. Approval is an `approve` attestation by the accountable person ([evidence/0 Attestations](../spec/evidence-0.md#attestations), [D4](decisions.md#d4-signatures-outside-the-object)).
- **Tools do not decide.** A tool's output authorises nothing by itself. It counts only once the accountable person signs `approve` on it ([open decision 8](#8-who-may-approve), [approvals.md](approvals.md)).
- **Derived values are never authoritative.** They are recomputed from the chain, never trusted from an index ([README](../README.md#proposed-prime-coordinates-for-faceted-queries): "derived, never authoritative").

## What the reader does

### The four steps

1. **Verify.** Run evidence/0 steps 1-5 and check the attestations, with the purposes the caller requires ([evidence/0 Verification](../spec/evidence-0.md#verification)). For an object read from a store, also check an inclusion proof against a checkpoint whose witness verifies under the genesis key ([store/0](../spec/store-0.md#checkpoint-and-tree)).
2. **Establish the lane as of that checkpoint.**
   - Find `heads(subject, lane)` at the checkpoint's size N.
   - Take the same-lane `prev` closure from those heads. Every object in a lane is an ancestor of one of its heads, or a head itself, so these two steps give exactly the lane as of N.
   - If the lane's profile defines a derived index, compute it for each head from the chain. For the Kernel/2 draft, that is S and H, as decimal strings tagged `kernel/2`.
   - If the reader cannot establish the heads at N (for example, it was handed some objects and their inclusion proofs), it reports *membership verified, completeness unknown*. It may give derived values for a named head, but never as the subject's state.
3. **Emit a read receipt** to the tool's own log ([below](#the-read-receipt)).
4. **If the read is the basis for an action, record it before acting.** The acting tool, not the reader, appends an `assert` that carries the basis ([below](#reads-that-become-evidence)). Which actions need one is policy ([open decision 4](#4-when-a-basis-assert-is-required)).

Step 2 replaces a weaker completeness test, "the lane is closed under `prev` back to its first event". That test finds a missing middle event but never a missing tip. A lane cut short after E3 passes every check and reports the lane as of E3, where the true state is as of E5.

The reader itself never writes: not to the store and not to any dictionary. A facet that is not in the profile's dictionary is a finding, never a new allocation.

### The rule

| Read | Recorded in the store? | Where | Why |
|---|---|---|---|
| Routine verification of an object or a lane | No. A read receipt only | The tool's own log | Reads add no entries; appending them would flood the log |
| A verification that fails | A finding in the read receipt. Whether it also becomes an `assert` is [open decision 5](#5-failed-verifications) | The tool's log, perhaps also the store | It may be a sign of tampering, and M3 records every reconciliation result |
| A read that is the basis for an agent's or a service's output that an action relies on | Yes: an `assert` carrying the basis, appended before the action | The store, in the subject's `assert:<schema>` lane | "What the model was given, what it returned, and who approved or overrode it" ([README](../README.md#what-it-is-for)) |
| A person's approval of that output | Yes: an `approve` attestation on the assert | The store, as an attestation entry | Approval is the accountable person's signature, not a content field |
| A person's own decision, or an override | Yes: an `assert` by a human actor, with an `approve` attestation | The store | The resolution pattern of [M3](roadmap.md#m3-two-way-reconciliation-report-only-recorded-as-evidence) |
| A register snapshot (a query result) | Yes: an `assert` whose content is the result at a recorded watermark (M9). An M4 bundle can also be sealed as an assert | The store | "What did the list show on date X?" gets a provable answer |
| A derived index (such as S and H), if the lane's profile defines one | Never as an authority. Recomputed on every read | Tool memory; a derived index column only from M4b or M9 | Derived values are never authoritative |

## The read receipt

### Shape

| Member | Meaning |
|---|---|
| `v` | `"read-receipt/0"`. A working name, so the shape can change. |
| `ok` | Whether every check passed. |
| `verified_at`, `verified_at_clock` | When the tool verified, in UTC with `Z`, and whose clock that is (`"tool"`). It is not the store's `received_at`. |
| `subject`, `lane` | The evidence/0 subject triple and the lane read. Heads are kept per subject key and lane. |
| `as_of` | `clock`, `log`, `checkpoint` (`id`, `size`, `root`) and `watermark` (`seq`, `entry_hash`). Heads, and any derived values, are true only relative to a point in a particular log. |
| `complete` | Whether step 2 established the heads at the checkpoint, or only membership. |
| `heads` | One entry per head, because a forked lane has several. Each has its `id`, and, if the lane's profile defines a derived index, the values with their profile tag: here `profile`, `S` and `H`, as decimal strings ([Kernel/2 3.3](kernel-2.md#33-representation-declared)). H answers "was it ever in state X?". |
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

A cheap check across turns: heads only move forward in store/0, so each head seen at turn n is an ancestor of a head at turn n+1, or still a head. Where the lane's profile defines H, H at the earlier head must therefore divide H at the later one. If it does not, the lane was rewritten or the reader was given a different log. A consistency proof between the two checkpoints is the stronger check, and it needs no derived index.

## Lane state

This section applies only when a lane's profile defines a derived index. The worked example uses the [Kernel/2 draft](kernel-2.md), which is planned for M9 and not built. [Open decision 9](#9-kernel2-rules-the-reader-depends-on) recommends a separate dictionary for installations and devices. A lane with no such profile has heads and ids, and no S or H.

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

The reader cannot be specified fully for a Kernel/2 lane until Kernel/2 settles these ([open decision 9](#9-kernel2-rules-the-reader-depends-on)):

- **Which lane its events use.** Kernel/2 names no `op`. As record changes, its events share the record lane, as above. As asserts, they land in `assert:<schema>` lanes. A chain that alternates between the two shows up in store/0 as a record-lane fork, and a fork inside an assert lane is not counted at all.
- **Order within one event.** Whether exits apply before entries changes S when one event exits and re-enters a block, which is how [Kernel/2 section 4](kernel-2.md#4-rules-verifier-gates) suggests changing a state.
- **Scope.** Kernel/2 is designed with certificate lifecycles in mind. Whether it, or a separate dictionary, applies to installations and devices is undecided.
- **Size.** Decided (27 November 2026): coordinates are decimal strings, and M9's acceptance test and the README now say so.
- **Draft status.** Kernel/2's own open questions (a `remove` list, `universal.exit` at retirement) change S. Until it ships at M9, coordinates are computed against a draft, and a receipt or basis that keeps them should say which.

## Reads that become evidence

### The pattern

[Vector 003](../test-vectors/objects/003-agent-assertion-approved/) already shows how this repository records an AI-assisted check (its ids are readable synthetic names; under [open decision 12](#12-privacy-of-actor-ids) they would be opaque). The agent's `assert` holds what the model was given (by hash) and what it returned. `actor.kind` is `agent`, with `on_behalf_of` naming the accountable person. That person's acceptance is an `approve` attestation. This note adds one thing to the pattern: the reads from the store that the agent's output relied on.

- **Domain inputs stay in `content`,** as in vector 003: the rule set and prompt by hash, the model, the result, and the objects the model itself cited.
- **The store-read basis goes in one fixed `ext` profile,** working name `read-basis/0`. It names the log, the checkpoint, the watermark and clock, and for each subject read, its lane, its heads and, where the lane's profile defines them, their derived values ([open decisions 1 and 2](#open-decisions)).
- **`prev` keeps its meaning,** "ids this object replaces": for example, the previous decision in the same lane ([open decision 3](#3-the-decisions-subject)).
- **Approval is an attestation.** The accountable person signs `approve` on the assert's id.
- **A refusal or an override is a new `assert` by a human actor,** with the disposition in its content and an `approve` attestation. If it is in the same lane as the agent's assert, it lists that assert in `prev`, because it replaces it.

### Example: a tool's completeness check and a person's approval

A tool acting for a person read the unit's record lane at the checkpoint covering E1 to E4 (head E4 in the table above). It checked that the record is complete under a synthetic rule set: the events name the blocks the rule set requires, and each event after the first lists the previous one in `prev`. The person it acts for approved its output. This is illustrative only: no scheme decision is automated, and a person decides. All data is synthetic.

```json
{
  "v": "evidence/0",
  "id": "sha256:3f7b450ed9bb2c1d6b474adbeeed2d15e5ec29adabf7024931c08b9c7fa82d09",
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

The two attestation lines kept beside it ([D4](decisions.md#d4-signatures-outside-the-object)): the tool's `author` line and the accountable person's `approve` line.

```json
{"at":"2026-11-27T01:25:00Z","key":"ed25519:wvPQephfnk4JouEcctsGTl3ljHqGz_6Z-PKEgWgVzFQ","object":"sha256:3f7b450ed9bb2c1d6b474adbeeed2d15e5ec29adabf7024931c08b9c7fa82d09","purpose":"author","sig":"DNH0EgZyFx7MNh8o4AzEb1RsxdiFDcEZau7u2Or0H3b_sB2fsoUffkJWsGb23VBUsxjEJBtsYyZW7s9FeJwWDg","signer":"agent:e7cc8fb8c7b86ecc223efea79fe7547f","v":"attestation/0"}
{"at":"2026-11-27T01:27:00Z","key":"ed25519:Bfuw65jk1PVIQjgnSDaNvwol0vGDLMQ9-di57qco334","object":"sha256:3f7b450ed9bb2c1d6b474adbeeed2d15e5ec29adabf7024931c08b9c7fa82d09","purpose":"approve","sig":"U6zDqv_yQlPXFP2T7uKBLLXjwjGlbr45GiKi-qL7hnjXH0YXOjeZWzh68z3fdjPS0IK5NUonyXLbj9M5e6dXBQ","signer":"person:f10c170c61109ad83fa7b99231f23fa0","v":"attestation/0"}
```

```sh
evidence verify check.json --attestations check.jsonl --require author --require approve
# {"attestations":{"approve":1,"author":1},"content_checked":true,"id":"sha256:3f7b450e…","ok":true}

evidence verify check.json --attestations author-only.jsonl --require approve
# {"attestations":{"author":1},"code":"E_ATTESTATION_MISSING",…,"ok":false}   exit status 1
```

What the example shows:

- **Subject.** The subject is the unit checked, so the check result appears in the unit's history and in M4's "latest assert per schema".
- **Lane.** The check result lands in lane `assert:example-case/completeness-check`. Appending it to a store/0 log left the unit's record-lane heads at E4.
- **Chaining.** It is the first check result in its lane, so `prev` is empty and it was appended with `if_heads` set to none.
- **Citations.** `content.cited` is the model's own list of what it relied on. The basis is what the reader verified. The two are kept apart, and a checker can require every cited id to be in the basis.
- **Derived values.** This lane's profile defines the Kernel/2 coordinates, so the basis carries S and H as strings tagged `kernel/2`. Here S and H differ, because E4 exited the measurement block. In a lane with no derived index, each head carries its id only.
- **Read receipt.** `receipt` names the read receipt above by its content hash. It is optional.
- **Privacy.** The tool and the person are pseudonymous ids of the form [open decision 12](#12-privacy-of-actor-ids) recommends, and nothing personal is inline.
- **Grant.** Under the design in [approvals.md](approvals.md), a tool that acts for a person also cites the grant it acts under ([open decision 13](#13-continuity-for-tools)). This example leaves the grant out, to keep to the read basis.
- **Checkability.** The ids under `ext` point into a throwaway synthetic store. They become checkable by others once the example is published as a vector together with its log.

### Where the basis goes

| Placement | What checks it | Cost |
|---|---|---|
| In `content` | Nothing but the content hash. An id that nobody holds is accepted. | Unchecked, and every tool invents its own shape |
| In `prev` | Intake: every id must be held and not dated after the decision (`E_PREV_MISSING`, `E_PREV_ORDER`). A `prev` into another lane leaves heads alone. | evidence/0 defines `prev` as "ids this object replaces", and status is a view over `prev` links, so a naive status view would read E4 as superseded by the decision. A basis held in another store or in a bundle fails `E_PREV_MISSING`. M4's history view would list every decision as a descendant of the records it read. |
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
| `reads[]` | For each subject read: `subject`, `lane`, `complete`, and `heads[]` with `id`, plus `profile` and the derived values (here `S` and `H`) where the lane's profile defines them |
| `left_out[]` | Scopes the reader did not or could not read, each with a reason code such as `not_permitted`, `out_of_scope` or `rejected`. A scope is a system or a lane. Never list refused subject keys: that would reveal that they exist |
| `truncated` | `true` when a limit cut the read short (M4 computes it by fetching one more than the limit) |
| `receipt` | Optional: the hash of the read receipt |

Proposed checks for its verifier, all computable from the log:

| Code (proposed) | Meaning |
|---|---|
| `P_BASIS_CHECKPOINT_UNKNOWN` | The cited checkpoint (`id`, `size`, `root`) is not one of this log's checkpoints. |
| `P_BASIS_HEADS_MISMATCH` | The claimed heads are not `heads(subject, lane)` at the watermark. The reader missed something that was there. |
| `P_BASIS_STALE` | Informational. The lane moved between the watermark and the assert's own `seq`. |
| `P_BASIS_COORDINATE_DRIFT` | A claimed derived value differs from the recompute (for Kernel/2, REQ-KRN-013). At a merge, H only, until [open decision 9](#9-kernel2-rules-the-reader-depends-on) fixes S. |

A prototype of these checks, run over the synthetic log, found nothing on the example check. It flagged `P_BASIS_STALE` on a later revision that cited the same head E4 after E5 had landed.

### What an approval does and does not prove

- **The core does not check who signed.** It checks that a valid `approve` attestation exists, not who signed it. An `approve` signed with the tool's own key, under the tool's own name, also verifies with `ok: true`. [approvals.md](approvals.md) proposes profile rules and an approval-status view under which an approval counts only under published rules; see [open decision 8](#8-who-may-approve).
- **Before key binding (M5), an attestation proves only that some key holder signed** ([evidence/0 Not in v0](../spec/evidence-0.md#not-in-v0), [roadmap decision 4](roadmap.md#decisions-confirmed-26-november-2026)). So before M5, anyone with the log can check *what informed the decision*: the basis ids, the checkpoint and the heads. Nobody can yet check *who decided*. Approvals made with development keys are not accountability evidence, and the proposed approval-status view says so on every status (`"accountability": "not-established-before-key-binding"`).

## Store mechanics

### Who emits

- **The acting tool.** The reader never writes. The tool that takes the action appends the assert.
- **Persist before acting.** Where policy says an action needs a basis assert, the action goes ahead only after the store has returned an append receipt for it. Failing to record means not acting.
- **Policy as data.** The owner of each tool publishes the list of action types that need a basis assert for that tool. The list is published data: an `assert` like M6's `register/0`, which the deployment's approval rules can name ([open decision 4](#4-when-a-basis-assert-is-required)). It is not a per-request flag or a deployment switch.
- **The channel does not exist yet.** Appending needs the store's single-writer lease; without it the store answers `S_LOCKED`. From M4 the service holds the lease and its API is read-only. M6 adds intake only for `sync/0` requests. Until there is an intake for asserts, a tool that records asserts must run inside the writer process ([open decision 7](#7-an-intake-for-asserts)). M2's mapping asserts and M3's resolutions have the same gap.

### Signing

- **Who signs what.** The tool signs `author`. The accountable person signs `approve`.
- **One batch.** Send the object and its attestation lines as one batch, so the store never holds a decision without the approval it was sent with. store/0 batches are all or nothing. The library supports them; the command-line tool does not, because it commits each file in its own transaction. Between the two appends, `store get … --require approve` fails with `E_ATTESTATION_MISSING`.
- **Coverage.** Do not rely on a decision outside the store until a checkpoint covers the entries of both the assert and its approval.
- **Order.** A verifier policy may flag an `approve` dated before the object's own `at`.

### The staleness window

`if_heads` compares the heads of the object's own subject and lane ([store/0 Intake](../spec/store-0.md#intake)). A decision lives in `assert:<schema>`, so its `if_heads` cannot see the record lane it read. In the synthetic store, after the record lane moved from E4 to E5, a decision citing E4 was still accepted, and only the order of `seq` showed that its basis was stale.

- **Default: record and detect.** The basis carries the watermark and the heads, and `P_BASIS_STALE` flags a decision whose lane moved before the decision landed.
- **Option: a precondition.** An optional check of other lanes' heads, made in the same transaction as the append, chosen per decision schema so that busy capture cannot starve approvals. Decide in M6 ([open decision 6](#6-a-basis-that-moved-on)).
- **Actions that change a record.** For an action that changes a record in the system of record, the check that matters is where the change lands. M6 requests carry the base version, M7 delivers with the system of record's row version, and a stale result is held, never merged. Record lanes trail the system of record by the capture lag, so no check in the store can catch an edit that capture has not yet seen. A store precondition helps only actions that write nothing to the system of record.

### Idempotency

- **Same bytes on retry.** The tool builds the whole object once, with `at` fixed, and keeps it durably before the first send. A retry resends the same bytes, and the store answers `duplicate: true`.
- **One decision per lane position.** Decisions of one schema about one subject chain in their lane. Each lists the previous decision in `prev` and is appended with `if_heads` set to the lane's heads as the tool last saw them; the first uses `if_heads` none. A retry that was rebuilt (new `at`, so a new id) is then refused with `S_STALE_HEADS`, and the heads returned name the decision that already landed. The tool reads that decision back instead of recording a second one. The cost is that decisions of one schema about one subject are serialised.
- **Unchained decisions pile up.** They become extra heads of their assert lane, and `counts.forks` counts only record lanes, so no report shows them.
- **From M6,** fold this into the change-key de-duplication.

### Completeness and freshness

- **Valid is not complete** (step 2). Against a local store today, `evidence store heads` reads a projection at the current head. It has no "as of" option, and the projection is trustworthy only after a whole-log verify. A reader should fold log entries 1 to N itself, or read the heads inside the same snapshot it verified, and record the watermark it used. M4's index should offer heads as of a watermark.
- **Remote readers and bundles.** An M4 bundle (objects, attestations, entries, a covering checkpoint and inclusion proofs) proves membership, not completeness. To prove "these are all the heads at N" without shipping the whole prefix, store/0 needs a commitment it does not have: checkpoint content is a closed set of members, and entries carry no subject or lane. That is a store/0 format change ([open decision 10](#10-proving-completeness-to-a-remote-reader)).
- **Freshness.** Nothing tells a reader that its checkpoint is the latest. Entries after the latest checkpoint, the decision itself included, are covered by no signature until the store writes another, and only the store can write one. The basis records the watermark so that an inclusion proof in a later checkpoint can confirm it. How often checkpoints are written is [open decision 11](#11-checkpoint-cadence).

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
11. **Continuity is explicit.** M4 memory reads are verified queries over the log, with each item labelled `observed`, `stated`, `tool-stated` or `note`, and nothing labelled `observed` unless capture wrote it ([approvals.md](approvals.md#8-hooks-for-tools-proposed)). They are not recall of a tool's earlier turns. Continuity between turns, where a tool needs it, is explicit notes under a recorded grant (proposed for M6; the timing is open): a forgotten version stops recall, and M5 erasure deletes the body ([open decision 13](#13-continuity-for-tools)).
12. **A decision stays with its basis.** A decision goes in the same store as its basis, separated by lane, whatever kind of actor wrote it. A basis in another store names that store's log and checkpoint.
13. **A decision carries its rules and its inputs' versions.** It carries its rule set by hash, the profile of any derived values, the clock and the basis.
14. **Two clocks.** Staleness is judged on `seq` and `received_at`, not on writers' claimed times ([D13](decisions.md#d13-two-clocks)).

## Open decisions

### 1. Where the basis goes

- Options: (a) in `content`, unchecked; (b) in `prev`; (c) in one `ext` profile; (d) the heads in `prev` and the full basis in `ext`.
- Recommended: (c). Keep `prev` for what the decision replaces, and keep domain inputs (rule set, prompt, photos) in `content` as vector 003 does.
- Why: (b) stretches "ids this object replaces" to mean "based on", risks status and history views reading records as superseded by decisions, and cannot cite a basis held in another store or a bundle. (d) is the fallback if intake checks are wanted before the store knows the profile. store/0 would then have to say that a `prev` into another lane means "based on".

### 2. The basis profile

- Options: (a) adopt the sketch above, name it, and give it vectors and a JavaScript reference ([D7](decisions.md#d7-the-vectors-are-the-contract)); (b) leave the shape to each tool.
- Recommended: (a), at M4, under a name such as `read-basis/0`.
- Why: third parties must recompute it, and only one fixed shape keeps every tool's recorded basis comparable.

### 3. The decision's subject

- Options: (a) the subject decided about, with decisions of one schema chained in their lane and appended with `if_heads`; (b) the decision's own record in a case system, where one exists; (c) a separate subject per decision, like M6's `evidence-sync/intent/<id>`.
- Recommended: (a) by default, and (b) where a case record already exists.
- Why: (a) puts the decision in the subject's history and state views and gives idempotency through `if_heads`. (c) takes the decision out of those views, and the de-duplication it offers is also available under (a).

### 4. When a basis assert is required

- Options: (a) the tool owner decides case by case; (b) a published list of action types that require one, enforced fail-closed; (c) every read.
- Recommended: (b). "Must" for listed action types, by default every action on a subject that relies on an agent's or a service's output, such as a recommendation or a check result. "May" otherwise. The list is published data: an `assert` that the tool owner publishes for that tool, like M6's `register/0`. The deployment's approval rules ([approvals.md](approvals.md#52-approval-rules)) can say which list applies to which tool, as they say which list supplies each role.
- Why: it keeps this a matter of policy, not architecture, while making the policy itself checkable. Under (b), the owner of each tool sets and publishes the list for that tool, so the decision stays with the tool owner.

### 5. Failed verifications

- Options: (a) a finding in the read receipt only; (b) always an `assert`; (c) an `assert` when the failure blocks or changes an action.
- Recommended: (c). Store-wide integrity stays with `store verify` and M3.
- Why: a failure that changes an action is a basis for it. Recording every failure would let a noisy source flood the log.

### 6. A basis that moved on

- Options: (a) record and detect, with `P_BASIS_STALE`; (b) an optional same-transaction check of other lanes' heads, chosen per decision schema; (c) both.
- Recommended: (a) now. Decide (b) in M6, and only for actions that write nothing to the system of record.
- Why: for record changes the system of record's own version check is the one that works.

### 7. An intake for asserts

- Options: (a) only tools inside the writer process can record asserts until M6; (b) an assert intake late in M4; (c) one in M6 beside `POST /intents`.
- Recommended: (b), or early M6 at the latest. One request is one batch: the object and its attestation lines, with optional `if_heads`, write scopes per lane, and store receipts in the answer. The intake also runs the pure pre-append checks proposed in [approvals.md](approvals.md): the actor id form, an `author` signer equal to `actor.id`, the kind of `on_behalf_of`, the record-lane writer, and grants. It refuses with every failed code, not only the first. Verification repeats the same checks, because a verifier never relies on intake.
- Why: M2 mapping asserts and M3 resolutions need the same path, and the command-line tool cannot send an object and its approval together. A check made before the append matters because the log cannot forget.
- **[open]** Which of these checks refuse at intake, and which only flag.

### 8. Who may approve

- Options: (a) any valid `approve` attestation, as the core checks today; (b) a profile rule on whose approval counts; (c) wait for M5 key binding.
- Recommended: (b), extended as below, now, and (c) at M5. Label approvals made before M5 as not accountability evidence. A recommended design is in [approvals.md](approvals.md).
  - An `approve` counts toward an approval rule only when it comes from a person or organisation listed for the role in the approver list in force at the line's log position, or from the object's `on_behalf_of`.
  - Agents and services never fill an approval step. They sign `author` and `witness`.
  - `on_behalf_of` must be a `person:` or `org:` id (`P_SPONSOR_NOT_ACCOUNTABLE`).
  - An `author` line's signer must equal `actor.id` (`P_AUTHOR_NOT_ACTOR`).
  - Until M5, a key seen under two signer names counts for neither (`P_KEY_SHARED`).
- Why: today an agent can approve its own recommendation and the core accepts it. Published lists and rules let anyone holding the log check whose approval counted.
- **[open]** Confirm that agents and services never fill an approval step or settle a disagreement, and that a tool's output counts as approved only through its `on_behalf_of`.

### 9. Kernel/2 rules the reader depends on

These belong in [Kernel/2's open questions](kernel-2.md#8-open-questions).

- **Lane of events.** Options: the record lane, with events as lifecycle changes that capture observes in the registry; assert lanes under one schema family; either. Recommended: the record lane, so that one fold and store/0's fork count both cover the whole chain.
- **Order at a merge.** Options: log order (`seq`); `at` order; a topological order with ties broken by (`at`, id); S defined only on linear chains, with a merge event restating the state. Recommended: topological order with (`at`, id) ties, because it depends on the objects alone and not on one store's positions. Run the gates per event over that event's own ancestors, and add merge vectors.
- **Order within an event.** Options: exits first; exits last; forbid an exit and an entry of the same block in one event. Recommended: exits first, which is the only reading under which exiting and re-entering a block in one event changes a state.
- **Scope.** Options: Kernel/2 also covers installations and devices; they get their own dictionary and profile tag at M9. Recommended: their own, with Kernel/2 kept for certificate lifecycles. The edge profile ([cer-edge.md](cer-edge.md)) leaves this open and defines no device index.
- **Size.** Decided (27 November 2026): decimal strings. M9's acceptance test and the README were amended to match.
- **Not coordinates.** Recorded here for any derived index: actor ids and approval status are never coordinates. A coordinate is decodable by design, so an actor id in one would publish who acted on what. An approval arrives after the object it approves, so approval status is a view at a log position ([approvals.md](approvals.md)), not a facet of the object.

### 10. Proving completeness to a remote reader

- Options: (a) ship the log prefix up to the checkpoint; (b) a commitment to all heads in a new checkpoint version, for example a sparse Merkle tree keyed by a digest of subject and lane; (c) a keyed digest of subject and lane in a new log-entry version, so a reader can scan entries against the root without seeing other subjects' objects; (d) the M4 service signs a completeness assert.
- Recommended: (a) or a local store until M4, then decide between (b) and (c) in M4's design. Either is a store/0 version change; evidence/0 is unchanged.
- Why: (b) is one commitment per checkpoint that `store verify` can recheck, but it is a store-signed claim that only the full log can audit. (c) is provable from the root but costs a scan. (d) is the weakest: attested, not proven.

### 11. Checkpoint cadence

- Options: (a) on a timer; (b) after each batch that others will rely on; (c) on request from a tool, through the service.
- Recommended: (a) and (b), written by the store or the M4 service. External publication of checkpoints comes with M5.
- Why: a decision becomes checkable outside the store only once a signed checkpoint covers it.

### 12. Privacy of actor ids

- Options: (a) no rule; (b) `actor.id`, `on_behalf_of` and attestation signers are pseudonymous, resolved to people only through an access-controlled directory or M5 key bindings, or, for keyed record references on an approver list, through the data owner's link-domain key and system of record.
- Recommended: (b), spelled out as below. evidence/0 stays frozen. A recommended design is in [approvals.md](approvals.md).
  - `person:` and `agent:` ids are 128 random bits written as 32 hex digits, assigned at enrolment. `service:` and `org:` ids may be public names, but a gateway or other service at someone's premises, a sole trader, or an organisation named after a person gets an opaque id.
  - An id is never derived from personal data, and never an unkeyed hash of it.
  - One pseudonym per party per directory.
  - The id is not the key. M5 key-binding objects bind keys to ids, with validity and revocation.
  - Who can resolve an id depends on access; the full table is in [approvals.md section 3](approvals.md#3-parties-and-ids-proposed). By default the public sees no pseudonyms: a deployment may publish the roles on an approver list, not who holds them. Parties with a status-only scope see status views that leave out signers. Parties entitled to see approval history also see approval lines, under M4's closed-by-default read scopes. A data owner can recognise its own users on an approver list that carries `record_refs`. An auditor also sees key bindings and grants. Only the body that keeps the directory sees the directory that maps pseudonyms to people. A deployment's configuration names that body; this note names none. The directory is access-controlled and kept off the log.
  - An actor id is never a coordinate ([open decision 9](#9-kernel2-rules-the-reader-depends-on)).
  - The assert intake refuses an id that breaks the form (`P_ACTOR_ID_FORM`, [open decision 7](#7-an-intake-for-asserts)), and verification checks it again.
- Why: today's privacy rules cover only inline content and guessable hashes, while actor ids are written into the log forever.
- **[open]** Random at enrolment (recommended) or a keyed digest held by the directory; whether one pseudonym per party per directory is kept; which body keeps the directory in a deployment; and whether store/0 Privacy gains a one-sentence rule or the rule stays in the profile.

<a id="13-agent-memory"></a>

### 13. Continuity for tools

- Options: (a) M4 also serves a tool's recall of its own earlier turns; (b) out of scope for M4: continuity between turns is explicit asserts under a recorded grant.
- Recommended: (b), with these definitions. A recommended design is in [approvals.md](approvals.md).
  - **The recorded grant** is an `approval/grant@0` assert by the accountable person, approved by them. It names the tool, what it may write and read, which purposes it may sign, a notes subject, a `recall_until` time and a validity period. Its periods are judged on the store's clock (`received_at`), never on the tool's own `at`. Changing or revoking a grant is a new version.
  - **Continuity notes** are asserts by reference in a dedicated notes system, under the grant. The hash is in the log; the body, with a random salt, is in the erasable blob store (M2). The reader labels them `note`, never `observed`, and returns them to the grantee only while the grant is in force and its recall window is open on the store's clock, and to the grantor at any time. A new version marked forgotten stops recall, and M5 erasure deletes the body. Revoking a grant ends the grantee's recall, and either party to the grant may record the forgotten version.
  - M4 serves no ranking or similarity search. Proposed timing: M4 serves no recall endpoint either, and continuity notes are built at M6, on the assert intake, once M5 erasure exists.
- Why: memory reads are verified queries over the log, and nothing comes back as `observed` unless capture wrote it. A tool's own earlier output must not come back as if it were observed ([design rule 10](#design-rules)).
- **[open]** When notes and their recall are built (at M6, once M5 erasure exists, as proposed, or sooner), the default recall window and retention, what a note body may hold (ids, codes and hashes only, or text the tool wrote, kept by reference), and whether a tool may file M6 requests under a grant.
