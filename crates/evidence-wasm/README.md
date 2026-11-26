# evidence-wasm

The evidence/0 core and the store/0 verifier compiled to one WebAssembly module, plus a JavaScript wrapper for browsers and Node. The module has no imports and the same file works in any WebAssembly host, so .NET (Wasmtime), Java (Chicory), Go or Python hosts can call the exact code the command-line tool runs.

## Build

```sh
rustup target add wasm32-unknown-unknown   # once
sh tools/build-wasm.sh                      # writes crates/evidence-wasm/pkg/
node crates/evidence-wasm/js/test.mjs       # every test vector through the module, store vectors included
```

`pkg/` holds `evidence_wasm.wasm` (about 400 KB), `evidence.mjs`, `evidence.d.ts`, `package.json` and `LICENSE`. CI uploads it as the `evidence-wasm-pkg` artifact.

## JavaScript

```js
import { load, Evidence } from "./pkg/evidence.mjs";

const ev = await load();   // fetches evidence_wasm.wasm beside evidence.mjs; in Node, pass a path
const report = ev.verify(objectBytes, { attestations, require: ["approve"] });
if (!report.ok) console.log(report.code, report.detail);

const seedHex = Evidence.generateSeed();   // development only
const line = ev.sign(objectBytes, { seedHex, signer: "person:me", purpose: "approve" });

// A store export: manifest.json and the segment files, in manifest order.
const storeReport = ev.verifyExport(manifestBytes, [segment1, segment2], { maxFindings: 1000 });
for (const f of storeReport.findings) console.log(f.code, f.seq);
```

Methods: `canonicalize`, `contentHash`, `fileHash`, `computeId`, `seal`, `verify`, `sign`, `publicKey`, `version`, and for stores (`spec/store-0.md`) `entryHash(input)`, `merkleRoot(leaves)`, `verifyInclusion(leafHash, index, size, proof, root)`, `verifyConsistency(oldSize, newSize, oldRoot, newRoot, proof)` and `verifyExport(manifest, segments, { maxFindings })`. Inputs may be strings or bytes; `leaves`, `segments` and `store` are arrays of them, and anything else there (a lone string or byte array included) is `E_USAGE`. Pass bytes when reading files, so a file that is not valid UTF-8 fails with `E_PARSE` instead of being silently repaired by a text decoder. Errors throw `EvidenceError` with `.code` and `.detail`.

## The ABI, for other hosts

Three exports plus `memory`:

| Export | Signature | Use |
|---|---|---|
| `evidence_alloc` | `(len: i32) -> i32` | Reserve `len` bytes; write the request there. |
| `evidence_call` | `(ptr: i32, len: i32) -> i32` | Run the request. Returns a pointer to a 4-byte little-endian length followed by that many bytes of UTF-8 JSON. |
| `evidence_free` | `(ptr: i32, len: i32)` | Release the request (`len`) and the result (`4 + length`). |

Request: `{"fn": "<name>", "args": {...}}`. Response: `{"result": ...}` or `{"error": {"code": "E_...", "detail": "..."}}`.

| `fn` | `args` | `result` |
|---|---|---|
| `version` | | `{"spec", "crate"}` |
| `canonicalize`, `content_hash`, `file_hash`, `compute_id`, `seal` | `input` | string |
| `verify` | `input`, optional `attestations`, `store` (array), `require` (array of purposes) | report object |
| `sign` | `input`, `seed_hex`, `signer`, `purpose`, `at` | attestation line |
| `public_key` | `seed_hex` | string |
| `entry_hash` | `input` (log-entry/0 JSON) | entry hash, `"sha256:…"` |
| `merkle_root` | `leaves` (array of leaf bytes) | tree hash, `"sha256:…"` |
| `verify_inclusion` | `leaf_hash`, `index` (seq − 1), `size`, `proof` (array), `root` | bool |
| `verify_consistency` | `old_size`, `new_size`, `old_root`, `new_root`, `proof` (array) | bool |
| `verify_export` | `manifest`, `segments` (array, manifest order), optional `max_findings` (default 1000) | store-report/0 object |

Byte inputs (`input`, `attestations`, `store` items, `leaves` items, `manifest`, `segments` items) go either as strings under their own name, or as standard base64 under `<name>_base64` to keep exact bytes; if both are given, `<name>` wins. Hashes (`leaf_hash`, roots, proof elements) are always text, `"sha256:"` and 64 lowercase hex digits. Sizes and indexes are integers from 0 to 2^53 − 1, `max_findings` from 1 to 2^53 − 1. WebAssembly has no clock, so `sign` needs `at`.

`E_USAGE` means the request itself was invalid: a missing or mistyped argument, a malformed hash, an integer out of range. For `entry_hash`, an input that is not a valid entry is `E_USAGE` whose detail begins with `S_ENTRY_INVALID`. Verification outcomes are never errors: proof checks answer `false`, and `verify_export` returns a report however damaged the export is. `spec/store-0.md` ("Programming interfaces") is the normative description of the store functions.

A host binding is about 30 lines: allocate, copy the request in, call, read the length and bytes, free both buffers. See `js/evidence.mjs` (`#call`).
