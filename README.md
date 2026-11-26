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
| [`crates/evidence-cli/`](crates/evidence-cli/) | `evidence` command-line tool |
| [`crates/evidence-wasm/`](crates/evidence-wasm/) | The core as one WebAssembly module, with a JavaScript wrapper for browsers and Node, and a small ABI for other hosts |
| [`crates/evidence-py/`](crates/evidence-py/) | Python package `evidence_layer`, built from the same core |
| [`tools/reference/evidence.mjs`](tools/reference/evidence.mjs) | Independent JavaScript reference, used to generate and cross-check the vectors |
| [`docs/decisions.md`](docs/decisions.md) | Design decisions and why |

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
cargo test                                      # Rust core against every vector
sh tools/build-wasm.sh && node crates/evidence-wasm/js/test.mjs       # WebAssembly module
pip install ./crates/evidence-py && python -m unittest discover -s crates/evidence-py/tests   # Python
node tools/check-vectors.mjs                    # JS reference against the vectors
python tools/check_schema_vectors.py            # JSON Schemas against the vectors (needs jsonschema)
node tools/make-vectors.mjs && git diff --exit-code test-vectors   # vectors are reproducible
```

CI runs all of these ([`.github/workflows/ci.yml`](.github/workflows/ci.yml)).

## Status

Scaffold. The core, the command-line tool, the WebAssembly module and JavaScript wrapper, the Python package, the spec and the vectors work and are tested.

## Next steps

1. **Packaging for other stacks.** The WebAssembly module and Python package are built and tested. Still to do: wheels for Windows and macOS (maturin in CI), npm and PyPI releases, and short host examples for .NET (Wasmtime) and Java (Chicory) using the same `.wasm` file.
2. **Capture adapter.** An example that reads changes from an existing database, through change-data-capture or an outbox table, and writes evidence objects without touching the source schema. This shows the "beside, not instead" model on real infrastructure.
3. **Store.** Append-only storage for objects and attestations, indexed by id and by subject, so `prev` links and histories can be followed.
4. **Trust policy.** Bind signing keys to people and organisations (a key registry, DIDs or a PKI), with rotation and revocation, and keep production keys in an HSM or key service.
5. **Trusted time.** RFC 3161 timestamp tokens, recorded as `witness` attestations.
6. **`centr/0` profile.** Domain rules for the Consumer Energy National Technical Regulator, with their own vectors ([notes](docs/centr-profile-notes.md)).
7. **Prime coordinates** (proposed; see below).

### Proposed: prime coordinates for faceted queries

**What.** A derived index number for each object. Each categorical facet gets its own prime number from one published dictionary: record type, product class, standard family, actor role, jurisdiction and so on. An object's coordinate is the product of the primes for the facets it has. For example, if installation record = 3, inverter = 5, AS/NZS 4777.2 = 7 and Victoria = 11, an installation record for an inverter under AS/NZS 4777.2 in Victoria has coordinate 3 × 5 × 7 × 11 = 1155.

**Why.**
- **Lossless.** By unique factorisation, a coordinate decodes back to exactly one set of facets.
- **Queries become arithmetic.** An object has a facet if that facet's prime divides its coordinate. It has all of a set of facets if their product divides it. "Every installation record in Victoria under AS/NZS 4777.2" becomes a divisibility test on one integer field, which any database or language can run.
- **Collision-free across domains.** Namespaces that use disjoint prime ranges can share one index: no product of one range's primes can equal a product of another's.
- **Portable.** Coordinates are plain integers. Kept below 2^53, they are exact in JSON and JavaScript.

**Proposed use.** Public registers and cross-agency indexes. The certified product list, the installer register, or a search across several systems of record becomes a filter on one field rather than a join across schemas.

**Conditions**, learned from an earlier draft where they were missing:
- **Derived, never authoritative.** The verifier computes the coordinate from the object and the profile's dictionary. It is never part of the id, and readers recompute it rather than trust a stored value.
- **One dictionary per namespace.** Published, versioned, and the only source; schemas and code are generated from it, not hand-copied.
- **Unknown facets are errors,** never silently skipped.
- **At most one prime per single-valued facet.** Multi-valued facets need an explicit rule.
- **A shared registry of prime ranges,** so a new namespace cannot overlap another prime-based scheme.
- **Kept below 2^53,** or carried as strings, so coordinates survive JSON.

## Licence

[Apache License 2.0](LICENSE). It is permissive, so an agency, its vendors and other regulators can adopt the spec and code with no obligation beyond keeping the notices. It also includes an explicit patent licence from contributors, which matters for a format that others are meant to implement.
