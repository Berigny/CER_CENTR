//! Store report codes (`S_` codes). See `spec/store-0.md`, "Report and codes".
//!
//! The declaration order is the sort order of findings in a report, so the
//! derived `Ord` sorts findings that share a seq.

use std::fmt;

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
    /// The manifest's `size`, `head`, `log` or `checkpoint` disagrees with the lines.
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
    /// The ref appeared on an earlier line.
    DuplicateRef,
    /// Seq 1 is not a valid genesis or seq 2 is not its author attestation,
    /// or the log has fewer than two lines.
    GenesisInvalid,
    /// An object with a store subject that is neither the genesis nor a
    /// checkpoint by the genesis signer. Also an intake rejection.
    ReservedSubject,
    /// The object fails evidence/0 steps 1-5, or step 6 within the log.
    ObjectInvalid,
    /// A `prev` is held only at a higher seq.
    PrevPosition,
    /// The attestation's shape or signature is invalid.
    AttestationInvalid,
    /// The attestation's object is not held at a lower seq. Also an intake rejection.
    AttestationTarget,
    /// A checkpoint by the genesis signer is not valid checkpoint/0 for this log.
    CheckpointInvalid,
    /// A checkpoint's `size` is not below its seq, or its `head` or `root`
    /// disagrees with the lines.
    CheckpointMismatch,
    /// A checkpoint has no valid witness by the genesis key.
    CheckpointUnsigned,
    /// The stored heads or checkpoints differ from a recompute.
    ProjectionDrift,
    /// The append-only triggers are missing.
    TriggersMissing,
    /// The stored log id differs from line 1.
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

#[cfg(test)]
mod tests {
    use super::FindingCode::{self, *};

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
}
