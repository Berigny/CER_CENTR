//! T2, writer agreement, and the library part of T11, proofs
//! (`docs/m1-store-spec.md` section 11; `spec/store-0.md`, "Conformance").
//!
//! Every valid export case, `test-vectors/store/exports/0xx-*`, holds the
//! `inputs.json` script that the JavaScript reference writer replayed to
//! build it. Replaying the same script through a backend here, with a
//! [`ScriptClock`] and the test store key, must export a manifest and
//! segment files byte-identical to the committed ones. The script is run as
//! `tools/reference/log.mjs` (`replay`) runs it: every operation succeeds,
//! none is a duplicate, and the clock holds exactly one reading per
//! operation that reads it.
//!
//! The export is checked in memory (`export_files`) and as written to a
//! directory (`export`), which must read back (`read_export_dir`) and verify
//! to the case's golden report.
//!
//! The proofs of each replayed log, and of the committed export's lines,
//! must then verify with `evidence_log::merkle` for every seq at every
//! checkpoint's size and at the head, fail with any one byte changed, and
//! reproduce the proofs committed in case 003 byte for byte (T11, library
//! part; the command line and the bindings are phase E).
//!
//! The scenarios are generic over [`Backend`] (`tests/common`); the
//! `writer_vectors!` macro turns them into tests for one backend. They run
//! against `MemLog` and `SqliteLog`, whose `reopen` closes the writer and
//! opens the store directory again with a clone of the same clock.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use evidence_core::canon;
use evidence_core::json::Value;
use evidence_core::object;
use evidence_core::time;
use evidence_log::manifest::MANIFEST_FILE;
use evidence_log::merkle::{self, Digest};
use evidence_log::{
    parse_line, verify_export, Genesis, Heads, Kind, Manifest, SubjectKey, Verifier, VerifyOptions,
};
use evidence_store::{
    consistency, export, export_files, inclusion, read_export_dir, verify_log, CheckpointInfo,
    Head, LogRead, ScriptClock, SeedSigner, StoreError, StoredAttestation, StoredObject,
};

mod common;
use common::{
    case_name as name_of, export_cases, files_in, load, nth_line, read, vectors, Backend, Mem,
    Sqlite, ALL,
};

// ---------------------------------------------------------------------------
// The vectors

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{key} is not a string in {}", canon::canonicalize(v)))
}

fn number(v: &Value, key: &str) -> u64 {
    match v.get(key) {
        Some(Value::Number(n)) if *n >= 0.0 && n.fract() == 0.0 => *n as u64,
        _ => panic!("{key} is not an integer in {}", canon::canonicalize(v)),
    }
}

fn list<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key)
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("{key} is not an array in {}", canon::canonicalize(v)))
}

/// The member names of an object, sorted.
fn members(v: &Value) -> Vec<&str> {
    let mut names: Vec<&str> = v
        .as_object()
        .unwrap_or_else(|| panic!("{} is not an object", canon::canonicalize(v)))
        .iter()
        .map(|(k, _)| k.as_str())
        .collect();
    names.sort_unstable();
    names
}

/// The valid export cases, `exports/0xx-*`, in name order. Each must hold
/// an `inputs.json`.
fn valid_cases() -> Vec<PathBuf> {
    let cases = export_cases(true);
    assert!(cases.len() >= 7, "valid export cases: {cases:?}");
    for case in &cases {
        assert!(
            case.join("inputs.json").is_file(),
            "{} has no inputs.json",
            case.display()
        );
    }
    cases
}

/// The committed export of a case: `manifest.json` and the segment files,
/// sorted by name.
fn committed(case: &Path) -> Vec<(String, Vec<u8>)> {
    files_in(case)
        .into_iter()
        .filter(|(n, _)| n == MANIFEST_FILE || n.ends_with(".jsonl"))
        .collect()
}

// ---------------------------------------------------------------------------
// Replaying inputs.json

/// A log replayed from a case's `inputs.json`.
struct Replayed<B> {
    backend: B,
    segment_size: u64,
}

/// Replay the `inputs.json` of `case` into a new `B`, as the reference
/// writer's `replay` does.
fn replay<B: Backend>(case: &Path) -> Replayed<B> {
    let where_ = name_of(case);
    let inputs = load(&case.join("inputs.json"));
    assert_eq!(
        members(&inputs),
        ["clock", "name", "ops", "seed", "segment_size"],
        "{where_}: inputs.json members"
    );
    let name = text(&inputs, "name");
    let segment_size = number(&inputs, "segment_size");
    let clock = ScriptClock::from_timestamps(list(&inputs, "clock").iter().map(|t| {
        let t = t.as_str().expect("a clock reading is text");
        time::parse(t).unwrap_or_else(|_| panic!("{where_}: clock reading {t:?}"))
    }));
    let seed = text(&inputs, "seed");
    let signer = SeedSigner::from_file(&vectors().join(format!("keys/{seed}.seed")))
        .unwrap_or_else(|e| panic!("{where_}: seed {seed}: {e}"));
    let ops = list(&inputs, "ops");
    assert!(
        ops.first().map(|op| text(op, "op")) == Some("init"),
        "{where_}: ops start with init"
    );

    let mut backend: Option<B> = None;
    for (i, op) in ops.iter().enumerate() {
        let at = format!("{where_}: ops[{i}]");
        let readings = clock.remaining();
        let kind = text(op, "op");
        let reads_clock = kind != "reopen";
        if kind == "init" {
            assert!(i == 0 && members(op) == ["op"], "{at}: init is first, only");
            let b = B::init(name, &signer, &clock).unwrap_or_else(|e| panic!("{at}: {e}"));
            assert_eq!(b.log().epoch(), 1, "{at}");
            backend = Some(b);
        } else {
            let b = backend.as_mut().expect("init ran first");
            let head = b.log().head().unwrap().seq;
            match kind {
                "append" => {
                    let bytes = match members(op).as_slice() {
                        ["file", "op"] => read(&vectors().join(text(op, "file"))),
                        ["object", "op"] => {
                            canon::canonicalize(op.get("object").unwrap()).into_bytes()
                        }
                        other => panic!("{at}: append takes file or object, not {other:?}"),
                    };
                    let receipt = b
                        .log_mut()
                        .append(&bytes)
                        .unwrap_or_else(|e| panic!("{at}: {e}"));
                    assert!(!receipt.duplicate, "{at} is a duplicate");
                    assert_eq!(receipt.seq, head + 1, "{at}");
                }
                "attest" => {
                    assert_eq!(members(op), ["file", "line", "op"], "{at}");
                    let file = read(&vectors().join(text(op, "file")));
                    let line = nth_line(&file, number(op, "line") as usize);
                    let receipt = b
                        .log_mut()
                        .attest(&line)
                        .unwrap_or_else(|e| panic!("{at}: {e}"));
                    assert!(!receipt.duplicate, "{at} is a duplicate");
                    assert_eq!(receipt.seq, head + 1, "{at}");
                }
                "reopen" => {
                    assert_eq!(members(op), ["op"], "{at}");
                    let epoch = b.log().epoch();
                    b.reopen(&clock);
                    assert_eq!(b.log().epoch(), epoch + 1, "{at}");
                    assert_eq!(b.log().head().unwrap().seq, head, "{at}");
                }
                "checkpoint" => {
                    assert_eq!(members(op), ["op"], "{at}");
                    let info = b
                        .log_mut()
                        .checkpoint(&signer)
                        .unwrap_or_else(|e| panic!("{at}: {e}"));
                    assert_eq!((info.seq, info.size), (head + 1, head), "{at}");
                    assert_eq!(b.log().head().unwrap().seq, head + 2, "{at}");
                }
                other => panic!("{at}: unknown op {other:?}"),
            }
        }
        assert_eq!(
            readings - clock.remaining(),
            usize::from(reads_clock),
            "{at}: one clock reading per operation that reads it"
        );
    }
    assert_eq!(clock.remaining(), 0, "{where_}: clock readings left unused");
    Replayed {
        backend: backend.expect("init ran"),
        segment_size,
    }
}

// ---------------------------------------------------------------------------
// A log read from an export's lines

/// The committed export of a case as a [`LogRead`]: its lines, and the
/// projections a verifier folds from them. Proofs taken from it are proofs
/// from the export's lines, whatever writer replayed the case.
struct ExportLog {
    log_id: String,
    genesis: Genesis,
    lines: Vec<String>,
    heads: Heads,
    checkpoints: Vec<CheckpointInfo>,
}

impl ExportLog {
    fn read(case: &Path) -> ExportLog {
        let bytes = read_export_dir(case).unwrap();
        let mut verifier = Verifier::new(VerifyOptions::default());
        let mut lines = Vec::new();
        for segment in &bytes.segments {
            verifier.push_segment(segment);
            let text = std::str::from_utf8(segment).unwrap();
            lines.extend(text.split_terminator('\n').map(str::to_string));
        }
        let first = parse_line(lines[0].as_bytes()).unwrap();
        let genesis = object::check_value(first.body.value().clone()).unwrap();
        let log = ExportLog {
            log_id: verifier.log_id().unwrap().to_string(),
            genesis: Genesis::from_object(&genesis).unwrap(),
            lines,
            heads: verifier.heads().clone(),
            checkpoints: verifier
                .checkpoints()
                .iter()
                .map(|(seq, id, c)| CheckpointInfo {
                    id: id.clone(),
                    seq: *seq,
                    size: c.size,
                    root: c.root.clone(),
                    head: c.head.clone(),
                })
                .collect(),
        };
        assert!(verifier.finish().ok(), "{}", name_of(case));
        log
    }
}

impl LogRead for ExportLog {
    fn log_id(&self) -> &str {
        &self.log_id
    }

    fn genesis(&self) -> &Genesis {
        &self.genesis
    }

    fn head(&self) -> Result<Head, StoreError> {
        let line = parse_line(self.lines.last().unwrap().as_bytes()).unwrap();
        Ok(Head {
            seq: line.entry.seq,
            entry_hash: line.entry.hash(),
        })
    }

    fn lines(&self, from_seq: u64, limit: usize) -> Result<Vec<(u64, String)>, StoreError> {
        Ok((1..)
            .zip(&self.lines)
            .skip(from_seq.saturating_sub(1) as usize)
            .take(limit)
            .map(|(seq, line)| (seq, line.clone()))
            .collect())
    }

    /// A proof reads the checkpoint objects it names: the object line with
    /// this ref, if any.
    fn object(&self, id: &str) -> Result<Option<StoredObject>, StoreError> {
        Ok(self.lines.iter().find_map(|line| {
            let line = parse_line(line.as_bytes()).unwrap();
            (line.entry.kind == Kind::Object && line.entry.reference == id).then(|| StoredObject {
                id: id.to_string(),
                seq: line.entry.seq,
                received_at: line.entry.received_at.clone(),
                bytes: canon::canonicalize(line.body.value()),
            })
        }))
    }

    /// A proof reads the witnesses of the checkpoints it names: the
    /// attestation lines whose `object` is this id, in seq order.
    fn attestations(&self, object_id: &str) -> Result<Vec<StoredAttestation>, StoreError> {
        Ok(self
            .lines
            .iter()
            .filter_map(|line| {
                let line = parse_line(line.as_bytes()).unwrap();
                let value = line.body.value();
                (line.entry.kind == Kind::Attestation
                    && value.get("object").and_then(Value::as_str) == Some(object_id))
                .then(|| StoredAttestation {
                    hash: line.entry.reference.clone(),
                    seq: line.entry.seq,
                    line: canon::canonicalize(value),
                })
            })
            .collect())
    }

    fn heads(&self, subject: &SubjectKey, lane: &str) -> Result<Vec<String>, StoreError> {
        Ok(self.heads.get(subject, lane))
    }

    fn head_rows(&self) -> Result<Vec<(SubjectKey, String, String)>, StoreError> {
        Ok(self.heads.rows())
    }

    fn checkpoints(&self) -> Result<Vec<CheckpointInfo>, StoreError> {
        Ok(self.checkpoints.clone())
    }

    fn leaf_hashes(&self, upto: u64) -> Result<Vec<Digest>, StoreError> {
        if upto > self.lines.len() as u64 {
            return Err(StoreError::Usage(format!("{upto} is above the head")));
        }
        Ok(self.lines[..upto as usize]
            .iter()
            .map(|l| {
                merkle::leaf_hash(parse_line(l.as_bytes()).unwrap().entry_canonical.as_bytes())
            })
            .collect())
    }
}

// ---------------------------------------------------------------------------
// Scenarios

/// T2: every valid case replays to its committed export, byte for byte, in
/// memory and written to a directory; the written export reads back and
/// verifies to the case's golden report.
fn replays_to_the_committed_export<B: Backend>() {
    for case in valid_cases() {
        let name = name_of(&case);
        let replayed = replay::<B>(&case);
        let log = replayed.backend.log();
        let want = committed(&case);

        let files = export_files(log, replayed.segment_size).unwrap();
        let names: Vec<&str> = files.files.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names.last(), Some(&MANIFEST_FILE), "{name}: manifest last");
        let mut got = files.files.clone();
        got.sort();
        assert_eq!(
            got.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
            want.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
            "{name}: file names"
        );
        for ((file, bytes), (_, expected)) in got.iter().zip(&want) {
            assert!(
                bytes == expected,
                "{name}/{file} differs:\n  got  {}\n  want {}",
                String::from_utf8_lossy(bytes),
                String::from_utf8_lossy(expected)
            );
        }
        assert_eq!(
            Manifest::parse(&read(&case.join(MANIFEST_FILE))),
            Ok(files.manifest.clone()),
            "{name}"
        );

        let tmp = tempfile::tempdir().unwrap();
        let out = tmp.path().join(&name);
        let manifest = export(log, &out, replayed.segment_size).unwrap();
        assert_eq!(manifest, files.manifest, "{name}");
        assert_eq!(files_in(&out), want, "{name}: the exported directory");
        assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 1, "{name}");

        let bytes = read_export_dir(&out).unwrap();
        let report = verify_export(
            &bytes.manifest,
            &bytes.segment_slices(),
            VerifyOptions::default(),
        );
        let expect = load(&case.join("case.json"));
        assert_eq!(
            canon::canonicalize(&report.to_value()),
            canon::canonicalize(expect.get("expect").unwrap()),
            "{name}: the report of the written export"
        );

        let live = verify_log(log, VerifyOptions::default()).unwrap();
        assert!(live.ok(), "{name}: {:?}", live.findings);
        assert_eq!(live.head, report.head, "{name}");
        assert_eq!(live.covered, report.covered, "{name}");
        // SqliteLog: a separate reader agrees, and verify_store is ok.
        replayed.backend.check();
    }
}

/// The leaf hashes of a log, from its lines: SHA-256(0x00 ‖ entry).
fn leaves_of(log: &dyn LogRead) -> Vec<Digest> {
    log.lines(1, ALL)
        .unwrap()
        .iter()
        .map(|(_, l)| {
            merkle::leaf_hash(parse_line(l.as_bytes()).unwrap().entry_canonical.as_bytes())
        })
        .collect()
}

/// A copy of `d` with one bit of byte `i` changed.
fn flipped(d: &Digest, i: usize) -> Digest {
    let mut d = *d;
    d[i % 32] ^= 1 << (i % 8);
    d
}

/// The library part of T11 for one log: at each checkpoint's size, the
/// inclusion proof of every seq verifies against the checkpoint's root and
/// names it; consecutive checkpoints have a consistency proof that
/// verifies; one changed byte in any proof element, leaf hash or root makes
/// it fail. Returns the number of inclusion proofs checked.
fn check_proofs(log: &dyn LogRead, what: &str) -> usize {
    let leaves = leaves_of(log);
    let head = log.head().unwrap().seq;
    assert_eq!(log.leaf_hashes(head).unwrap(), leaves, "{what}");
    let checkpoints = log.checkpoints().unwrap();
    let mut checked = 0;
    // Every checkpoint's size, and the head (with or without a checkpoint).
    let mut sizes: Vec<(u64, Option<&CheckpointInfo>)> =
        checkpoints.iter().map(|c| (c.size, Some(c))).collect();
    if checkpoints.iter().all(|c| c.size != head) {
        sizes.push((head, None));
    }
    for (size, checkpoint) in sizes {
        let root = merkle::root(&leaves[..size as usize]);
        if let Some(c) = checkpoint {
            assert_eq!(merkle::to_text(&root), c.root, "{what}: checkpoint root");
        }
        for seq in 1..=size {
            let p = inclusion(log, seq, size).unwrap();
            let index = seq - 1;
            assert_eq!(p.root, root, "{what}: seq {seq} size {size}");
            assert_eq!(p.leaf_hash, leaves[index as usize], "{what}: seq {seq}");
            assert_eq!(p.entry.seq, seq, "{what}");
            assert_eq!(
                p.checkpoint.as_deref(),
                checkpoint.map(|c| c.id.as_str()),
                "{what}: seq {seq} size {size}"
            );
            assert!(
                merkle::verify_inclusion(&p.leaf_hash, index, size, &p.proof, &p.root),
                "{what}: seq {seq} size {size}"
            );
            assert!(p.verify());
            for (i, element) in p.proof.iter().enumerate() {
                let mut bad = p.proof.clone();
                bad[i] = flipped(element, i + seq as usize);
                assert!(
                    !merkle::verify_inclusion(&p.leaf_hash, index, size, &bad, &p.root),
                    "{what}: seq {seq} size {size}, element {i} changed"
                );
            }
            let leaf = flipped(&p.leaf_hash, seq as usize);
            assert!(!merkle::verify_inclusion(
                &leaf, index, size, &p.proof, &p.root
            ));
            let root = flipped(&p.root, seq as usize + 7);
            assert!(!merkle::verify_inclusion(
                &p.leaf_hash,
                index,
                size,
                &p.proof,
                &root
            ));
            checked += 1;
        }
    }
    for pair in checkpoints.windows(2) {
        let (old, new) = (&pair[0], &pair[1]);
        let p = consistency(log, old.size, new.size).unwrap();
        assert_eq!(merkle::to_text(&p.old_root), old.root, "{what}");
        assert_eq!(merkle::to_text(&p.new_root), new.root, "{what}");
        assert!(
            merkle::verify_consistency(p.old_size, p.new_size, &p.old_root, &p.new_root, &p.proof),
            "{what}: {} to {}",
            old.size,
            new.size
        );
        assert!(p.verify());
        assert!(!p.proof.is_empty(), "{what}");
        for (i, element) in p.proof.iter().enumerate() {
            let mut bad = p.proof.clone();
            bad[i] = flipped(element, i);
            assert!(
                !merkle::verify_consistency(p.old_size, p.new_size, &p.old_root, &p.new_root, &bad),
                "{what}: consistency element {i} changed"
            );
        }
        let old_root = flipped(&p.old_root, 3);
        assert!(!merkle::verify_consistency(
            p.old_size,
            p.new_size,
            &old_root,
            &p.new_root,
            &p.proof
        ));
        let new_root = flipped(&p.new_root, 4);
        assert!(!merkle::verify_consistency(
            p.old_size,
            p.new_size,
            &p.old_root,
            &new_root,
            &p.proof
        ));
    }
    checked
}

/// T11 over every valid case, for the replayed log and for the committed
/// export's lines.
fn proofs_of_every_valid_case<B: Backend>() {
    let mut checked = 0;
    for case in valid_cases() {
        let name = name_of(&case);
        let replayed = replay::<B>(&case);
        checked += check_proofs(replayed.backend.log(), &name);
        check_proofs(&ExportLog::read(&case), &format!("{name} (export)"));
    }
    assert!(checked > 0);
}

/// The members of a committed proof, as the store's proof gives them
/// (without the members the case does not commit: `entry` and `checkpoint`).
fn committed_form(proof: &Value) -> String {
    canon::canonicalize(&proof.without("entry").without("checkpoint"))
}

/// The proofs of case 003, `case.json` `proofs`, reproduced byte for byte
/// from the replayed log and from the export's lines.
fn case_003_proofs_byte_for_byte<B: Backend>() {
    let case = vectors().join("store/exports/003-attestations-two-checkpoints");
    let committed = load(&case.join("case.json"));
    let proofs = committed.get("proofs").expect("case 003 commits proofs");
    let replayed = replay::<B>(&case);
    let export_log = ExportLog::read(&case);
    let logs: [(&str, &dyn LogRead); 2] = [
        ("replayed", replayed.backend.log()),
        ("export", &export_log),
    ];
    for (what, log) in logs {
        let checkpoints = log.checkpoints().unwrap();
        assert_eq!(checkpoints.len(), 2, "{what}");
        let latest = &checkpoints[1];

        let inclusion_list = list(proofs, "inclusion");
        assert_eq!(inclusion_list.len() as u64, latest.size, "{what}");
        for (seq, want) in (1..).zip(inclusion_list) {
            assert_eq!(number(want, "seq"), seq, "{what}: in seq order");
            assert_eq!(number(want, "size"), latest.size, "{what}");
            let p = inclusion(log, seq, latest.size).unwrap();
            assert_eq!(
                committed_form(&p.to_value()),
                canon::canonicalize(want),
                "{what}: inclusion of seq {seq}"
            );
            assert_eq!(p.checkpoint.as_deref(), Some(latest.id.as_str()), "{what}");
            let value = p.to_value();
            assert_eq!(
                canon::canonicalize(value.get("entry").unwrap()),
                p.entry.canonical(),
                "{what}"
            );
        }

        let consistency_list = list(proofs, "consistency");
        assert_eq!(consistency_list.len(), 1, "{what}");
        let want = &consistency_list[0];
        assert_eq!(
            (number(want, "old_size"), number(want, "new_size")),
            (checkpoints[0].size, latest.size),
            "{what}"
        );
        let p = consistency(log, checkpoints[0].size, latest.size).unwrap();
        assert_eq!(
            canon::canonicalize(&p.to_value()),
            canon::canonicalize(want),
            "{what}: consistency"
        );
    }
}

/// A replay leaves the projections the lines give: the heads fold and the
/// checkpoints of the export's lines.
fn projections_match_the_export<B: Backend>() {
    for case in valid_cases() {
        let name = name_of(&case);
        let replayed = replay::<B>(&case);
        let log = replayed.backend.log();
        let export_log = ExportLog::read(&case);
        assert_eq!(log.log_id(), export_log.log_id(), "{name}");
        assert_eq!(log.genesis(), export_log.genesis(), "{name}");
        assert_eq!(log.head().unwrap(), export_log.head().unwrap(), "{name}");
        assert_eq!(
            log.head_rows().unwrap(),
            export_log.head_rows().unwrap(),
            "{name}"
        );
        assert_eq!(
            log.checkpoints().unwrap(),
            export_log.checkpoints().unwrap(),
            "{name}"
        );
        let lines = log.lines(1, ALL).unwrap();
        let theirs = export_log.lines(1, ALL).unwrap();
        assert_eq!(lines, theirs, "{name}");
        // The epochs of the lines: 1, and one more after each reopen.
        let epochs: BTreeMap<u64, u64> = lines
            .iter()
            .map(|(seq, l)| (*seq, parse_line(l.as_bytes()).unwrap().entry.epoch))
            .collect();
        assert!(
            epochs
                .values()
                .zip(epochs.values().skip(1))
                .all(|(a, b)| a <= b),
            "{name}"
        );
        assert_eq!(epochs.get(&1), Some(&1), "{name}");
    }
}

macro_rules! writer_vectors {
    ($backend:ty) => {
        #[test]
        fn every_valid_case_replays_to_its_committed_export() {
            crate::replays_to_the_committed_export::<$backend>();
        }

        #[test]
        fn proofs_of_every_valid_case_verify() {
            crate::proofs_of_every_valid_case::<$backend>();
        }

        #[test]
        fn case_003_proofs_are_reproduced_byte_for_byte() {
            crate::case_003_proofs_byte_for_byte::<$backend>();
        }

        #[test]
        fn projections_match_the_export_lines() {
            crate::projections_match_the_export::<$backend>();
        }
    };
}

mod mem {
    writer_vectors!(crate::Mem);
}

mod sqlite {
    writer_vectors!(crate::Sqlite);
}
