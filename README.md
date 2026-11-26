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

## Checks

```sh
cargo fmt --all --check && cargo clippy --all-targets -- -D warnings
cargo test                                      # Rust core against every vector
node tools/check-vectors.mjs                    # JS reference against the vectors
python tools/check_schema_vectors.py            # JSON Schemas against the vectors (needs jsonschema)
node tools/make-vectors.mjs && git diff --exit-code test-vectors   # vectors are reproducible
```

CI runs all of these ([`.github/workflows/ci.yml`](.github/workflows/ci.yml)).

## Status and next steps

Scaffold. The core, the command-line tool, the spec and the vectors work and are tested. Not yet built:

- WebAssembly and Python bindings, so other stacks call the same core
- Trust policy: binding keys to people and organisations, rotation and revocation
- Trusted timestamps (RFC 3161)
- A store backend and an example capture adapter (change-data-capture or outbox) for an existing database
- The `centr/0` profile, with its own vectors

No licence has been chosen yet. Choose one before sharing outside the team.
