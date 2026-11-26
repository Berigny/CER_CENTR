//! The store report, `store-report/0`: finding codes (`S_` codes), findings
//! and the report itself. See `spec/store-0.md`, "Report and codes", and
//! `spec/schema/store-report-0.schema.json`.
//!
//! The declaration order of [`FindingCode`] is the sort order of findings in
//! a report, so the derived `Ord` sorts findings that share a seq.
//!
//! A report holds no free text, so every implementation emits identical
//! bytes for the same input.

use std::fmt;

use evidence_core::json::Value;
use evidence_core::Code;

/// `v` of every store report.
pub const STORE_REPORT_V: &str = "store-report/0";

/// A store/0 report code. These strings are part of the spec: test vectors
/// name them, and every implementation must report the same code for the
/// same input.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FindingCode {
    /// `manifest.json` is not strict JSON, does not match segments/0, or
    /// lists the wrong segments.
    ManifestInvalid,
    /// A segment file does not hash to its manifest `hash`.
    SegmentHash,
    /// The manifest's `size`, `head`, `log` or `checkpoint` disagrees with the
    /// lines, or it lists a different number of segment files than were given.
    ManifestMismatch,
    /// Not strict JSON, wrong members, not in canonical form, or a body member
    /// that `entry.kind` does not name.
    LineInvalid,
    /// The entry is not valid `log-entry/0`.
    EntryInvalid,
    /// `entry.log` is not the log id.
    WrongLog,
    /// `entry.seq` is not the expected seq.
    SeqGap,
    /// `entry.prev` is not the previous entry hash (or the log id at seq 1).
    ChainBreak,
    /// `received_at` is earlier than the previous usable line's.
    ReceivedRegression,
    /// `epoch` is lower than the previous usable line's.
    EpochRegression,
    /// `entry.ref` is not the object's id or the attestation's hash.
    RefMismatch,
    /// The ref appeared on an earlier usable line.
    DuplicateRef,
    /// Seq 1 is not a valid genesis or seq 2 is not its author attestation,
    /// or no usable line has seq 1 or none has seq 2 (no seq).
    GenesisInvalid,
    /// An object with a store subject that is neither the genesis nor a
    /// checkpoint by the genesis signer. Also an intake rejection.
    ReservedSubject,
    /// The object fails evidence/0 steps 1-5, or step 6 within the log.
    ObjectInvalid,
    /// A `prev` is held only on a later line.
    PrevPosition,
    /// The attestation's shape or signature is invalid.
    AttestationInvalid,
    /// The attestation's object is not held on an earlier line. Also an
    /// intake rejection.
    AttestationTarget,
    /// A checkpoint by the genesis signer is not valid checkpoint/0 for this log.
    CheckpointInvalid,
    /// A checkpoint's `size` is not below its seq, or its `head` or `root`
    /// disagrees with the usable lines before it.
    CheckpointMismatch,
    /// A checkpoint has no valid witness by the genesis key.
    CheckpointUnsigned,
    /// The stored heads or checkpoints differ from a recompute.
    ProjectionDrift,
    /// The append-only triggers are missing.
    TriggersMissing,
    /// The stored log id differs from the log id of the lines.
    MetaMismatch,
}

impl FindingCode {
    pub fn as_str(self) -> &'static str {
        match self {
            FindingCode::ManifestInvalid => "S_MANIFEST_INVALID",
            FindingCode::SegmentHash => "S_SEGMENT_HASH",
            FindingCode::ManifestMismatch => "S_MANIFEST_MISMATCH",
            FindingCode::LineInvalid => "S_LINE_INVALID",
            FindingCode::EntryInvalid => "S_ENTRY_INVALID",
            FindingCode::WrongLog => "S_WRONG_LOG",
            FindingCode::SeqGap => "S_SEQ_GAP",
            FindingCode::ChainBreak => "S_CHAIN_BREAK",
            FindingCode::ReceivedRegression => "S_RECEIVED_REGRESSION",
            FindingCode::EpochRegression => "S_EPOCH_REGRESSION",
            FindingCode::RefMismatch => "S_REF_MISMATCH",
            FindingCode::DuplicateRef => "S_DUPLICATE_REF",
            FindingCode::GenesisInvalid => "S_GENESIS_INVALID",
            FindingCode::ReservedSubject => "S_RESERVED_SUBJECT",
            FindingCode::ObjectInvalid => "S_OBJECT_INVALID",
            FindingCode::PrevPosition => "S_PREV_POSITION",
            FindingCode::AttestationInvalid => "S_ATTESTATION_INVALID",
            FindingCode::AttestationTarget => "S_ATTESTATION_TARGET",
            FindingCode::CheckpointInvalid => "S_CHECKPOINT_INVALID",
            FindingCode::CheckpointMismatch => "S_CHECKPOINT_MISMATCH",
            FindingCode::CheckpointUnsigned => "S_CHECKPOINT_UNSIGNED",
            FindingCode::ProjectionDrift => "S_PROJECTION_DRIFT",
            FindingCode::TriggersMissing => "S_TRIGGERS_MISSING",
            FindingCode::MetaMismatch => "S_META_MISMATCH",
        }
    }
}

impl fmt::Display for FindingCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One finding: a code, plus what it is about.
///
/// - `seq` and `reference`: the `entry.seq` and `entry.ref` of the line the
///   finding is about. Always both or neither. Findings of line checks a, b
///   and b2, manifest findings, live-store findings and the end-of-log
///   `S_GENESIS_INVALID` have neither.
/// - `core`: the evidence/0 code behind `S_OBJECT_INVALID` or
///   `S_ATTESTATION_INVALID`, and only those.
/// - `file`: the segment file behind `S_SEGMENT_HASH`, and only that.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    pub code: FindingCode,
    pub seq: Option<u64>,
    pub reference: Option<String>,
    pub core: Option<Code>,
    pub file: Option<String>,
}

impl Finding {
    /// A finding with only a code: about no line in particular.
    pub fn new(code: FindingCode) -> Finding {
        Finding {
            code,
            seq: None,
            reference: None,
            core: None,
            file: None,
        }
    }

    /// A finding about the line whose entry has this seq and ref.
    pub fn at(code: FindingCode, seq: u64, reference: &str) -> Finding {
        Finding {
            seq: Some(seq),
            reference: Some(reference.to_string()),
            ..Finding::new(code)
        }
    }

    /// `S_SEGMENT_HASH` for the segment file with this name.
    pub fn segment_hash(file: &str) -> Finding {
        Finding {
            file: Some(file.to_string()),
            ..Finding::new(FindingCode::SegmentHash)
        }
    }

    /// The same finding with `core` set to an evidence/0 code.
    pub fn with_core(self, core: Code) -> Finding {
        Finding {
            core: Some(core),
            ..self
        }
    }

    /// The finding as a JSON object: `code`, then whichever of `seq`, `ref`,
    /// `core` and `file` are set.
    pub fn to_value(&self) -> Value {
        let mut m = vec![("code".to_string(), Value::String(self.code.as_str().into()))];
        if let Some(seq) = self.seq {
            m.push(("seq".into(), Value::Number(seq as f64)));
        }
        if let Some(reference) = &self.reference {
            m.push(("ref".into(), Value::String(reference.clone())));
        }
        if let Some(core) = self.core {
            m.push(("core".into(), Value::String(core.as_str().into())));
        }
        if let Some(file) = &self.file {
            m.push(("file".into(), Value::String(file.clone())));
        }
        Value::Object(m)
    }
}

/// What was verified: a live store's lines, or an export.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Store,
    Export,
}

impl Source {
    /// `"store"` or `"export"`, as written in a report's `source`.
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Store => "store",
            Source::Export => "export",
        }
    }
}

/// Counts of usable lines, each counted once. `entries` = `objects` +
/// `attestations` + `checkpoints`. Lines with findings are counted; unusable
/// lines are not.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Checked {
    /// Every usable line.
    pub entries: u64,
    /// Usable object lines that are not checkpoints, the genesis included.
    pub objects: u64,
    /// Usable attestation lines.
    pub attestations: u64,
    /// Object lines recognised as checkpoints (item 2 of step j), whatever
    /// their content and tree checks found.
    pub checkpoints: u64,
}

/// A `store-report/0`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoreReport {
    pub source: Source,
    /// The log id of the lines; None when no line passes line checks a, b and b2.
    pub log: Option<String>,
    /// `seq` and entry hash of the last usable line in reading order.
    pub head: Option<(u64, String)>,
    /// `size` and id of the largest checkpoint with no body finding and a
    /// valid witness.
    pub covered: Option<(u64, String)>,
    pub checked: Checked,
    /// `counts.forks`: record-lane subjects with more than one head.
    pub forks: u64,
    /// `counts.at_after_received`: held objects dated more than 300 seconds
    /// after their entry's `received_at`.
    pub at_after_received: u64,
    /// Sorted by seq (none first), then code, then line order.
    pub findings: Vec<Finding>,
    /// True when findings were dropped because of the limit.
    pub truncated: bool,
}

impl StoreReport {
    /// True exactly when there are no findings.
    pub fn ok(&self) -> bool {
        self.findings.is_empty()
    }

    /// The report as a JSON object. `log`, `head` and `covered` are omitted
    /// when there are none.
    pub fn to_value(&self) -> Value {
        let n = |x: u64| Value::Number(x as f64);
        let s = |t: &str| Value::String(t.to_string());
        let mut m = vec![
            ("v".to_string(), s(STORE_REPORT_V)),
            ("ok".into(), Value::Bool(self.ok())),
            ("source".into(), s(self.source.as_str())),
        ];
        if let Some(log) = &self.log {
            m.push(("log".into(), s(log)));
        }
        if let Some((seq, entry_hash)) = &self.head {
            m.push((
                "head".into(),
                Value::Object(vec![
                    ("seq".into(), n(*seq)),
                    ("entry_hash".into(), s(entry_hash)),
                ]),
            ));
        }
        if let Some((size, checkpoint)) = &self.covered {
            m.push((
                "covered".into(),
                Value::Object(vec![
                    ("size".into(), n(*size)),
                    ("checkpoint".into(), s(checkpoint)),
                ]),
            ));
        }
        let c = &self.checked;
        m.push((
            "checked".into(),
            Value::Object(vec![
                ("entries".into(), n(c.entries)),
                ("objects".into(), n(c.objects)),
                ("attestations".into(), n(c.attestations)),
                ("checkpoints".into(), n(c.checkpoints)),
            ]),
        ));
        m.push((
            "counts".into(),
            Value::Object(vec![
                ("forks".into(), n(self.forks)),
                ("at_after_received".into(), n(self.at_after_received)),
            ]),
        ));
        m.push((
            "findings".into(),
            Value::Array(self.findings.iter().map(Finding::to_value).collect()),
        ));
        m.push(("truncated".into(), Value::Bool(self.truncated)));
        Value::Object(m)
    }
}

#[cfg(test)]
mod tests {
    use super::FindingCode::{self, *};
    use super::{Checked, Finding, Source, StoreReport};
    use evidence_core::canon::canonicalize;
    use evidence_core::Code;

    /// The report code table of `spec/store-0.md`, in sort order.
    const TABLE: &[(FindingCode, &str)] = &[
        (ManifestInvalid, "S_MANIFEST_INVALID"),
        (SegmentHash, "S_SEGMENT_HASH"),
        (ManifestMismatch, "S_MANIFEST_MISMATCH"),
        (LineInvalid, "S_LINE_INVALID"),
        (EntryInvalid, "S_ENTRY_INVALID"),
        (WrongLog, "S_WRONG_LOG"),
        (SeqGap, "S_SEQ_GAP"),
        (ChainBreak, "S_CHAIN_BREAK"),
        (ReceivedRegression, "S_RECEIVED_REGRESSION"),
        (EpochRegression, "S_EPOCH_REGRESSION"),
        (RefMismatch, "S_REF_MISMATCH"),
        (DuplicateRef, "S_DUPLICATE_REF"),
        (GenesisInvalid, "S_GENESIS_INVALID"),
        (ReservedSubject, "S_RESERVED_SUBJECT"),
        (ObjectInvalid, "S_OBJECT_INVALID"),
        (PrevPosition, "S_PREV_POSITION"),
        (AttestationInvalid, "S_ATTESTATION_INVALID"),
        (AttestationTarget, "S_ATTESTATION_TARGET"),
        (CheckpointInvalid, "S_CHECKPOINT_INVALID"),
        (CheckpointMismatch, "S_CHECKPOINT_MISMATCH"),
        (CheckpointUnsigned, "S_CHECKPOINT_UNSIGNED"),
        (ProjectionDrift, "S_PROJECTION_DRIFT"),
        (TriggersMissing, "S_TRIGGERS_MISSING"),
        (MetaMismatch, "S_META_MISMATCH"),
    ];

    #[test]
    fn strings_and_sort_order_follow_the_spec_table() {
        assert_eq!(TABLE.len(), 24);
        for (code, text) in TABLE {
            assert_eq!(code.as_str(), *text);
            assert_eq!(code.to_string(), *text);
        }
        for pair in TABLE.windows(2) {
            assert!(
                pair[0].0 < pair[1].0,
                "{} sorts before {}",
                pair[0].1,
                pair[1].1
            );
        }
    }

    fn h(c: char) -> String {
        format!("sha256:{}", c.to_string().repeat(64))
    }

    #[test]
    fn findings_carry_only_what_is_set() {
        assert_eq!(
            canonicalize(&Finding::new(LineInvalid).to_value()),
            r#"{"code":"S_LINE_INVALID"}"#
        );
        assert_eq!(
            canonicalize(&Finding::at(SeqGap, 7, &h('a')).to_value()),
            format!(r#"{{"code":"S_SEQ_GAP","ref":"{}","seq":7}}"#, h('a'))
        );
        assert_eq!(
            canonicalize(
                &Finding::at(ObjectInvalid, 4, &h('b'))
                    .with_core(Code::IdMismatch)
                    .to_value()
            ),
            format!(
                r#"{{"code":"S_OBJECT_INVALID","core":"E_ID_MISMATCH","ref":"{}","seq":4}}"#,
                h('b')
            )
        );
        let f = Finding::segment_hash("000002.jsonl");
        assert_eq!(f.code, SegmentHash);
        assert_eq!(
            canonicalize(&f.to_value()),
            r#"{"code":"S_SEGMENT_HASH","file":"000002.jsonl"}"#
        );
    }

    #[test]
    fn an_empty_report_omits_log_head_and_covered() {
        let report = StoreReport {
            source: Source::Export,
            log: None,
            head: None,
            covered: None,
            checked: Checked::default(),
            forks: 0,
            at_after_received: 0,
            findings: vec![Finding::new(GenesisInvalid)],
            truncated: false,
        };
        assert!(!report.ok());
        assert_eq!(
            canonicalize(&report.to_value()),
            "{\"checked\":{\"attestations\":0,\"checkpoints\":0,\"entries\":0,\"objects\":0},\
             \"counts\":{\"at_after_received\":0,\"forks\":0},\
             \"findings\":[{\"code\":\"S_GENESIS_INVALID\"}],\"ok\":false,\
             \"source\":\"export\",\"truncated\":false,\"v\":\"store-report/0\"}"
        );
    }

    #[test]
    fn a_full_report_has_every_member() {
        let report = StoreReport {
            source: Source::Store,
            log: Some(h('1')),
            head: Some((12, h('2'))),
            covered: Some((10, h('3'))),
            checked: Checked {
                entries: 12,
                objects: 7,
                attestations: 4,
                checkpoints: 1,
            },
            forks: 2,
            at_after_received: 3,
            findings: vec![],
            truncated: false,
        };
        assert!(report.ok());
        assert_eq!(Source::Store.as_str(), "store");
        assert_eq!(Source::Export.as_str(), "export");
        let v = report.to_value();
        let names: Vec<&str> = v
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, _)| k.as_str())
            .collect();
        assert_eq!(
            names,
            [
                "v",
                "ok",
                "source",
                "log",
                "head",
                "covered",
                "checked",
                "counts",
                "findings",
                "truncated"
            ]
        );
        assert_eq!(
            canonicalize(&v),
            format!(
                "{{\"checked\":{{\"attestations\":4,\"checkpoints\":1,\"entries\":12,\"objects\":7}},\
                 \"counts\":{{\"at_after_received\":3,\"forks\":2}},\
                 \"covered\":{{\"checkpoint\":\"{}\",\"size\":10}},\"findings\":[],\
                 \"head\":{{\"entry_hash\":\"{}\",\"seq\":12}},\"log\":\"{}\",\
                 \"ok\":true,\"source\":\"store\",\"truncated\":false,\"v\":\"store-report/0\"}}",
                h('3'),
                h('2'),
                h('1')
            )
        );
    }
}
