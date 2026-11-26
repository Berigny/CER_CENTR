//! Merkle proofs for a log: inclusion of an entry in a tree size, and
//! consistency between two tree sizes. See `spec/store-0.md`, "Proofs".
//!
//! The values here are exactly what `evidence store prove` prints (section 8
//! of `docs/m1-store-spec.md`), and what `evidence_log::merkle`'s verifiers,
//! the WebAssembly module and the Python package check:
//!
//! ```text
//! {"seq":n,"size":m,"entry":{…},"leaf_hash":"sha256:…","proof":["sha256:…"],"root":"sha256:…","checkpoint":"sha256:…"}
//! {"old_size":…,"new_size":…,"old_root":"sha256:…","new_root":"sha256:…","proof":["sha256:…"]}
//! ```
//!
//! The functions take explicit sizes. Choosing a default size (the latest
//! checkpoint's) is the command line's job.
//!
//! The proofs of `test-vectors/store/exports/003-attestations-two-checkpoints`
//! (`case.json`, `proofs`) are what these functions give for that log: the
//! `seq`, `size`, `leaf_hash`, `proof` and `root` members of each inclusion
//! proof, and every member of the consistency proof.

use evidence_core::json::Value;
use evidence_log::merkle::{self, Digest};
use evidence_log::{parse_line, Entry};

use crate::api::{damaged, LogRead, StoreError};

/// That the entry at `seq` is leaf `seq` − 1 of the tree of size `size`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InclusionProof {
    pub seq: u64,
    pub size: u64,
    /// The entry at `seq`.
    pub entry: Entry,
    /// SHA-256(0x00 ‖ canonical entry).
    pub leaf_hash: Digest,
    /// RFC 9162 `PATH(seq − 1, D[size])`, from the leaf upwards.
    pub proof: Vec<Digest>,
    /// The tree hash over entries 1 to `size`.
    pub root: Digest,
    /// The id of the checkpoint whose `size` is `size`, if there is one.
    /// Sizes of checkpoints strictly increase, so there is at most one.
    pub checkpoint: Option<String>,
}

impl InclusionProof {
    /// `{"seq","size","entry","leaf_hash","proof","root","checkpoint"}`;
    /// `checkpoint` omitted when None. Digests as `"sha256:"` + hex.
    pub fn to_value(&self) -> Value {
        let mut m = vec![
            ("seq".to_string(), Value::Number(self.seq as f64)),
            ("size".into(), Value::Number(self.size as f64)),
            ("entry".into(), self.entry.to_value()),
            (
                "leaf_hash".into(),
                Value::String(merkle::to_text(&self.leaf_hash)),
            ),
            ("proof".into(), digests(&self.proof)),
            ("root".into(), Value::String(merkle::to_text(&self.root))),
        ];
        if let Some(id) = &self.checkpoint {
            m.push(("checkpoint".into(), Value::String(id.clone())));
        }
        Value::Object(m)
    }

    /// `evidence_log::merkle::verify_inclusion` over this proof, with index
    /// `seq` − 1. Does not check that `leaf_hash` is the hash of `entry`.
    pub fn verify(&self) -> bool {
        self.seq >= 1
            && merkle::verify_inclusion(
                &self.leaf_hash,
                self.seq - 1,
                self.size,
                &self.proof,
                &self.root,
            )
    }
}

/// That the tree of size `new_size` extends the tree of size `old_size`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsistencyProof {
    pub old_size: u64,
    pub new_size: u64,
    pub old_root: Digest,
    pub new_root: Digest,
    /// RFC 9162 `PROOF(old_size, D[new_size])`.
    pub proof: Vec<Digest>,
}

impl ConsistencyProof {
    /// `{"old_size","new_size","old_root","new_root","proof"}`.
    pub fn to_value(&self) -> Value {
        Value::Object(vec![
            ("old_size".into(), Value::Number(self.old_size as f64)),
            ("new_size".into(), Value::Number(self.new_size as f64)),
            (
                "old_root".into(),
                Value::String(merkle::to_text(&self.old_root)),
            ),
            (
                "new_root".into(),
                Value::String(merkle::to_text(&self.new_root)),
            ),
            ("proof".into(), digests(&self.proof)),
        ])
    }

    /// `evidence_log::merkle::verify_consistency` over this proof.
    pub fn verify(&self) -> bool {
        merkle::verify_consistency(
            self.old_size,
            self.new_size,
            &self.old_root,
            &self.new_root,
            &self.proof,
        )
    }
}

fn digests(list: &[Digest]) -> Value {
    Value::Array(
        list.iter()
            .map(|d| Value::String(merkle::to_text(d)))
            .collect(),
    )
}

/// The inclusion proof of the entry at `seq` in the tree of size `size`.
///
/// - `E_USAGE` unless 1 ≤ `seq` ≤ `size` ≤ the head's seq.
/// - The leaves are [`LogRead::leaf_hashes`]`(size)`; `root` =
///   `merkle::root` of them; `proof` = `merkle::inclusion_proof(leaves, seq −
///   1)`.
/// - `entry` is read with [`LogRead::lines`]`(seq, 1)` and
///   `evidence_log::parse_line`; `S_IO` if that seq has no line, its line
///   does not parse or holds the entry of another seq (a damaged store), or
///   its leaf hash is not leaf `seq` − 1. `S_IO` too if `leaf_hashes` does
///   not give `size` leaves.
/// - `checkpoint` is the id of the checkpoint in [`LogRead::checkpoints`]
///   whose `size` is `size`, if any.
pub fn inclusion(log: &dyn LogRead, seq: u64, size: u64) -> Result<InclusionProof, StoreError> {
    let head = log.head()?;
    if !(1 <= seq && seq <= size && size <= head.seq) {
        return Err(StoreError::Usage(format!(
            "an inclusion proof needs 1 <= seq <= size <= {} (the head), not seq {seq} and size {size}",
            head.seq
        )));
    }
    let leaves = leaves(log, size)?;
    let lines = log.lines(seq, 1)?;
    let line = match lines.as_slice() {
        [(got, line)] if *got == seq => line,
        _ => return Err(damaged(format!("the log has no line at seq {seq}"))),
    };
    let line = parse_line(line.as_bytes())
        .map_err(|code| damaged(format!("the line at seq {seq} is {code}")))?;
    if line.entry.seq != seq {
        return Err(damaged(format!(
            "the line at seq {seq} holds the entry of seq {}",
            line.entry.seq
        )));
    }
    let leaf_hash = merkle::leaf_hash(line.entry_canonical.as_bytes());
    // seq ≤ size = the number of leaves, so index seq − 1 is a leaf.
    let index = seq - 1;
    if usize::try_from(index).ok().and_then(|i| leaves.get(i)) != Some(&leaf_hash) {
        return Err(damaged(format!(
            "the leaf hash of seq {seq} is not the hash of its entry"
        )));
    }
    let checkpoint = log
        .checkpoints()?
        .into_iter()
        .find(|c| c.size == size)
        .map(|c| c.id);
    Ok(InclusionProof {
        seq,
        size,
        entry: line.entry,
        leaf_hash,
        proof: merkle::inclusion_proof(&leaves, index),
        root: merkle::root(&leaves),
        checkpoint,
    })
}

/// The consistency proof from tree size `old_size` to `new_size`.
///
/// - `E_USAGE` unless 1 ≤ `old_size` < `new_size` ≤ the head's seq (RFC
///   9162 verification is defined only for 0 < old < new, so no other proof
///   could verify).
/// - The leaves are [`LogRead::leaf_hashes`]`(new_size)` (`S_IO` unless it
///   gives `new_size` of them); `old_root` and `new_root` are `merkle::root`
///   of the first `old_size` and of all of them; `proof` =
///   `merkle::consistency_proof(leaves, old_size)`.
pub fn consistency(
    log: &dyn LogRead,
    old_size: u64,
    new_size: u64,
) -> Result<ConsistencyProof, StoreError> {
    let head = log.head()?;
    if !(1 <= old_size && old_size < new_size && new_size <= head.seq) {
        return Err(StoreError::Usage(format!(
            "a consistency proof needs 1 <= old size < new size <= {} (the head), not {old_size} and {new_size}",
            head.seq
        )));
    }
    let leaves = leaves(log, new_size)?;
    // old_size < new_size = the number of leaves.
    let old = usize::try_from(old_size)
        .ok()
        .and_then(|n| leaves.get(..n))
        .ok_or_else(|| damaged(format!("the log has fewer than {old_size} leaves")))?;
    Ok(ConsistencyProof {
        old_size,
        new_size,
        old_root: merkle::root(old),
        new_root: merkle::root(&leaves),
        proof: merkle::consistency_proof(&leaves, old_size),
    })
}

/// [`LogRead::leaf_hashes`]`(size)`, which must give exactly `size` leaves.
fn leaves(log: &dyn LogRead, size: u64) -> Result<Vec<Digest>, StoreError> {
    let leaves = log.leaf_hashes(size)?;
    if leaves.len() as u64 != size {
        return Err(damaged(format!(
            "asked for {size} leaf hashes, the log gave {}",
            leaves.len()
        )));
    }
    Ok(leaves)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::api::{CheckpointInfo, Head, LogWrite, StoredAttestation, StoredObject};
    use crate::clock::ScriptClock;
    use crate::mem::MemLog;
    use crate::signer::SeedSigner;
    use evidence_core::canon;
    use evidence_log::{Genesis, SubjectKey};

    fn vectors() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-vectors")
    }

    fn store_key() -> SeedSigner {
        SeedSigner::from_file(&vectors().join("keys/store-demo.seed")).unwrap()
    }

    /// Genesis, object vector 001, a checkpoint (seq 4, size 3), object
    /// vector 002, a checkpoint (seq 7, size 6): seqs 1 to 8.
    fn log() -> MemLog {
        let clock = ScriptClock::new(&[
            "2026-11-26T00:00:00Z",
            "2026-11-26T00:00:01Z",
            "2026-11-26T00:00:02Z",
            "2026-11-26T00:00:03Z",
            "2026-11-26T00:00:04Z",
        ]);
        let mut log = MemLog::init("cer-demo", &store_key(), Box::new(clock.clone())).unwrap();
        for case in ["001-create-inline", "002-update-with-prev"] {
            let path = vectors().join("objects").join(case).join("object.json");
            log.append(&std::fs::read(path).unwrap()).unwrap();
            log.checkpoint(&store_key()).unwrap();
        }
        assert_eq!(clock.remaining(), 0);
        assert_eq!(log.head().unwrap().seq, 8);
        log
    }

    #[test]
    fn inclusion_proofs_verify_for_every_seq_and_size() {
        let log = log();
        let checkpoints = log.checkpoints().unwrap();
        assert_eq!(
            checkpoints.iter().map(|c| c.size).collect::<Vec<_>>(),
            [3, 6]
        );
        let lines = log.lines(1, 100).unwrap();
        for size in 1..=8 {
            let leaves = log.leaf_hashes(size).unwrap();
            for seq in 1..=size {
                let p = inclusion(&log, seq, size).unwrap();
                assert!(p.verify(), "seq {seq} size {size}");
                assert_eq!((p.seq, p.size), (seq, size));
                let line = parse_line(lines[seq as usize - 1].1.as_bytes()).unwrap();
                assert_eq!(p.entry, line.entry);
                assert_eq!(p.leaf_hash, leaves[seq as usize - 1]);
                assert_eq!(p.root, merkle::root(&leaves));
                assert_eq!(p.proof, merkle::inclusion_proof(&leaves, seq - 1));
                let checkpoint = checkpoints.iter().find(|c| c.size == size);
                assert_eq!(p.checkpoint, checkpoint.map(|c| c.id.clone()));
                if let Some(c) = checkpoint {
                    assert_eq!(merkle::to_text(&p.root), c.root);
                }
                // Each member's change makes it fail.
                for i in 0..p.proof.len() {
                    let mut bad = p.clone();
                    bad.proof[i][0] ^= 1;
                    assert!(!bad.verify());
                }
                let mut bad = p.clone();
                bad.leaf_hash[31] ^= 0x80;
                assert!(!bad.verify());
                let mut bad = p.clone();
                bad.root[5] ^= 4;
                assert!(!bad.verify());
                let mut bad = p.clone();
                bad.seq = if seq == 1 { 2 } else { seq - 1 };
                assert!(!bad.verify());
            }
        }
    }

    #[test]
    fn inclusion_proof_value_has_the_cli_shape() {
        let log = log();
        let p = inclusion(&log, 2, 3).unwrap();
        let text = canon::canonicalize(&p.to_value());
        let leaf = merkle::to_text(&p.leaf_hash);
        let root = merkle::to_text(&p.root);
        let proof: Vec<String> = p.proof.iter().map(merkle::to_text).collect();
        assert_eq!(proof.len(), 2);
        let id = log.checkpoints().unwrap()[0].id.clone();
        assert_eq!(
            text,
            format!(
                "{{\"checkpoint\":\"{id}\",\"entry\":{},\"leaf_hash\":\"{leaf}\",\
                 \"proof\":[\"{}\",\"{}\"],\"root\":\"{root}\",\"seq\":2,\"size\":3}}",
                p.entry.canonical(),
                proof[0],
                proof[1]
            )
        );
        // No checkpoint has size 5: the member is omitted.
        let p = inclusion(&log, 5, 5).unwrap();
        assert_eq!(p.checkpoint, None);
        assert!(p.to_value().get("checkpoint").is_none());
    }

    #[test]
    fn consistency_proofs_verify_between_every_two_sizes() {
        let log = log();
        let leaves = log.leaf_hashes(8).unwrap();
        for new_size in 2..=8u64 {
            for old_size in 1..new_size {
                let p = consistency(&log, old_size, new_size).unwrap();
                assert!(p.verify(), "{old_size} to {new_size}");
                assert_eq!(p.old_root, merkle::root(&leaves[..old_size as usize]));
                assert_eq!(p.new_root, merkle::root(&leaves[..new_size as usize]));
                for i in 0..p.proof.len() {
                    let mut bad = p.clone();
                    bad.proof[i][17] ^= 2;
                    assert!(!bad.verify());
                }
                let mut bad = p.clone();
                bad.old_root[0] ^= 1;
                assert!(!bad.verify());
                let mut bad = p.clone();
                bad.new_root[0] ^= 1;
                assert!(!bad.verify());
            }
        }
        // Between the two checkpoints, the roots are theirs.
        let c = log.checkpoints().unwrap();
        let p = consistency(&log, c[0].size, c[1].size).unwrap();
        assert_eq!(merkle::to_text(&p.old_root), c[0].root);
        assert_eq!(merkle::to_text(&p.new_root), c[1].root);
        let proof: Vec<String> = p
            .proof
            .iter()
            .map(|d| format!("\"{}\"", merkle::to_text(d)))
            .collect();
        assert_eq!(
            canon::canonicalize(&p.to_value()),
            format!(
                "{{\"new_root\":\"{}\",\"new_size\":6,\"old_root\":\"{}\",\"old_size\":3,\
                 \"proof\":[{}]}}",
                c[1].root,
                c[0].root,
                proof.join(",")
            )
        );
    }

    #[test]
    fn positions_outside_the_log_are_usage_errors() {
        let log = log();
        for (seq, size) in [(0, 1), (0, 0), (2, 1), (1, 9), (9, 9), (u64::MAX, u64::MAX)] {
            let e = inclusion(&log, seq, size).unwrap_err();
            assert_eq!(e.code_str(), "E_USAGE", "seq {seq} size {size}");
        }
        for (old, new) in [(0, 1), (0, 8), (1, 1), (8, 8), (3, 2), (1, 9), (8, 9)] {
            let e = consistency(&log, old, new).unwrap_err();
            assert_eq!(e.code_str(), "E_USAGE", "{old} to {new}");
        }
        assert!(inclusion(&log, 8, 8).is_ok());
        assert!(consistency(&log, 7, 8).is_ok());
    }

    /// What the doctored view changes.
    #[derive(Clone, Copy)]
    enum Damage {
        /// `lines` gives nothing.
        NoLine,
        /// `lines` gives the line of the seq after the one asked.
        NextLine,
        /// `lines` gives a line that does not parse.
        Garbage,
        /// `lines` gives the right seq with the line of the next seq.
        OtherEntry,
        /// `leaf_hashes` gives one leaf too few.
        ShortLeaves,
        /// `leaf_hashes` gives a wrong leaf at every position.
        WrongLeaves,
    }

    struct View<'a>(&'a MemLog, Damage);

    impl LogRead for View<'_> {
        fn log_id(&self) -> &str {
            self.0.log_id()
        }
        fn genesis(&self) -> &Genesis {
            self.0.genesis()
        }
        fn head(&self) -> Result<Head, StoreError> {
            self.0.head()
        }
        fn lines(&self, from_seq: u64, limit: usize) -> Result<Vec<(u64, String)>, StoreError> {
            let lines = self.0.lines(from_seq, limit)?;
            Ok(match self.1 {
                Damage::NoLine => Vec::new(),
                Damage::NextLine => self.0.lines(from_seq + 1, limit)?,
                Damage::Garbage => lines.into_iter().map(|(s, _)| (s, "{}".into())).collect(),
                Damage::OtherEntry => {
                    let next = self.0.lines(from_seq + 1, limit)?;
                    lines
                        .iter()
                        .zip(next)
                        .map(|((s, _), (_, l))| (*s, l))
                        .collect()
                }
                _ => lines,
            })
        }
        fn object(&self, id: &str) -> Result<Option<StoredObject>, StoreError> {
            self.0.object(id)
        }
        fn attestations(&self, object_id: &str) -> Result<Vec<StoredAttestation>, StoreError> {
            self.0.attestations(object_id)
        }
        fn heads(&self, subject: &SubjectKey, lane: &str) -> Result<Vec<String>, StoreError> {
            LogRead::heads(self.0, subject, lane)
        }
        fn head_rows(&self) -> Result<Vec<(SubjectKey, String, String)>, StoreError> {
            self.0.head_rows()
        }
        fn checkpoints(&self) -> Result<Vec<CheckpointInfo>, StoreError> {
            self.0.checkpoints()
        }
        fn leaf_hashes(&self, upto: u64) -> Result<Vec<Digest>, StoreError> {
            let mut leaves = self.0.leaf_hashes(upto)?;
            match self.1 {
                Damage::ShortLeaves => {
                    leaves.pop();
                }
                Damage::WrongLeaves => leaves.iter_mut().for_each(|l| l[0] ^= 1),
                _ => {}
            }
            Ok(leaves)
        }
    }

    #[test]
    fn damaged_content_is_s_io() {
        let log = log();
        for damage in [
            Damage::NoLine,
            Damage::NextLine,
            Damage::Garbage,
            Damage::OtherEntry,
            Damage::ShortLeaves,
            Damage::WrongLeaves,
        ] {
            let view = View(&log, damage);
            let e = inclusion(&view, 3, 6).unwrap_err();
            assert_eq!(e.code_str(), "S_IO", "{e}");
        }
        let e = consistency(&View(&log, Damage::ShortLeaves), 3, 6).unwrap_err();
        assert_eq!(e.code_str(), "S_IO", "{e}");
        // Lines are not read for a consistency proof.
        assert!(consistency(&View(&log, Damage::NoLine), 3, 6).is_ok());
    }
}
