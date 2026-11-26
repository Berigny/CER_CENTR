//! store/0 with I/O: the writer, the readers, export, import, verification
//! and proofs of a verifiable append-only evidence log. See
//! `spec/store-0.md` and `docs/m1-store-spec.md` section 7.
//!
//! The formats and the verifier are pure and live in `evidence_log`; this
//! crate adds the clock, the store key, storage and files.
//!
//! - [`api`]: the [`LogRead`] and [`LogWrite`] traits, items, receipts and
//!   errors, and the rules every writer here follows
//! - [`clock`]: [`SystemClock`] and the scripted [`ScriptClock`]
//! - [`signer`]: the store key, [`SeedSigner`]
//! - [`intake`]: the intake rules, shared by every writer
//! - [`mem`]: [`MemLog`], the in-memory reference log
//! - [`sqlite`]: [`SqliteLog`], the writer of a store on disk (`log.db`), and
//!   [`SqliteReader`]
//! - [`export`](mod@export) and [`import`](mod@import): the portable form,
//!   segments/0
//! - [`verify`]: [`verify_log`], [`verify_store`] and [`verify_export_dir`]
//! - [`prove`]: inclusion and consistency proofs
//!
//! Nothing here panics on untrusted input: objects, attestation lines,
//! store files and export files. [`ScriptClock`], a test clock, panics when
//! it runs out of readings.

pub mod api;
pub mod clock;
pub mod export;
mod files;
pub mod import;
pub mod intake;
pub mod mem;
pub mod prove;
pub mod signer;
pub mod sqlite;
pub mod verify;

pub use api::{
    CheckpointInfo, Head, Item, LogRead, LogWrite, Receipt, RejectCode, Rejected, StoreError,
    StoredAttestation, StoredObject,
};
pub use clock::{Clock, ScriptClock, SystemClock};
pub use export::{
    export, export_files, read_export_dir, ExportBytes, ExportFiles, DEFAULT_SEGMENT_SIZE,
};
pub use import::import;
pub use mem::MemLog;
pub use prove::{consistency, inclusion, ConsistencyProof, InclusionProof};
pub use signer::{SeedSigner, Signer};
pub use sqlite::{SqliteLog, SqliteReader};
pub use verify::{verify_export_dir, verify_log, verify_store};
