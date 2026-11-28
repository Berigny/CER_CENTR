//! coordinate/0: facet coordinates. See `spec/coordinate-0.md`.
//!
//! A published dictionary gives each facet of a profile, such as
//! `jurisdiction.vic`, its own prime. The coordinate of a list of facets is
//! the product of their primes, written as a canonical decimal string below
//! 2^1024; by unique factorisation it decodes back to exactly those facets,
//! and a query matches a record when the query's coordinate divides the
//! record's.
//!
//! - [`Dictionary::parse`] loads a `coordinate-dictionary/0` document, with
//!   its checks in the spec's order, and [`Dictionary::id`] is its id.
//! - [`Dictionary::encode`], [`Dictionary::decode`], [`Dictionary::divides`],
//!   [`Dictionary::check`] and [`extends`] are the five operations. The
//!   `_value` forms take a JSON value as it is, so that a JSON number in
//!   place of a coordinate gives the spec's error.
//! - [`Error`] carries the spec's code and members; [`Error::to_value`] is
//!   the error object, the same bytes in every implementation once
//!   canonicalised.
//!
//! The library is pure: no file system, network, clock, threads or
//! randomness. The arithmetic is a small big integer of its own (`big`), and
//! the only dependency is evidence-core, for strict JSON, the canonical form
//! and SHA-256.
//!
//! A coordinate is derived. It is never stored in the object it describes or
//! part of that object's id, and which facets a record has is a profile's
//! rule, not this crate's.
//!
//! ```
//! use evidence_coordinate::{Dictionary, Error};
//!
//! let d = Dictionary::parse(br#"{"v": "coordinate-dictionary/0", "profile": "example/0",
//!   "families": [{"name": "jurisdiction", "values": "one"}, {"name": "record", "values": "one"}],
//!   "facets": [{"id": "record.installation", "prime": 3}, {"id": "jurisdiction.vic", "prime": 11},
//!              {"id": "jurisdiction.nsw", "prime": 31}]}"#)?;
//! assert_eq!(d.encode(["jurisdiction.vic", "record.installation"])?, "33");
//! assert!(d.divides("11", "33")?);
//! assert!(!d.divides("31", "33")?);
//! assert_eq!(d.decode("341"), Err(Error::Cardinality { family: "jurisdiction".into() }));
//! # Ok::<(), Error>(())
//! ```

mod big;
pub mod dictionary;
pub mod error;
pub mod operations;

/// The spec version this crate implements.
pub const SPEC: &str = "coordinate/0";

pub use dictionary::{Dictionary, Facet, Family, Values, DICTIONARY_V, MAX_FACETS, PRIME_BOUND};
pub use error::{DictionaryDetail, Error, ExtensionDetail};
pub use operations::{extends, factors_value, is_canonical, Factor, MAX_BITS, MAX_DIGITS};
