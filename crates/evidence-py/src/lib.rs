//! Python binding to the evidence/0 core and the store/0 verifier. The
//! public, documented API is the `evidence_layer` package
//! (python/evidence_layer/__init__.py); this module is its native half and
//! takes and returns bytes, strings, integers, booleans and lists of them.

use evidence_core::api::{self, VerifyArgs};
use evidence_core::{Code, Error};
use evidence_log::merkle::{self, Digest};
use evidence_log::{Entry, VerifyOptions};
use pyo3::create_exception;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

create_exception!(
    _native,
    EvidenceError,
    PyValueError,
    "Raised with args (code, detail)."
);

fn to_py(e: evidence_core::Error) -> PyErr {
    EvidenceError::new_err((e.code.as_str(), e.detail))
}

fn usage(detail: impl Into<String>) -> PyErr {
    to_py(Error::new(Code::Usage, detail))
}

/// The largest index, size or `max_findings`: 2^53 − 1.
const MAX_INTEGER: u64 = merkle::MAX_SIZE;

/// `value` if it is from `min` to 2^53 − 1.
fn integer(name: &str, value: u64, min: u64) -> PyResult<u64> {
    if (min..=MAX_INTEGER).contains(&value) {
        Ok(value)
    } else {
        Err(usage(format!(
            "{name} must be an integer from {min} to 2**53 - 1"
        )))
    }
}

const HASH_TEXT: &str = "\"sha256:\" and 64 lowercase hex digits";

fn digest(name: &str, text: &str) -> PyResult<Digest> {
    merkle::from_text(text).ok_or_else(|| usage(format!("{name} must be {HASH_TEXT}")))
}

fn digests(name: &str, texts: &[String]) -> PyResult<Vec<Digest>> {
    texts
        .iter()
        .map(|t| {
            merkle::from_text(t)
                .ok_or_else(|| usage(format!("{name} must be a list of {HASH_TEXT}")))
        })
        .collect()
}

#[pymodule]
mod _native {
    use super::*;

    #[pymodule_export]
    use super::EvidenceError;

    #[pymodule_init]
    fn init(m: &Bound<'_, PyModule>) -> PyResult<()> {
        m.add("SPEC_VERSION", api::SPEC_VERSION)?;
        m.add("CRATE_VERSION", env!("CARGO_PKG_VERSION"))?;
        Ok(())
    }

    #[pyfunction]
    fn canonicalize(input: &[u8]) -> PyResult<String> {
        api::canonicalize(input).map_err(to_py)
    }

    #[pyfunction]
    fn content_hash(input: &[u8]) -> PyResult<String> {
        api::content_hash(input).map_err(to_py)
    }

    #[pyfunction]
    fn file_hash(input: &[u8]) -> String {
        api::file_hash(input)
    }

    #[pyfunction]
    fn compute_id(input: &[u8]) -> PyResult<String> {
        api::compute_id(input).map_err(to_py)
    }

    #[pyfunction]
    fn seal(input: &[u8]) -> PyResult<String> {
        api::seal(input).map_err(to_py)
    }

    /// Returns the report as canonical JSON text.
    #[pyfunction]
    #[pyo3(signature = (input, attestations, store, require))]
    fn verify(
        input: &[u8],
        attestations: Option<&[u8]>,
        store: Option<Vec<Vec<u8>>>,
        require: Vec<String>,
    ) -> PyResult<String> {
        let report = api::verify(
            input,
            &VerifyArgs {
                attestations,
                store: store.as_deref(),
                require: &require,
            },
        )
        .map_err(to_py)?;
        Ok(evidence_core::canon::canonicalize(&report.to_value()))
    }

    #[pyfunction]
    fn sign(
        input: &[u8],
        seed_hex: &str,
        signer: &str,
        purpose: &str,
        at: &str,
    ) -> PyResult<String> {
        api::sign(input, seed_hex, signer, purpose, at).map_err(to_py)
    }

    #[pyfunction]
    fn public_key(seed_hex: &str) -> PyResult<String> {
        api::public_key(seed_hex).map_err(to_py)
    }

    /// E_USAGE whose detail begins with S_ENTRY_INVALID for an invalid entry.
    #[pyfunction]
    fn entry_hash(input: &[u8]) -> PyResult<String> {
        Entry::parse(input)
            .map(|entry| entry.hash())
            .map_err(|code| usage(format!("{code}: input is not valid log-entry/0")))
    }

    #[pyfunction]
    fn merkle_root(leaves: Vec<Vec<u8>>) -> String {
        let hashes: Vec<Digest> = leaves.iter().map(|l| merkle::leaf_hash(l)).collect();
        merkle::to_text(&merkle::root(&hashes))
    }

    #[pyfunction]
    fn verify_inclusion(
        leaf_hash: &str,
        index: u64,
        size: u64,
        proof: Vec<String>,
        root: &str,
    ) -> PyResult<bool> {
        Ok(merkle::verify_inclusion(
            &digest("leaf_hash", leaf_hash)?,
            integer("index", index, 0)?,
            integer("size", size, 0)?,
            &digests("proof", &proof)?,
            &digest("root", root)?,
        ))
    }

    #[pyfunction]
    fn verify_consistency(
        old_size: u64,
        new_size: u64,
        old_root: &str,
        new_root: &str,
        proof: Vec<String>,
    ) -> PyResult<bool> {
        Ok(merkle::verify_consistency(
            integer("old_size", old_size, 0)?,
            integer("new_size", new_size, 0)?,
            &digest("old_root", old_root)?,
            &digest("new_root", new_root)?,
            &digests("proof", &proof)?,
        ))
    }

    /// Returns the store-report/0 object as canonical JSON text.
    #[pyfunction]
    fn verify_export(
        manifest: &[u8],
        segments: Vec<Vec<u8>>,
        max_findings: u64,
    ) -> PyResult<String> {
        let max_findings = integer("max_findings", max_findings, 1)?;
        let segments: Vec<&[u8]> = segments.iter().map(Vec::as_slice).collect();
        let report = evidence_log::verify_export(
            manifest,
            &segments,
            VerifyOptions {
                // A limit above what this platform can count is no limit.
                max_findings: usize::try_from(max_findings).unwrap_or(usize::MAX),
                ..VerifyOptions::default()
            },
        );
        Ok(evidence_core::canon::canonicalize(&report.to_value()))
    }
}
