//! evidence/0 core.
//!
//! A small, portable evidence object that sits beside an existing system of
//! record: it proves what a record said, under which schema version, who wrote
//! it, when, and what it replaced. See `spec/evidence-0.md`.
//!
//! - [`json`]: strict JSON parsing (duplicate keys, unsafe numbers and lone
//!   surrogates are errors)
//! - [`canon`]: RFC 8785 canonical JSON and SHA-256 digests
//! - [`object`]: shape rules, ids, and integrity checks
//! - [`attest`]: Ed25519 attestations kept outside the object
//! - [`verify`]: the full verification pipeline and its report

pub mod attest;
pub mod canon;
pub mod error;
pub mod json;
pub mod object;
pub mod time;
pub mod verify;

pub use error::{Code, Error, Result};
pub use json::Value;
