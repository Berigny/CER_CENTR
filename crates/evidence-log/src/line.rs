//! Segment lines: one canonical JSON object per log entry, holding the entry
//! and its item. See `spec/store-0.md`, "Segment files".
//!
//! - Object line: `{"entry":E,"object":O}`.
//! - Attestation line: `{"attestation":A,"entry":E}`.
//!
//! E, O and A are canonical, and written in this order the line is canonical
//! too, because RFC 8785 sorts the members exactly as written. The same lines,
//! without their LF, are the lines of a live store.
//!
//! A line nests one level deeper than the item it carries. Lines may
//! therefore nest up to [`MAX_LINE_DEPTH`] (one more than the evidence/0
//! parser's [`json::MAX_DEPTH`]), so every object that evidence/0 accepts fits
//! in a line.

use evidence_core::canon;
use evidence_core::json::{self, Value};

use crate::entry::{Entry, Kind};
use crate::report::FindingCode;

/// The item a line carries, as parsed. It has not been checked: the verifier
/// runs the evidence/0 object or attestation rules over it.
#[derive(Clone, Debug, PartialEq)]
pub enum Body {
    Object(Value),
    Attestation(Value),
}

impl Body {
    /// The kind of item: the line member it was read from.
    pub fn kind(&self) -> Kind {
        match self {
            Body::Object(_) => Kind::Object,
            Body::Attestation(_) => Kind::Attestation,
        }
    }

    /// The item's JSON value.
    pub fn value(&self) -> &Value {
        match self {
            Body::Object(v) | Body::Attestation(v) => v,
        }
    }
}

/// A line that passed the line checks: strict canonical JSON with the right
/// members, a valid entry, and a body member that `entry.kind` names.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub entry: Entry,
    /// The entry's canonical text, as it appears in the line: the leaf bytes.
    pub entry_canonical: String,
    pub body: Body,
}

/// The line for an entry and its item, both already canonical. No trailing LF.
pub fn render_line(entry_canonical: &str, kind: Kind, body_canonical: &str) -> String {
    match kind {
        Kind::Object => format!("{{\"entry\":{entry_canonical},\"object\":{body_canonical}}}"),
        Kind::Attestation => {
            format!("{{\"attestation\":{body_canonical},\"entry\":{entry_canonical}}}")
        }
    }
}

/// How deep arrays and objects may nest in a line: one more than
/// [`json::MAX_DEPTH`], the limit for an object or attestation on its own,
/// because the line wraps the item in one more object.
pub const MAX_LINE_DEPTH: usize = json::MAX_DEPTH + 1;

/// Run the line checks (steps a, b and b2 of verification) over one line,
/// given without its LF.
///
/// - `S_LINE_INVALID`: not strict JSON nesting at most [`MAX_LINE_DEPTH`]
///   deep, not exactly the members `entry` and one of `object` or
///   `attestation`, or not byte for byte its canonical form.
/// - `S_ENTRY_INVALID`: `entry` is not valid log-entry/0.
/// - `S_LINE_INVALID`: the body member is not the one `entry.kind` names.
///
/// A canonical line with those members is exactly `{"entry":E,"object":O}`
/// or `{"attestation":A,"entry":E}`, so the two member values are found by
/// that frame and each is parsed on its own under evidence/0's strict rules.
/// That gives the same verdict as parsing the whole line with a nesting limit
/// of [`MAX_LINE_DEPTH`], and an item at the evidence/0 limit still fits.
pub fn parse_line(bytes: &[u8]) -> Result<Line, FindingCode> {
    let invalid = FindingCode::LineInvalid;
    // The frame: the first member's name, the separator before the second
    // member, and the closing brace.
    let (kind, rest, separator) = if let Some(rest) = bytes.strip_prefix(b"{\"entry\":") {
        (Kind::Object, rest, &b",\"object\":"[..])
    } else if let Some(rest) = bytes.strip_prefix(b"{\"attestation\":") {
        (Kind::Attestation, rest, &b",\"entry\":"[..])
    } else {
        return Err(invalid);
    };
    let inner = rest.strip_suffix(b"}").ok_or(invalid)?;
    let (first, rest) = inner.split_at(first_value_len(inner).ok_or(invalid)?);
    let second = rest.strip_prefix(separator).ok_or(invalid)?;
    let (first_text, first) = canonical_member(first)?;
    let (second_text, second) = canonical_member(second)?;
    let (entry_text, entry_value, body) = match kind {
        Kind::Object => (first_text, first, Body::Object(second)),
        Kind::Attestation => (second_text, second, Body::Attestation(first)),
    };
    let entry = Entry::from_value(&entry_value)?;
    if entry.kind != body.kind() {
        return Err(invalid);
    }
    Ok(Line {
        entry,
        entry_canonical: entry_text.to_string(),
        body,
    })
}

/// One member value of a line: strict JSON (nesting at most
/// [`json::MAX_DEPTH`]) that is byte for byte its own canonical form.
/// Returns the text and the value; `S_LINE_INVALID` otherwise.
fn canonical_member(bytes: &[u8]) -> Result<(&str, Value), FindingCode> {
    let invalid = FindingCode::LineInvalid;
    let value = json::parse(bytes).map_err(|_| invalid)?;
    if canon::canonicalize(&value).as_bytes() != bytes {
        return Err(invalid);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| invalid)?;
    Ok((text, value))
}

/// The length of the JSON value at the start of `bytes`: the position of the
/// first comma outside every string, array and object. None if there is no
/// such comma, or a bracket closes one that never opened.
///
/// This only finds where the value would end; it does not check it. In a
/// valid line it finds the true end of the first member's value, and the
/// caller then parses both values strictly, so a wrong guess on invalid input
/// can only lead to `S_LINE_INVALID`. It counts rather than recurses, so
/// hostile nesting cannot exhaust the stack.
fn first_value_len(bytes: &[u8]) -> Option<usize> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (i, &b) in bytes.iter().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' | b'[' => depth += 1,
            b'}' | b']' => depth = depth.checked_sub(1)?,
            b',' if depth == 0 => return Some(i),
            _ => {}
        }
    }
    None
}

/// The `ref` of an attestation: `"sha256:"` + hex SHA-256 of its canonical
/// form, including `sig`.
pub fn attestation_ref(attestation: &Value) -> String {
    canon::digest(attestation)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(c: char) -> String {
        format!("sha256:{}", c.to_string().repeat(64))
    }

    fn entry(kind: Kind) -> Entry {
        Entry {
            log: h('a'),
            seq: 4,
            kind,
            reference: h('b'),
            received_at: "2026-11-26T00:00:01.000Z".into(),
            epoch: 2,
            prev: h('c'),
        }
    }

    const OBJECT: &str = r#"{"id":"x","v":"evidence/0"}"#;
    const ATTESTATION: &str = r#"{"object":"y","sig":"z","v":"attestation/0"}"#;

    fn object_line() -> String {
        render_line(&entry(Kind::Object).canonical(), Kind::Object, OBJECT)
    }

    fn attestation_line() -> String {
        render_line(
            &entry(Kind::Attestation).canonical(),
            Kind::Attestation,
            ATTESTATION,
        )
    }

    #[test]
    fn rendered_lines_are_canonical_and_parse_back() {
        for (line, kind, body) in [
            (object_line(), Kind::Object, OBJECT),
            (attestation_line(), Kind::Attestation, ATTESTATION),
        ] {
            assert!(!line.ends_with('\n'));
            let v = json::parse(line.as_bytes()).unwrap();
            assert_eq!(canon::canonicalize(&v), line, "already canonical");

            let parsed = parse_line(line.as_bytes()).unwrap();
            assert_eq!(parsed.entry, entry(kind));
            assert_eq!(parsed.entry_canonical, entry(kind).canonical());
            assert_eq!(parsed.body.kind(), kind);
            assert_eq!(canon::canonicalize(parsed.body.value()), body);
        }
        assert!(object_line().starts_with("{\"entry\":{\"epoch\":2,"));
        assert!(attestation_line().starts_with("{\"attestation\":{\"object\":\"y\""));
    }

    #[test]
    fn line_checks() {
        let line = object_line();
        let invalid = Err(FindingCode::LineInvalid);
        let check = |text: &str| parse_line(text.as_bytes()).map(|_| ());

        // Not strict JSON.
        assert_eq!(check(""), invalid);
        assert_eq!(check(&line[..line.len() - 1]), invalid);
        assert_eq!(parse_line(b"{\"entry\":\xff}").map(|_| ()), invalid);
        // Wrong members.
        assert_eq!(check("{}"), invalid);
        assert_eq!(check("[]"), invalid);
        assert_eq!(
            check(&format!(
                "{{\"entry\":{}}}",
                entry(Kind::Object).canonical()
            )),
            invalid
        );
        assert_eq!(check(&format!("{{\"object\":{OBJECT}}}")), invalid);
        assert_eq!(
            check(&format!(
                "{{\"attestation\":{ATTESTATION},\"entry\":{},\"object\":{OBJECT}}}",
                entry(Kind::Object).canonical()
            )),
            invalid
        );
        assert_eq!(
            check(&format!(
                "{{\"entry\":{},\"item\":{OBJECT}}}",
                entry(Kind::Object).canonical()
            )),
            invalid
        );
        // Not canonical: whitespace, member order, a trailing LF, a BOM.
        assert_eq!(
            check(&line.replacen("{\"entry\":", "{ \"entry\":", 1)),
            invalid
        );
        assert_eq!(
            check(&format!(
                "{{\"object\":{OBJECT},\"entry\":{}}}",
                entry(Kind::Object).canonical()
            )),
            invalid
        );
        assert_eq!(check(&format!("{line}\n")), invalid);
        assert_eq!(check(&format!("\u{feff}{line}")), invalid);
        assert_eq!(check(&line.replace("\"seq\":4", "\"seq\":4.0")), invalid);
        // The body member must be the one entry.kind names.
        assert_eq!(
            check(&render_line(
                &entry(Kind::Attestation).canonical(),
                Kind::Object,
                OBJECT
            )),
            invalid
        );
        assert_eq!(
            check(&render_line(
                &entry(Kind::Object).canonical(),
                Kind::Attestation,
                ATTESTATION
            )),
            invalid
        );
    }

    #[test]
    fn an_invalid_entry_is_entry_invalid() {
        let bad = entry(Kind::Object)
            .to_value()
            .with("kind", Value::String("blob".into()));
        let line = render_line(&canon::canonicalize(&bad), Kind::Object, OBJECT);
        assert_eq!(parse_line(line.as_bytes()), Err(FindingCode::EntryInvalid));

        let no_prev = entry(Kind::Object).to_value().without("prev");
        let line = render_line(&canon::canonicalize(&no_prev), Kind::Object, OBJECT);
        assert_eq!(parse_line(line.as_bytes()), Err(FindingCode::EntryInvalid));

        // An entry that is not an object at all.
        let line = format!("{{\"entry\":1,\"object\":{OBJECT}}}");
        assert_eq!(parse_line(line.as_bytes()), Err(FindingCode::EntryInvalid));

        // The entry is checked before the body member is matched to its kind.
        let line = render_line(&canon::canonicalize(&bad), Kind::Attestation, ATTESTATION);
        assert_eq!(parse_line(line.as_bytes()), Err(FindingCode::EntryInvalid));
    }

    #[test]
    fn the_body_is_not_checked_here() {
        // Any JSON value is carried through for the verifier to judge.
        let line = render_line(&entry(Kind::Object).canonical(), Kind::Object, "\"text\"");
        let parsed = parse_line(line.as_bytes()).unwrap();
        assert_eq!(parsed.body, Body::Object(Value::String("text".into())));
    }

    /// `depth` nested arrays: `[[…]]`.
    fn nested(depth: usize) -> String {
        format!("{}{}", "[".repeat(depth), "]".repeat(depth))
    }

    #[test]
    fn an_item_at_the_evidence_0_nesting_limit_fits_in_a_line() {
        let deepest = nested(json::MAX_DEPTH);
        assert!(json::parse(deepest.as_bytes()).is_ok());
        assert!(json::parse(nested(json::MAX_DEPTH + 1).as_bytes()).is_err());
        assert_eq!(MAX_LINE_DEPTH, json::MAX_DEPTH + 1);

        for kind in [Kind::Object, Kind::Attestation] {
            let entry = entry(kind).canonical();
            // At the limit: the line nests one level deeper, and is accepted.
            let line = render_line(&entry, kind, &deepest);
            let parsed = parse_line(line.as_bytes()).unwrap();
            assert_eq!(canon::canonicalize(parsed.body.value()), deepest);
            assert_eq!(parsed.entry_canonical, entry);
            // One level more is not strict JSON for evidence/0, nor here.
            let line = render_line(&entry, kind, &nested(json::MAX_DEPTH + 1));
            assert_eq!(parse_line(line.as_bytes()), Err(FindingCode::LineInvalid));
        }
    }

    #[test]
    fn hostile_nesting_is_line_invalid_not_a_crash() {
        let entry = entry(Kind::Object).canonical();
        for depth in [10_000, 1_000_000] {
            let open = "[".repeat(depth);
            for line in [
                render_line(&entry, Kind::Object, &nested(depth)),
                render_line(&entry, Kind::Attestation, &nested(depth)),
                render_line(&entry, Kind::Object, &open),
                format!("{{\"entry\":{open}"),
                format!("{{\"attestation\":{open},\"entry\":{entry}}}"),
            ] {
                assert_eq!(parse_line(line.as_bytes()), Err(FindingCode::LineInvalid));
            }
        }
    }

    #[test]
    fn strings_may_hold_brackets_commas_and_quotes() {
        // The member values are found by matching brackets outside strings.
        let object = r#"{"a":"q\",}]{[\\","b":[1,{"c":","}],"d":"\\"}"#;
        let attestation = r#"{"object":"}]\",","sig":"[{","v":"attestation/0"}"#;
        for (kind, body) in [(Kind::Object, object), (Kind::Attestation, attestation)] {
            let line = render_line(&entry(kind).canonical(), kind, body);
            let parsed = parse_line(line.as_bytes()).unwrap();
            assert_eq!(canon::canonicalize(parsed.body.value()), body);
            assert_eq!(parsed.entry, entry(kind));
        }
    }

    /// The line checks as one parse of the whole line, with the nesting limit
    /// of evidence/0. For lines that nest no deeper than that, `parse_line`
    /// must agree with it exactly.
    fn whole_line(bytes: &[u8]) -> Result<(Entry, String, Body), FindingCode> {
        let invalid = FindingCode::LineInvalid;
        let value = json::parse(bytes).map_err(|_| invalid)?;
        let (entry_value, body) = match value.as_object().ok_or(invalid)? {
            [(a, av), (b, bv)] => match (a.as_str(), b.as_str()) {
                ("entry", "object") => (av, Body::Object(bv.clone())),
                ("attestation", "entry") => (bv, Body::Attestation(av.clone())),
                _ => return Err(invalid),
            },
            _ => return Err(invalid),
        };
        if canon::canonicalize(&value).as_bytes() != bytes {
            return Err(invalid);
        }
        let entry = Entry::from_value(entry_value)?;
        if entry.kind != body.kind() {
            return Err(invalid);
        }
        Ok((entry, canon::canonicalize(entry_value), body))
    }

    #[test]
    fn agrees_with_parsing_the_whole_line() {
        let object = r#"{"a":"q\",}]","b":[1,{"c":2}],"d":null}"#;
        let attestation = r#"{"object":"y,]","sig":"z","v":"attestation/0"}"#;
        let mut seeds = vec![
            object_line(),
            attestation_line(),
            render_line(&entry(Kind::Object).canonical(), Kind::Object, object),
            render_line(
                &entry(Kind::Attestation).canonical(),
                Kind::Attestation,
                attestation,
            ),
            format!("{{\"entry\":1,\"object\":{OBJECT}}}"),
            format!("{{\"attestation\":[1,2],\"entry\":\"e,\"}}"),
        ];
        seeds.push(render_line(
            &entry(Kind::Attestation).canonical(),
            Kind::Object,
            OBJECT,
        ));
        // Every seed, with each byte deleted, and with each of these inserted
        // at each position. Inside a run of one repeated byte (the hashes)
        // only the run's first position is used: the rest add nothing.
        let inserts: &[&[u8]] = &[b",", b"\"", b"\\", b"[", b"]", b"{", b"}", b" ", b":", b"1"];
        let mut lines: Vec<Vec<u8>> = Vec::new();
        for seed in &seeds {
            let seed = seed.as_bytes();
            lines.push(seed.to_vec());
            for i in 0..=seed.len() {
                if i > 0 && i < seed.len() && seed[i - 1] == seed[i] {
                    continue;
                }
                if i < seed.len() {
                    lines.push([&seed[..i], &seed[i + 1..]].concat());
                }
                for ins in inserts {
                    lines.push([&seed[..i], ins, &seed[i..]].concat());
                }
            }
        }
        let mut accepted = 0;
        for line in &lines {
            let got = parse_line(line).map(|l| (l.entry, l.entry_canonical, l.body));
            let want = whole_line(line);
            assert_eq!(got, want, "line {}", String::from_utf8_lossy(line));
            accepted += usize::from(got.is_ok());
        }
        assert!(accepted >= seeds.len() - 3, "the valid seeds parse");
    }

    #[test]
    fn attestation_ref_hashes_the_canonical_form_with_sig() {
        let v = json::parse(br#"{"v":"attestation/0","sig":"z","object":"y"}"#).unwrap();
        assert_eq!(
            attestation_ref(&v),
            canon::sha256_hex(ATTESTATION.as_bytes())
        );
        assert_ne!(attestation_ref(&v), attestation_ref(&v.without("sig")));
    }
}
