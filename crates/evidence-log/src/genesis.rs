//! The genesis object (store/0): seq 1 of every store, whose id is the log id.
//! See `spec/store-0.md`, "Genesis", and `spec/schema/store-genesis-0.schema.json`.
//!
//! A genesis is fully determined by its name, its key and its `at`: building
//! the object from those three values and sealing it gives the same canonical
//! bytes, and so the same log id, in every implementation. A valid genesis is
//! an object whose content is valid store/0 and which is exactly the object
//! built from its own `name`, `key` and `at`.

use evidence_core::canon;
use evidence_core::json::Value;
use evidence_core::object::{self, Checked};

use crate::report::FindingCode;
use crate::schemas::{GENESIS_SCHEMA_HASH, GENESIS_SCHEMA_REF};

/// `v` of the genesis content.
pub const STORE_V: &str = crate::STORE_SPEC;

/// `subject.system` of the store's own objects. Reserved: only the store
/// writes objects with this system.
pub const STORE_SYSTEM: &str = "evidence-store";

/// `subject.type` of the genesis.
pub const GENESIS_TYPE: &str = "log";

/// The store signer is this prefix followed by the store's name.
pub const SIGNER_PREFIX: &str = "service:evidence-store/";

const KEY_PREFIX: &str = "ed25519:";

/// The members of the genesis content. All are required and no others are allowed.
pub const GENESIS_MEMBERS: &[&str] = &["v", "name", "signer", "key"];

/// The store's name, its signer and its key, as the genesis declares them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Genesis {
    pub name: String,
    /// `"service:evidence-store/"` + name: the actor of the genesis and of
    /// every checkpoint, and the signer of their attestations.
    pub signer: String,
    /// The store key: `"ed25519:"` + 32 bytes of unpadded canonical base64url.
    pub key: String,
}

impl Genesis {
    /// The genesis of a store with this name and key. `S_GENESIS_INVALID` if
    /// the name does not match `^[a-z0-9][a-z0-9.-]{0,62}$` or the key is not
    /// an Ed25519 public key written as evidence/0 writes keys.
    pub fn new(name: &str, key: &str) -> Result<Genesis, FindingCode> {
        if !is_store_name(name) || !is_store_key(key) {
            return Err(FindingCode::GenesisInvalid);
        }
        Ok(Genesis {
            name: name.to_string(),
            signer: format!("{SIGNER_PREFIX}{name}"),
            key: key.to_string(),
        })
    }

    /// The genesis content (`content.inline`).
    pub fn content(&self) -> Value {
        Value::Object(vec![
            ("v".into(), Value::String(STORE_V.into())),
            ("name".into(), Value::String(self.name.clone())),
            ("signer".into(), Value::String(self.signer.clone())),
            ("key".into(), Value::String(self.key.clone())),
        ])
    }

    /// The sealed genesis object dated `at`. Its id is the log id.
    ///
    /// `at` is used as given: a writer passes its service time, formatted with
    /// `clock::format_millis`.
    pub fn object(&self, at: &str) -> Value {
        store_object(
            (GENESIS_SCHEMA_REF, GENESIS_SCHEMA_HASH),
            (GENESIS_TYPE, &self.name),
            self.content(),
            &self.signer,
            at,
            &[],
        )
    }

    /// Read the genesis from an object that passed evidence/0 steps 1-5.
    /// `S_GENESIS_INVALID` unless its content is valid store/0 and the object
    /// is exactly the one built from its own name, key and `at`.
    pub fn from_object(o: &Checked) -> Result<Genesis, FindingCode> {
        let invalid = FindingCode::GenesisInvalid;
        let content = o
            .value
            .get("content")
            .and_then(|c| c.get("inline"))
            .ok_or(invalid)?;
        if !exact_members(content, GENESIS_MEMBERS)
            || content.get("v").and_then(Value::as_str) != Some(STORE_V)
        {
            return Err(invalid);
        }
        let field = |k: &str| content.get(k).and_then(Value::as_str).ok_or(invalid);
        let genesis = Genesis::new(field("name")?, field("key")?)?;
        if field("signer")? != genesis.signer {
            return Err(invalid);
        }
        let at = o.value.get("at").and_then(Value::as_str).ok_or(invalid)?;
        if canon::canonicalize(&genesis.object(at)) != canon::canonicalize(&o.value) {
            return Err(invalid);
        }
        Ok(genesis)
    }
}

/// `^[a-z0-9][a-z0-9.-]{0,62}$`
pub fn is_store_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    matches!(bytes.next(), Some(b'a'..=b'z' | b'0'..=b'9'))
        && name.len() <= 63
        && bytes.all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'.' | b'-'))
}

/// `^ed25519:[A-Za-z0-9_-]{42}[AEIMQUYcgkosw048]$`: 32 bytes of unpadded
/// base64url in its one canonical spelling (the last character carries 4
/// data bits and 2 zero bits).
pub fn is_store_key(key: &str) -> bool {
    let Some(b64) = key.strip_prefix(KEY_PREFIX) else {
        return false;
    };
    let b = b64.as_bytes();
    b.len() == 43
        && b[..42]
            .iter()
            .all(|c| c.is_ascii_alphanumeric() || *c == b'-' || *c == b'_')
        && b"AEIMQUYcgkosw048".contains(&b[42])
}

/// True if `v` is an object with exactly these members, each once.
pub(crate) fn exact_members(v: &Value, names: &[&str]) -> bool {
    match v.as_object() {
        Some(members) => {
            members.len() == names.len()
                && members.iter().all(|(k, _)| names.contains(&k.as_str()))
                && names.iter().all(|k| v.get(k).is_some())
        }
        None => false,
    }
}

/// A sealed object the store authors about one of its own subjects:
/// `op` assert, inline JSON content, a service actor, no `uri` and no `ext`.
pub(crate) fn store_object(
    (schema_ref, schema_hash): (&str, &str),
    (subject_type, subject_key): (&str, &str),
    inline: Value,
    signer: &str,
    at: &str,
    prev: &[&str],
) -> Value {
    let s = |t: &str| Value::String(t.to_string());
    let object = Value::Object(vec![
        ("v".into(), s(object::VERSION)),
        ("op".into(), s("assert")),
        (
            "schema".into(),
            Value::Object(vec![
                ("ref".into(), s(schema_ref)),
                ("hash".into(), s(schema_hash)),
            ]),
        ),
        (
            "subject".into(),
            Value::Object(vec![
                ("system".into(), s(STORE_SYSTEM)),
                ("type".into(), s(subject_type)),
                ("key".into(), s(subject_key)),
            ]),
        ),
        (
            "content".into(),
            Value::Object(vec![
                ("media_type".into(), s(object::JSON_MEDIA_TYPE)),
                ("hash".into(), s(&canon::digest(&inline))),
                ("inline".into(), inline),
            ]),
        ),
        (
            "actor".into(),
            Value::Object(vec![
                ("id".into(), s(signer)),
                ("kind".into(), s("service")),
            ]),
        ),
        ("at".into(), s(at)),
        (
            "prev".into(),
            Value::Array(prev.iter().map(|p| s(p)).collect()),
        ),
    ]);
    object::seal(&object)
}

#[cfg(test)]
mod tests {
    use super::*;
    use evidence_core::attest;
    use evidence_core::canon::{canonicalize, sha256_hex};
    use evidence_core::object::check;

    /// One way to change a built object, for the rejection tests.
    type Change<'a> = Box<dyn Fn(Value) -> Value + 'a>;

    const AT: &str = "2026-11-26T00:00:00.000Z";

    fn key() -> String {
        attest::public_key(&[7u8; 32])
    }

    fn checked(v: &Value) -> Checked {
        check(canonicalize(v).as_bytes()).expect("passes evidence/0 steps 1-5")
    }

    /// Change a built genesis and make it a valid evidence/0 object again,
    /// so only the store/0 rules can reject it.
    fn tamper(v: &Value, f: impl Fn(Value) -> Value) -> Value {
        let changed = f(v.without("id"));
        let content = changed.get("content").unwrap();
        let fixed = match content.get("inline") {
            Some(inline) => changed.with(
                "content",
                content.with("hash", Value::String(canon::digest(inline))),
            ),
            None => changed,
        };
        object::seal(&fixed)
    }

    fn with_inline(v: Value, key: &str, value: Value) -> Value {
        let content = v.get("content").unwrap();
        let inline = content.get("inline").unwrap().with(key, value);
        v.with("content", content.with("inline", inline))
    }

    #[test]
    fn new_checks_name_and_key() {
        let g = Genesis::new("cer-demo", &key()).unwrap();
        assert_eq!(g.signer, "service:evidence-store/cer-demo");
        assert_eq!(g.key, key());

        for name in ["a", "0", "a.b-c", &"x".repeat(63)] {
            assert!(Genesis::new(name, &key()).is_ok(), "{name}");
        }
        let long = "x".repeat(64);
        for name in ["", "-a", ".a", "A", "a_b", "a b", "a/b", "é", long.as_str()] {
            assert_eq!(
                Genesis::new(name, &key()),
                Err(FindingCode::GenesisInvalid),
                "{name:?}"
            );
        }
        let k = key();
        let bad_keys = [
            String::new(),
            k.replace("ed25519:", "ed448:"),
            k.replace("ed25519:", ""),
            format!("{k}A"),
            k[..k.len() - 1].to_string(),
            format!("{}=", &k[..k.len() - 1]),
            // A last character with nonzero padding bits is not canonical.
            format!("{}B", &k[..k.len() - 1]),
            format!("{}+", &k[..k.len() - 1]),
        ];
        for bad in &bad_keys {
            assert_eq!(
                Genesis::new("cer-demo", bad),
                Err(FindingCode::GenesisInvalid),
                "{bad:?}"
            );
        }
        // Every key the core writes is accepted.
        for seed in 0u8..32 {
            assert!(is_store_key(&attest::public_key(&[seed; 32])));
        }
    }

    #[test]
    fn the_object_has_exactly_the_spec_form() {
        let g = Genesis::new("cer-demo", &key()).unwrap();
        let inline = format!(
            r#"{{"key":"{}","name":"cer-demo","signer":"service:evidence-store/cer-demo","v":"store/0"}}"#,
            key()
        );
        let content = format!(
            r#""content":{{"hash":"{}","inline":{inline},"media_type":"application/json"}}"#,
            sha256_hex(inline.as_bytes())
        );
        let head = format!(
            r#"{{"actor":{{"id":"service:evidence-store/cer-demo","kind":"service"}},"at":"{AT}",{content}"#
        );
        let tail = format!(
            r#""op":"assert","prev":[],"schema":{{"hash":"{GENESIS_SCHEMA_HASH}","ref":"evidence-store/genesis@0"}},"subject":{{"key":"cer-demo","system":"evidence-store","type":"log"}},"v":"evidence/0"}}"#
        );
        let id = sha256_hex(format!("{head},{tail}").as_bytes());
        let want = format!(r#"{head},"id":"{id}",{tail}"#);

        let object = g.object(AT);
        assert_eq!(canonicalize(&object), want);
        assert_eq!(canonicalize(&g.content()), inline);

        let c = checked(&object);
        assert_eq!(c.id, id);
        assert!(c.content_checked);
        assert!(c.prev.is_empty());
    }

    #[test]
    fn same_inputs_same_log_id() {
        let g = Genesis::new("cer-demo", &key()).unwrap();
        assert_eq!(canonicalize(&g.object(AT)), canonicalize(&g.object(AT)));
        let other_time = g.object("2026-11-26T00:00:00.001Z");
        let other_name = Genesis::new("cer-demo2", &key()).unwrap().object(AT);
        let other_key = Genesis::new("cer-demo", &attest::public_key(&[8u8; 32]))
            .unwrap()
            .object(AT);
        let id = |v: &Value| v.get("id").unwrap().clone();
        let base = id(&g.object(AT));
        for other in [other_time, other_name, other_key] {
            assert_ne!(id(&other), base);
        }
    }

    #[test]
    fn from_object_round_trips() {
        for name in ["cer-demo", "a", "store.example-1"] {
            let g = Genesis::new(name, &key()).unwrap();
            for at in [AT, "2026-11-26T00:00:00Z", "2026-11-26T00:00:00.123456789Z"] {
                let c = checked(&g.object(at));
                assert_eq!(Genesis::from_object(&c), Ok(g.clone()), "{name} {at}");
            }
        }
    }

    #[test]
    fn from_object_rejects_anything_else() {
        let g = Genesis::new("cer-demo", &key()).unwrap();
        let good = g.object(AT);
        let s = |t: &str| Value::String(t.to_string());
        let obj = |members: &[(&str, Value)]| {
            Value::Object(
                members
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.clone()))
                    .collect(),
            )
        };

        let changes: Vec<(&str, Change<'_>)> = vec![
            (
                "inline name differs from subject key",
                Box::new(|v| {
                    let v = with_inline(v, "name", s("other"));
                    with_inline(v, "signer", s("service:evidence-store/other"))
                }),
            ),
            (
                "subject key differs from name",
                Box::new(|v| {
                    let subject = v.get("subject").unwrap().with("key", s("other"));
                    v.with("subject", subject)
                }),
            ),
            (
                "signer not prefix + name",
                Box::new(|v| with_inline(v, "signer", s("service:other"))),
            ),
            (
                "actor not the signer",
                Box::new(|v| {
                    let actor = v.get("actor").unwrap().with("id", s("service:other"));
                    v.with("actor", actor)
                }),
            ),
            (
                "actor kind",
                Box::new(|v| {
                    let actor = v.get("actor").unwrap().with("kind", s("human"));
                    v.with("actor", actor)
                }),
            ),
            ("inline v", Box::new(|v| with_inline(v, "v", s("store/1")))),
            (
                "inline extra member",
                Box::new(|v| with_inline(v, "note", s("x"))),
            ),
            (
                "inline key not a key",
                Box::new(|v| with_inline(v, "key", s("ed25519:abc"))),
            ),
            (
                "inline name invalid",
                Box::new(|v| {
                    let v = with_inline(v, "name", s("Cer"));
                    with_inline(v, "signer", s("service:evidence-store/Cer"))
                }),
            ),
            (
                "inline not an object",
                Box::new(|v| {
                    let content = v.get("content").unwrap().with("inline", s("store/0"));
                    v.with("content", content)
                }),
            ),
            (
                "no inline content",
                Box::new(|v| {
                    let content = obj(&[
                        ("media_type", s("application/json")),
                        (
                            "hash",
                            v.get("content").unwrap().get("hash").unwrap().clone(),
                        ),
                    ]);
                    v.with("content", content)
                }),
            ),
            (
                "content uri",
                Box::new(|v| {
                    let content = v
                        .get("content")
                        .unwrap()
                        .with("uri", s("https://example.org/g"));
                    v.with("content", content)
                }),
            ),
            (
                "schema ref",
                Box::new(|v| {
                    let schema = v
                        .get("schema")
                        .unwrap()
                        .with("ref", s("evidence-store/genesis@1"));
                    v.with("schema", schema)
                }),
            ),
            (
                "schema hash",
                Box::new(|v| {
                    let schema = v
                        .get("schema")
                        .unwrap()
                        .with("hash", s(&format!("sha256:{}", "0".repeat(64))));
                    v.with("schema", schema)
                }),
            ),
            (
                "subject system",
                Box::new(|v| {
                    let subject = v.get("subject").unwrap().with("system", s("other"));
                    v.with("subject", subject)
                }),
            ),
            (
                "subject type",
                Box::new(|v| {
                    let subject = v.get("subject").unwrap().with("type", s("checkpoint"));
                    v.with("subject", subject)
                }),
            ),
            ("op", Box::new(|v| v.with("op", s("create")))),
            (
                "prev",
                Box::new(|v| {
                    v.with(
                        "prev",
                        Value::Array(vec![s(&format!("sha256:{}", "1".repeat(64)))]),
                    )
                }),
            ),
            (
                "ext",
                Box::new(|v| v.with("ext", obj(&[("centr/0", obj(&[]))]))),
            ),
        ];
        for (what, change) in &changes {
            let bad = tamper(&good, change);
            let c = checked(&bad);
            assert_eq!(
                Genesis::from_object(&c),
                Err(FindingCode::GenesisInvalid),
                "{what}"
            );
        }
        // The untampered object passes through the same helper.
        let same = tamper(&good, |v| v);
        assert_eq!(canonicalize(&same), canonicalize(&good));
        assert_eq!(Genesis::from_object(&checked(&same)), Ok(g));
    }
}
