# evidence/0

Status: draft for discussion. Version tag `evidence/0`; breaking changes bump the number.

## Purpose

An evidence object proves five things about a record or a decision that already lives in some other system:

1. what it said (a hash of its content, and optionally the content itself)
2. under which definition (a hash of the schema, DDL or data dictionary the system of record already uses)
3. who wrote it (a person, a service, or an AI agent with a named accountable sponsor)
4. when
5. which earlier version(s) it replaces

It does not model the domain. Products, installers, installations and decisions stay in the system of record and its schema. The evidence object points at them. That keeps it small enough to adopt beside an existing system without changing it, and general enough to reuse across regulatory functions.

## The object

```json
{
  "v": "evidence/0",
  "id": "sha256:…",
  "op": "update",
  "schema":  { "ref": "rec-registry/installation@12", "hash": "sha256:…" },
  "subject": { "system": "rec-registry", "type": "installation", "key": "INS-TEST-000001" },
  "content": { "media_type": "application/json", "hash": "sha256:…", "inline": { "…": "…" } },
  "actor":   { "id": "service:rec-registry-cdc", "kind": "service" },
  "at": "2026-11-21T01:00:00Z",
  "prev": ["sha256:…"]
}
```

The names in this example (`rec-registry`, the version `@12` and the capture service `rec-registry-cdc`) are synthetic placeholders, not a real system's schema, version or interface.

| Field | Required | Meaning |
|---|---|---|
| `v` | yes | Always `"evidence/0"`. |
| `id` | yes | `sha256:` + hex SHA-256 of the canonical object **without** `id`. |
| `op` | yes | `create`, `update` or `delete` for a record change. `assert` for a statement about a subject that is not a record change: a decision, a check result, an AI output. For `delete`, `content` is the record's last state. |
| `schema.ref` | yes | The system of record's own name and version for the definition. |
| `schema.hash` | yes | SHA-256 of the definition document's bytes as published. A reader (person or AI) can fetch the exact definition and interpret the record without guessing. |
| `subject` | yes | `system`, `type` and `key` of the record in the system of record. |
| `content.media_type` | yes | Lowercase `type/subtype`. |
| `content.hash` | yes | For `application/json`: SHA-256 of the record's canonical form. Otherwise: SHA-256 of the bytes. |
| `content.uri` | no | Where the bytes can be fetched. |
| `content.inline` | no | The record itself, for small JSON records. Requires `application/json`. Leave it out for personal information and keep only the hash (see Privacy). |
| `actor.id` | yes | Identifier of the writer. |
| `actor.kind` | yes | `human`, `service` or `agent`. |
| `actor.on_behalf_of` | for agents | The person or body accountable for an agent's output. |
| `at` | yes | UTC, `YYYY-MM-DDTHH:MM:SS[.fraction]Z`, and a real calendar instant. |
| `prev` | yes | Ids this object replaces. Empty for a first version. A list, so histories can merge. |
| `ext` | no | Profile data keyed by profile id such as `centr/0`. Covered by `id`. |

Unknown members are rejected. The JSON Schema is [`schema/evidence-object-0.schema.json`](schema/evidence-object-0.schema.json).

## Parsing and canonical form

Objects are JSON, parsed under rules stricter than most libraries, because two parsers that read the same bytes differently would compute different ids:

- UTF-8, no byte order mark;
- duplicate member names are an error (`E_DUPLICATE_KEY`), even when one is spelled with an escape;
- strings must be valid Unicode: lone surrogates are an error;
- numbers are IEEE 754 doubles, and any number with magnitude above 2^53 − 1 is an error (`E_NUMBER_RANGE`). Identifiers, big integers and money go in strings. Because of this rule, every canonical form parses again under the same rules.

The canonical form is [RFC 8785](https://www.rfc-editor.org/rfc/rfc8785) (JSON Canonicalization Scheme): no whitespace, members sorted by UTF-16 code units, strings and numbers serialised as ECMAScript `JSON.stringify` does. Formatting in a file does not matter; only the canonical bytes are hashed.

## Attestations

Signatures live outside the object, one JSON object per line in a `.jsonl` file, so an object can collect approvals and witnesses without its id changing.

```json
{"at":"2026-11-21T03:05:00Z","key":"ed25519:…","object":"sha256:…","purpose":"approve","sig":"…","signer":"person:assessor-01","v":"attestation/0"}
```

- `purpose`: `author` (I made this), `approve` (I accept this as the accountable person or body), `witness` (I saw this at this time).
- `key`: Ed25519 public key, `ed25519:` + 32 bytes of unpadded base64url.
- `sig`: Ed25519 signature (64 bytes, unpadded base64url) over the canonical form of the attestation **without** `sig`. The signature covers the object id, purpose, signer and time.
- Base64url must use its one canonical spelling. Signatures are checked strictly: malleable signatures and weak keys are rejected.

An attestation proves that the holder of `key` signed. Whether `key` belongs to `signer` is a trust-policy question for v1 (see Not in v0). The JSON Schema is [`schema/attestation-0.schema.json`](schema/attestation-0.schema.json).

## Verification

Checks run in this order and stop at the first failure. Every implementation reports the same code for the same input.

| Step | Check | Code |
|---|---|---|
| 1 | Valid strict JSON | `E_PARSE`, `E_DUPLICATE_KEY`, `E_NUMBER_RANGE` |
| 2 | Matches the object schema | `E_SHAPE` |
| 3 | `at` is a real calendar instant | `E_TIMESTAMP` |
| 4 | `id` equals the computed id | `E_ID_MISMATCH` |
| 5 | `content.inline`, if present, hashes to `content.hash` | `E_CONTENT_HASH_MISMATCH` |
| 6 | With a store: each `prev` is in it and is not dated after `at` | `E_PREV_MISSING`, `E_PREV_ORDER` |
| 7 | Each attestation line is well formed; each line for this id has a valid signature (lines for other ids are ignored) | `E_ATTESTATION_SHAPE`, `E_SIGNATURE_INVALID` |
| 8 | Every required purpose has a valid attestation | `E_ATTESTATION_MISSING` |

Programming interfaces also use `E_USAGE` for invalid arguments, such as a malformed seed, an unknown purpose or an invalid store object. It is never a verification result.

When content is held elsewhere (`uri`, no `inline`), step 5 is skipped and the report says `content_checked: false`. Fetching and checking external bytes is the caller's job in v0.

## Profiles

A profile adds rules for one domain under `ext["<name>/<version>"]`, for example `ext["centr/0"]`. The core hashes profile data but does not interpret it. A profile verifier runs after the core verifier. See [`../docs/centr-profile-notes.md`](../docs/centr-profile-notes.md).

## Privacy

- `content.inline` copies the record into the object. Do not inline personal information; keep only `content.hash` and keep the bytes where access is already controlled.
- A bare hash of a guessable value (an address, a name, a licence number) does not hide it: anyone can hash candidates and compare. A profile that needs to link records on such values should use a keyed hash (HMAC) whose key stays with the data owner.

## Not in v0

Deliberately left out so v0 stays small:

- **Trust policy:** binding keys to people and organisations (a key registry, DIDs or a PKI), and key rotation and revocation.
- **Trusted time:** RFC 3161 timestamp tokens. In v0, `at` is the writer's claim, and a `witness` attestation by a timestamping service is the stand-in.
- **Status:** whether an object is current, superseded, suspended or revoked. That is a view computed over `prev` links and later objects, not a field.
- **Domain gates, prime coordinates and registers.** These belong in profiles and query layers, once a use needs them.

## Conformance

An implementation conforms to evidence/0 if it gives the expected result for every case in `canon.json`, `numbers.json` and `objects/` in [`../test-vectors/`](../test-vectors/). The vectors are the contract; the Rust and JavaScript code in this repository are two implementations of it. [store/0](store-0.md#conformance) and [coordinate/0](coordinate-0.md#conformance) state their own conformance.
