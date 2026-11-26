# evidence-layer (Python)

Python binding to the evidence/0 Rust core and the store/0 verifier. Results are byte-for-byte the same as the `evidence` command-line tool and the WebAssembly module.

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

# A store export: manifest.json and the segment files, in manifest order.
store_report = ev.verify_export(open("export/manifest.json", "rb").read(),
                                [open("export/000001.jsonl", "rb").read()])
print(store_report["ok"], [f["code"] for f in store_report["findings"]])
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

Store functions (`spec/store-0.md`, "Programming interfaces"), positional or by keyword:

| Function | Returns |
|---|---|
| `entry_hash(data)` | the entry hash of a log-entry/0 entry |
| `merkle_root(leaves)` | the RFC 9162 tree hash of a list of leaf bytes |
| `verify_inclusion(leaf_hash, index, size, proof, root)` | `True` or `False`; `index` is seq − 1 |
| `verify_consistency(old_size, new_size, old_root, new_root, proof)` | `True` or `False` |
| `verify_export(manifest, segments, max_findings=1000)` | the `store-report/0` report as a dict |

Byte inputs are `bytes` or `str` (encoded as UTF-8). Hashes are `str`, `"sha256:"` and 64 lowercase hex digits. Sizes and indexes are `int` (not `bool`) from 0 to 2**53 - 1, `max_findings` from 1. Verification outcomes are never errors: proofs answer `False`, and `verify_export` returns a report however damaged the export is.

Errors raise `EvidenceError` with `.code` (for example `E_ID_MISMATCH`, or `E_USAGE` for invalid arguments, including an invalid entry given to `entry_hash`, whose detail begins with `S_ENTRY_INVALID`) and `.detail`. One abi3 wheel covers CPython 3.10 and later.

Tests run every vector in `test-vectors/`, the store vectors included (`tests/test_store_vectors.py`):

```sh
python -m unittest discover -s crates/evidence-py/tests
```
