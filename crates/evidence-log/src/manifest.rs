//! The export manifest, `segments/0`: `manifest.json` of a store export. See
//! `spec/store-0.md`, "Manifest (segments/0)", and
//! `spec/schema/segments-0.schema.json`.
//!
//! `manifest.json` holds exactly the canonical form of the manifest, with no
//! trailing newline. The segment list follows from `size` and
//! `segment_size`: segment i, counting from 1, is [`Manifest::file_name`]`(i)`
//! and holds seqs (i − 1) × `segment_size` + 1 to the smaller of
//! i × `segment_size` and `size`.

use evidence_core::canon;
use evidence_core::json::{self, Value, MAX_SAFE_INTEGER};
use evidence_core::object::is_hash;

use crate::genesis::exact_members;
use crate::report::FindingCode;

/// `v` of every manifest.
pub const SEGMENTS_V: &str = "segments/0";

/// The name of the manifest file in an export directory.
pub const MANIFEST_FILE: &str = "manifest.json";

/// The required members of a manifest. `checkpoint` is the only optional one.
pub const MANIFEST_MEMBERS: &[&str] = &["v", "log", "size", "head", "segment_size", "segments"];

/// The members of one segment in the list. All are required.
pub const SEGMENT_MEMBERS: &[&str] = &["file", "first", "last", "hash"];

/// One segment file of an export.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SegmentRef {
    /// `NNNNNN.jsonl`, see [`Manifest::file_name`].
    pub file: String,
    /// The seq of the segment's first line.
    pub first: u64,
    /// The seq of the segment's last line.
    pub last: u64,
    /// `"sha256:"` + hex SHA-256 of the file's bytes.
    pub hash: String,
}

/// A segments/0 manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    /// The log id.
    pub log: String,
    /// The number of entries exported: the seq of the last one.
    pub size: u64,
    /// The entry hash of seq = `size`.
    pub head: String,
    /// Lines per segment file.
    pub segment_size: u64,
    pub segments: Vec<SegmentRef>,
    /// (id, size) of the latest checkpoint; None if the log has none.
    pub checkpoint: Option<(String, u64)>,
}

impl Manifest {
    /// The name of segment `index`, counting from 1: the index in decimal,
    /// zero-padded to six digits, then `.jsonl` (`1` → `"000001.jsonl"`).
    pub fn file_name(index: usize) -> String {
        format!("{index:06}.jsonl")
    }

    /// How many segment files an export of `size` entries has, with
    /// `segment_size` lines per file. Zero if either is zero.
    pub fn segment_count(size: u64, segment_size: u64) -> u64 {
        if segment_size == 0 {
            return 0;
        }
        size.div_ceil(segment_size)
    }

    /// The seqs (`first`, `last`) that segment `index` (counting from 1)
    /// holds, or None if an export of `size` entries has no such segment.
    pub fn segment_range(index: u64, size: u64, segment_size: u64) -> Option<(u64, u64)> {
        if index == 0 || index > Manifest::segment_count(size, segment_size) {
            return None;
        }
        // index ≤ ceil(size / segment_size), so (index − 1) × segment_size is
        // below size; checked arithmetic only guards hand-built arguments.
        let first = (index - 1).checked_mul(segment_size)?.checked_add(1)?;
        let last = index
            .checked_mul(segment_size)
            .map_or(size, |l| l.min(size));
        Some((first, last))
    }

    /// The manifest as a JSON object. `checkpoint` is omitted when None.
    pub fn to_value(&self) -> Value {
        let n = |x: u64| Value::Number(x as f64);
        let s = |t: &str| Value::String(t.to_string());
        let mut m = vec![
            ("v".to_string(), s(SEGMENTS_V)),
            ("log".into(), s(&self.log)),
            ("size".into(), n(self.size)),
            ("head".into(), s(&self.head)),
            ("segment_size".into(), n(self.segment_size)),
            (
                "segments".into(),
                Value::Array(
                    self.segments
                        .iter()
                        .map(|seg| {
                            Value::Object(vec![
                                ("file".into(), s(&seg.file)),
                                ("first".into(), n(seg.first)),
                                ("last".into(), n(seg.last)),
                                ("hash".into(), s(&seg.hash)),
                            ])
                        })
                        .collect(),
                ),
            ),
        ];
        if let Some((id, size)) = &self.checkpoint {
            m.push((
                "checkpoint".into(),
                Value::Object(vec![("id".into(), s(id)), ("size".into(), n(*size))]),
            ));
        }
        Value::Object(m)
    }

    /// The bytes of `manifest.json`: the canonical form, no trailing newline.
    pub fn canonical(&self) -> String {
        canon::canonicalize(&self.to_value())
    }

    /// Read `manifest.json`. `S_MANIFEST_INVALID` unless the bytes are, byte
    /// for byte, the canonical form of a strict JSON value that matches
    /// segments/0 and whose segment list is the one its `size` and
    /// `segment_size` give.
    pub fn parse(bytes: &[u8]) -> Result<Manifest, FindingCode> {
        let invalid = FindingCode::ManifestInvalid;
        let v = json::parse(bytes).map_err(|_| invalid)?;
        if canon::canonicalize(&v).as_bytes() != bytes {
            return Err(invalid);
        }
        let members = v.as_object().ok_or(invalid)?;
        let has_checkpoint = v.get("checkpoint").is_some();
        if members.len() != MANIFEST_MEMBERS.len() + usize::from(has_checkpoint)
            || members
                .iter()
                .any(|(k, _)| !MANIFEST_MEMBERS.contains(&k.as_str()) && k != "checkpoint")
            || MANIFEST_MEMBERS.iter().any(|k| v.get(k).is_none())
        {
            return Err(invalid);
        }
        if v.get("v").and_then(Value::as_str) != Some(SEGMENTS_V) {
            return Err(invalid);
        }
        let size = position(v.get("size")).ok_or(invalid)?;
        let segment_size = position(v.get("segment_size")).ok_or(invalid)?;
        let checkpoint = match v.get("checkpoint") {
            None => None,
            Some(c) => {
                if !exact_members(c, &["id", "size"]) {
                    return Err(invalid);
                }
                Some((
                    hash(c.get("id")).ok_or(invalid)?,
                    position(c.get("size")).ok_or(invalid)?,
                ))
            }
        };
        let list = v.get("segments").and_then(Value::as_array).ok_or(invalid)?;
        // The list must be exactly the one size and segment_size give, so its
        // length is known before any member is read.
        if list.len() as u64 != Manifest::segment_count(size, segment_size) {
            return Err(invalid);
        }
        let mut segments = Vec::with_capacity(list.len());
        for (i, seg) in list.iter().enumerate() {
            if !exact_members(seg, SEGMENT_MEMBERS) {
                return Err(invalid);
            }
            let index = i + 1;
            let (first, last) =
                Manifest::segment_range(index as u64, size, segment_size).ok_or(invalid)?;
            let file = seg.get("file").and_then(Value::as_str).ok_or(invalid)?;
            if file != Manifest::file_name(index)
                || position(seg.get("first")) != Some(first)
                || position(seg.get("last")) != Some(last)
            {
                return Err(invalid);
            }
            segments.push(SegmentRef {
                file: file.to_string(),
                first,
                last,
                hash: hash(seg.get("hash")).ok_or(invalid)?,
            });
        }
        Ok(Manifest {
            log: hash(v.get("log")).ok_or(invalid)?,
            size,
            head: hash(v.get("head")).ok_or(invalid)?,
            segment_size,
            segments,
            checkpoint,
        })
    }
}

/// An integer from 1 to 2^53 − 1 written as a JSON number.
fn position(v: Option<&Value>) -> Option<u64> {
    match v {
        Some(Value::Number(n)) if *n >= 1.0 && *n <= MAX_SAFE_INTEGER && n.fract() == 0.0 => {
            Some(*n as u64)
        }
        _ => None,
    }
}

/// `"sha256:"` + 64 lowercase hex digits.
fn hash(v: Option<&Value>) -> Option<String> {
    match v.and_then(Value::as_str) {
        Some(s) if is_hash(s) => Some(s.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(c: char) -> String {
        format!("sha256:{}", c.to_string().repeat(64))
    }

    fn sample(size: u64, segment_size: u64, checkpoint: Option<(String, u64)>) -> Manifest {
        let count = Manifest::segment_count(size, segment_size);
        Manifest {
            log: h('a'),
            size,
            head: h('b'),
            segment_size,
            segments: (1..=count)
                .map(|i| {
                    let (first, last) = Manifest::segment_range(i, size, segment_size).unwrap();
                    SegmentRef {
                        file: Manifest::file_name(i as usize),
                        first,
                        last,
                        hash: h('c'),
                    }
                })
                .collect(),
            checkpoint,
        }
    }

    #[test]
    fn file_names() {
        assert_eq!(Manifest::file_name(1), "000001.jsonl");
        assert_eq!(Manifest::file_name(42), "000042.jsonl");
        assert_eq!(Manifest::file_name(999_999), "999999.jsonl");
        assert_eq!(Manifest::file_name(1_000_000), "1000000.jsonl");
    }

    #[test]
    fn segment_layout() {
        assert_eq!(Manifest::segment_count(12, 10_000), 1);
        assert_eq!(Manifest::segment_count(12, 4), 3);
        assert_eq!(Manifest::segment_count(13, 4), 4);
        assert_eq!(Manifest::segment_count(1, 1), 1);
        assert_eq!(Manifest::segment_count(5, 0), 0);
        assert_eq!(Manifest::segment_range(1, 12, 10_000), Some((1, 12)));
        assert_eq!(Manifest::segment_range(3, 12, 4), Some((9, 12)));
        assert_eq!(Manifest::segment_range(4, 13, 4), Some((13, 13)));
        assert_eq!(Manifest::segment_range(4, 12, 4), None);
        assert_eq!(Manifest::segment_range(0, 12, 4), None);
        let max = 9_007_199_254_740_991;
        assert_eq!(Manifest::segment_count(max, max), 1);
        assert_eq!(Manifest::segment_range(1, max, max), Some((1, max)));
        assert_eq!(Manifest::segment_range(2, max, max - 1), Some((max, max)));
    }

    #[test]
    fn canonical_form_and_round_trip() {
        let one = sample(12, 10_000, Some((h('d'), 10)));
        assert_eq!(
            one.canonical(),
            format!(
                "{{\"checkpoint\":{{\"id\":\"{d}\",\"size\":10}},\"head\":\"{b}\",\"log\":\"{a}\",\
                 \"segment_size\":10000,\"segments\":[{{\"file\":\"000001.jsonl\",\"first\":1,\
                 \"hash\":\"{c}\",\"last\":12}}],\"size\":12,\"v\":\"segments/0\"}}",
                a = h('a'),
                b = h('b'),
                c = h('c'),
                d = h('d')
            )
        );
        for m in [
            one,
            sample(12, 4, None),
            sample(13, 4, Some((h('e'), 12))),
            sample(1, 1, None),
        ] {
            assert_eq!(Manifest::parse(m.canonical().as_bytes()), Ok(m));
        }
    }

    #[test]
    fn rejects_every_invalid_manifest() {
        let good = sample(9, 4, Some((h('d'), 7)));
        let invalid = Err(FindingCode::ManifestInvalid);
        let parse = |v: &Value| Manifest::parse(canon::canonicalize(v).as_bytes());
        assert!(parse(&good.to_value()).is_ok());
        let text = good.canonical();

        // Not the canonical form of strict JSON.
        for bytes in [
            format!("{text}\n"),
            format!(" {text}"),
            text.replacen("\"size\":9", "\"size\":9.0", 1),
            text.replacen("{", "{ ", 1),
            format!("\u{feff}{text}"),
            String::new(),
            "[]".to_string(),
            text[..text.len() - 1].to_string(),
        ] {
            assert_eq!(Manifest::parse(bytes.as_bytes()), invalid, "{bytes}");
        }
        assert_eq!(Manifest::parse(b"{\"v\":\xff}"), invalid);

        let v = good.to_value();
        let s = |t: &str| Value::String(t.into());
        let n = Value::Number;
        let seg = |i: usize| v.get("segments").unwrap().as_array().unwrap()[i].clone();
        let with_seg = |i: usize, seg: Value| {
            let mut list = v.get("segments").unwrap().as_array().unwrap().to_vec();
            list[i] = seg;
            v.with("segments", Value::Array(list))
        };
        let cases: Vec<Value> = vec![
            // Members.
            v.without("v"),
            v.without("log"),
            v.without("size"),
            v.without("head"),
            v.without("segment_size"),
            v.without("segments"),
            v.with("extra", n(1.0)),
            v.with("v", s("segments/1")),
            // Types and patterns.
            v.with("log", s("sha256:abc")),
            v.with("head", s(&h('A'))),
            v.with("size", n(0.0)),
            v.with("size", s("9")),
            v.with("segment_size", n(0.0)),
            v.with("segment_size", n(2.5)),
            v.with("segment_size", n(9_007_199_254_740_992.0)),
            v.with("segments", Value::Array(vec![])),
            v.with("segments", Value::Null),
            v.with("checkpoint", Value::Null),
            v.with("checkpoint", Value::Object(vec![("id".into(), s(&h('d')))])),
            v.with(
                "checkpoint",
                Value::Object(vec![("id".into(), s("x")), ("size".into(), n(7.0))]),
            ),
            v.with(
                "checkpoint",
                Value::Object(vec![("id".into(), s(&h('d'))), ("size".into(), n(0.0))]),
            ),
            v.with(
                "checkpoint",
                Value::Object(vec![
                    ("id".into(), s(&h('d'))),
                    ("size".into(), n(7.0)),
                    ("root".into(), s(&h('e'))),
                ]),
            ),
            with_seg(0, seg(0).without("hash")),
            with_seg(0, seg(0).with("hash", s("sha256:"))),
            with_seg(0, seg(0).with("extra", n(1.0))),
            with_seg(1, n(1.0)),
            // The list is not the one size and segment_size give.
            v.with("size", n(8.0)),
            v.with("size", n(13.0)),
            v.with("segment_size", n(3.0)),
            v.with("segment_size", n(5.0)),
            with_seg(1, seg(1).with("file", s("000003.jsonl"))),
            with_seg(1, seg(1).with("file", s("2.jsonl"))),
            with_seg(1, seg(1).with("first", n(4.0))),
            with_seg(2, seg(2).with("last", n(12.0))),
            v.with("segments", Value::Array(vec![seg(1), seg(0), seg(2)])),
        ];
        for (i, c) in cases.iter().enumerate() {
            assert_eq!(parse(c), invalid, "case {i}: {}", canon::canonicalize(c));
        }
        // A checkpoint whose size is not below the manifest's is still well
        // formed: agreement with the lines is checked by the verifier.
        assert!(parse(&v.with(
            "checkpoint",
            Value::Object(vec![("id".into(), s(&h('d'))), ("size".into(), n(100.0))])
        ))
        .is_ok());
    }

    #[test]
    fn a_huge_declared_size_is_rejected_without_building_the_list() {
        let v = sample(1, 1, None)
            .to_value()
            .with("size", Value::Number(9_007_199_254_740_991.0));
        assert_eq!(
            Manifest::parse(canon::canonicalize(&v).as_bytes()),
            Err(FindingCode::ManifestInvalid)
        );
    }
}
