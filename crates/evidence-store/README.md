# evidence-store

The store/0 library ([`spec/store-0.md`](../../spec/store-0.md)): an append-only log of evidence/0 objects and attestations in one SQLite file, with the store's own position (`seq`) and received time for every entry, hash-chained entries and signed checkpoints over an RFC 9162 Merkle tree.

- **The writer**, `SqliteLog`: creates a store (`init`) or opens one, appends objects and attestation lines through the intake rules, with compare-and-set on a subject's heads, and writes signed checkpoints. Each call is one transaction.
- **The reader**, `SqliteReader`: reads a store without the lease, in one snapshot when asked, while a writer appends.
- **Export and import**: the portable form, a `manifest.json` and canonical JSON Lines segment files (segments/0). An export verifies offline, in the CLI, the WebAssembly module, Python or a browser.
- **Verification**: `verify_store` checks a live store (its lines, and the tables and triggers beside them), `verify_export_dir` an export, and `verify_log` any log; each returns a store-report/0 with every finding.
- **Proofs**: `inclusion` and `consistency` proofs against the log's tree.
- `MemLog`, an in-memory reference log that writes the same lines as `SqliteLog`, used in tests.

The formats and the export verifier are pure and live in [`evidence-log`](../evidence-log/), which also builds for WebAssembly and Python. This crate adds the clock, the store key, storage and files. The command-line tool runs it as `evidence store …` ([`crates/evidence-cli`](../evidence-cli/README.md)).

## A store on disk

```
<store>/
  log.db            the log, an SQLite database in WAL mode (SQLite keeps log.db-wal and log.db-shm beside it)
  LOCK              the writer's lease; its text ("pid <pid>", "epoch <n>") is for diagnostics only
  log.db.init       only while init builds the store, or after an init that was killed
```

`log.db` holds the entries, the objects and attestations they carry, and two projections written in the same transaction: the heads of each subject and lane, and the checkpoints. Triggers refuse UPDATE and DELETE on the log tables. `init` builds the store under another name and renames it to `log.db` when it is complete, so `log.db` only ever names a complete store. Export and import do the same with their target directory. Seed files are never kept in the store. The store must be on a local disk.

**Lease.** A writer holds an OS file lock on `LOCK` (`File::try_lock`) from the moment it opens the store until it is dropped. Another writer, in this process or another, gets `S_LOCKED` at once and does not wait. The operating system releases the lock when the process ends, even when it is killed, so there is nothing to clean up. Readers take no lease.

**Epoch.** Every writer has an epoch: 1 for the writer `init` returns, and one more each time `SqliteLog::open` takes the lease, which it records before it appends anything. Every entry carries its writer's epoch, and verification reports an entry whose epoch is lower than the one before it (`S_EPOCH_REGRESSION`). `import` sets the epoch to the highest in the export, so the next writer appends above it.

## Example

```rust
use std::path::Path;

use evidence_log::VerifyOptions;
use evidence_store::{verify_store, LogWrite, SeedSigner, SqliteLog, StoreError, SystemClock};

fn main() -> Result<(), StoreError> {
    // The public test key: for tests only.
    let signer = SeedSigner::from_file(Path::new("test-vectors/keys/store-demo.seed"))?;
    let dir = Path::new("demo-store"); // absent or empty

    let mut log = SqliteLog::init(dir, "demo", &signer, Box::new(SystemClock))?;
    let object = std::fs::read("test-vectors/objects/001-create-inline/object.json")?;
    let receipt = log.append(&object)?;
    println!("seq {}: {}", receipt.seq, receipt.entry_hash);
    let checkpoint = log.checkpoint(&signer)?;
    println!("checkpoint of size {}: root {}", checkpoint.size, checkpoint.root);
    drop(log); // closes the store and releases the lease

    let report = verify_store(dir, VerifyOptions::default())?;
    println!("ok: {}", report.ok());
    Ok(())
}
```

Run from the repository root, it prints `seq 3: sha256:…`, `checkpoint of size 3: root sha256:…` and `ok: true`. An object refused at intake is `StoreError::Rejected`, with its code; nothing of its transaction is written.

## More

- The writer's rules (transactions, the clock, batches, creating a store, checkpoints, epochs, errors) are in the rustdoc of [`src/api.rs`](src/api.rs): `cargo doc -p evidence-store --no-deps`, then open `target/doc/evidence_store/api/index.html`.
- The formats, intake, verification order and codes: [`spec/store-0.md`](../../spec/store-0.md). The design and the acceptance tests: [`docs/m1-store-spec.md`](../../docs/m1-store-spec.md), sections 7 and 11.
- Tests: `cargo test -p evidence-store` runs the contract suite against `MemLog` and `SqliteLog`, the SQLite-only, lease and killed-init tests, a property test (`PROPTEST_CASES` sets its cases, 256 by default), and the replay of every valid export case byte for byte.
- `cargo run --release -p evidence-store --example bench_append -- 10000` measures append throughput and verification time. It prints one JSON line.
