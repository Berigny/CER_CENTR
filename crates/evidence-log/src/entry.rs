//! The log entry, `log-entry/0`: the store's own facts about one item in the
//! log, kept beside the item and never inside it. See `spec/store-0.md`, "Log
//! entry", and `spec/schema/log-entry-0.schema.json`.
//!
//! [`Entry::from_value`] checks that an entry is valid on its own. The rules
//! that relate an entry to the rest of the log (dense seq, the chain, the log
//! id, non-decreasing `received_at` and epoch) are checked by the verifier,
//! each with its own code.

use evidence_core::canon;
use evidence_core::json::{self, Value, MAX_SAFE_INTEGER};
use evidence_core::object::is_hash;
use evidence_core::time;

use crate::report::FindingCode;

/// `v` of every log entry.
pub const ENTRY_V: &str = "log-entry/0";

/// The members of a log entry. All are required and no others are allowed.
pub const ENTRY_MEMBERS: &[&str] = &[
    "v",
    "log",
    "seq",
    "kind",
    "ref",
    "received_at",
    "epoch",
    "prev",
];

/// What the entry's item is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// An evidence/0 object; `ref` is its id.
    Object,
    /// An attestation/0 line; `ref` is the hash of its canonical form.
    Attestation,
}

impl Kind {
    /// `"object"` or `"attestation"`, as written in `entry.kind`.
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Object => "object",
            Kind::Attestation => "attestation",
        }
    }

    /// The kind named by `s`, if it names one.
    pub fn parse(s: &str) -> Option<Kind> {
        match s {
            "object" => Some(Kind::Object),
            "attestation" => Some(Kind::Attestation),
            _ => None,
        }
    }
}

/// One log entry. `reference` is the `ref` member (a Rust keyword).
///
/// An entry built by [`Entry::from_value`] is valid log-entry/0. One built by
/// hand is only valid if its fields follow the same rules: `seq` and `epoch`
/// between 1 and 2^53 − 1, hashes in `log`, `reference` and `prev`, and a
/// real timestamp in `received_at`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub log: String,
    pub seq: u64,
    pub kind: Kind,
    pub reference: String,
    pub received_at: String,
    pub epoch: u64,
    pub prev: String,
}

impl Entry {
    /// The entry as a JSON object.
    pub fn to_value(&self) -> Value {
        Value::Object(vec![
            ("v".into(), Value::String(ENTRY_V.into())),
            ("log".into(), Value::String(self.log.clone())),
            ("seq".into(), Value::Number(self.seq as f64)),
            ("kind".into(), Value::String(self.kind.as_str().into())),
            ("ref".into(), Value::String(self.reference.clone())),
            (
                "received_at".into(),
                Value::String(self.received_at.clone()),
            ),
            ("epoch".into(), Value::Number(self.epoch as f64)),
            ("prev".into(), Value::String(self.prev.clone())),
        ])
    }

    /// The RFC 8785 canonical form: the entry's leaf bytes in the Merkle tree.
    pub fn canonical(&self) -> String {
        canon::canonicalize(&self.to_value())
    }

    /// The entry hash: `"sha256:"` + hex SHA-256 of the canonical form.
    pub fn hash(&self) -> String {
        canon::sha256_hex(self.canonical().as_bytes())
    }

    /// Check that `v` is valid log-entry/0 and read it. Every failure is
    /// `S_ENTRY_INVALID`.
    pub fn from_value(v: &Value) -> Result<Entry, FindingCode> {
        let invalid = FindingCode::EntryInvalid;
        let members = v.as_object().ok_or(invalid)?;
        // Exactly the required members: as many as there are names, each one
        // allowed and each one present, so none can be missing or repeated.
        if members.len() != ENTRY_MEMBERS.len()
            || members
                .iter()
                .any(|(k, _)| !ENTRY_MEMBERS.contains(&k.as_str()))
            || ENTRY_MEMBERS.iter().any(|k| v.get(k).is_none())
        {
            return Err(invalid);
        }
        let get = |k: &str| v.get(k).ok_or(invalid);
        let hash = |k: &str| match get(k)?.as_str() {
            Some(s) if is_hash(s) => Ok(s.to_string()),
            _ => Err(invalid),
        };

        if get("v")?.as_str() != Some(ENTRY_V) {
            return Err(invalid);
        }
        let kind = get("kind")?.as_str().and_then(Kind::parse).ok_or(invalid)?;
        let received_at = get("received_at")?.as_str().ok_or(invalid)?;
        // The pattern and a real calendar instant.
        if time::parse(received_at).is_err() {
            return Err(invalid);
        }
        Ok(Entry {
            log: hash("log")?,
            seq: position(get("seq")?).ok_or(invalid)?,
            kind,
            reference: hash("ref")?,
            received_at: received_at.to_string(),
            epoch: position(get("epoch")?).ok_or(invalid)?,
            prev: hash("prev")?,
        })
    }

    /// Parse one entry from JSON text under evidence/0's strict rules. Text
    /// that is not strict JSON is `S_ENTRY_INVALID` too.
    pub fn parse(bytes: &[u8]) -> Result<Entry, FindingCode> {
        let v = json::parse(bytes).map_err(|_| FindingCode::EntryInvalid)?;
        Entry::from_value(&v)
    }
}

/// An integer from 1 to 2^53 − 1 written as a JSON number with no fractional
/// part (`seq` and `epoch`).
fn position(v: &Value) -> Option<u64> {
    match v {
        Value::Number(n) if *n >= 1.0 && *n <= MAX_SAFE_INTEGER && n.fract() == 0.0 => {
            Some(*n as u64)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(c: char) -> String {
        format!("sha256:{}", c.to_string().repeat(64))
    }

    fn sample() -> Entry {
        Entry {
            log: h('a'),
            seq: 3,
            kind: Kind::Object,
            reference: h('b'),
            received_at: "2026-11-26T00:00:01.000Z".into(),
            epoch: 1,
            prev: h('c'),
        }
    }

    fn with(entry: &Entry, key: &str, value: Value) -> Value {
        entry.to_value().with(key, value)
    }

    fn parse_text(text: &str) -> Result<Entry, FindingCode> {
        Entry::parse(text.as_bytes())
    }

    #[test]
    fn canonical_form_sorts_members_and_writes_integers() {
        let e = sample();
        assert_eq!(
            e.canonical(),
            format!(
                "{{\"epoch\":1,\"kind\":\"object\",\"log\":\"{}\",\"prev\":\"{}\",\
                 \"received_at\":\"2026-11-26T00:00:01.000Z\",\"ref\":\"{}\",\"seq\":3,\
                 \"v\":\"log-entry/0\"}}",
                h('a'),
                h('c'),
                h('b')
            )
        );
        assert_eq!(e.hash(), canon::sha256_hex(e.canonical().as_bytes()));
        assert_eq!(e.hash(), canon::digest(&e.to_value()));
    }

    #[test]
    fn round_trips() {
        let genesis = Entry {
            log: h('1'),
            seq: 1,
            kind: Kind::Object,
            reference: h('1'),
            received_at: "2026-11-26T00:00:00.000Z".into(),
            epoch: 1,
            prev: h('1'),
        };
        let attestation = Entry {
            kind: Kind::Attestation,
            seq: 2,
            ..genesis.clone()
        };
        let nanos = Entry {
            received_at: "2026-11-26T00:00:01.123456789Z".into(),
            ..sample()
        };
        let big = Entry {
            seq: 9_007_199_254_740_991,
            epoch: 7,
            received_at: "2026-11-26T00:00:01Z".into(),
            ..sample()
        };
        for e in [sample(), genesis, attestation, nanos, big] {
            assert_eq!(Entry::from_value(&e.to_value()), Ok(e.clone()));
            assert_eq!(parse_text(&e.canonical()), Ok(e.clone()));
            // Re-encoding a parsed entry gives the same bytes and hash.
            let again = parse_text(&e.canonical()).unwrap();
            assert_eq!(again.canonical(), e.canonical());
            assert_eq!(again.hash(), e.hash());
        }
        assert_eq!(
            Entry::from_value(&sample().to_value())
                .unwrap()
                .kind
                .as_str(),
            "object"
        );
    }

    #[test]
    fn member_order_in_the_input_does_not_matter() {
        let e = sample();
        let mut members = e.to_value().as_object().unwrap().to_vec();
        members.reverse();
        assert_eq!(Entry::from_value(&Value::Object(members)), Ok(e));
    }

    #[test]
    fn rejects_every_invalid_entry() {
        let e = sample();
        let invalid = Err(FindingCode::EntryInvalid);
        let s = |t: &str| Value::String(t.into());
        let n = Value::Number;

        // Not an object.
        for v in [
            Value::Null,
            n(1.0),
            s("x"),
            Value::Array(vec![e.to_value()]),
        ] {
            assert_eq!(Entry::from_value(&v), invalid, "{v:?}");
        }

        // Member set: one missing, one extra, one repeated.
        for k in ENTRY_MEMBERS {
            assert_eq!(Entry::from_value(&e.to_value().without(k)), invalid, "{k}");
        }
        assert_eq!(Entry::from_value(&with(&e, "extra", n(1.0))), invalid);
        let mut repeated = e.to_value().without("prev").as_object().unwrap().to_vec();
        repeated.push(("seq".into(), n(3.0)));
        assert_eq!(Entry::from_value(&Value::Object(repeated)), invalid);

        let bad: &[(&str, Value)] = &[
            ("v", s("log-entry/1")),
            ("v", s("")),
            ("v", n(0.0)),
            // seq and epoch: integers from 1 to 2^53 - 1
            ("seq", n(0.0)),
            ("seq", n(-1.0)),
            ("seq", n(1.5)),
            ("seq", n(9_007_199_254_740_992.0)),
            ("seq", n(f64::NAN)),
            ("seq", n(f64::INFINITY)),
            ("seq", s("3")),
            ("seq", Value::Null),
            ("epoch", n(0.0)),
            ("epoch", n(2.5)),
            ("epoch", s("1")),
            ("epoch", Value::Bool(true)),
            // kind: an enum
            ("kind", s("blob")),
            ("kind", s("Object")),
            ("kind", n(1.0)),
            // hashes
            ("log", s("sha256:abc")),
            ("ref", s(&h('A'))),
            ("prev", s("not a hash")),
            ("prev", s(&format!("sha512:{}", "a".repeat(64)))),
            ("prev", Value::Null),
            // received_at: an evidence/0 timestamp that is a real instant
            ("received_at", s("2026-11-26T11:00:01.000+11:00")),
            ("received_at", s("2026-11-26T00:00:01.0000000001Z")),
            ("received_at", s("2026-11-26 00:00:01Z")),
            ("received_at", s("2026-02-30T00:00:00Z")),
            ("received_at", s("2026-11-26T24:00:00Z")),
            ("received_at", s("")),
            ("received_at", n(0.0)),
        ];
        for (k, v) in bad {
            assert_eq!(
                Entry::from_value(&with(&e, k, v.clone())),
                invalid,
                "{k}: {v:?}"
            );
        }
    }

    #[test]
    fn parse_rejects_text_that_is_not_strict_json() {
        let e = sample();
        let text = e.canonical();
        assert_eq!(parse_text(&text), Ok(e.clone()));
        let invalid = Err(FindingCode::EntryInvalid);
        assert_eq!(parse_text(""), invalid);
        assert_eq!(parse_text(&text[..text.len() - 1]), invalid);
        // A duplicate member is a parse error, not "last one wins".
        assert_eq!(
            parse_text(&text.replacen("\"epoch\":1,", "\"epoch\":1,\"epoch\":1,", 1)),
            invalid
        );
        // seq 1.5 and a number above 2^53 - 1.
        assert_eq!(
            parse_text(&text.replace("\"seq\":3", "\"seq\":1.5")),
            invalid
        );
        assert_eq!(
            parse_text(&text.replace("\"seq\":3", "\"seq\":9007199254740992")),
            invalid
        );
        // An integer written with an exponent or a zero fraction is still an
        // integer; the entry is valid (its canonical form differs, which only
        // a segment line's canonical check cares about).
        assert_eq!(
            parse_text(&text.replace("\"seq\":3", "\"seq\":3.0")),
            Ok(e.clone())
        );
        assert_eq!(parse_text(&text.replace("\"seq\":3", "\"seq\":3e0")), Ok(e));
    }
}
