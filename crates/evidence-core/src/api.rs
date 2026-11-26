//! Byte-level entry points shared by the CLI and the language bindings, so
//! every surface behaves identically. Inputs are raw bytes: an object file that
//! is not valid UTF-8 must fail with `E_PARSE` everywhere, not be silently
//! repaired by a host's text decoder.

use crate::error::{Code, Error, Result};
use crate::verify::{Options, Report, Store};
use crate::{attest, canon, json, object, time};

/// The spec version these functions implement.
pub const SPEC_VERSION: &str = object::VERSION;

/// Canonical JSON (RFC 8785) of any JSON text, under the strict parsing rules.
pub fn canonicalize(input: &[u8]) -> Result<String> {
    Ok(canon::canonicalize(&json::parse(input)?))
}

/// `content.hash` for a JSON record: sha256 of its canonical form.
pub fn content_hash(input: &[u8]) -> Result<String> {
    Ok(canon::digest(&json::parse(input)?))
}

/// sha256 of raw bytes: `schema.hash`, and `content.hash` for non-JSON content.
pub fn file_hash(bytes: &[u8]) -> String {
    canon::sha256_hex(bytes)
}

/// The id an object should have (its `id` member, if any, is ignored).
pub fn compute_id(input: &[u8]) -> Result<String> {
    Ok(object::compute_id(&json::parse(input)?))
}

/// The object with `id` filled in, as canonical JSON. Only parses the input;
/// call [`verify`] to check the result.
pub fn seal(input: &[u8]) -> Result<String> {
    Ok(canon::canonicalize(&object::seal(&json::parse(input)?)))
}

#[derive(Debug, Default)]
pub struct VerifyArgs<'a> {
    /// Attestation lines (JSON Lines).
    pub attestations: Option<&'a [u8]>,
    /// Objects that `prev` may point at. `None` skips the prev check;
    /// `Some(&[])` means "there is a store and it is empty".
    pub store: Option<&'a [Vec<u8>]>,
    /// Purposes that must have a valid attestation.
    pub require: &'a [String],
}

/// Run the full verification pipeline. Verification failures are reported in
/// the returned [`Report`]; `Err` (with `E_USAGE`) means the arguments were
/// invalid: an unknown purpose, or a store object that does not verify.
pub fn verify(input: &[u8], args: &VerifyArgs) -> Result<Report> {
    for purpose in args.require {
        check_purpose(purpose)?;
    }
    let store = match args.store {
        Some(objects) => {
            let mut store = Store::new();
            for (i, bytes) in objects.iter().enumerate() {
                store
                    .insert(bytes)
                    .map_err(|e| usage(format!("store object {i} is not valid: {e}")))?;
            }
            Some(store)
        }
        None => None,
    };
    Ok(crate::verify::verify(
        input,
        &Options {
            attestations: args.attestations,
            store: store.as_ref(),
            require: args.require,
        },
    ))
}

/// Sign a valid object and return one canonical attestation line.
///
/// `at` is required (UTC, `YYYY-MM-DDTHH:MM:SS[.fraction]Z`): WebAssembly has
/// no clock, so callers supply the time. If the object itself is invalid, its
/// verification error is returned unchanged.
pub fn sign(input: &[u8], seed_hex: &str, signer: &str, purpose: &str, at: &str) -> Result<String> {
    check_purpose(purpose)?;
    let seed = parse_seed(seed_hex)?;
    if signer.is_empty() {
        return Err(usage("signer must not be empty"));
    }
    if time::parse(at).is_err() {
        return Err(usage(format!(
            "at {at:?} must be a real UTC instant, YYYY-MM-DDTHH:MM:SS[.fraction]Z"
        )));
    }
    let checked = object::check(input)?;
    let attestation = attest::sign(&checked.id, purpose, signer, at, &seed)?;
    Ok(canon::canonicalize(&attestation))
}

/// The `ed25519:` public key for a hex seed.
pub fn public_key(seed_hex: &str) -> Result<String> {
    Ok(attest::public_key(&parse_seed(seed_hex)?))
}

/// 64 hex digits (surrounding whitespace ignored) -> 32-byte Ed25519 seed.
pub fn parse_seed(seed_hex: &str) -> Result<[u8; 32]> {
    let hex = seed_hex.trim();
    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(usage("seed must be 64 hex digits"));
    }
    let mut seed = [0u8; 32];
    for (i, byte) in seed.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).expect("hex checked");
    }
    Ok(seed)
}

fn check_purpose(purpose: &str) -> Result<()> {
    if attest::PURPOSES.contains(&purpose) {
        Ok(())
    } else {
        Err(usage(format!(
            "purpose {purpose:?} must be one of {:?}",
            attest::PURPOSES
        )))
    }
}

fn usage(detail: impl Into<String>) -> Error {
    Error::new(Code::Usage, detail)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_errors() {
        assert_eq!(parse_seed("abc").unwrap_err().code, Code::Usage);
        let args = VerifyArgs {
            require: &["bless".to_string()],
            ..Default::default()
        };
        assert_eq!(verify(b"{}", &args).unwrap_err().code, Code::Usage);
        let store = [b"{}".to_vec()];
        let args = VerifyArgs {
            store: Some(&store),
            ..Default::default()
        };
        assert_eq!(verify(b"{}", &args).unwrap_err().code, Code::Usage);
    }

    #[test]
    fn sign_propagates_object_errors() {
        let seed = "00".repeat(32);
        let err = sign(b"{}", &seed, "person:x", "approve", "2026-11-26T00:00:00Z").unwrap_err();
        assert_eq!(err.code, Code::Shape);
        let err = sign(b"{}", &seed, "person:x", "approve", "yesterday").unwrap_err();
        assert_eq!(err.code, Code::Usage);
    }
}
