# CER_CENTR: an evidence layer for regulatory records

A small, portable **evidence object** that sits beside an existing system of record. For each record change or decision it proves what was recorded, under which schema version, who recorded it (a person, a service, or an AI agent and the person accountable for it), when, and which earlier version it replaces.

It does not replace any database, schema or register. It reads from them and never writes back. Each object points at the system of record's own schema by hash, so people and AI tools can interpret a record exactly, and anyone with the object can check it without access to the source database.

Third parties can reproduce both the evidence and the exact results of faceted queries over it: the same dictionary version, watermark and query give the same rows in any implementation. A coordinate decides which records a query returns, not whether a record was classified correctly or captured completely.

> Not an official publication of, or endorsed by, the Clean Energy Regulator or any other government body. It draws only on publicly available information about the programs it mentions, and all data in this repository is synthetic.

## What it is for

- **Audit trails** for decisions and for the service and AI outputs they rely on, with who approved each.
- **Data shared with other bodies** that the receiver can verify without access to the source system.
- **Chain of custody** for records that may later be relied on, showing each version in order and who signed it.
- **Records of AI-assisted checks:** what the model was given, what it returned, and who approved or overrode it.
- **Public registers as views:** a published list is an evidence object recording a query result, so "what did the list show on date X?" has a provable answer.
- **Reproducible catalogue queries:** a faceted query such as "every installation record in Victoria under AS/NZS 4777.2" has one exact meaning, so anyone holding the records, the profile's rules for which facets a record has, the dictionary and the watermark gets the same rows ([prime coordinates](#prime-coordinates-for-faceted-queries)).

The first domain profile in view, as an example drawn from public information, is CENTR, the proposed Consumer Energy National Technical Regulator ([public description](https://cer.gov.au/national-technical-regulator); [notes](docs/centr-profile-notes.md)). The core is deliberately domain-neutral.

## Layout

| Path | What |
|---|---|
| [`spec/evidence-0.md`](spec/evidence-0.md) | The one-page spec: object, canonical form, attestations, verification order and error codes |
| [`spec/schema/`](spec/schema/) | JSON Schemas for the object, attestations, the store/0 formats and coordinate/0 dictionaries |
| [`test-vectors/`](test-vectors/) | The conformance contract: canonicalisation, number formatting, object, store and coordinate cases with expected results |
| [`crates/evidence-core/`](crates/evidence-core/) | Rust core: strict JSON parser, RFC 8785 canonicalisation, ids, Ed25519 attestations, verifier |
| [`crates/evidence-cli/`](crates/evidence-cli/) | `evidence` command-line tool: objects, and `evidence store` to create, append to, verify, export, import and prove stores |
| [`crates/evidence-wasm/`](crates/evidence-wasm/) | The core as one WebAssembly module, with a JavaScript wrapper for browsers and Node, and a small ABI for other hosts |
| [`crates/evidence-py/`](crates/evidence-py/) | Python package `evidence_layer`, built from the same core |
| [`tools/reference/`](tools/reference/) | Independent JavaScript reference for objects, stores and coordinates, used to generate and cross-check the vectors |
| [`spec/store-0.md`](spec/store-0.md) | Store spec: log entries, checkpoints, exports and their verification (milestone M1) |
| [`crates/evidence-log/`](crates/evidence-log/) | Pure store formats and the export verifier: log entries, RFC 9162 tree and proofs, genesis, checkpoints, store reports (M1) |
| [`crates/evidence-store/`](crates/evidence-store/) | The store: an append-only log in one SQLite file (and an in-memory reference), writer lease, signed checkpoints, export, import, live verification and Merkle proofs (M1) |
| [`tools/verify-page/`](tools/verify-page/) | A static page that verifies a store export in the browser, offline (M1) |
| [`spec/coordinate-0.md`](spec/coordinate-0.md) | Facet coordinates spec (coordinate/0): dictionaries and their ids, the coordinate as a decimal string, encode, decode, divides, check and extends, their error codes, and rules for indexes. A separate format outside the core |
| [`crates/evidence-coordinate/`](crates/evidence-coordinate/) | Rust implementation of coordinate/0: dictionaries, the five operations, and coordinates up to 1,024 bits. Pure: no I/O |
| [`docs/decisions.md`](docs/decisions.md) | Design decisions and why |
| [`docs/turn-reader.md`](docs/turn-reader.md) | Design note: a reader that tools and AI agents call before they rely on evidence, and when a read must be recorded (input to M4, M6 and M9; not built) |
| [`docs/approvals.md`](docs/approvals.md) | Design note: approver lists and approval rules as published data, an approval-status view, disagreement resolved by a person, pseudonymous actor ids, and hooks that let any tool, AI agents included, act under a grant and keep continuity notes (input to M4, M5 and M6; not built) |
| [`docs/cer-edge.md`](docs/cer-edge.md) | Draft profile for edge-device records in consumer energy resources, for the roadmap's edge track EG1-EG4 (synthetic data only; not built) |
| [`docs/analytics.md`](docs/analytics.md) | Design note: trends and anomaly flags computed from the log as known at a stated position, with published detector rules, flags routed to a person for review, and no scores of people or households (input to M3, M4 and M5, and the roadmap's proposed analytics track AN1-AN3; not built) |

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

The WebAssembly module and the Python package are the same Rust core, so they return byte-for-byte the same results as the command-line tool. Both run the canonicalisation, object and store/0 vectors in CI. coordinate/0 is not in the bindings yet: its vectors, like the number-formatting ones, are checked by the Rust crates and the JavaScript reference.

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
node tools/make-vectors.mjs && git diff --exit-code test-vectors && test -z "$(git status --porcelain -- test-vectors)"   # vectors are reproducible, none added
node tools/verify-page/smoke.mjs                # the browser page over every export case (needs Playwright)
```

`cargo test` builds `target/debug/evidence`, which `check-proofs.mjs` and the Python tests run; without `EVIDENCE_BIN` the Python proof tests are skipped. The page smoke test needs Playwright and its Chromium ([`tools/verify-page/README.md`](tools/verify-page/README.md)).

CI runs all of these ([`.github/workflows/ci.yml`](.github/workflows/ci.yml)). A nightly run ([`.github/workflows/nightly.yml`](.github/workflows/nightly.yml)) repeats the property test at 4,096 cases and the crash test at 1,000 iterations, and measures appending 1,000,000 objects.

## Status

Scaffold. Milestone M1, the verifiable store, is complete: its acceptance tests pass in CI. The core, the command-line tool, the WebAssembly module and JavaScript wrapper, the Python package, the specs and the vectors work and are tested. The store is a Rust library (a SQLite store, with an in-memory reference), and the command-line tool runs it: init, append, attest, checkpoint, get, heads, log, verify, export, import and prove. Rust, the WebAssembly module, Python, the independent JavaScript reference and a static browser page give the same report, byte for byte, on all 42 export cases. A crash test kills appends and imports at random and checks that every receipt printed is kept and no half-written store is left. Ahead of M4, facet coordinates ([coordinate/0](spec/coordinate-0.md)) are specified and built, in Rust and in the independent JavaScript reference, with vectors on synthetic dictionaries. Read-only capture (M2) comes next.

## Next steps

The aim is for the layer to become an organisation's memory: a durable, verifiable record that applications read from and, later, write through, with a connector that keeps it reconciled with existing databases. The full plan, with deliverables, acceptance tests and the confirmed decisions, is in [`docs/roadmap.md`](docs/roadmap.md). The first milestone has a build specification: [`docs/m1-store-spec.md`](docs/m1-store-spec.md).

1. **Verifiable store (M1), built.** An append-only log with its own position and received time for every entry, signed checkpoints over a Merkle tree, and exports that anyone can verify offline, including in a browser.
2. **Read-only capture (M2).** Turn every change in an existing database into evidence, using read access only and no schema change. Capture refuses to start until every column is classified for privacy.
3. **Two-way reconciliation (M3).** Compare the store and the database on a schedule, in both directions, and record every difference as evidence, including edits made outside the normal process. Report only; nothing is repaired automatically.
4. **Memory reads (M4).** Ask what a record said on any date, under which schema, who changed it and why, through a query API whose answers can be verified. M4b, planned, adds a coordinate column and a factor table on [coordinate/0](spec/coordinate-0.md), which is already built, so that faceted queries give the same rows in any implementation (below).
5. **Real-data gate (M5).** Keys bound to people and organisations, trusted timestamps (RFC 3161), copies held off the machine, privacy operations, and legal and security review before any real data.
6. **Change requests in shadow mode (M6).** Applications hand over requests instead of changing the database; requests and their outcomes are recorded as evidence and compared with what staff actually did. No write code.
7. **Write-through to a test database (M7).** Requests are delivered through the database's own interface, and a change counts as done only when it is read back. Off by default; writing to any real system is a separate decision for its owner.
8. **Optional:** Postgres and vendor-platform capture; the `centr/0` profile ([notes](docs/centr-profile-notes.md)), public registers as views, and a real published dictionary for coordinates.

Alongside: Python wheels for Windows and macOS, npm and PyPI releases, and short .NET (Wasmtime), Java (Chicory) and Go (wazero) host examples for the WebAssembly module.

### Prime coordinates for faceted queries

**What.** A derived index number for each object. Each categorical facet gets its own prime number from one published dictionary: record type, product class, standard family, actor role, jurisdiction and so on. An object's coordinate is the product of the primes for the facets it has. For example, if installation record = 3, inverter = 5, AS/NZS 4777.2 = 7 and Victoria = 11, an installation record for an inverter under AS/NZS 4777.2 in Victoria has coordinate 3 × 5 × 7 × 11 = 1155. The arithmetic is long established: it is the idea behind Gödel numbering.

**Status.** [coordinate/0](spec/coordinate-0.md) specifies how a list of facets becomes a coordinate and back, and it is built: a Rust crate ([`crates/evidence-coordinate/`](crates/evidence-coordinate/)), an independent JavaScript implementation ([`tools/reference/coordinate.mjs`](tools/reference/coordinate.mjs)) and vectors ([`test-vectors/coordinate/`](test-vectors/coordinate/)) on synthetic dictionaries. The primes in the example are those of the spec's synthetic `catalogue-demo` dictionary, which is not a published dictionary. coordinate/0 is a separate format outside the core, with its own vectors ([D6](docs/decisions.md#d6-one-small-core-many-profiles)), and changes nothing in evidence/0 or store/0. The coordinate column and factor table in M4's index are planned as M4b ([roadmap](docs/roadmap.md#m4b-coordinate-column-and-factor-table)); a real published dictionary waits for M9.

**Why.**
- **Lossless.** By unique factorisation, a coordinate decodes back to exactly its facets, with the number of times each occurs.
- **One exact meaning for every query.** An object has a facet if that facet's prime divides its coordinate, and all of a set of facets if their product divides it. "Every installation record in Victoria under AS/NZS 4777.2" is the query 231 (3 × 7 × 11), and a record matches when 231 divides its coordinate. Divisibility is the reference meaning of a query, the same in every implementation.
- **Databases answer from factor rows.** The coordinate defines what a query means; a database answers it from a table of factors, one row (profile, object, prime, exponent) for each prime of each object, indexed on (profile, prime): "has all these primes, each with at least these exponents". SQL integers stop at 64 bits, and coordinates pass that quickly. SQLite silently gives wrong remainders on larger values stored as text: on SQLite 3.53, the product of the first 16 primes, a 65-bit number, gave 33 for `% 53` instead of 0, and no error was raised. PostgreSQL's `numeric` takes `mod` exactly at any size coordinate/0 allows. M4b's acceptance test will check that the factor rows return exactly the rows that divisibility gives, including coordinates past 2^63.
- **Not a speed device.** Testing the divisibility of every stored coordinate reads every row, so it does not make retrieval faster. Speed comes from the indexed factor rows; divisibility is the check on them.
- **Namespaces stay apart by tag.** Every coordinate carries its dictionary's profile id and indexes key on (profile, coordinate), so dictionaries that reuse the same primes cannot be confused ([Kernel/2](docs/kernel-2.md)). Namespaces that also choose disjoint prime ranges get a further guarantee: no product of one or more of one range's primes can equal a product of one or more of another's.
- **Portable.** Coordinates are plain integers, written as canonical decimal strings so they stay exact in JSON, JavaScript and SQL text columns at any size coordinate/0 allows (below 2^1024). A product of a handful of primes soon passes 2^53, where JSON numbers stop being exact.

**Boundary.** A coordinate decides which records a query returns, not whether a record was classified correctly or captured completely. Which facets a record has is the profile's rule. There is no ranking or similarity: a record matches a query or it does not.

A draft profile that applies this to certificate lifecycles, using coordinate/0's encoding, is in [`docs/kernel-2.md`](docs/kernel-2.md).

**Use** (roadmap M4b, then M9). Faceted filters in M4's index, first on a synthetic dictionary, then public registers and cross-agency indexes. The certified product list, the installer register, or a search across several systems of record becomes a filter on facets rather than a join across schemas.

**Conditions** (the full rules are in [coordinate/0](spec/coordinate-0.md)):
- **Derived, never authoritative.** A coordinate is computed from the object, the profile's rules for which facets it has, and the profile's dictionary. It is never part of the id, and readers recompute it rather than trust a stored value.
- **One dictionary per namespace.** Published, versioned by content (its id names each version), and the only source; schemas and code are generated from it, not hand-copied. A later version may only add families and facets ([versioning](spec/coordinate-0.md#versioning)).
- **Unknown facets are errors,** never silently skipped, and so is a coordinate with a factor outside the dictionary.
- **Every family has an explicit rule:** at most one of its facets (`one`), distinct facets (`set`), or facets that may repeat (`multiset`).
- **Tagged with their profile id and dictionary id,** with indexes keyed on (profile, coordinate) ([tagged form](spec/coordinate-0.md#tagged-form)). A shared registry of prime ranges is optional, for namespaces that also want disjoint ranges.
- **Carried as decimal strings,** never as JSON numbers or SQL integers, and answered from factor rows ([indexing](spec/coordinate-0.md#indexing)).
- **Never personal.** A facet never identifies a person, and actor ids and approval status are never facets. A coordinate hides nothing: anyone holding the dictionary decodes it ([privacy](spec/coordinate-0.md#privacy)).

## Licence

[Apache License 2.0](LICENSE). It is permissive, so anyone, public bodies and their suppliers included, can adopt the spec and code. Anyone who redistributes them must include a copy of the licence, keep the notices and mark the files they changed. It also includes an explicit patent licence from contributors, which matters for a format that others are meant to implement.
