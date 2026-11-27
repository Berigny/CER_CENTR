# Kernel/2: prime-coordinate facets (profile `kernel/2`)

**Status:** draft for review, 26 November 2026. Corrected from the submitted draft CER-KERNEL-002 v1.0.0 (changes listed in [Appendix B](#appendix-b-changes-from-the-submitted-draft)). Built at roadmap milestone M9 ([roadmap](roadmap.md)); nothing here is implemented yet.

**Labels.** **[derived]** means mechanically checked by a test. **[declared]** means a design decision. **[open]** means a decision the owner still has to confirm. The ontology that motivated the design (the "native register") is in [Appendix A](#appendix-a-native-register-non-normative) and is not normative.

## 1. Purpose and scope

Kernel/2 gives every attribution facet of a regulated unit (for example a certificate's lifecycle: methodology, project, measurement, reporting, retirement) its own prime number, so that a set of facets can be summarised as one integer, the **coordinate**, and recovered from it exactly.

- It is a **profile** of evidence/0, carried under `ext["kernel/2"]` ([spec](../spec/evidence-0.md#profiles)). The core verifier hashes it but does not interpret it.
- Its first use is certificate lifecycles in the Clean Energy Regulator's schemes (for example ACCUs, LGCs and STCs). It complements CENTR, which concerns products, installers and installations: an STC arising from a small-scale installation can reference that installation's evidence.
- **The evidence chain holds the path; the coordinate is an index.** Order, timing, who acted and why come from the chain of evidence objects (`prev` links, `at`, attestations). The coordinate summarises which facets hold, for fast filtering. It never replaces the chain and is never part of an object's id.

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

Seventy distinct primes, the first seventy, largest 349. Primes from 353 up are reserved for later kernel versions.

## 3. Events, the chain and the two coordinates

### 3.1 Events [declared]

A lifecycle event is an evidence/0 object about the unit (its `subject`) whose profile data names the facets the event adds:

```json
"ext": { "kernel/2": { "add": ["measurement.exit", "reporting.entry", "reporting.quantified", "reporting.linked"] } }
```

- `add` is a non-empty list of facet identifiers from section 2.2, in any order. Duplicates within one event are not allowed.
- The event's place in the lifecycle is its place in the chain: its `prev` points at the previous event for the same subject.
- Coordinates are **not** stored in the event. A verifier or index derives them (section 3.3).

### 3.2 Two coordinates [declared]

One product cannot be both "everything that ever happened" and "what is true now", because states replace each other and the product can only grow. Kernel/2 therefore defines two coordinates for any event `e`, computed over `e` and its ancestors in the chain (each ancestor counted once, even where the chain merges):

- **History coordinate `H(e)`**: the product of every facet every event added, with repeats. It only grows. It answers "has this unit ever been in state X?" and "how many times did it enter block B?".
- **State coordinate `S(e)`**: the product of the facets that hold now. Applying events oldest first:
  - `<block>.entry` adds the block's entry facet;
  - a state facet adds that state;
  - `<block>.exit` removes the block's entry and all its states.

  `S` can shrink. Exit facets appear in `H`, never in `S`.

### 3.3 Representation [declared]

- A coordinate is written as a **decimal string** (`"8321327376004027894"`), never as a JSON number. Coordinates outgrow 64 bits and the 2^53 limit of evidence/0 numbers quickly: the example in section 5 reaches 100 bits, and all seventy primes once would be 467 bits.
- `1` means **unclassified as a whole** (no facets). It is distinct from `universal.unclassified` (5), which is an explicit assertion. [REQ-KRN-011]
- Indexes may store the coordinate as a vector of 70 exponents instead of a number. Both decode to the same facets.
- Every coordinate is tagged with its profile id, `kernel/2`. Indexes key on (profile, coordinate). Kernel/2 shares primes 2 to 349 with other prime-based schemes, so the tag, not the primes, keeps namespaces apart.

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
- **REQ-KRN-007** (A) Decoding a coordinate of any size takes under 1 ms on reference hardware. Pass: p99 < 1 ms for coordinates up to 1,024 bits. (Decoding is at most 70 trial divisions per factor; a measured Python decode of a 108-bit coordinate takes about 4 µs.)
- **REQ-KRN-008** (A) Whether a coordinate contains a facet is a divisibility test (`coordinate mod p == 0`), or an exponent lookup when stored as a vector. Pass: agrees with the facet multiset on every REQ-KRN-006 sample.
- **REQ-KRN-011** (A) `"1"` decodes to UNCLASSIFIED and `"5"` to `universal.unclassified`; outputs never conflate them.
- **REQ-KRN-012** (A) [declared] Coordinates are decimal strings without leading zeros, never JSON numbers. Pass: every coordinate in vectors and reports matches `^[1-9][0-9]*$`.
- **REQ-KRN-013** (A) [declared] Coordinates are derived and never part of an object's id. A stored or exported coordinate that differs from the recomputed one is a finding (`P_KERNEL_COORDINATE_DRIFT`).

**Events**

- **REQ-KRN-020** (A) [declared] Every `add` entry is a facet identifier from section 2.2, with no duplicates within an event. Otherwise `P_KERNEL_UNKNOWN_FACET` or `P_KERNEL_DUPLICATE_FACET`.
- **REQ-KRN-009** (A) [declared] **Domain presence.** A state facet of block B (other than `universal`) may be added only while B is entered: B's entry is in `S` before the event, or the same event adds it. An exit of B requires B to be entered. Otherwise `P_KERNEL_NOT_ENTERED`. The `universal` block's states are exempt; its entry and exit follow the same rule.
- **REQ-KRN-010** (A) [declared, table [open]] **Coexistence.** After applying an event, `S` must not contain two states of the same block that the table below marks as contradictory. Otherwise `P_KERNEL_CONTRADICTION`. States of different blocks never contradict, and `H` is never checked (history may hold both "was quantified" and "was in transfer").

  Proposed contradiction table, applied within each block [open]:

  | State | Contradicts |
  |---|---|
  | `unclassified` | every other state of the same block |
  | `quantified` | `transfer` |
  | `committed` | `executed` |

  Every other pair may coexist. To change a state, an event removes it by exiting and re-entering the block, or (proposed, [open]) by naming it in a `remove` list alongside `add`.

## 5. Example: an ACCU lifecycle

Five events for one unit, each pointing at the previous one with `prev`. The values are computed with the dictionary in section 2.3.

| Event | `add` | S after the event | H after the event |
|---|---|---|---|
| E1 Methodology approved | `universal.entry`, `registry.entry`, `registry.locked` | `179422` | `179422` |
| E2 Project registered | `infrastructure.entry`, `infrastructure.quantified`, `infrastructure.committed` | `2822653806194` | `2822653806194` |
| E3 Measured and issued | `measurement.entry`, `measurement.quantified`, `measurement.unit` | `229518448943052722` | `229518448943052722` |
| E4 Reported | `measurement.exit`, `reporting.entry`, `reporting.quantified`, `reporting.linked` | `8321327376004027894` | `25035387438225574245358414` |
| E5 Retired | `reporting.exit`, `registry.executed` | `985106178361706` | `1144592878288235028923541329666` |

- After E4, `S` no longer holds the measurement facets (the unit left that block), while `H` still records them, and the exit.
- After E5, `S` holds the registry facets `entry`, `locked` and `executed` and the infrastructure facets; `H` is 100 bits, which is why coordinates are strings.
- Retirement uses `registry.executed`, not a second `registry.entry`: the unit is still in the registry, now retired. Whether retirement should also add `universal.exit` is [open] (the submitted draft used a second universal entry here, which reads as re-entering the scheme).

## 6. Versioning

- Changing the dictionary is a new kernel version (`kernel/3`), never an edit to `kernel/2`. Events keep the version they were written under, and verifiers check them against that version's dictionary.
- Profile rules that do not change the dictionary (for example confirming the contradiction table) are recorded in this document before the profile ships at M9; after that, a rule change also needs a new version.

## 7. Conformance

As for evidence/0 (decision D7), the vectors are the contract. At M9 this profile ships with:

- dictionary and decode vectors for REQ-KRN-001 to 008, 011 and 012;
- event-chain vectors (valid and invalid) for REQ-KRN-009, 010, 013 and 020, including merges, exits and re-entries;
- an independent JavaScript reference, alongside the Rust verifier, for coordinate derivation and the gates.

## 8. Open questions for the owner

1. Confirm the contradiction table in REQ-KRN-010, or supply the intended one.
2. Allow a `remove` list in events, or require exit and re-entry to change a state?
3. Should retirement add `universal.exit`?
4. Confirm the tightening in REQ-KRN-009: an exit requires the block to have been entered (the submitted draft accepted either the entry or the exit facet as evidence of presence).

---

## Appendix A. Native register (non-normative)

The submitted draft described the same arithmetic in a second vocabulary, drawn from a wave-domain reading of the design. It is kept here as the motivation for the structure, not as a derivation: physics can motivate a data rule, but the rules in section 4 are design decisions.

| Normative identifier | Native name |
|---|---|
| `universal` | GEN (the unified field substrate) |
| `measurement` | Visual |
| `emissions` | Olfactory |
| `reporting` | Auditory |
| `inputs` | Gustatory |
| `infrastructure` | Tactile |
| `registry` | Symbolic |
| `entry` / `exit` | Sink / Branch (imposing and releasing a boundary condition) |
| `unclassified` | Null (radiative balance; the I = 0 condition) |
| `quantified` | Electric (storage projection) |
| `transfer` | Magnetic (transport projection) |
| `locked` | Resonance (trapped mode) |
| `committed` | Potential (energy held) |
| `unit` | Unit (quanta) |
| `linked` | Relation (binding, phase-locking) |
| `executed` | Action (directed force) |

In that reading, the `universal` block is the field and each other block a boundary-conditioned mode of it, so a universal state and a block state of the same kind may coexist (the same configuration read at two scales). The draft also proposed a geometric correspondence with a 27-node lattice (one mediator node for the universal block, six face centres for the blocks, eight vertices for the states, twelve edge midpoints for the entry and exit transitions); the edge assignment is a convention, not a theorem.

## Appendix B. Changes from the submitted draft

| Submitted draft v1.0.0 | This draft | Why |
|---|---|---|
| Coordinates assumed to fit 64 bits (REQ-KRN-007) | Decimal strings (or exponent vectors); REQ-KRN-007 up to 1,024 bits; REQ-KRN-012 added | The draft's own ACCU example is 108 bits, and all 70 primes once is 467 bits. evidence/0 numbers stop at 2^53. |
| "The final coordinate encodes the full provenance path" | The chain holds the path; the coordinate is an index | Multiplication ignores order: entering then leaving two blocks in either order gives the same number. |
| Factors only accumulate, and REQ-KRN-010 rejects contradictory states | Two coordinates: history `H` (accumulates, never gated) and state `S` (can shrink, gated) | A unit whose state changes would otherwise fail the coexistence gate. |
| One contradictory pair given (Electric × Magnetic) | A proposed table, marked [open] | The gate was untestable beyond one example; the draft's own §4.2 example (Null with Electric) was ambiguous. |
| Retirement multiplied in a second GEN.Sink (entry) | `registry.executed`; `universal.exit` is an open question | Entry for retirement read as re-entering the scheme. |
| GEN + domain coexistence and the gates labelled [derived] from physics | Gates are [declared]; [derived] only for mechanically checked arithmetic | A physical theory cannot derive a data-validity rule; it can motivate one. |
| Native register normative alongside the CER register | CER register normative; native register in Appendix A | Reviewers can check the rules without adopting the ontology. |
| Prime range 2 to 349 presented as a standalone namespace | Every coordinate tagged `kernel/2`; indexes key on (profile, coordinate) | The range overlaps other prime-based schemes, so separation must come from the tag. |
| Coordinate stored as the certificate's fingerprint | Coordinates derived, never in an id (REQ-KRN-013) | Same rule as the prime-coordinate conditions in the [README](../README.md#proposed-prime-coordinates-for-faceted-queries). |
