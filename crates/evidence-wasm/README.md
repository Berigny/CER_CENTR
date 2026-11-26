# evidence-wasm

The evidence/0 core compiled to one WebAssembly module, plus a JavaScript wrapper for browsers and Node. The module has no imports and the same file works in any WebAssembly host, so .NET (Wasmtime), Java (Chicory), Go or Python hosts can call the exact code the command-line tool runs.

## Build

```sh
rustup target add wasm32-unknown-unknown   # once
sh tools/build-wasm.sh                      # writes crates/evidence-wasm/pkg/
node crates/evidence-wasm/js/test.mjs       # every test vector through the module
```

`pkg/` holds `evidence_wasm.wasm` (about 300 KB), `evidence.mjs`, `evidence.d.ts`, `package.json` and `LICENSE`. CI uploads it as the `evidence-wasm-pkg` artifact.

## JavaScript

```js
import { load, Evidence } from "./pkg/evidence.mjs";

const ev = await load();   // fetches evidence_wasm.wasm beside evidence.mjs; in Node, pass a path
const report = ev.verify(objectBytes, { attestations, require: ["approve"] });
if (!report.ok) console.log(report.code, report.detail);

const seedHex = Evidence.generateSeed();   // development only
const line = ev.sign(objectBytes, { seedHex, signer: "person:me", purpose: "approve" });
```

Methods: `canonicalize`, `contentHash`, `fileHash`, `computeId`, `seal`, `verify`, `sign`, `publicKey`, `version`. Inputs may be strings or bytes. Pass bytes when reading files, so a file that is not valid UTF-8 fails with `E_PARSE` instead of being silently repaired by a text decoder. Errors throw `EvidenceError` with `.code` and `.detail`.

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

Byte inputs (`input`, `attestations`, `store` items) go either as strings under their own name, or as standard base64 under `<name>_base64` to keep exact bytes. WebAssembly has no clock, so `sign` needs `at`. `E_USAGE` means the request itself was invalid.

A host binding is about 30 lines: allocate, copy the request in, call, read the length and bytes, free both buffers. See `js/evidence.mjs` (`#call`).
