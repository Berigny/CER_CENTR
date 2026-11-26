//! The checkpoint object (checkpoint/0): the store's signed statement of the
//! log's size, head and Merkle tree hash. See `spec/store-0.md`, "Checkpoint
//! and tree", and `spec/schema/checkpoint-0.schema.json`.
//!
//! A checkpoint object is fully determined by its content, the genesis signer,
//! its `at` and its `prev` (the previous checkpoint, if any).

use evidence_core::canon;
use evidence_core::json::{Value, MAX_SAFE_INTEGER};
use evidence_core::object::{is_hash, Checked};

use crate::genesis::{exact_members, store_object, Genesis};
use crate::report::FindingCode;
use crate::schemas::{CHECKPOINT_SCHEMA_HASH, CHECKPOINT_SCHEMA_REF};

/// `v` of the checkpoint content.
pub const CHECKPOINT_V: &str = "checkpoint/0";

/// `subject.type` of a checkpoint. Its `subject.key` is the log id.
pub const CHECKPOINT_TYPE: &str = "checkpoint";

/// The members of the checkpoint content. All are required and no others are allowed.
pub const CHECKPOINT_MEMBERS: &[&str] = &["v", "log", "size", "head", "root"];

/// The checkpoint content: which log, how many entries it covers, the entry
/// hash at that size, and the tree hash over entries 1 to `size`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checkpoint {
    pub log: String,
    /// The seq of the last entry before the checkpoint's own entry.
    pub size: u64,
    /// The entry hash of seq = `size`.
    pub head: String,
    /// The RFC 9162 tree hash over entries 1 to `size`, as `"sha256:"` + hex.
    pub root: String,
}

impl Checkpoint {
    /// The checkpoint content (`content.inline`).
    pub fn content(&self) -> Value {
        Value::Object(vec![
            ("v".into(), Value::String(CHECKPOINT_V.into())),
            ("log".into(), Value::String(self.log.clone())),
            ("size".into(), Value::Number(self.size as f64)),
            ("head".into(), Value::String(self.head.clone())),
            ("root".into(), Value::String(self.root.clone())),
        ])
    }

    /// The sealed checkpoint object, authored by the genesis signer, dated
    /// `at` (the store's service time), with `prev` the previous checkpoint's
    /// id, or none for the first checkpoint.
    pub fn object(&self, genesis: &Genesis, at: &str, prev: Option<&str>) -> Value {
        self.build(&genesis.signer, at, prev)
    }

    fn build(&self, signer: &str, at: &str, prev: Option<&str>) -> Value {
        let prev: Vec<&str> = prev.into_iter().collect();
        store_object(
            (CHECKPOINT_SCHEMA_REF, CHECKPOINT_SCHEMA_HASH),
            (CHECKPOINT_TYPE, &self.log),
            self.content(),
            signer,
            at,
            &prev,
        )
    }

    /// Read a checkpoint content value. `S_CHECKPOINT_INVALID` unless it is
    /// valid checkpoint/0: exactly the five members, `v` constant, hashes in
    /// `log`, `head` and `root`, and `size` an integer from 1 to 2^53 − 1.
    pub fn from_content(v: &Value) -> Result<Checkpoint, FindingCode> {
        let invalid = FindingCode::CheckpointInvalid;
        if !exact_members(v, CHECKPOINT_MEMBERS)
            || v.get("v").and_then(Value::as_str) != Some(CHECKPOINT_V)
        {
            return Err(invalid);
        }
        let hash = |k: &str| match v.get(k).and_then(Value::as_str) {
            Some(s) if is_hash(s) => Ok(s.to_string()),
            _ => Err(invalid),
        };
        let size = match v.get("size") {
            Some(Value::Number(n)) if *n >= 1.0 && *n <= MAX_SAFE_INTEGER && n.fract() == 0.0 => {
                *n as u64
            }
            _ => return Err(invalid),
        };
        Ok(Checkpoint {
            log: hash("log")?,
            size,
            head: hash("head")?,
            root: hash("root")?,
        })
    }

    /// Read the checkpoint from an object that passed evidence/0 steps 1-5.
    /// `S_CHECKPOINT_INVALID` unless its content is valid checkpoint/0 and the
    /// object is exactly the one built from that content, its actor (the
    /// signer), its `at` and its `prev` (none, or one previous checkpoint).
    ///
    /// The checks that need the rest of the log are the verifier's: that the
    /// actor is the genesis signer, that `log` is the log id, and that `size`,
    /// `head` and `root` agree with the lines.
    ///
    /// Which id `prev` holds is not checked, here or by the verifier: that it
    /// names the previous checkpoint (none for the first) is a writer rule.
    /// A checkpoint is checked on its own against the lines, so the link adds
    /// nothing to verification. See `spec/store-0.md`, step j.
    pub fn from_object(o: &Checked) -> Result<Checkpoint, FindingCode> {
        let invalid = FindingCode::CheckpointInvalid;
        let content = o
            .value
            .get("content")
            .and_then(|c| c.get("inline"))
            .ok_or(invalid)?;
        let checkpoint = Checkpoint::from_content(content)?;
        let signer = o
            .value
            .get("actor")
            .and_then(|a| a.get("id"))
            .and_then(Value::as_str)
            .ok_or(invalid)?;
        let at = o.value.get("at").and_then(Value::as_str).ok_or(invalid)?;
        let prev = match o.prev.as_slice() {
            [] => None,
            [one] => Some(one.as_str()),
            _ => return Err(invalid),
        };
        let rebuilt = checkpoint.build(signer, at, prev);
        if canon::canonicalize(&rebuilt) != canon::canonicalize(&o.value) {
            return Err(invalid);
        }
        Ok(checkpoint)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evidence_core::attest;
    use evidence_core::canon::{canonicalize, sha256_hex};
    use evidence_core::object::{self, check};

    /// One way to change a built object, for the rejection tests.
    type Change<'a> = Box<dyn Fn(Value) -> Value + 'a>;

    const AT: &str = "2026-11-26T00:00:05.000Z";

    fn h(c: char) -> String {
        format!("sha256:{}", c.to_string().repeat(64))
    }

    fn genesis() -> Genesis {
        Genesis::new("cer-demo", &attest::public_key(&[7u8; 32])).unwrap()
    }

    fn sample() -> Checkpoint {
        Checkpoint {
            log: h('a'),
            size: 10,
            head: h('b'),
            root: h('c'),
        }
    }

    fn checked(v: &Value) -> Checked {
        check(canonicalize(v).as_bytes()).expect("passes evidence/0 steps 1-5")
    }

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
    fn the_object_has_exactly_the_spec_form() {
        let cp = sample();
        let prev = h('d');
        let object = cp.object(&genesis(), AT, Some(&prev));
        let inline = format!(
            r#"{{"head":"{}","log":"{}","root":"{}","size":10,"v":"checkpoint/0"}}"#,
            h('b'),
            h('a'),
            h('c')
        );
        let head = format!(
            r#"{{"actor":{{"id":"service:evidence-store/cer-demo","kind":"service"}},"at":"{AT}","content":{{"hash":"{}","inline":{inline},"media_type":"application/json"}}"#,
            sha256_hex(inline.as_bytes())
        );
        let tail = format!(
            r#""op":"assert","prev":["{prev}"],"schema":{{"hash":"{CHECKPOINT_SCHEMA_HASH}","ref":"evidence-store/checkpoint@0"}},"subject":{{"key":"{}","system":"evidence-store","type":"checkpoint"}},"v":"evidence/0"}}"#,
            h('a')
        );
        let id = sha256_hex(format!("{head},{tail}").as_bytes());
        assert_eq!(
            canonicalize(&object),
            format!(r#"{head},"id":"{id}",{tail}"#)
        );
        assert_eq!(canonicalize(&cp.content()), inline);

        // The first checkpoint has no prev.
        let first = cp.object(&genesis(), AT, None);
        assert_eq!(first.get("prev"), Some(&Value::Array(vec![])));
        let c = checked(&first);
        assert!(c.content_checked);
    }

    #[test]
    fn from_object_round_trips() {
        let g = genesis();
        for cp in [
            sample(),
            Checkpoint {
                size: 1,
                ..sample()
            },
            Checkpoint {
                size: 9_007_199_254_740_991,
                ..sample()
            },
        ] {
            for prev in [None, Some(h('e'))] {
                let c = checked(&cp.object(&g, AT, prev.as_deref()));
                assert_eq!(Checkpoint::from_object(&c), Ok(cp.clone()));
                assert_eq!(c.prev, prev.iter().cloned().collect::<Vec<_>>());
            }
        }
        // The signer is read from the object: the verifier compares it with
        // the genesis signer.
        let other = Genesis::new("other", &attest::public_key(&[8u8; 32])).unwrap();
        let c = checked(&sample().object(&other, AT, None));
        assert_eq!(Checkpoint::from_object(&c), Ok(sample()));
        assert_eq!(
            c.value.get("actor").unwrap().get("id").unwrap().as_str(),
            Some("service:evidence-store/other")
        );
    }

    #[test]
    fn from_content_rules() {
        let good = sample().content();
        assert_eq!(Checkpoint::from_content(&good), Ok(sample()));
        let s = |t: &str| Value::String(t.to_string());
        let n = Value::Number;
        let invalid = Err(FindingCode::CheckpointInvalid);
        for k in CHECKPOINT_MEMBERS {
            assert_eq!(Checkpoint::from_content(&good.without(k)), invalid, "{k}");
        }
        let bad: &[(&str, Value)] = &[
            ("extra", n(1.0)),
            ("v", s("checkpoint/1")),
            ("log", s("sha256:1")),
            ("head", s(&h('B'))),
            ("root", Value::Null),
            ("size", n(0.0)),
            ("size", n(-3.0)),
            ("size", n(2.5)),
            ("size", n(9_007_199_254_740_992.0)),
            ("size", s("10")),
        ];
        for (k, v) in bad {
            assert_eq!(
                Checkpoint::from_content(&good.with(k, v.clone())),
                invalid,
                "{k}"
            );
        }
        assert_eq!(Checkpoint::from_content(&s("x")), invalid);
        let mut repeated = good.without("size").as_object().unwrap().to_vec();
        repeated.push(("log".into(), s(&h('a'))));
        assert_eq!(Checkpoint::from_content(&Value::Object(repeated)), invalid);
    }

    #[test]
    fn from_object_rejects_anything_else() {
        let g = genesis();
        let good = sample().object(&g, AT, Some(&h('e')));
        let s = |t: &str| Value::String(t.to_string());
        let changes: Vec<(&str, Change<'_>)> = vec![
            (
                "subject key is not the log",
                Box::new(|v| {
                    let subject = v.get("subject").unwrap().with("key", s(&h('f')));
                    v.with("subject", subject)
                }),
            ),
            (
                "subject type",
                Box::new(|v| {
                    let subject = v.get("subject").unwrap().with("type", s("log"));
                    v.with("subject", subject)
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
                "schema ref",
                Box::new(|v| {
                    let schema = v
                        .get("schema")
                        .unwrap()
                        .with("ref", s("evidence-store/checkpoint@1"));
                    v.with("schema", schema)
                }),
            ),
            (
                "schema hash",
                Box::new(|v| {
                    let schema = v.get("schema").unwrap().with("hash", s(&h('0')));
                    v.with("schema", schema)
                }),
            ),
            ("op", Box::new(|v| v.with("op", s("update")))),
            (
                "actor kind",
                Box::new(|v| {
                    let actor = v.get("actor").unwrap().with("kind", s("human"));
                    v.with("actor", actor)
                }),
            ),
            (
                "two prevs",
                Box::new(|v| v.with("prev", Value::Array(vec![s(&h('e')), s(&h('f'))]))),
            ),
            (
                "content uri",
                Box::new(|v| {
                    let content = v
                        .get("content")
                        .unwrap()
                        .with("uri", s("https://example.org/c"));
                    v.with("content", content)
                }),
            ),
            (
                "ext",
                Box::new(|v| {
                    v.with(
                        "ext",
                        Value::Object(vec![("centr/0".into(), Value::Object(vec![]))]),
                    )
                }),
            ),
            (
                "inline v",
                Box::new(|v| with_inline(v, "v", s("checkpoint/1"))),
            ),
            (
                "inline size",
                Box::new(|v| with_inline(v, "size", Value::Number(0.0))),
            ),
            ("inline extra", Box::new(|v| with_inline(v, "note", s("x")))),
            (
                "no inline",
                Box::new(|v| {
                    let content = v.get("content").unwrap().without("inline");
                    v.with("content", content)
                }),
            ),
        ];
        for (what, change) in &changes {
            let c = checked(&tamper(&good, change));
            assert_eq!(
                Checkpoint::from_object(&c),
                Err(FindingCode::CheckpointInvalid),
                "{what}"
            );
        }
        // Changing log in the content and the subject key together is still a
        // valid checkpoint/0 object (for another log): the verifier, not this
        // function, compares log with the log id.
        let moved = tamper(&good, |v| {
            let v = with_inline(v, "log", s(&h('f')));
            let subject = v.get("subject").unwrap().with("key", s(&h('f')));
            v.with("subject", subject)
        });
        assert_eq!(
            Checkpoint::from_object(&checked(&moved)).map(|c| c.log),
            Ok(h('f'))
        );
    }
}
