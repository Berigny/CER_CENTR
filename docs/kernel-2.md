# Kernel/2: prime-coordinate facets (profile `kernel/2`)

**Status:** draft for review. Revised 26 November 2026; the main design choices are listed in the [appendix](#appendix-design-choices-and-why). Its encoding (the dictionary format, and the arithmetic of encoding, decoding and divisibility) is [coordinate/0](../spec/coordinate-0.md), which is built. Kernel/2 itself (its events, its two coordinates and its gates) is not implemented. It is planned for roadmap milestone [M9](roadmap.md#m9-optional-centr0-profile-register-views-and-a-published-dictionary), as one of that milestone's deliverables. The open questions were settled on 28 November 2026 ([section 8](#8-open-questions)); those that need a decision outside the project are marked gated. Not an official publication of any agency; it describes no agency's process (see the [README](../README.md) notice).

**Labels.** **[derived]** means mechanically checked by a test. **[declared]** means a design decision. **[gated]** means a default that waits for a decision outside the project; nothing real proceeds on it until that decision is made.

## 1. Purpose and scope

Kernel/2 gives every attribution facet of a regulated unit (for example a certificate's lifecycle: methodology, project, measurement, reporting, retirement) its own prime number, so that a set of facets can be summarised as one integer, the **coordinate**, and recovered from it exactly.

- It is a **profile** of evidence/0, carried under `ext["kernel/2"]` ([spec](../spec/evidence-0.md#profiles)). The core verifier hashes it but does not interpret it.
- It is designed with certificate lifecycles in mind (for example ACCUs, LGCs and STCs); it is a draft profile, not part of any scheme's rules. It complements CENTR, which concerns products, installers and installations: an STC arising from a small-scale installation can reference that installation's evidence.
- **Certificate lifecycles only.** Installation and device records do not use Kernel/2's dictionary. They get a coordinate/0 dictionary and profile tag of their own, on a synthetic dictionary through M4b ([turn-reader decision 9](turn-reader.md#9-kernel2-rules-the-reader-depends-on)). Its facets are non-personal classes (a product class, a standard, a jurisdiction), never a site, premises or household attribute. **[gated]** A real dictionary for them ships only at M9, once an external publisher and a first consumer decide it; until then none is computed for a real record.
- **The evidence chain holds the path; the coordinate is an index.** Order, timing, who acted and why come from the chain of evidence objects (`prev` links, `at`, attestations). The coordinate summarises which facets hold, for exact filtering. It never replaces the chain and is never part of an object's id.

## 2. The dictionary

### 2.1 Allocation rule [declared]

Seven blocks of ten consecutive primes. Block `b` (0 to 6), position `i` (0 to 9) gets prime `P[10b + i]`, where `P` is the sequence of primes from 2. Position 0 is the block's **entry** transition, position 1 its **exit** transition, positions 2 to 9 its eight **states**.

### 2.2 Facet identifiers (normative)

A facet is written `<block>.<facet>`, for example `registry.locked`.

| Block (`b`) | Identifier | Covers |
|---|---|---|
| 0 | `universal` | Scheme-wide attributes: registry serial, legal status, vintage, compliance pathway |
| 1 | `measurement` | Metering, remote sensing, monitoring and verification plans |
| 2 | `emissions` | Baselines, leakage monitoring, greenhouse gas accounting |
| 3 | `reporting` | Submissions, public registers, audit trails, disclosures |
| 4 | `inputs` | Feedstocks, fuel types, supply-chain provenance |
| 5 | `infrastructure` | Project assets, installations, location, permanence |
| 6 | `registry` | Unit serials, methodology classes, ownership chains, legal instruments |

| Position | Identifier | Meaning |
|---|---|---|
| 0 | `entry` | Entered the block: issuance, registration, import, approval |
| 1 | `exit` | Left the block: retirement, surrender, export, transfer out, cancellation |
| 2 | `unclassified` | Explicitly not yet classified |
| 3 | `quantified` | Measured or quantified data |
| 4 | `transfer` | A transfer or movement |
| 5 | `locked` | Formally fixed: a structure or instrument locked in place |
| 6 | `committed` | Committed or future-dated, not yet converted |
| 7 | `unit` | Serialised as an individual unit |
| 8 | `linked` | Linked to or cross-referenced with another record |
| 9 | `executed` | Executed, final, retired |

### 2.3 Primes [derived]

| Block | entry | exit | unclassified | quantified | transfer | locked | committed | unit | linked | executed |
|---|---|---|---|---|---|---|---|---|---|---|
| universal | 2 | 3 | 5 | 7 | 11 | 13 | 17 | 19 | 23 | 29 |
| measurement | 31 | 37 | 41 | 43 | 47 | 53 | 59 | 61 | 67 | 71 |
| emissions | 73 | 79 | 83 | 89 | 97 | 101 | 103 | 107 | 109 | 113 |
| reporting | 127 | 131 | 137 | 139 | 149 | 151 | 157 | 163 | 167 | 173 |
| inputs | 179 | 181 | 191 | 193 | 197 | 199 | 211 | 223 | 227 | 229 |
| infrastructure | 233 | 239 | 241 | 251 | 257 | 263 | 269 | 271 | 277 | 281 |
| registry | 283 | 293 | 307 | 311 | 313 | 317 | 331 | 337 | 347 | 349 |

Seventy distinct primes, the first seventy, largest 349. Primes from 353 up are not used by `kernel/2`. That is not a migration path: a later kernel version is a new profile id with a dictionary of its own (section 6), and coordinates under different profile ids are never compared.

## 3. Events, the chain and the two coordinates

### 3.1 Events [declared]

A lifecycle event is an evidence/0 object about the unit (its `subject`) whose profile data names the facets the event adds, and optionally the states it removes:

```json
"ext": { "kernel/2": { "add": ["measurement.exit", "reporting.entry", "reporting.quantified", "reporting.linked"] } }
```

```json
"ext": { "kernel/2": { "add": ["measurement.quantified"], "remove": ["measurement.unclassified"] } }
```

- `add` is a non-empty list of facet identifiers from section 2.2, in any order. Duplicates within one event are not allowed.
- `remove` is optional: a list of state facets to take out of `S`, in any order, without duplicates. It names only states that `S` holds before the event, of a block the unit has entered and not exited (or of `universal`, whose states need no entry); never an entry or exit facet, and never a facet that is also in `add` (REQ-KRN-021). It changes `S` only; `H` only grows.
- An exit is the only facet of its block that its event names (REQ-KRN-022). An event must not exit and enter the same block, or exit a block and add or remove one of its states. Leaving a block and coming back takes two events. So no result depends on the order of the facets inside one event.
- The event's place in the lifecycle is its place in the chain: its `prev` points at the previous event for the same subject.
- **Lane.** Events are record changes in the subject's record lane ([store/0 Lanes and heads](../spec/store-0.md#lanes-and-heads)), so one fold and store/0's fork count cover the whole chain. Only capture writes a record lane ([D11](decisions.md#d11-observe-before-writing)), so Kernel/2 facets come from capture's mapping ([M2](roadmap.md#m2-read-only-capture-from-a-synthetic-register)'s `mapping/0`) or a capture baseline. A lifecycle step stated by a person or a tool is not an event.
- Every event of a subject's chain is under the kernel version of the chain's first event (REQ-KRN-023).
- Coordinates are **not** stored in the event. A verifier or index derives them (section 3.3).

### 3.2 Two coordinates [declared]

One product cannot be both "everything that ever happened" and "what is true now", because states replace each other and the product can only grow. Kernel/2 therefore defines two coordinates for any event `e`, computed over `e` and its ancestors in the chain (each ancestor counted once, even where the chain merges):

- **History coordinate `H(e)`**: the product of every facet every event added, with repeats. It only grows. It answers "has this unit ever been in state X?" and "how many times did it enter block B?".
- **State coordinate `S(e)`**: the product of the facets that hold now. `S` is a set: it holds each facet at most once. Applying events in the order below, and within one event its `remove` before its `add`:
  - a facet in `remove` leaves `S`;
  - `<block>.entry` adds the block's entry facet;
  - a state facet adds that state;
  - `<block>.exit` removes the block's entry and all its states;
  - a facet that `S` already holds, a state or the entry of a block already entered, stays once: adding it again leaves `S` unchanged, and `H` still counts it.

  `S` can shrink. Exit facets appear in `H`, never in `S`. A removed state leaves `S` and stays in `H`.

**Order of events.** Events are applied in topological order, one at a time: each step takes, from the events whose earlier events in the chain (those their `prev` names) have all been applied, the one with the earliest `at`, compared as instants, and of those with equal `at` the smaller id, compared as strings. So on two branches before a merge, the earlier branch event goes first. The order depends on the objects alone, not on any store's log positions, so stores and replicas that hold the same objects compute the same `S`. `H` does not depend on the order.

"`S` before an event" means `S` over that event's ancestors, without the event. Each event's gates (section 4) are checked against `S` before it, so an event on one branch is judged by its own ancestors, never by the other branch. An event that fails a gate adds nothing to `S` or `H` (section 4).

### 3.3 Representation [declared]

- A coordinate is written as a **decimal string** (`"8321327376004027894"`), never as a JSON number. Coordinates outgrow 64 bits and the 2^53 limit of evidence/0 numbers quickly: the example in section 5 reaches 100 bits, and all seventy primes once would be 467 bits. `S` holds each facet at most once, so it never passes 467 bits. `H` can pass coordinate/0's bound of 2^1024 on a long chain (REQ-KRN-014).
- `1` means **unclassified as a whole** (no facets). It is distinct from `universal.unclassified` (5), which is an explicit assertion. [REQ-KRN-011]
- Indexes may store the coordinate as a vector of 70 exponents instead of a number. Both decode to the same facets.
- Every coordinate is tagged with its profile id, `kernel/2`. Kernel/2 derives two coordinates per event, so each is named, `S` or `H`. An export, a receipt or a basis carries each in coordinate/0's tagged form (profile, dictionary id, coordinate) under its name ([coordinate/0 Tagged form](../spec/coordinate-0.md#tagged-form)). Indexes key on (profile, name, coordinate), and factor rows carry the name, with an index on (profile, name, prime). Kernel/2 shares primes 2 to 349 with other prime-based schemes, so the tag, not the primes, keeps namespaces apart.
- Encoding, decoding and divisibility are those of [coordinate/0](../spec/coordinate-0.md#operations). Kernel/2's dictionary is a coordinate/0 dictionary: profile `kernel/2`, its seven blocks as `multiset` families, and its 70 facets at the primes of section 2.3, with id `sha256:28ec5d10ee5e6e5fd8effc7515e9adec7f049f75beeccce5064d738d922cac71`. The vector generator builds it from the rule in section 2.1 and checks it against the table in section 2.3, and every coordinate/0 implementation loads it from the vectors, so REQ-KRN-001 to 005 are now tested mechanically ([dictionary cases](../spec/coordinate-0.md#dictionary-cases)). Indexes answer queries from factor rows or exponent vectors, never by arithmetic on SQL integers ([coordinate/0 Indexing](../spec/coordinate-0.md#indexing)).

## 4. Rules (verifier gates)

Each rule has an id, a verification method (A: analysis/automated test), and a pass/fail criterion. "Event" means an evidence object carrying `ext["kernel/2"]`.

**Dictionary**

- **REQ-KRN-001** (A) [derived] Every dictionary value is prime. Pass: 70 of 70.
- **REQ-KRN-002** (A) [derived] The 70 values are distinct. Pass: set size 70.
- **REQ-KRN-003** (A) [derived] The largest value is 349 and the dictionary equals the first 70 primes in order. Pass: exact match.
- **REQ-KRN-004** (A) [derived] Every block follows section 2.1: entry, exit, then the eight states in the order of section 2.2. Pass: all 7 blocks.
- **REQ-KRN-005** (A) [derived] In every block, entry < exit < every state. This holds by construction; the test guards against a hand-edited table.

**Coordinates**

- **REQ-KRN-006** (A) [derived] Factorising a coordinate over the dictionary recovers exactly the multiset of facets that built it. Pass: 1,000 random multisets (up to 40 facets, repeats allowed) round-trip with no loss.
- **REQ-KRN-007** (A) [declared] A performance target, not a gate: decoding a coordinate of up to 1,024 bits takes under 1 ms at p99 on reference hardware. It is reported for information, never run as a CI test and never a vector, as for [coordinate/0's target](../spec/coordinate-0.md#performance). (Decoding is at most 70 trial divisions per factor; a measured Python decode of a 108-bit coordinate takes about 4 µs.)
- **REQ-KRN-008** (A) Whether a coordinate contains a facet is a divisibility test (`coordinate mod p == 0`), or an exponent lookup when stored as a vector. Pass: agrees with the facet multiset on every REQ-KRN-006 sample.
- **REQ-KRN-011** (A) `"1"` decodes to UNCLASSIFIED and `"5"` to `universal.unclassified`; outputs never conflate them.
- **REQ-KRN-012** (A) [declared] Coordinates are decimal strings without leading zeros, never JSON numbers. Pass: every coordinate in vectors and reports matches `^[1-9][0-9]*$`.
- **REQ-KRN-013** (A) [declared] Coordinates are derived and never part of an object's id. A stored or exported coordinate that differs from the recomputed one is a finding (`P_KERNEL_COORDINATE_DRIFT`).
- **REQ-KRN-014** (A) [declared] **Range.** coordinate/0 rejects a coordinate of 2^1024 or more. When `H` for an event reaches that bound, `H` is not reported for the event and the finding is `P_KERNEL_RANGE`; `S` is still reported, and the event still applies. `H` only grows, so every later event in the chain gets the same finding. `S` cannot reach the bound (section 3.3). Pass: vectors with `H` just below the bound, and at or past it.

**Events**

- **REQ-KRN-020** (A) [declared] Every `add` and `remove` entry is a facet identifier from section 2.2, with no duplicates within either list. Otherwise `P_KERNEL_UNKNOWN_FACET` or `P_KERNEL_DUPLICATE_FACET`.
- **REQ-KRN-021** (A) [declared] **Remove list.** `remove` names state facets only, never an entry or exit facet (`P_KERNEL_REMOVE_TRANSITION`). No facet is in both `add` and `remove` (`P_KERNEL_ADD_AND_REMOVE`). Every facet in `remove` is in `S` before the event (`P_KERNEL_REMOVE_NOT_HELD`); a removed state of a block other than `universal` also needs that block entered (REQ-KRN-009). `remove` changes `S` only, never `H`.
- **REQ-KRN-022** (A) [declared] **An exit stands alone in its block.** An event that adds `<block>.exit` names no other facet of that block, in `add` or in `remove`. It must not exit and enter the same block, and must not exit a block and add or remove one of its states. Otherwise `P_KERNEL_EXIT_NOT_ALONE`.
- **REQ-KRN-023** (A) [declared] **One version per chain.** A subject's chain stays on the kernel version of its first event, in the order of section 3.2. An object in the chain that carries another kernel version's profile data (`ext["kernel/3"]`, for example), alone or beside `ext["kernel/2"]`, is a finding, `P_KERNEL_VERSION_MIXED`, and adds nothing to `S` or `H`.
- **REQ-KRN-009** (A) [declared] **Domain presence.** A state facet of block B (other than `universal`) may be added only while B is entered: B's entry is in `S` before the event, or the same event adds it. It may be removed only if B's entry is in `S` before the event. An exit of B requires B's entry in `S` before the event; an entry in the same event never counts, because an event may not exit and enter one block (REQ-KRN-022). Otherwise `P_KERNEL_NOT_ENTERED`. The `universal` block's states are exempt; its entry and exit follow the same rule.
- **REQ-KRN-010** (A) [declared] **Coexistence.** After applying an event (its `remove`, then its `add`), `S` must not contain two states of the same block that the table below marks as contradictory. Otherwise `P_KERNEL_CONTRADICTION`. States of different blocks never contradict, and `H` is never checked (history may hold both "was quantified" and "was in transfer").

  Contradiction table, applied within each block (confirmed 28 November 2026):

  | State | Contradicts |
  |---|---|
  | `unclassified` | every other state of the same block |
  | `quantified` | `transfer` |
  | `committed` | `executed` |

  Every other pair may coexist. To change a state, an event names the old state in `remove` and the new one in `add`: a block leaves `unclassified` with `"remove": ["measurement.unclassified"], "add": ["measurement.quantified"]`, and a block that holds `committed` drops it in the event that adds `executed`. The gate checks `S` after both lists, so such an event passes. Exiting and re-entering the block also clears its states, but takes two events (REQ-KRN-022), and `H` then records the exit and a second entry.

**Order of checks and failed events** [declared]

- A verifier checks each event of a chain, and any other object REQ-KRN-023 covers, in this order and reports the first failure only: `P_KERNEL_VERSION_MIXED` (REQ-KRN-023); `P_KERNEL_UNKNOWN_FACET`, then `P_KERNEL_DUPLICATE_FACET` (020); `P_KERNEL_REMOVE_TRANSITION`, then `P_KERNEL_ADD_AND_REMOVE` (021); `P_KERNEL_EXIT_NOT_ALONE` (022); `P_KERNEL_NOT_ENTERED` (009); `P_KERNEL_REMOVE_NOT_HELD` (021); `P_KERNEL_CONTRADICTION` (010).
- An event that fails one of these is a **failed event**. It adds nothing to `S` or `H`: its own coordinates, and those of every later event, are computed as if it named no facets. (An unknown facet has no prime, so `H` could not always count a failed event anyway.) Later events are checked against `S` without it.
- For an event that passes, the findings about derived values follow, also first only: `P_KERNEL_RANGE` (014), then `P_KERNEL_COORDINATE_DRIFT` (013). They do not make the event a failed one.
- Findings are collected across events and sorted as store/0 sorts its findings: by the event's log position, then by the code's place in this list ([store/0](../spec/store-0.md#collecting-and-ordering-findings)). Kernel/2 reports them in the shared report for profile findings that [M2](roadmap.md#m2-read-only-capture-from-a-synthetic-register) introduces.

## 5. Example: an ACCU lifecycle

Five events for one unit, each pointing at the previous one with `prev`. The values are computed with the dictionary in section 2.3. The sequence is a simplified illustration of how facets combine, not the ACCU Scheme's process: under the scheme, units are issued only after a project report is assessed ([Clean Energy Regulator](https://cer.gov.au/schemes/australian-carbon-credit-unit-scheme/how-to-participate-accu-scheme/project-reporting-and-audits)).

| Event | `add` | S after the event | H after the event |
|---|---|---|---|
| E1 Methodology approved | `universal.entry`, `registry.entry`, `registry.locked` | `179422` | `179422` |
| E2 Project registered | `infrastructure.entry`, `infrastructure.quantified`, `infrastructure.committed` | `2822653806194` | `2822653806194` |
| E3 Measured and issued | `measurement.entry`, `measurement.quantified`, `measurement.unit` | `229518448943052722` | `229518448943052722` |
| E4 Reported | `measurement.exit`, `reporting.entry`, `reporting.quantified`, `reporting.linked` | `8321327376004027894` | `25035387438225574245358414` |
| E5 Retired | `reporting.exit`, `registry.executed` | `985106178361706` | `1144592878288235028923541329666` |

- After E4, `S` no longer holds the measurement facets (the unit left that block), while `H` still records them, and the exit.
- After E5, `S` holds the universal entry, the registry facets `entry`, `locked` and `executed`, and the infrastructure facets; `H` is 100 bits, which is why coordinates are strings.
- Retirement adds `registry.executed` only: not a second `registry.entry`, because the unit is still in the registry, now retired, and not `universal.exit`. Universal state that stays true of a retired unit (its serial, legal status, vintage and pathway) stays in `S`. `universal.exit` would remove the universal entry and every universal state, and the same exit also stands for export, transfer out, surrender and cancellation. A second `universal.entry` was also rejected: it reads as re-entering the scheme.
- Checked against the rules of section 4 as settled on 28 November 2026, every event passes. No event has a `remove` list. Each exit is the only facet of its block in its event (`measurement.exit` in E4, `reporting.exit` in E5) and follows that block's entry in an earlier event. No event re-adds a facet that `S` holds, and no block holds a contradictory pair: `infrastructure.committed` and `registry.executed` are in different blocks. So no value in the table changes, and coordinate/0's section 5 vectors stand as they are.

## 6. Versioning

- Changing the dictionary is a new kernel version (`kernel/3`), never an edit to `kernel/2`. A new version is a new profile id and a new namespace: coordinate/0's `extends` check does not apply across profile ids, so moving to `kernel/3` means rebuilding the index, and the primes that `kernel/2` leaves unused give no compatibility between the two. Events keep the version they were written under, and verifiers check them against that version's dictionary.
- A subject's chain does not mix versions (REQ-KRN-023), so `S` and `H` are always computed under one version.
- Profile rules that do not change the dictionary (for example the contradiction table, the `remove` list and the order of checks, settled on 28 November 2026) are recorded in this document before the profile ships at M9; after that, a rule change also needs a new version.
- Until the profile ships, a rule change could still alter `S` without changing the dictionary id. Values computed under this draft, such as those in section 5, are illustrative. Receipts and bases carry them in tagged form (section 3.3), and the `read-basis/0` vectors at M4 use a synthetic coordinate/0 profile, not Kernel/2 values ([turn-reader decision 9](turn-reader.md#9-kernel2-rules-the-reader-depends-on)).

## 7. Conformance

As for evidence/0 (decision D7), the vectors are the contract. coordinate/0's vectors already cover the dictionary (REQ-KRN-001 to 005) and the arithmetic Kernel/2 shares with it (section 3.3). At M9 this profile ships with:

- dictionary and decode vectors for REQ-KRN-006, 008, 011 and 012, and range vectors for REQ-KRN-014 at the 2^1024 bound;
- event-chain vectors (valid and invalid) for REQ-KRN-009, 010, 013 and 020 to 023, including merges (with branches ordered by `at` and by id), exits and re-entries in separate events, `remove` lists, re-added facets, a failed event followed by valid ones, a chain that mixes versions, and events with several faults, which give the first code in the order of section 4;
- an independent JavaScript reference, alongside the Rust verifier, for coordinate derivation and the gates.

REQ-KRN-007 is a performance target, reported for information; it has no vector.

## 8. Open questions

Settled on 28 November 2026; the part of question 14 that needs a decision outside the project is gated. Questions 5 to 15 were raised in review of this draft and of the [turn-reader note](turn-reader.md#9-kernel2-rules-the-reader-depends-on), which depends on them.

1. Confirm the contradiction table in REQ-KRN-010, or replace it. **Decided (28 November 2026):** confirmed, with its three rows. A state change names the old state in `remove` (question 2), so a block leaves `unclassified`, or drops `committed` at retirement, in the event that adds the new state.
2. Allow a `remove` list in events, or require exit and re-entry to change a state? **Decided (28 November 2026):** an optional, constrained `remove` list (section 3.1, REQ-KRN-021): only states that `S` holds, of a block the unit has entered and not exited, never an entry or exit facet and never a facet also in `add`; applied before `add`; changing `S` only. `H` only grows, so a state change no longer writes an exit and a second entry into `H`.
3. Should retirement add `universal.exit`? **Decided (28 November 2026):** no. Retirement adds `registry.executed` only, and universal state that stays true of a retired unit stays in `S`. Section 5 already does this, so no value changes.
4. Confirm the rule in REQ-KRN-009 that an exit requires the block to have been entered. An alternative considered accepts either the entry or the exit facet as evidence of presence. **Decided (28 November 2026):** confirmed. The block's entry must be in `S` before the event; an entry in the same event never counts, since an event may not exit and enter one block (REQ-KRN-022). A capture baseline for a unit first seen after it left a block therefore records no exit for that block, rather than an entry nobody observed.
5. Which lane do events use? **Decided (28 November 2026):** the subject's record lane (section 3.1), so one fold and store/0's fork count cover the whole chain. Facets come from capture's mapping or a capture baseline.
6. In what order are two branches applied at a merge? **Decided (28 November 2026):** topological order, with ties broken by `at` and then id (section 3.2). The gates run per event over its own ancestors, and the M9 vectors include merges.
7. When one event exits a block and enters it again, which applies first? **Decided (28 November 2026):** neither: such an event is invalid (REQ-KRN-022, `P_KERNEL_EXIT_NOT_ALONE`), as is one that exits a block and adds or removes one of its states. With the `remove` list no event needs it, and no result depends on the order of facets inside an event. Applying exits first is not adopted.
8. What does `S` do when an event adds a facet it already holds? **Decided (28 November 2026):** nothing: `S` is a set, and `H` still counts the repeat (section 3.2).
9. What happens when `H` reaches 2^1024, which coordinate/0 rejects? **Decided (28 November 2026):** a finding, `P_KERNEL_RANGE` (REQ-KRN-014): `H` is omitted, `S` is still reported, and vectors test the bound.
10. In what order are the gates checked, is there one finding per event or several, and does a failed event change `S`? **Decided (28 November 2026):** a fixed order with the first failure per event, findings sorted as store/0 sorts them, and a failed event adds nothing to `S` or `H` (section 4, Order of checks and failed events).
11. Is a later kernel that only adds facets a new profile id, and how are chains that mix versions computed? **Decided (28 November 2026):** every later kernel is a new profile id and namespace, and a chain stays on its first event's version (section 6, REQ-KRN-023).
12. Is REQ-KRN-007 a test or a target? **Decided (28 November 2026):** a target, reported for information, as coordinate/0's performance target is. It is not in the conformance vectors (section 7).
13. Where is Kernel/2 built? **Decided (28 November 2026):** at M9, listed among that milestone's deliverables, and nothing is built before then.
14. Which dictionary do installation and device records use? **Decided (28 November 2026):** a coordinate/0 dictionary and profile tag of their own, with Kernel/2 kept for certificate lifecycles, synthetic through M4b (section 1). **Gated:** an external publisher and a first consumer decide when a real dictionary ships, before M9. Until then, coordinates for installations and devices are synthetic only, and none is computed for a real record.
15. How does a receipt or a basis say which dictionary and rules its `S` and `H` were computed with? **Decided (28 November 2026):** coordinate/0's tagged form under each coordinate's name (section 3.3). Values computed before the profile ships are illustrative (section 6).

---

## Appendix. Design choices and why

| Choice | Instead of | Why |
|---|---|---|
| Coordinates as decimal strings (or exponent vectors); REQ-KRN-007 up to 1,024 bits; REQ-KRN-012 | Assuming a coordinate fits 64 bits | The example in section 5 reaches 100 bits, and all seventy primes once would be 467 bits. evidence/0 numbers stop at 2^53. SQL integers stop at 64 bits, and SQLite gives a wrong remainder silently for a larger coordinate stored as text (on SQLite 3.53, the 65-bit product of the first 16 primes gave 33 for `% 53` instead of 0), so indexes answer from factor rows ([coordinate/0](../spec/coordinate-0.md#indexing)). |
| The chain holds the path; the coordinate is an index | A coordinate that encodes the full path | Multiplication ignores order: entering then leaving two blocks in either order gives the same number. |
| Two coordinates: history `H` (accumulates, never gated) and state `S` (can shrink, gated) | One coordinate whose factors only accumulate, with REQ-KRN-010 rejecting contradictory states | A unit whose state changes would otherwise fail the coexistence gate. |
| A contradiction table of three rows, confirmed 28 November 2026 | A single contradictory pair (`quantified` × `transfer`) | One pair leaves the gate untestable beyond that pair, and leaves cases such as `unclassified` with `quantified` unclear. |
| An optional `remove` list: held states only, applied before `add`, changing `S` only | Exit and re-entry as the only way to change a state | Every state change would write an exit and a second entry into `H`, so "how many times did it enter block B?" would count bookkeeping as lifecycle events. |
| An exit is the only facet of its block in its event (REQ-KRN-022) | Applying exits first, or last, within one event | With a `remove` list no event needs to exit and re-enter one block, and no result then depends on the order of facets inside an event. |
| `S` is a set; re-adding a held facet leaves it unchanged | Refusing the event, or counting repeats in `S` as `H` does | `S` says what holds now, and a capture baseline may restate a held state. A squared prime in `S` has no meaning. |
| Branches at a merge in topological order, ties by `at` then id | Log order | The order depends on the objects alone, so stores and replicas holding the same objects agree. |
| A fixed order of checks, the first failure per event, and a failed event adds nothing to `S` or `H` | Every failure per event, or an order left to implementations | One expected result per vector, as decision D7 needs; later events do not inherit an invalid event's facets. |
| Retirement adds `registry.executed` only | Retirement also adding `universal.exit`, or a second `universal.entry` | `universal.exit` would drop universal facts that stay true of a retired unit, and an entry at retirement reads as re-entering the scheme. |
| Gates are [declared]; [derived] only for mechanically checked arithmetic | Labelling the gates [derived], including the coexistence of `universal` and block states | A data-validity rule is a design decision; only arithmetic can be checked mechanically. |
| Every coordinate tagged `kernel/2` and named `S` or `H`; indexes key on (profile, name, coordinate) | Treating primes 2 to 349 as a namespace of their own | The range overlaps other prime-based schemes, so separation must come from the tag. |
| Coordinates derived, never in an id (REQ-KRN-013) | Storing the coordinate as the certificate's fingerprint | Same rule as the prime-coordinate conditions in the [README](../README.md#prime-coordinates-for-faceted-queries). |
