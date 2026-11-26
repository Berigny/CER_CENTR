//! store/0 formats: the pure part of a verifiable append-only evidence store.
//! See `spec/store-0.md`.
//!
//! Everything here is a function of its inputs. There is no file system,
//! network, clock, process, thread or randomness, so the same code runs in a
//! native store, in WebAssembly and in the language bindings, and gives the
//! same bytes everywhere.
//!
//! - [`entry`]: the log entry, `log-entry/0`, and the entry hash
//! - [`line`](mod@line): segment lines that pair an entry with its object or attestation
//! - [`merkle`]: the RFC 9162 tree hash, inclusion and consistency proofs
//! - [`genesis`]: the store/0 genesis object, whose id is the log id
//! - [`checkpoint`]: the checkpoint/0 object
//! - [`lane`]: subject keys, lanes and the heads fold
//! - [`clock`]: received time, service time and millisecond formatting
//! - [`manifest`]: the export manifest, segments/0
//! - [`report`]: finding codes (`S_` codes), findings and `store-report/0`
//! - [`verify`]: the streaming verifier, for a live store's lines and for exports
//! - [`schemas`]: refs and hashes of the store's own schemas

pub mod checkpoint;
pub mod clock;
pub mod entry;
pub mod genesis;
pub mod lane;
pub mod line;
pub mod manifest;
pub mod merkle;
pub mod report;
pub mod schemas;
pub mod verify;

/// The spec version this crate implements, and `v` of the genesis content.
pub const STORE_SPEC: &str = "store/0";

pub use checkpoint::Checkpoint;
pub use entry::{Entry, Kind, ENTRY_V};
pub use genesis::{Genesis, STORE_SYSTEM};
pub use lane::{lane_of, subject_of, Heads, SubjectKey, RECORD};
pub use line::{attestation_ref, parse_line, render_line, Body, Line};
pub use manifest::{Manifest, SegmentRef};
pub use merkle::Digest;
pub use report::{Finding, FindingCode, Source, StoreReport};
pub use verify::{verify_export, Verifier, VerifyOptions};
