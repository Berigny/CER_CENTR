//! Schema refs and hashes of the store's own objects.
//!
//! Each hash is `"sha256:"` + hex SHA-256 of the bytes of the schema file in
//! `spec/schema/`. The files are authoritative; the unit tests below check
//! these constants against them, so editing a schema without updating its
//! constant fails the build's tests.

/// `schema.ref` of the genesis object (seq 1).
pub const GENESIS_SCHEMA_REF: &str = "evidence-store/genesis@0";

/// sha256 of `spec/schema/store-genesis-0.schema.json`.
pub const GENESIS_SCHEMA_HASH: &str =
    "sha256:369979e1a083afc7096df02394b74ebf42a0cf4712b2bd377df9fc0d1f57be03";

/// `schema.ref` of a checkpoint object.
pub const CHECKPOINT_SCHEMA_REF: &str = "evidence-store/checkpoint@0";

/// sha256 of `spec/schema/checkpoint-0.schema.json`.
pub const CHECKPOINT_SCHEMA_HASH: &str =
    "sha256:fa9f7b8bcfc8d2adba92090d2722e0ab9e89de8eb298d5641b7a074bc91ee540";

#[cfg(test)]
mod tests {
    use super::*;
    use evidence_core::canon::sha256_hex;

    // The files are read at compile time, so this crate's sources never name
    // a file-system API (see the purity guard in the M1 spec, T14).
    const GENESIS_SCHEMA: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../spec/schema/store-genesis-0.schema.json"
    ));
    const CHECKPOINT_SCHEMA: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../spec/schema/checkpoint-0.schema.json"
    ));

    #[test]
    fn genesis_schema_hash_matches_the_file() {
        assert_eq!(sha256_hex(GENESIS_SCHEMA), GENESIS_SCHEMA_HASH);
    }

    #[test]
    fn checkpoint_schema_hash_matches_the_file() {
        assert_eq!(sha256_hex(CHECKPOINT_SCHEMA), CHECKPOINT_SCHEMA_HASH);
    }

    #[test]
    fn refs_name_the_store_lanes() {
        // lane.rs cuts a ref at its first '@'; the store's lanes depend on it.
        assert_eq!(GENESIS_SCHEMA_REF.split_once('@').unwrap().1, "0");
        assert_eq!(CHECKPOINT_SCHEMA_REF.split_once('@').unwrap().1, "0");
    }
}
