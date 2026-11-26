//! The store key: what signs the genesis author attestation and every
//! checkpoint witness. See `spec/store-0.md`, "Genesis" and "Checkpoint and
//! tree".
//!
//! A writer never trusts what a [`Signer`] returns. It parses the line as
//! attestation/0, checks the signature, and checks that the line attests
//! exactly what was asked with the genesis key; anything else is refused with
//! `S_STORE_KEY` before anything is written (see [`crate::api`]).
//!
//! [`SeedSigner`] holds an Ed25519 seed in memory. It is for development and
//! the test vectors only; production keys belong in an HSM or a key service
//! (M5), behind this same trait.

use std::fmt;
use std::fs;
use std::path::Path;

use evidence_core::{api, attest, canon};

use crate::api::StoreError;

/// Signs attestations with the store key.
pub trait Signer {
    /// The store key's public key: `"ed25519:"` + 32 bytes of unpadded
    /// canonical base64url, as evidence/0 writes keys.
    fn public_key(&self) -> String;

    /// One canonical attestation/0 line (no LF) over `object_id`, with this
    /// `purpose`, `signer` and `at`, and the key [`public_key`](Self::public_key)
    /// names.
    ///
    /// `S_STORE_KEY` when the key cannot sign; `E_USAGE` when an argument is
    /// one attestation/0 does not allow (an unknown purpose, an empty signer,
    /// an `at` that is not a timestamp).
    fn sign(
        &self,
        object_id: &str,
        purpose: &str,
        signer: &str,
        at: &str,
    ) -> Result<String, StoreError>;
}

/// A store key held as a 32-byte Ed25519 seed. Development and test vectors
/// only. `Debug` shows the public key, never the seed.
#[derive(Clone)]
pub struct SeedSigner {
    seed: [u8; 32],
}

impl SeedSigner {
    /// A signer for a seed written as 64 hex digits; surrounding whitespace,
    /// such as a seed file's final LF, is ignored. `E_USAGE` otherwise.
    pub fn from_hex(hex: &str) -> Result<SeedSigner, StoreError> {
        let seed = api::parse_seed(hex).map_err(|e| StoreError::Usage(e.detail))?;
        Ok(SeedSigner { seed })
    }

    /// A signer for the seed in a seed file (64 hex digits). `S_IO` if the
    /// file cannot be read; `E_USAGE` if it does not hold a seed.
    pub fn from_file(path: &Path) -> Result<SeedSigner, StoreError> {
        let bytes =
            fs::read(path).map_err(|e| StoreError::Io(format!("{}: {e}", path.display())))?;
        let text = std::str::from_utf8(&bytes).map_err(|_| {
            StoreError::Usage(format!("{}: seed must be 64 hex digits", path.display()))
        })?;
        SeedSigner::from_hex(text).map_err(|_| {
            StoreError::Usage(format!("{}: seed must be 64 hex digits", path.display()))
        })
    }
}

impl Signer for SeedSigner {
    fn public_key(&self) -> String {
        attest::public_key(&self.seed)
    }

    fn sign(
        &self,
        object_id: &str,
        purpose: &str,
        signer: &str,
        at: &str,
    ) -> Result<String, StoreError> {
        if !attest::PURPOSES.contains(&purpose) {
            return Err(StoreError::Usage(format!(
                "purpose {purpose:?} is not one of {}",
                attest::PURPOSES.join(", ")
            )));
        }
        if signer.is_empty() {
            return Err(StoreError::Usage("signer must not be empty".into()));
        }
        if evidence_core::time::parse(at).is_err() {
            return Err(StoreError::Usage(format!("at {at:?} is not a timestamp")));
        }
        // attest::sign refuses to emit a line its own verifier would reject,
        // so a failure here is an object id that is not a hash.
        let signed = attest::sign(object_id, purpose, signer, at, &self.seed)
            .map_err(|e| StoreError::Usage(e.detail))?;
        Ok(canon::canonicalize(&signed))
    }
}

impl fmt::Debug for SeedSigner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SeedSigner")
            .field("public_key", &self.public_key())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evidence_core::json::{self, Value};
    use std::path::PathBuf;

    fn keys() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-vectors/keys")
    }

    fn demo() -> SeedSigner {
        SeedSigner::from_file(&keys().join("store-demo.seed")).unwrap()
    }

    const LOG_ID: &str = "sha256:0000000000000000000000000000000000000000000000000000000000000001";

    #[test]
    fn store_demo_seed_gives_the_published_key() {
        let public = json::parse(&fs::read(keys().join("public.json")).unwrap()).unwrap();
        let want = public.get("store-demo").and_then(Value::as_str).unwrap();
        assert_eq!(demo().public_key(), want);
        assert_eq!(want, "ed25519:EWYM90TYNQCehYuLOjVj_6zmMZwHTihcle67Z093xEI");
    }

    #[test]
    fn from_hex_and_from_file_agree() {
        let text = fs::read_to_string(keys().join("store-demo.seed")).unwrap();
        let from_hex = SeedSigner::from_hex(&text).unwrap();
        assert_eq!(from_hex.public_key(), demo().public_key());
    }

    #[test]
    fn signs_a_canonical_line_that_verifies() {
        let signer = demo();
        let at = "2026-11-26T00:00:00.000Z";
        let line = signer
            .sign(LOG_ID, "author", "service:evidence-store/cer-demo", at)
            .unwrap();
        let parsed = attest::parse(line.as_bytes()).unwrap();
        parsed.verify_signature().unwrap();
        assert_eq!(canon::canonicalize(&parsed.value), line);
        assert_eq!(parsed.object, LOG_ID);
        assert_eq!(parsed.purpose, "author");
        assert_eq!(parsed.signer, "service:evidence-store/cer-demo");
        assert_eq!(
            parsed.value.get("key").and_then(Value::as_str),
            Some(signer.public_key().as_str())
        );
        assert_eq!(parsed.value.get("at").and_then(Value::as_str), Some(at));
        assert!(!line.ends_with('\n'));
    }

    #[test]
    fn refuses_arguments_attestation_0_does_not_allow() {
        let signer = demo();
        let at = "2026-11-26T00:00:00.000Z";
        let code = |r: Result<String, StoreError>| r.unwrap_err().code_str();
        assert_eq!(code(signer.sign(LOG_ID, "notary", "s", at)), "E_USAGE");
        assert_eq!(code(signer.sign(LOG_ID, "witness", "", at)), "E_USAGE");
        assert_eq!(
            code(signer.sign(LOG_ID, "witness", "s", "2026-02-30T00:00:00Z")),
            "E_USAGE"
        );
        assert_eq!(
            code(signer.sign("not-a-hash", "witness", "s", at)),
            "E_USAGE"
        );
    }

    #[test]
    fn bad_seeds_are_usage_errors_and_missing_files_io_errors() {
        assert_eq!(
            SeedSigner::from_hex("abc").unwrap_err().code_str(),
            "E_USAGE"
        );
        assert_eq!(
            SeedSigner::from_hex(&"g".repeat(64))
                .unwrap_err()
                .code_str(),
            "E_USAGE"
        );
        let missing = keys().join("no-such-key.seed");
        assert_eq!(
            SeedSigner::from_file(&missing).unwrap_err().code_str(),
            "S_IO"
        );
        let not_a_seed = keys().join("public.json");
        assert_eq!(
            SeedSigner::from_file(&not_a_seed).unwrap_err().code_str(),
            "E_USAGE"
        );
    }

    #[test]
    fn debug_never_shows_the_seed() {
        let text = format!("{:?}", demo());
        assert!(text.contains("EWYM90TYNQCehYuLOjVj_6zmMZwHTihcle67Z093xEI"));
        assert!(!text.contains("08e40f5a"));
    }
}
