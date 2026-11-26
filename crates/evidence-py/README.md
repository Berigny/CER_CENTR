# evidence-layer (Python)

Python binding to the evidence/0 Rust core. Results are byte-for-byte the same as the `evidence` command-line tool and the WebAssembly module.

```sh
pip install ./crates/evidence-py        # builds the Rust extension (needs a Rust toolchain)
```

```python
import evidence_layer as ev

obj = open("object.json", "rb").read()
report = ev.verify(obj, attestations=open("attestations.jsonl", "rb").read(), require=["approve"])
print(report["ok"], report.get("code"))

seed = ev.generate_seed()                       # development only
line = ev.sign(obj, seed_hex=seed, signer="person:me", purpose="approve")
```

| Function | Returns |
|---|---|
| `canonicalize(data)` | canonical JSON (RFC 8785) |
| `content_hash(data)` | `content.hash` for a JSON record |
| `file_hash(data)` | sha256 of raw bytes (`schema.hash`, non-JSON content) |
| `compute_id(data)` / `seal(data)` | the id / the object with its id filled in |
| `verify(data, *, attestations=None, store=None, require=())` | report dict: `ok`, `id`, `code`, `detail`, `content_checked`, `attestations` |
| `sign(data, *, seed_hex, signer, purpose, at=None)` | one attestation line |
| `public_key(seed_hex)`, `generate_seed()`, `now()` | key and time helpers |

Errors raise `EvidenceError` with `.code` (for example `E_ID_MISMATCH`, or `E_USAGE` for invalid arguments) and `.detail`. One abi3 wheel covers CPython 3.10 and later.

Tests run every vector in `test-vectors/`:

```sh
python -m unittest discover -s crates/evidence-py/tests
```
