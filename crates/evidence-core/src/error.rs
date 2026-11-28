//! Stable error codes. These strings are part of the spec: test vectors name
//! them, and every implementation must report the same code for the same input.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Code {
    /// Not UTF-8, not JSON, a lone surrogate, or nesting deeper than the limit.
    /// Step 1's three codes come in a fixed order: this one first.
    Parse,
    /// The same member name appears twice in one object. Step 1's last code:
    /// only for bytes that are strict JSON with every number in range.
    DuplicateKey,
    /// A number whose magnitude exceeds 2^53 - 1 (includes overflow to infinity).
    /// Only for bytes that are strict JSON, whatever their member names.
    NumberRange,
    /// The object does not match the evidence/0 JSON Schema.
    Shape,
    /// The timestamp matches the pattern but is not a real calendar instant.
    Timestamp,
    /// `id` is not the hash of the canonical object without `id`.
    IdMismatch,
    /// `content.inline` does not hash to `content.hash`.
    ContentHashMismatch,
    /// A `prev` hash is not in the supplied store.
    PrevMissing,
    /// A `prev` object is dated after this object.
    PrevOrder,
    /// An attestation line is not valid JSON or does not match the attestation schema.
    AttestationShape,
    /// An attestation signature does not verify.
    SignatureInvalid,
    /// A required attestation purpose is absent.
    AttestationMissing,
    /// API only, never a verification outcome: an invalid argument such as a
    /// malformed seed, an unknown purpose or an invalid store object.
    Usage,
}

impl Code {
    pub fn as_str(self) -> &'static str {
        match self {
            Code::Parse => "E_PARSE",
            Code::DuplicateKey => "E_DUPLICATE_KEY",
            Code::NumberRange => "E_NUMBER_RANGE",
            Code::Shape => "E_SHAPE",
            Code::Timestamp => "E_TIMESTAMP",
            Code::IdMismatch => "E_ID_MISMATCH",
            Code::ContentHashMismatch => "E_CONTENT_HASH_MISMATCH",
            Code::PrevMissing => "E_PREV_MISSING",
            Code::PrevOrder => "E_PREV_ORDER",
            Code::AttestationShape => "E_ATTESTATION_SHAPE",
            Code::SignatureInvalid => "E_SIGNATURE_INVALID",
            Code::AttestationMissing => "E_ATTESTATION_MISSING",
            Code::Usage => "E_USAGE",
        }
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub code: Code,
    pub detail: String,
}

impl Error {
    pub fn new(code: Code, detail: impl Into<String>) -> Self {
        Error {
            code,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.detail)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;
