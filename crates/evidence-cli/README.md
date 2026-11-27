# evidence (command-line tool)

The `evidence` command creates and verifies evidence/0 objects ([`spec/evidence-0.md`](../../spec/evidence-0.md)) and keeps them in store/0 stores ([`spec/store-0.md`](../../spec/store-0.md)). Its results are byte for byte those of the WebAssembly module and the Python package, which run the same Rust code.

```sh
cargo build --release -p evidence-cli      # target/release/evidence
target/release/evidence --help             # every command, in brief
```

This page is a summary. The full contract of the store commands, with every rule and output shape, is section 8 of [`docs/m1-store-spec.md`](../../docs/m1-store-spec.md).

## Object commands

| Command | Prints |
|---|---|
| `evidence canon <file>` | canonical JSON (RFC 8785) |
| `evidence content-hash <file>` | `content.hash` for a JSON record: sha256 of its canonical form |
| `evidence file-hash <file>` | sha256 of a file's bytes (`schema.hash`, content that is not JSON) |
| `evidence id <file>` | the id an object should have |
| `evidence seal <file>` | the object with its id filled in, as canonical JSON |
| `evidence verify <file> [--attestations <file.jsonl>] [--store <dir>] [--require <purpose>]...` | a JSON report (`ok`, `id`, `code`, `detail`, `content_checked`, `attestations`) |
| `evidence sign <file> --seed <file> --signer <id> --purpose <author\|approve\|witness> [--at <time>]` | one attestation line for a valid object |
| `evidence keygen <seed-file>` | the public key of a new random seed, which it writes to the file |

`verify --store <dir>` names the objects that `prev` may point at: a folder of `.json` files, or a store (a directory holding `log.db`). A store is read without the lease and used only for `prev`; attestations still come from `--attestations`. Both forms give the same report for the same objects.

Exit status: 0 for success or a valid object, 1 for an object that does not verify, 2 for a usage or I/O error (a failed write to standard output included). Errors go to standard error only, and nothing is printed on standard output. Unknown options are ignored.

## Store commands

A store is a directory holding `log.db`, one SQLite file.

| Command | Prints |
|---|---|
| `evidence store init <dir> --name <name> --seed <seed-file>` | `{head, key, log, name}` of the new store |
| `evidence store append <dir> <object.json>... [--if-heads <id>[,<id>...] \| --if-heads none]` | one line per file: its receipt (`seq`, `entry_hash`, `received_at`, `ref`, `duplicate`) and `file`, or `{code, file, ok:false}` |
| `evidence store attest <dir> <attestations.jsonl>...` | one line per line that is not blank, as for `append`, with `line` |
| `evidence store checkpoint <dir> --seed <seed-file>` | `{checkpoint, head, root, seq, size}` |
| `evidence store get <dir> <id> [--require <purpose>]...` | `{attestations, entry, object, report}`: the object, its log entry, its attestations, and the evidence/0 report of the stored bytes, checked again on every read |
| `evidence store heads <dir> <system> <type> <key> [--lane <lane>]` | `{heads, lane}`; the lane is `record` unless given |
| `evidence store log <dir> [--from <seq>] [--limit <n>]` | log lines, exactly as in an export's segment files (from seq 1, at most 100, unless given) |
| `evidence store verify <dir> [--max-findings <n>]` | the store-report/0 of a store, or of an export (a directory holding `manifest.json`) |
| `evidence store export <dir> --out <dir> [--segment-size <n>]` | the manifest it wrote with the segment files |
| `evidence store import <export-dir> --into <dir>` | the report of the new store, or of the export when it does not verify |
| `evidence store prove <dir> --seq <n> [--size <m>]` | an inclusion proof, by default against the latest checkpoint |
| `evidence store prove <dir> --from <old-size> --to <new-size>` | a consistency proof |

- `append` and `attest` commit each file, or each line, in a transaction of its own, and print its line before they read the next. An item refused at intake does not stop the run. `--if-heads` (one file only) appends only if the subject's heads in the object's lane are exactly the ids given, or none.
- `init`, `append`, `attest` and `checkpoint` are writers. One writer at a time holds the store's lease; another gets `S_LOCKED` at once, without waiting. Each writer that opens the store raises its epoch, which every entry it appends carries. The other commands take no lease and run while a writer appends.
- Writers take `received_at` from the system clock. No option or environment variable changes it.
- `init` and `checkpoint` sign with the seed file given. Seed files are for development and test vectors only.
- `export` and `import` build their target beside it under another name and rename it into place, so an interrupted run never leaves a half-written target.
- `verify` checks a live store, its projections and triggers included, or an export, with the same algorithm, and reports every finding up to `--max-findings` (1000 unless given).
- The commands that only read (`get`, `heads`, `log`, `verify`, `export`, `prove`, and `evidence verify --store`) create no file in a store whose `log.db` belongs to another user, even when run by root or by a member of the store's group: they read `log.db` as it is or, while a writer has the store open, through that writer's `log.db-wal` and `log.db-shm`.
- Every argument must be UTF-8 text; a path whose name is not is refused with `E_USAGE`, and so is an empty path (an unset shell variable in `"$dir"`, for example), which would otherwise name the working directory.
- `get` and `log` read lines from the store and check each before they print it. A line that is not the one asked for, or not a valid segment line, can only come from a damaged store: it is `S_IO`, and `store verify` says what is wrong.

## Output

- **Standard output** holds only canonical JSON: one value per line, each line ending in LF and written as soon as it is known. Nothing else is printed there.
- **Standard error** holds human-readable lines starting `evidence: `. They are not part of the contract, except that the first line of an operation error starts `evidence: <code>`, `evidence: <file>: <code>` or `evidence: <file>:<line>: <code>`. A refused item gets one line, `evidence: <file>[:<line>]: <code>: <detail>`. `store verify` and `store import` add one summary line when the report is not ok.
- `evidence store help` (or `-h`, `--help`) prints the usage text on standard output.

## Exit status

| Status | Meaning | Standard output |
|---|---|---|
| 0 | Success: every item accepted (duplicates included), or a report that is ok. | The result. |
| 1 | An item refused at intake (`append`, `attest`), or a report that is not ok (`get`, `verify`, `import`). | The whole result, as for 0. |
| 2 | An operation error. | Its last line is `{"code":"<code>","ok":false}`, with `file` (and `line`) when the error concerns one input of `append` or `attest`. Lines before it report work that was committed. |

## Codes

An item refused at intake (exit 1) gets the code that names the reason:

- an evidence/0 code from verifying the object or the attestation line, such as `E_ID_MISMATCH`, `E_PREV_MISSING`, `E_PREV_ORDER` or `E_SIGNATURE_INVALID`;
- `S_STALE_HEADS`: `--if-heads` did not match; the line also holds the current `heads`;
- `S_RESERVED_SUBJECT`: the object's `subject.system` is `evidence-store`, which only the store itself writes;
- `S_ATTESTATION_TARGET`: the store does not hold the attestation's object.

An operation error (exit 2) has one of these codes:

| Code | Meaning |
|---|---|
| `E_USAGE` | An invalid command line, checked before anything is read or opened (an argument that is not UTF-8 text included), or an argument only the store can judge, such as a store name, a seed file's content or a proof position past the head. |
| `S_LOCKED` | Another writer holds the store. |
| `S_STORE_KEY` | The seed given to `checkpoint` is not the store key. Nothing is written. |
| `S_NOT_A_STORE` | The directory is not a store. For `verify`, it is neither a store nor an export; for `import`, it is not an export. |
| `S_EXISTS` | The directory given to `init` already holds something, or the target of `export` or `import` already exists. |
| `S_FORMAT` | The store's format version is not one this tool supports. |
| `S_IO` | A file could not be read or written, the storage engine failed, stored content is damaged, or whether a directory holds a store cannot be told (one this user cannot search, a symlink loop). A failed write to standard output is `S_IO` too. |
| `S_NOT_HELD` | `store get` names an id the store holds no object for. |

## Tests

- `tests/store_cli.rs`: the store commands and the rules of section 8 (stores damaged by hand and output that cannot be written included), all 20 object vectors through a store (T3), the lease (T7), export, import and every export case (T10), proofs (T11), and `verify --store` (T12).
- `tests/store_crash.rs` (Unix only): `store append` and `store import` killed at random moments; every receipt printed before the kill is in the store, and an import leaves no store or a complete one (T6). `EVIDENCE_CRASH_ITERS` sets the number of append iterations (10 by default; CI runs 50 and the nightly run 1,000), and `EVIDENCE_CRASH_SEED` repeats a run's random choices; each run prints its seed on standard error.

```sh
cargo test -p evidence-cli
EVIDENCE_CRASH_ITERS=50 cargo test -p evidence-cli --test store_crash -- --nocapture
```
