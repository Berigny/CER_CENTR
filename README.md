# CER_CENTR: an evidence layer for regulatory records

A small, portable **evidence object** that sits beside an existing system of record. For each record change or decision it proves what was recorded, under which schema version, who recorded it (a person, a service, or an AI agent and the person accountable for it), when, and which earlier version it replaces.

It does not replace any database, schema or register. It reads from them and never writes back. Each object points at the system of record's own schema by hash, so people and AI tools can interpret a record exactly, and anyone with the object can check it without access to the source database.

> Independent work. Not affiliated with or endorsed by the Clean Energy Regulator. All data in this repository is synthetic.

## What it is for

- **Audit trails** for decisions, whether made by people, services or AI agents.
- **Data shared with other bodies** that the receiver can verify without access to the source system.
- **Chain of custody** for evidence in compliance and enforcement matters.
- **Records of AI-assisted checks:** what the model was given, what it returned, and who approved or overrode it.
- **Public registers as views:** a published list is an evidence object recording a query result, so "what did the list show on date X?" has a provable answer.

The first domain profile in view is CENTR, the proposed Consumer Energy National Technical Regulator ([notes](docs/centr-profile-notes.md)). The core is deliberately domain-neutral.

## Layout

| Path | What |
|---|---|
| [`spec/evidence-0.md`](spec/evidence-0.md) | The one-page spec: object, canonical form, attestations, verification order and error codes |
| [`spec/schema/`](spec/schema/) | JSON Schemas for the object and for attestations |
| [`test-vectors/`](test-vectors/) | The conformance contract: canonicalisation, number formatting and object cases with expected results |
| [`crates/evidence-core/`](crates/evidence-core/) | Rust core: strict JSON parser, RFC 8785 canonicalisation, ids, Ed25519 attestations, verifier |
| [`crates/evidence-cli/`](crates/evidence-cli/) | `evidence` command-line tool: objects, and `evidence store` to create, append to, verify, export, import and prove stores |
| [`crates/evidence-wasm/`](crates/evidence-wasm/) | The core as one WebAssembly module, with a JavaScript wrapper for browsers and Node, and a small ABI for other hosts |
| [`crates/evidence-py/`](crates/evidence-py/) | Python package `evidence_layer`, built from the same core |
| [`tools/reference/`](tools/reference/) | Independent JavaScript reference for objects and stores, used to generate and cross-check the vectors |
| [`spec/store-0.md`](spec/store-0.md) | Store spec: log entries, checkpoints, exports and their verification (milestone M1) |
| [`crates/evidence-log/`](crates/evidence-log/) | Pure store formats and the export verifier: log entries, RFC 9162 tree and proofs, genesis, checkpoints, store reports (M1) |
| [`crates/evidence-store/`](crates/evidence-store/) | The store: an append-only log in one SQLite file (and an in-memory reference), writer lease, signed checkpoints, export, import, live verification and Merkle proofs (M1) |
| [`tools/verify-page/`](tools/verify-page/) | A static page that verifies a store export in the browser, offline (M1) |
| [`docs/decisions.md`](docs/decisions.md) | Design decisions and why |
| [`docs/turn-reader.md`](docs/turn-reader.md) | Design note: a reader that tools and AI agents call before they rely on evidence, and when a read must be recorded (input to M4, M6 and M9; not built) |
| [`docs/approvals.md`](docs/approvals.md) | Design note: approver lists and approval rules as published data, an approval-status view, disagreement resolved by a person, pseudonymous actor ids, and hooks that let any tool, AI agents included, act under a grant and keep continuity notes (input to M4, M5 and M6; not built) |
| [`docs/cer-edge.md`](docs/cer-edge.md) | Draft profile for edge-device records in consumer energy resources, and the edge track EG1-EG4 under the roadmap (synthetic data only; not built) |

## Quick start

Requires Rust (stable). Node 20+ and Python 3.10+ only for the cross-checks.

```sh
cargo build --release
E=target/release/evidence
V=test-vectors/objects/003-agent-assertion-approved

$E verify $V/object.json --attestations $V/attestations.jsonl --require approve
# {"attestations":{"approve":1,"author":1},"content_checked":true,"id":"sha256:7d7f…","ok":true}

$E verify test-vectors/objects/106-duplicate-member/object.json
# {"attestations":{},"code":"E_DUPLICATE_KEY",…,"ok":false}   exit status 1
```

Writing an object:

```sh
$E content-hash record.json          # content.hash for a JSON record
$E file-hash schema.sql              # schema.hash for the definition it follows
$E seal draft.json > object.json     # fills in id, prints canonical JSON
$E keygen dev.seed                   # development key only
$E sign object.json --seed dev.seed --signer person:me --purpose approve >> attestations.jsonl
$E verify object.json --attestations attestations.jsonl --require approve
```

### Store

A store is an append-only log of objects and attestations in one directory ([`spec/store-0.md`](spec/store-0.md)). Every entry gets the store's own position (`seq`) and received time, and signed checkpoints cover the log with a Merkle tree. The commands below use the object vectors and `test-vectors/keys/store-demo.seed`, a public test key: anyone can sign with it, so use it for tests only. Each command prints canonical JSON lines; hashes and times are shortened here, and most differ on every run.

```sh
E=target/release/evidence
O=test-vectors/objects
K=test-vectors/keys/store-demo.seed     # public test key: for tests only
S=$(mktemp -d)/demo                     # the store directory, created by init

$E store init $S --name demo --seed $K
# {"head":{"entry_hash":"sha256:…","seq":2},"key":"ed25519:EWYM…","log":"sha256:…","name":"demo"}

$E store append $S $O/001-create-inline/object.json $O/002-update-with-prev/object.json $O/003-agent-assertion-approved/object.json
# {"duplicate":false,"entry_hash":"sha256:…","file":"test-vectors/objects/001-create-inline/object.json","kind":"object","ok":true,"received_at":"2026-…Z","ref":"sha256:2419…","seq":3}
# …one line per file, each committed on its own: seqs 4 and 5

$E store attest $S $O/003-agent-assertion-approved/attestations.jsonl
# three lines, seqs 6 to 8: the author and approve lines of 003, and the author line of 001

$E store get $S $($E id $O/003-agent-assertion-approved/object.json) --require approve
# {"attestations":[…],"entry":{…,"seq":5,…},"object":{…},"report":{"attestations":{"approve":1,"author":1},"content_checked":true,"id":"sha256:7d7f…","ok":true}}

$E store checkpoint $S --seed $K
# {"checkpoint":"sha256:…","head":"sha256:…","root":"sha256:…","seq":9,"size":8}

$E store verify $S
# {"checked":{"attestations":5,"checkpoints":1,"entries":10,"objects":4},…,"covered":{…,"size":8},"findings":[],"head":{…,"seq":10},…,"ok":true,"source":"store",…}

$E store export $S --out $S-export      # writes manifest.json and 000001.jsonl; prints the manifest
$E store verify $S-export
# the same report, with "source":"export"

$E store prove $S --seq 3
# {"checkpoint":"sha256:…","entry":{…,"seq":3,…},"leaf_hash":"sha256:…","proof":["sha256:…","sha256:…","sha256:…"],"root":"sha256:…","seq":3,"size":8}
```

The export verifies in a browser too, offline, with the same verifier. Build the WebAssembly module and serve the repository root:

```sh
sh tools/build-wasm.sh                          # needs: rustup target add wasm32-unknown-unknown
echo $S-export                                  # the folder to choose on the page
python3 -m http.server 8000 --bind 127.0.0.1    # from the repository root
```

Then open `http://127.0.0.1:8000/tools/verify-page/` and choose that folder. The page shows the report `store verify` printed for the export; the files are read in the browser and sent nowhere. The page takes exports of up to 400 MB; larger ones need `store verify`. The commands, their output and exit status are in [`crates/evidence-cli/README.md`](crates/evidence-cli/README.md), and the page in [`tools/verify-page/README.md`](tools/verify-page/README.md).

## From JavaScript and Python

The WebAssembly module and the Python package are the same Rust core, so they return byte-for-byte the same results as the command-line tool. Both run every test vector in CI.

```js
// sh tools/build-wasm.sh   (needs: rustup target add wasm32-unknown-unknown)
import { load } from "./crates/evidence-wasm/pkg/evidence.mjs";
const ev = await load();   // in Node: await load("crates/evidence-wasm/pkg/evidence_wasm.wasm")
const report = ev.verify(objectBytes, { attestations, require: ["approve"] });
```

```python
# pip install ./crates/evidence-py
import evidence_layer as ev
report = ev.verify(open("object.json", "rb").read(), require=["approve"],
                   attestations=open("attestations.jsonl", "rb").read())
```

Details: [`crates/evidence-wasm/README.md`](crates/evidence-wasm/README.md) (including the ABI for .NET, Java and other WebAssembly hosts) and [`crates/evidence-py/README.md`](crates/evidence-py/README.md).

## Checks

```sh
cargo fmt --all --check && cargo clippy --all-targets -- -D warnings
cargo test                                      # Rust: every vector, the store, the CLI and a short crash test
sh tools/build-wasm.sh && node crates/evidence-wasm/js/test.mjs       # WebAssembly module
EVIDENCE_BIN=target/debug/evidence node tools/check-proofs.mjs        # the CLI's proofs, checked through the module
pip install ./crates/evidence-py && EVIDENCE_BIN=$PWD/target/debug/evidence python -m unittest discover -s crates/evidence-py/tests   # Python, the CLI's proofs included
node tools/check-vectors.mjs                    # JS reference against the vectors
python tools/check_schema_vectors.py            # JSON Schemas against the vectors (needs jsonschema)
node tools/make-vectors.mjs && git diff --exit-code test-vectors   # vectors are reproducible
node tools/verify-page/smoke.mjs                # the browser page over every export case (needs Playwright)
```

`cargo test` builds `target/debug/evidence`, which `check-proofs.mjs` and the Python tests run; without `EVIDENCE_BIN` the Python proof tests are skipped. The page smoke test needs Playwright and its Chromium ([`tools/verify-page/README.md`](tools/verify-page/README.md)).

CI runs all of these ([`.github/workflows/ci.yml`](.github/workflows/ci.yml)). A nightly run ([`.github/workflows/nightly.yml`](.github/workflows/nightly.yml)) repeats the property test at 4,096 cases and the crash test at 1,000 iterations, and measures appending 1,000,000 objects.

## Status

Scaffold. Milestone M1, the verifiable store, is complete: its acceptance tests pass in CI. The core, the command-line tool, the WebAssembly module and JavaScript wrapper, the Python package, the specs and the vectors work and are tested. The store is a Rust library (a SQLite store, with an in-memory reference), and the command-line tool runs it: init, append, attest, checkpoint, get, heads, log, verify, export, import and prove. Rust, the WebAssembly module, Python, the independent JavaScript reference and a static browser page give the same report, byte for byte, on all 42 export cases. A crash test kills appends and imports at random and checks that every receipt printed is kept and no half-written store is left. Read-only capture (M2) comes next.

## Next steps

The aim is for the layer to become an organisation's memory: a durable, verifiable record that applications read from and, later, write through, with a connector that keeps it reconciled with existing databases. The full plan, with deliverables, acceptance tests and the confirmed decisions, is in [`docs/roadmap.md`](docs/roadmap.md). The first milestone has a build specification: [`docs/m1-store-spec.md`](docs/m1-store-spec.md).

1. **Verifiable store (M1), built.** An append-only log with its own position and received time for every entry, signed checkpoints over a Merkle tree, and exports that anyone can verify offline, including in a browser.
2. **Read-only capture (M2).** Turn every change in an existing database into evidence, using read access only and no schema change. Capture refuses to start until every column is classified for privacy.
3. **Two-way reconciliation (M3).** Compare the store and the database on a schedule, in both directions, and record every difference as evidence, including edits made outside the normal process. Report only; nothing is repaired automatically.
4. **Memory reads (M4).** Ask what a record said on any date, under which schema, who changed it and why, through a query API whose answers can be verified.
5. **Pilot gate (M5).** Keys bound to people and organisations, trusted timestamps (RFC 3161), copies held off the machine, privacy operations, and legal and security review before any real data.
6. **Change requests in shadow mode (M6).** Applications hand over requests instead of changing the database; requests and their outcomes are recorded as evidence and compared with what staff actually did. No write code.
7. **Write-through to a test database (M7).** Requests are delivered through the database's own interface, and a change counts as done only when it is read back. Off by default; writing to any real system is a separate decision for its owner.
8. **Optional:** Postgres and vendor-platform capture; the `centr/0` profile ([notes](docs/centr-profile-notes.md)), public registers as views, and prime coordinates (below).

Alongside: Python wheels for Windows and macOS, npm and PyPI releases, and short .NET (Wasmtime) and Java (Chicory) host examples for the WebAssembly module.

### Proposed: prime coordinates for faceted queries

**What.** A derived index number for each object. Each categorical facet gets its own prime number from one published dictionary: record type, product class, standard family, actor role, jurisdiction and so on. An object's coordinate is the product of the primes for the facets it has. For example, if installation record = 3, inverter = 5, AS/NZS 4777.2 = 7 and Victoria = 11, an installation record for an inverter under AS/NZS 4777.2 in Victoria has coordinate 3 × 5 × 7 × 11 = 1155.

**Why.**
- **Lossless.** By unique factorisation, a coordinate decodes back to exactly one set of facets.
- **Queries become arithmetic.** An object has a facet if that facet's prime divides its coordinate. It has all of a set of facets if their product divides it. "Every installation record in Victoria under AS/NZS 4777.2" becomes a divisibility test on one integer field, which any database or language can run.
- **Collision-free across domains.** Namespaces that use disjoint prime ranges can share one index: no product of one range's primes can equal a product of another's.
- **Portable.** Coordinates are plain integers, written as decimal strings so they stay exact in JSON and JavaScript at any size. A product of a handful of primes soon passes 2^53, where JSON numbers stop being exact.

A draft profile that applies this to certificate lifecycles is in [`docs/kernel-2.md`](docs/kernel-2.md).

**Proposed use** (roadmap M9). Public registers and cross-agency indexes. The certified product list, the installer register, or a search across several systems of record becomes a filter on one field rather than a join across schemas.

**Conditions:**
- **Derived, never authoritative.** The verifier computes the coordinate from the object and the profile's dictionary. It is never part of the id, and readers recompute it rather than trust a stored value.
- **One dictionary per namespace.** Published, versioned, and the only source; schemas and code are generated from it, not hand-copied.
- **Unknown facets are errors,** never silently skipped.
- **At most one prime per single-valued facet.** Multi-valued facets need an explicit rule.
- **A shared registry of prime ranges,** so a new namespace cannot overlap another prime-based scheme.
- **Carried as decimal strings,** never as JSON numbers, so coordinates of any size survive JSON.

## Licence

[Apache License 2.0](LICENSE). It is permissive, so an agency, its vendors and other regulators can adopt the spec and code with no obligation beyond keeping the notices. It also includes an explicit patent licence from contributors, which matters for a format that others are meant to implement.
