//! The evidence/0 object: shape rules (mirroring `spec/schema/evidence-object-0.schema.json`),
//! id computation, and the integrity checks that need nothing outside the object.

use std::collections::BTreeSet;
use std::fmt::Display;

use crate::canon;
use crate::error::{Code, Error, Result};
use crate::json::{self, Value};
use crate::time::{self, Timestamp, TimestampError};

pub const VERSION: &str = "evidence/0";
pub const OPS: &[&str] = &["create", "update", "delete", "assert"];
pub const ACTOR_KINDS: &[&str] = &["human", "service", "agent"];
pub const JSON_MEDIA_TYPE: &str = "application/json";

/// An object that passed every check that needs only its own bytes.
#[derive(Debug, Clone)]
pub struct Checked {
    pub value: Value,
    pub id: String,
    pub at: Timestamp,
    pub prev: Vec<String>,
    /// True when `content.inline` was present and matched `content.hash`.
    /// False means the content bytes live elsewhere and were not checked.
    pub content_checked: bool,
}

/// `sha256` of the canonical object with `id` removed.
pub fn compute_id(v: &Value) -> String {
    canon::digest(&v.without("id"))
}

/// The object with `id` set to its computed value.
pub fn seal(v: &Value) -> Value {
    v.with("id", Value::String(compute_id(v)))
}

/// Parse, shape, timestamp, id and inline-content checks, in that order.
/// The first failure is returned.
pub fn check(bytes: &[u8]) -> Result<Checked> {
    let value = json::parse(bytes)?;
    check_value(value)
}

pub fn check_value(value: Value) -> Result<Checked> {
    check_shape(&value)?;
    let at_text = value
        .get("at")
        .and_then(Value::as_str)
        .expect("shape checked");
    let at = match time::parse(at_text) {
        Ok(t) => t,
        Err(TimestampError::Pattern) => unreachable!("shape checked"),
        Err(TimestampError::Calendar) => {
            return Err(Error::new(
                Code::Timestamp,
                format!("at: {at_text:?} is not a real UTC instant"),
            ))
        }
    };

    let declared = value
        .get("id")
        .and_then(Value::as_str)
        .expect("shape checked")
        .to_string();
    let computed = compute_id(&value);
    if declared != computed {
        return Err(Error::new(
            Code::IdMismatch,
            format!("id is {declared} but the object hashes to {computed}"),
        ));
    }

    let content = value.get("content").expect("shape checked");
    let content_checked = match content.get("inline") {
        Some(inline) => {
            let declared = content
                .get("hash")
                .and_then(Value::as_str)
                .expect("shape checked");
            let computed = canon::digest(inline);
            if declared != computed {
                return Err(Error::new(
                    Code::ContentHashMismatch,
                    format!("content.hash is {declared} but content.inline hashes to {computed}"),
                ));
            }
            true
        }
        None => false,
    };

    let prev = value
        .get("prev")
        .and_then(Value::as_array)
        .expect("shape checked")
        .iter()
        .map(|h| h.as_str().expect("shape checked").to_string())
        .collect();

    Ok(Checked {
        value,
        id: declared,
        at,
        prev,
        content_checked,
    })
}

/// The JSON Schema rules, hand-written so the core has no schema-engine
/// dependency. `tools/check_schema_vectors.py` keeps the two in step.
pub fn check_shape(v: &Value) -> Result<()> {
    members(
        v,
        "",
        &[
            "v", "id", "op", "schema", "subject", "content", "actor", "at", "prev",
        ],
        &["ext"],
    )?;
    if v.get("v").and_then(Value::as_str) != Some(VERSION) {
        return Err(shape("/v", format!("must be {VERSION:?}")));
    }
    hash(v.get("id").unwrap(), "/id")?;
    one_of(v.get("op").unwrap(), "/op", OPS)?;

    let schema = v.get("schema").unwrap();
    members(schema, "/schema", &["ref", "hash"], &[])?;
    text(schema.get("ref").unwrap(), "/schema/ref")?;
    hash(schema.get("hash").unwrap(), "/schema/hash")?;

    let subject = v.get("subject").unwrap();
    members(subject, "/subject", &["system", "type", "key"], &[])?;
    for k in ["system", "type", "key"] {
        text(subject.get(k).unwrap(), &format!("/subject/{k}"))?;
    }

    let content = v.get("content").unwrap();
    members(
        content,
        "/content",
        &["media_type", "hash"],
        &["uri", "inline"],
    )?;
    let media_type = text(content.get("media_type").unwrap(), "/content/media_type")?;
    if !is_media_type(media_type) {
        return Err(shape(
            "/content/media_type",
            "must be a lowercase type/subtype",
        ));
    }
    hash(content.get("hash").unwrap(), "/content/hash")?;
    if let Some(uri) = content.get("uri") {
        text(uri, "/content/uri")?;
    }
    if content.get("inline").is_some() && media_type != JSON_MEDIA_TYPE {
        return Err(shape(
            "/content/media_type",
            format!("must be {JSON_MEDIA_TYPE:?} when content.inline is present"),
        ));
    }

    let actor = v.get("actor").unwrap();
    members(actor, "/actor", &["id", "kind"], &["on_behalf_of"])?;
    text(actor.get("id").unwrap(), "/actor/id")?;
    let kind = one_of(actor.get("kind").unwrap(), "/actor/kind", ACTOR_KINDS)?;
    match actor.get("on_behalf_of") {
        Some(sponsor) => {
            text(sponsor, "/actor/on_behalf_of")?;
        }
        None if kind == "agent" => {
            return Err(shape("/actor", "an agent must name on_behalf_of"));
        }
        None => {}
    }

    let at = text(v.get("at").unwrap(), "/at")?;
    if !time::matches_pattern(at) {
        return Err(shape("/at", "must be UTC: YYYY-MM-DDTHH:MM:SS[.fraction]Z"));
    }

    let prev = v
        .get("prev")
        .unwrap()
        .as_array()
        .ok_or_else(|| shape("/prev", "expected array"))?;
    // The first id that repeats an earlier one, found in O(n log n): a prev
    // list comes from untrusted input and may be long.
    let mut seen = BTreeSet::new();
    for (i, h) in prev.iter().enumerate() {
        let h = hash(h, &format!("/prev/{i}"))?;
        if !seen.insert(h) {
            return Err(shape(&format!("/prev/{i}"), "duplicate hash"));
        }
    }

    if let Some(ext) = v.get("ext") {
        let ext = ext
            .as_object()
            .ok_or_else(|| shape("/ext", "expected object"))?;
        for (k, _) in ext {
            if !is_ext_key(k) {
                return Err(shape(
                    "/ext",
                    format!("{k:?} is not a profile id like \"centr/0\""),
                ));
            }
        }
    }
    Ok(())
}

fn shape(path: &str, msg: impl Display) -> Error {
    let path = if path.is_empty() { "/" } else { path };
    Error::new(Code::Shape, format!("{path}: {msg}"))
}

pub(crate) fn members<'a>(
    v: &'a Value,
    path: &str,
    required: &[&str],
    optional: &[&str],
) -> Result<&'a [(String, Value)]> {
    let members = v
        .as_object()
        .ok_or_else(|| shape(path, format!("expected object, got {}", v.kind())))?;
    for (k, _) in members {
        if !required.contains(&k.as_str()) && !optional.contains(&k.as_str()) {
            return Err(shape(path, format!("unknown member {k:?}")));
        }
    }
    for r in required {
        if v.get(r).is_none() {
            return Err(shape(path, format!("missing member {r:?}")));
        }
    }
    Ok(members)
}

pub(crate) fn text<'a>(v: &'a Value, path: &str) -> Result<&'a str> {
    match v.as_str() {
        Some(s) if !s.is_empty() => Ok(s),
        Some(_) => Err(shape(path, "must not be empty")),
        None => Err(shape(path, format!("expected string, got {}", v.kind()))),
    }
}

pub(crate) fn hash<'a>(v: &'a Value, path: &str) -> Result<&'a str> {
    let s = text(v, path)?;
    if is_hash(s) {
        Ok(s)
    } else {
        Err(shape(
            path,
            "must be sha256: followed by 64 lowercase hex digits",
        ))
    }
}

pub(crate) fn one_of<'a>(v: &'a Value, path: &str, allowed: &[&str]) -> Result<&'a str> {
    let s = text(v, path)?;
    if allowed.contains(&s) {
        Ok(s)
    } else {
        Err(shape(path, format!("must be one of {allowed:?}")))
    }
}

pub fn is_hash(s: &str) -> bool {
    s.len() == 71
        && s.starts_with("sha256:")
        && s[7..]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// `^[a-z0-9][a-z0-9!#$&^_.+-]*/[a-z0-9][a-z0-9!#$&^_.+-]*$`
fn is_media_type(s: &str) -> bool {
    fn token(t: &str) -> bool {
        let mut bytes = t.bytes();
        matches!(bytes.next(), Some(b'a'..=b'z' | b'0'..=b'9'))
            && bytes.all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'!' | b'#' | b'$' | b'&' | b'^' | b'_' | b'.' | b'+' | b'-'))
    }
    matches!(s.split_once('/'), Some((t, sub)) if token(t) && token(sub))
}

/// `^[a-z0-9][a-z0-9.-]*/[0-9]+$`
fn is_ext_key(s: &str) -> bool {
    match s.split_once('/') {
        Some((name, ver)) => {
            let mut b = name.bytes();
            matches!(b.next(), Some(b'a'..=b'z' | b'0'..=b'9'))
                && b.all(|c| matches!(c, b'a'..=b'z' | b'0'..=b'9' | b'.' | b'-'))
                && !ver.is_empty()
                && ver.bytes().all(|c| c.is_ascii_digit())
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn h(i: usize) -> Value {
        Value::String(format!("sha256:{i:064x}"))
    }

    /// A shape-valid object with this prev list.
    fn with_prev(prev: Vec<Value>) -> Value {
        let s = |t: &str| Value::String(t.to_string());
        let obj = |members: &[(&str, Value)]| {
            Value::Object(
                members
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.clone()))
                    .collect(),
            )
        };
        obj(&[
            ("v", s(VERSION)),
            ("id", h(0)),
            ("op", s("update")),
            ("schema", obj(&[("ref", s("test/thing@1")), ("hash", h(0))])),
            (
                "subject",
                obj(&[("system", s("test")), ("type", s("thing")), ("key", s("1"))]),
            ),
            (
                "content",
                obj(&[("media_type", s("text/plain")), ("hash", h(0))]),
            ),
            (
                "actor",
                obj(&[("id", s("service:test")), ("kind", s("service"))]),
            ),
            ("at", s("2026-11-26T00:00:00Z")),
            ("prev", Value::Array(prev)),
        ])
    }

    fn shape_error(prev: Vec<Value>) -> Option<String> {
        check_shape(&with_prev(prev)).err().map(|e| {
            assert_eq!(e.code, Code::Shape);
            e.detail
        })
    }

    #[test]
    fn the_first_repeated_prev_is_reported() {
        assert_eq!(shape_error(vec![h(1), h(2), h(3)]), None);
        let detail = shape_error(vec![h(1), h(2), h(1), h(2)]).unwrap();
        assert!(detail.starts_with("/prev/2:"), "{detail}");
        let detail = shape_error(vec![h(1), h(2), h(2), Value::Null]).unwrap();
        assert!(detail.starts_with("/prev/2:"), "{detail}");
        // A malformed id before the first repeat is reported first.
        let detail = shape_error(vec![h(1), Value::Null, h(1)]).unwrap();
        assert!(detail.starts_with("/prev/1:"), "{detail}");
    }

    /// The duplicate check is not quadratic: a list of 100,000 ids, which
    /// one line of an export can carry, takes well under a second even in a
    /// debug build, where comparing every pair takes minutes.
    #[test]
    fn a_long_prev_list_is_checked_quickly() {
        let n = 100_000;
        let mut prev: Vec<Value> = (1..=n).map(h).collect();
        let start = Instant::now();
        assert_eq!(shape_error(prev.clone()), None);
        prev.push(h(n));
        let detail = shape_error(prev).unwrap();
        assert!(detail.starts_with(&format!("/prev/{n}:")), "{detail}");
        let elapsed = start.elapsed();
        assert!(elapsed < Duration::from_secs(10), "took {elapsed:?}");
    }
}
