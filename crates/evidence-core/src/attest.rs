//! Attestations: signatures kept outside the object, one JSON object per line.
//!
//! The signed message is the canonical attestation without `sig`, so the
//! signature covers the object id, the purpose, the signer and the time.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};

use crate::canon;
use crate::error::{Code, Error, Result};
use crate::json::{self, Value};
use crate::object::{hash, members, one_of, text};
use crate::time::{self, Timestamp};

pub const VERSION: &str = "attestation/0";
pub const PURPOSES: &[&str] = &["author", "approve", "witness"];
const KEY_PREFIX: &str = "ed25519:";

#[derive(Debug, Clone)]
pub struct Attestation {
    pub value: Value,
    pub object: String,
    pub purpose: String,
    pub signer: String,
    pub at: Timestamp,
    key: [u8; 32],
    sig: [u8; 64],
}

/// Parse and shape-check one attestation. Every failure is `E_ATTESTATION_SHAPE`.
pub fn parse(bytes: &[u8]) -> Result<Attestation> {
    let value = json::parse(bytes).map_err(|e| bad(&e.to_string()))?;
    shape(&value).map_err(|e| bad(&e.detail))?;
    let get = |k: &str| {
        value
            .get(k)
            .and_then(Value::as_str)
            .expect("shape checked")
            .to_string()
    };
    let at = time::parse(&get("at")).map_err(|_| bad("/at: not a real UTC instant"))?;
    let key = decode_b64::<32>(get("key").strip_prefix(KEY_PREFIX).expect("shape checked"))
        .expect("shape checked");
    let sig = decode_b64::<64>(&get("sig")).expect("shape checked");
    Ok(Attestation {
        object: get("object"),
        purpose: get("purpose"),
        signer: get("signer"),
        at,
        key,
        sig,
        value,
    })
}

fn shape(v: &Value) -> Result<()> {
    members(
        v,
        "",
        &["v", "object", "purpose", "signer", "key", "at", "sig"],
        &[],
    )?;
    if v.get("v").and_then(Value::as_str) != Some(VERSION) {
        return Err(bad(&format!("/v: must be {VERSION:?}")));
    }
    hash(v.get("object").unwrap(), "/object")?;
    one_of(v.get("purpose").unwrap(), "/purpose", PURPOSES)?;
    text(v.get("signer").unwrap(), "/signer")?;
    let key = text(v.get("key").unwrap(), "/key")?;
    if key
        .strip_prefix(KEY_PREFIX)
        .and_then(decode_b64::<32>)
        .is_none()
    {
        return Err(bad(
            "/key: must be ed25519: followed by 32 bytes of unpadded base64url",
        ));
    }
    let at = text(v.get("at").unwrap(), "/at")?;
    if !time::matches_pattern(at) {
        return Err(bad("/at: must be UTC: YYYY-MM-DDTHH:MM:SS[.fraction]Z"));
    }
    let sig = text(v.get("sig").unwrap(), "/sig")?;
    if decode_b64::<64>(sig).is_none() {
        return Err(bad("/sig: must be 64 bytes of unpadded base64url"));
    }
    Ok(())
}

impl Attestation {
    /// Checks the Ed25519 signature with `verify_strict`, which rejects
    /// malleable signatures and weak keys.
    pub fn verify_signature(&self) -> Result<()> {
        let invalid = |why: &str| {
            Error::new(
                Code::SignatureInvalid,
                format!("{} signature by {}: {why}", self.purpose, self.signer),
            )
        };
        let key =
            VerifyingKey::from_bytes(&self.key).map_err(|_| invalid("not a valid public key"))?;
        let message = canon::canonicalize(&self.value.without("sig"));
        key.verify_strict(message.as_bytes(), &Signature::from_bytes(&self.sig))
            .map_err(|_| invalid("does not verify"))
    }
}

/// The `ed25519:` public key string for a 32-byte seed.
pub fn public_key(seed: &[u8; 32]) -> String {
    let key = SigningKey::from_bytes(seed).verifying_key();
    format!("{KEY_PREFIX}{}", URL_SAFE_NO_PAD.encode(key.to_bytes()))
}

/// Build and sign an attestation over `object_id`.
pub fn sign(
    object_id: &str,
    purpose: &str,
    signer: &str,
    at: &str,
    seed: &[u8; 32],
) -> Result<Value> {
    let unsigned = Value::Object(vec![
        ("v".into(), Value::String(VERSION.into())),
        ("object".into(), Value::String(object_id.into())),
        ("purpose".into(), Value::String(purpose.into())),
        ("signer".into(), Value::String(signer.into())),
        ("key".into(), Value::String(public_key(seed))),
        ("at".into(), Value::String(at.into())),
    ]);
    let message = canon::canonicalize(&unsigned);
    let sig = SigningKey::from_bytes(seed).sign(message.as_bytes());
    let signed = unsigned.with("sig", Value::String(URL_SAFE_NO_PAD.encode(sig.to_bytes())));
    // Refuse to emit something our own verifier would reject.
    let parsed = parse(canon::canonicalize(&signed).as_bytes())?;
    parsed.verify_signature()?;
    Ok(signed)
}

/// Unpadded base64url of exactly `N` bytes, in its one canonical spelling.
fn decode_b64<const N: usize>(s: &str) -> Option<[u8; N]> {
    let bytes = URL_SAFE_NO_PAD.decode(s).ok()?;
    if URL_SAFE_NO_PAD.encode(&bytes) != s {
        return None;
    }
    bytes.try_into().ok()
}

fn bad(detail: &str) -> Error {
    Error::new(Code::AttestationShape, detail.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_then_verify_and_tamper() {
        let seed = [7u8; 32];
        let id = format!("sha256:{}", "ab".repeat(32));
        let att = sign(&id, "approve", "person:test", "2026-11-26T00:00:00Z", &seed).unwrap();
        let line = canon::canonicalize(&att);
        parse(line.as_bytes()).unwrap().verify_signature().unwrap();

        let tampered = att.with("purpose", Value::String("witness".into()));
        let err = parse(canon::canonicalize(&tampered).as_bytes())
            .unwrap()
            .verify_signature()
            .unwrap_err();
        assert_eq!(err.code, Code::SignatureInvalid);
    }
}
