//! Python binding to the evidence/0 core. The public, documented API is the
//! `evidence_layer` package (python/evidence_layer/__init__.py); this module
//! is its native half and takes and returns bytes and strings only.

use evidence_core::api::{self, VerifyArgs};
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
}
