//! Verification of a store's lines: one algorithm for a live store and for an
//! export. See `spec/store-0.md`, "Verification".
//!
//! [`Verifier`] reads lines one at a time in reading order and collects every
//! finding; nothing is repaired and nothing stops it. A live store pushes its
//! lines in seq order with [`Verifier::push_line`]; an export pushes the
//! bytes of each segment file with [`Verifier::push_segment`], which splits
//! them into lines. [`verify_export`] does the whole export, manifest checks
//! included.
//!
//! For each line, in order:
//!
//! - line checks a, b, b2 and c (`S_LINE_INVALID`, `S_ENTRY_INVALID`,
//!   `S_WRONG_LOG`). A line that fails one is unusable and is read no further;
//! - sequence and clock checks d to g (`S_SEQ_GAP`, `S_CHAIN_BREAK`,
//!   `S_RECEIVED_REGRESSION`, `S_EPOCH_REGRESSION`), each of which can add a
//!   finding while reading continues;
//! - body checks h to k, which stop at the first finding.
//!
//! At the end of the log, step 6 findings left open are decided, checkpoints
//! without a valid witness are reported, and a log with no usable line at
//! seq 1 or at seq 2 gets one `S_GENESIS_INVALID`.
//!
//! Findings sort by seq (none first), then by code, then by the order of the
//! lines they are about, and the report keeps the first `max_findings`. Every
//! implementation therefore gives the same report, byte for byte.
//!
//! Nothing here panics on any input, and nothing reads a file, a clock or
//! anything else outside its arguments.

use std::collections::{HashMap, HashSet};

use evidence_core::attest;
use evidence_core::canon;
use evidence_core::json::Value;
use evidence_core::object::{self, Checked as Object};
use evidence_core::time::{self, Timestamp};
use evidence_core::Code;
use sha2::{Digest as _, Sha256};

use crate::checkpoint::{Checkpoint, CHECKPOINT_TYPE};
use crate::entry::Entry;
use crate::genesis::{Genesis, STORE_SYSTEM};
use crate::lane::{lane_of, subject_of, Heads};
use crate::line::{parse_line, Body, Line};
use crate::manifest::Manifest;
use crate::merkle::{self, Digest};
use crate::report::{Checked, Finding, FindingCode, Source, StoreReport};

/// The default limit on findings in a report.
pub const DEFAULT_MAX_FINDINGS: usize = 1000;

/// An object dated more than this many seconds after its entry's
/// `received_at` is counted in `counts.at_after_received`.
pub const LATE_CLAIM_SECS: i64 = 300;

/// How to verify.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VerifyOptions {
    /// Keep at most this many findings, the first in sort order, and set
    /// `truncated` when there are more. At least 1; 0 is taken as 1.
    pub max_findings: usize,
    /// `source` of the report. [`verify_export`] always reports `export`.
    pub source: Source,
}

impl Default for VerifyOptions {
    fn default() -> Self {
        VerifyOptions {
            max_findings: DEFAULT_MAX_FINDINGS,
            source: Source::Export,
        }
    }
}

/// The line just read, for the chain check (step e).
#[derive(Clone, Copy)]
enum Previous {
    /// No line has been read.
    Start,
    /// The previous line is unusable: the chain check is skipped.
    Unusable,
    /// The previous line is usable, with this entry hash.
    Usable(Digest),
}

/// What step 6 found for an object.
enum Step6 {
    /// Every `prev` is held on an earlier line and not dated after the object.
    Clear,
    /// The first `prev` that stops step 6 is held on an earlier line but
    /// dated after the object: `E_PREV_ORDER`.
    Order,
    /// The first `prev` that stops step 6 is not held on an earlier line.
    /// The end of the log decides between `S_PREV_POSITION` and
    /// `E_PREV_MISSING`.
    Open(String),
}

/// A step 6 finding left open until the end of the log.
struct Pending {
    line: u64,
    seq: u64,
    reference: String,
    prev: String,
}

/// A checkpoint under item 2 of step j, whatever items 3 and 4 found.
struct Recognised {
    line: u64,
    seq: u64,
    reference: String,
    id: String,
    /// Its `size` when it has no body finding; None otherwise.
    covers: Option<u64>,
}

/// A finding and where it sorts among findings with the same seq and code:
/// the line it is about (in reading order), or, for a finding added from
/// outside, after every line in the order added.
struct Ranked {
    finding: Finding,
    order: (u64, u64),
}

impl Ranked {
    fn key(&self) -> (Option<u64>, FindingCode, (u64, u64)) {
        (self.finding.seq, self.finding.code, self.order)
    }
}

/// Verifies a store's lines one at a time. See the [module docs](self).
///
/// A live store pushes its lines with [`push_line`](Self::push_line), adds
/// its own findings (`S_PROJECTION_DRIFT`, `S_TRIGGERS_MISSING`,
/// `S_META_MISMATCH`) with [`add_finding`](Self::add_finding), comparing its
/// tables with [`heads`](Self::heads), [`checkpoints`](Self::checkpoints) and
/// [`log_id`](Self::log_id), and calls [`finish`](Self::finish).
///
/// Memory grows with the log: a few hashes per line.
pub struct Verifier {
    max_findings: usize,
    source: Source,
    /// Lines read so far, usable or not: the reading position of the next one.
    lines: u64,
    /// Findings added from outside so far.
    added: u64,
    /// The log id of the lines: `entry.log` of the first line that passes
    /// line checks a, b and b2.
    log: Option<String>,
    expected_seq: u64,
    previous: Previous,
    /// `received_at` and epoch of the last usable line.
    last_usable: Option<(Timestamp, u64)>,
    /// Seq and entry hash of the last usable line.
    head: Option<(u64, Digest)>,
    /// The tree over the canonical entries of the usable lines.
    tree: Tree,
    /// The entry hashes of the usable lines.
    entry_hashes: Vec<Digest>,
    /// Every `entry.ref` that a usable line carried.
    refs: HashSet<Digest>,
    /// Held objects: id → `at`. The first holder wins.
    held: HashMap<Digest, Timestamp>,
    heads: Heads,
    /// The genesis and its `at`, once found.
    genesis: Option<(Genesis, Timestamp)>,
    /// Whether a usable line has seq 1, and seq 2.
    seq_1: bool,
    seq_2: bool,
    checked: Checked,
    at_after_received: u64,
    pending: Vec<Pending>,
    recognised: Vec<Recognised>,
    /// Ids that a valid witness names.
    witnessed: HashSet<Digest>,
    /// Checkpoints without `S_CHECKPOINT_INVALID`: (seq, id, content).
    checkpoints: Vec<(u64, String, Checkpoint)>,
    findings: Vec<Ranked>,
    /// Findings found so far, kept or not.
    found: u64,
}

impl Verifier {
    pub fn new(opts: VerifyOptions) -> Verifier {
        Verifier {
            max_findings: opts.max_findings.max(1),
            source: opts.source,
            lines: 0,
            added: 0,
            log: None,
            expected_seq: 1,
            previous: Previous::Start,
            last_usable: None,
            head: None,
            tree: Tree::default(),
            entry_hashes: Vec::new(),
            refs: HashSet::new(),
            held: HashMap::new(),
            heads: Heads::default(),
            genesis: None,
            seq_1: false,
            seq_2: false,
            checked: Checked::default(),
            at_after_received: 0,
            pending: Vec::new(),
            recognised: Vec::new(),
            witnessed: HashSet::new(),
            checkpoints: Vec::new(),
            findings: Vec::new(),
            found: 0,
        }
    }

    /// Read one line, given without its LF.
    pub fn push_line(&mut self, bytes: &[u8]) {
        let n = self.next_line();
        match parse_line(bytes) {
            Ok(line) => self.read(n, line),
            Err(code) => self.unusable(n, Finding::new(code)),
        }
    }

    /// Read the lines of one segment file, given as its bytes. Files are
    /// pushed in manifest order, so the lines of all of them form one
    /// sequence.
    ///
    /// The file is split at every LF byte, before any decoding. A line is the
    /// bytes before an LF, without it; an empty line is a line of zero bytes,
    /// and CR has no special meaning. If the file does not end in LF, the
    /// bytes after its last LF form one more line, which is `S_LINE_INVALID`
    /// whatever it holds. An empty file has no lines.
    pub fn push_segment(&mut self, bytes: &[u8]) {
        let mut rest = bytes;
        while let Some(end) = rest.iter().position(|&b| b == b'\n') {
            self.push_line(&rest[..end]);
            rest = &rest[end + 1..];
        }
        if !rest.is_empty() {
            let n = self.next_line();
            self.unusable(n, Finding::new(FindingCode::LineInvalid));
        }
    }

    /// Add a finding that is not about a line: a manifest finding, or a
    /// live-store finding. Findings added with the same seq and code keep the
    /// order they were added in.
    pub fn add_finding(&mut self, f: Finding) {
        let order = (u64::MAX, self.added);
        self.added += 1;
        self.record(order, f);
    }

    /// The heads fold over the objects held so far, in reading order.
    pub fn heads(&self) -> &Heads {
        &self.heads
    }

    /// The checkpoints read so far that have no `S_CHECKPOINT_INVALID`, in
    /// reading order: (seq, id, content). The last one is the latest
    /// checkpoint.
    pub fn checkpoints(&self) -> &[(u64, String, Checkpoint)] {
        &self.checkpoints
    }

    /// The log id of the lines read so far, if a line has passed line checks
    /// a, b and b2.
    pub fn log_id(&self) -> Option<&str> {
        self.log.as_deref()
    }

    /// The seq and entry hash of the last usable line read so far.
    pub fn head(&self) -> Option<(u64, String)> {
        self.head.map(|(seq, hash)| (seq, merkle::to_text(&hash)))
    }

    /// Run the end-of-log checks and give the report.
    pub fn finish(mut self) -> StoreReport {
        // Step 6 findings left open.
        for p in std::mem::take(&mut self.pending) {
            let finding = if self.is_held(&p.prev) {
                Finding::at(FindingCode::PrevPosition, p.seq, &p.reference)
            } else {
                Finding::at(FindingCode::ObjectInvalid, p.seq, &p.reference)
                    .with_core(Code::PrevMissing)
            };
            self.record((p.line, 0), finding);
        }

        // Checkpoints without a valid witness, and the covered checkpoint.
        let mut covered: Option<(u64, String)> = None;
        for c in std::mem::take(&mut self.recognised) {
            let witnessed = merkle::from_text(&c.id).is_some_and(|id| self.witnessed.contains(&id));
            if !witnessed {
                let finding = Finding::at(FindingCode::CheckpointUnsigned, c.seq, &c.reference);
                self.record((c.line, 0), finding);
                continue;
            }
            if let Some(size) = c.covers {
                if covered.as_ref().is_none_or(|(largest, _)| size > *largest) {
                    covered = Some((size, c.id));
                }
            }
        }

        if !(self.seq_1 && self.seq_2) {
            let n = self.lines;
            self.record((n, 0), Finding::new(FindingCode::GenesisInvalid));
        }

        self.prune();
        let max = u64::try_from(self.max_findings).unwrap_or(u64::MAX);
        let c = self.checked;
        StoreReport {
            source: self.source,
            log: self.log,
            head: self.head.map(|(seq, hash)| (seq, merkle::to_text(&hash))),
            covered,
            checked: Checked {
                objects: c
                    .entries
                    .saturating_sub(c.attestations)
                    .saturating_sub(c.checkpoints),
                ..c
            },
            forks: self.heads.record_forks(),
            at_after_received: self.at_after_received,
            findings: self.findings.into_iter().map(|r| r.finding).collect(),
            truncated: self.found > max,
        }
    }

    fn next_line(&mut self) -> u64 {
        let n = self.lines;
        self.lines += 1;
        n
    }

    fn record(&mut self, order: (u64, u64), finding: Finding) {
        self.found += 1;
        self.findings.push(Ranked { finding, order });
        // Only the max_findings smallest can be reported, and a finding found
        // later never makes one already kept any larger, so the rest can go.
        if self.findings.len() >= self.max_findings.saturating_mul(2).max(64) {
            self.prune();
        }
    }

    /// Sort the findings and keep the first `max_findings`.
    fn prune(&mut self) {
        self.findings.sort_by_key(Ranked::key);
        self.findings.truncate(self.max_findings);
    }

    fn unusable(&mut self, n: u64, finding: Finding) {
        self.record((n, 0), finding);
        self.previous = Previous::Unusable;
    }

    fn is_held(&self, id: &str) -> bool {
        merkle::from_text(id).is_some_and(|id| self.held.contains_key(&id))
    }

    /// Every check of one line that passed steps a, b and b2.
    fn read(&mut self, n: u64, line: Line) {
        let Line {
            entry,
            entry_canonical,
            body,
        } = line;
        let at = |code| Finding::at(code, entry.seq, &entry.reference);
        // `Entry::from_value` accepts only a real instant, so this holds.
        let Ok(received) = time::parse(&entry.received_at) else {
            self.unusable(n, Finding::new(FindingCode::EntryInvalid));
            return;
        };

        // Step c. The first line that gets this far names the log.
        let log = self.log.get_or_insert_with(|| entry.log.clone());
        if *log != entry.log {
            self.unusable(n, at(FindingCode::WrongLog));
            return;
        }

        // Step d. Unusable lines do not move the expected seq.
        if entry.seq != self.expected_seq {
            self.record((n, 0), at(FindingCode::SeqGap));
        }
        self.expected_seq = entry.seq.saturating_add(1);

        // Step e.
        let entry_hash: Digest = Sha256::digest(entry_canonical.as_bytes()).into();
        let chained = if entry.seq == 1 {
            Some(self.log.as_deref() == Some(entry.prev.as_str()))
        } else if let Previous::Usable(previous) = self.previous {
            Some(merkle::from_text(&entry.prev) == Some(previous))
        } else {
            None
        };
        if chained == Some(false) {
            self.record((n, 0), at(FindingCode::ChainBreak));
        }

        // Steps f and g, against the previous usable line.
        if let Some((last_received, last_epoch)) = self.last_usable {
            if received < last_received {
                self.record((n, 0), at(FindingCode::ReceivedRegression));
            }
            if entry.epoch < last_epoch {
                self.record((n, 0), at(FindingCode::EpochRegression));
            }
        }

        // The line is usable.
        self.last_usable = Some((received, entry.epoch));
        self.previous = Previous::Usable(entry_hash);
        self.head = Some((entry.seq, entry_hash));
        self.checked.entries += 1;
        self.seq_1 |= entry.seq == 1;
        self.seq_2 |= entry.seq == 2;

        // Body checks, steps h to k. A checkpoint is checked against the
        // usable lines before this one, so this line joins the tree after.
        let finding = match body {
            Body::Object(value) => self.object(n, &entry, received, value),
            Body::Attestation(value) => {
                self.checked.attestations += 1;
                self.attestation(&entry, value)
            }
        };
        if let Some(f) = finding {
            self.record((n, 0), f);
        }
        self.tree
            .push(merkle::leaf_hash(entry_canonical.as_bytes()));
        self.entry_hashes.push(entry_hash);
    }

    /// Step h's duplicate check, and its verdict: `matches` says whether
    /// `entry.ref` is the item's id or hash.
    fn reference(&mut self, reference: &str, matches: bool) -> Option<FindingCode> {
        let seen = merkle::from_text(reference).is_some_and(|r| !self.refs.insert(r));
        if !matches {
            Some(FindingCode::RefMismatch)
        } else if seen {
            Some(FindingCode::DuplicateRef)
        } else {
            None
        }
    }

    /// Steps h, i and j for an object line. Returns the body finding, if one
    /// is known now.
    fn object(
        &mut self,
        n: u64,
        entry: &Entry,
        received: Timestamp,
        value: Value,
    ) -> Option<Finding> {
        let at = |code| Finding::at(code, entry.seq, &entry.reference);

        // Step h. An object without a string id does not match.
        let matches = value.get("id").and_then(Value::as_str) == Some(entry.reference.as_str());
        if let Some(code) = self.reference(&entry.reference, matches) {
            return Some(at(code));
        }

        // Step i: evidence/0 steps 1-5. The line's value was parsed under
        // the same strict rules, so step 1 has passed.
        let object = match object::check_value(value) {
            Ok(o) => o,
            Err(e) => return Some(at(FindingCode::ObjectInvalid).with_core(e.code)),
        };

        // Step 6 looks only at earlier lines, then the object is held,
        // whatever step 6 or step j finds.
        let step_6 = self.step_6(&object);
        self.hold(&object, received);
        match step_6 {
            Step6::Clear => {}
            Step6::Order => {
                return Some(at(FindingCode::ObjectInvalid).with_core(Code::PrevOrder));
            }
            Step6::Open(prev) => {
                self.pending.push(Pending {
                    line: n,
                    seq: entry.seq,
                    reference: entry.reference.clone(),
                    prev,
                });
                return None;
            }
        }

        // Step j.
        self.store_subject(n, entry, &object).map(at)
    }

    /// evidence/0 step 6 within the log: the `prev` ids in list order, up to
    /// the first that is not held on an earlier line or is dated after the
    /// object.
    fn step_6(&self, object: &Object) -> Step6 {
        for p in &object.prev {
            match merkle::from_text(p).and_then(|id| self.held.get(&id)) {
                Some(prev_at) if *prev_at > object.at => return Step6::Order,
                Some(_) => {}
                None => return Step6::Open(p.clone()),
            }
        }
        Step6::Clear
    }

    /// The object is held from this line on: it enters the id map (the
    /// first holder wins) and the heads fold, and is counted if it is a late
    /// claim.
    fn hold(&mut self, object: &Object, received: Timestamp) {
        if let Some(id) = merkle::from_text(&object.id) {
            self.held.entry(id).or_insert(object.at);
        }
        self.heads.apply(
            &object.id,
            &subject_of(&object.value),
            &lane_of(&object.value),
            &object.prev,
        );
        let late = Timestamp {
            secs: received.secs.saturating_add(LATE_CLAIM_SECS),
            nanos: received.nanos,
        };
        if object.at > late {
            self.at_after_received += 1;
        }
    }

    /// Step j: the genesis, and objects with a store subject.
    fn store_subject(&mut self, n: u64, entry: &Entry, object: &Object) -> Option<FindingCode> {
        // Item 1.
        if entry.seq == 1 {
            let is_log_id = self.log.as_deref() == Some(object.id.as_str());
            return match Genesis::from_object(object) {
                Ok(genesis) if is_log_id => {
                    if self.genesis.is_none() {
                        self.genesis = Some((genesis, object.at));
                    }
                    None
                }
                _ => Some(FindingCode::GenesisInvalid),
            };
        }
        if entry.seq == 2 {
            return Some(FindingCode::GenesisInvalid);
        }
        let field = |outer: &str, inner: &str| {
            object
                .value
                .get(outer)
                .and_then(|v| v.get(inner))
                .and_then(Value::as_str)
        };
        if field("subject", "system") != Some(STORE_SYSTEM) {
            return None;
        }

        // Item 2.
        let is_checkpoint = match &self.genesis {
            Some((genesis, _)) => {
                field("subject", "type") == Some(CHECKPOINT_TYPE)
                    && field("actor", "id") == Some(genesis.signer.as_str())
            }
            None => false,
        };
        if !is_checkpoint {
            return Some(FindingCode::ReservedSubject);
        }
        self.checked.checkpoints += 1;

        // Items 3 and 4.
        let result = self.checkpoint(entry, object);
        self.recognised.push(Recognised {
            line: n,
            seq: entry.seq,
            reference: entry.reference.clone(),
            id: object.id.clone(),
            covers: result.ok(),
        });
        result.err()
    }

    /// Items 3 and 4 of step j for a checkpoint. Its size if both pass.
    fn checkpoint(&mut self, entry: &Entry, object: &Object) -> Result<u64, FindingCode> {
        let checkpoint = match Checkpoint::from_object(object) {
            Ok(c) if self.log.as_deref() == Some(c.log.as_str()) => c,
            _ => return Err(FindingCode::CheckpointInvalid),
        };
        let size = checkpoint.size;
        let agrees = self.agrees(entry.seq, &checkpoint);
        self.checkpoints
            .push((entry.seq, object.id.clone(), checkpoint));
        if agrees {
            Ok(size)
        } else {
            Err(FindingCode::CheckpointMismatch)
        }
    }

    /// Item 4: `size` is below the checkpoint's seq, at least `size` usable
    /// lines come before it, and the `size`-th has entry hash `head` and the
    /// first `size` have tree hash `root`.
    fn agrees(&self, seq: u64, checkpoint: &Checkpoint) -> bool {
        let Ok(size) = usize::try_from(checkpoint.size) else {
            return false;
        };
        let head = size
            .checked_sub(1)
            .and_then(|i| self.entry_hashes.get(i))
            .copied();
        // None when fewer than `size` usable lines come before it.
        let root = self.tree.root(size);
        checkpoint.size < seq
            && head.is_some()
            && head == merkle::from_text(&checkpoint.head)
            && root.is_some()
            && root == merkle::from_text(&checkpoint.root)
    }

    /// Steps h and k for an attestation line. Returns the body finding.
    fn attestation(&mut self, entry: &Entry, value: Value) -> Option<Finding> {
        let at = |code| Finding::at(code, entry.seq, &entry.reference);
        // The line's attestation was canonical, so these are its own bytes.
        let text = canon::canonicalize(&value);

        // Step h.
        let matches = canon::sha256_hex(text.as_bytes()) == entry.reference;
        if let Some(code) = self.reference(&entry.reference, matches) {
            return Some(at(code));
        }

        // Step k, items 1 to 3.
        let Ok(a) = attest::parse(text.as_bytes()) else {
            return Some(at(FindingCode::AttestationInvalid).with_core(Code::AttestationShape));
        };
        if a.verify_signature().is_err() {
            return Some(at(FindingCode::AttestationInvalid).with_core(Code::SignatureInvalid));
        }
        if !self.is_held(&a.object) {
            return Some(at(FindingCode::AttestationTarget));
        }

        // Item 4: seq 1 is the genesis, and seq 2 its author attestation.
        let key = a.value.get("key").and_then(Value::as_str);
        let by_store_key = |genesis: &Genesis| key == Some(genesis.key.as_str());
        match entry.seq {
            1 => return Some(at(FindingCode::GenesisInvalid)),
            2 => {
                let author = match &self.genesis {
                    Some((genesis, genesis_at)) => {
                        self.log.as_deref() == Some(a.object.as_str())
                            && a.purpose == "author"
                            && a.signer == genesis.signer
                            && by_store_key(genesis)
                            && a.at == *genesis_at
                    }
                    None => false,
                };
                if !author {
                    return Some(at(FindingCode::GenesisInvalid));
                }
            }
            _ => {}
        }

        // No body finding: a valid witness if it is one.
        if a.purpose == "witness" && self.genesis.as_ref().is_some_and(|(g, _)| by_store_key(g)) {
            if let Some(id) = merkle::from_text(&a.object) {
                self.witnessed.insert(id);
            }
        }
        None
    }
}

/// Verify an export: the bytes of `manifest.json` and of the segment files,
/// in the order the manifest lists them. The report's `source` is `export`,
/// whatever `opts.source` says.
///
/// A host passes one byte string for each file the manifest lists, using
/// empty bytes for a file it cannot read, so that file gets `S_SEGMENT_HASH`.
/// A host that cannot read a list from `manifest.json` passes the export's
/// segment files in file-name order.
///
/// - `S_MANIFEST_INVALID` if `manifest.json` is not a valid segments/0
///   manifest, and the manifest is then not used further;
/// - otherwise `S_SEGMENT_HASH` for each given file that does not hash to its
///   listed hash;
/// - the lines of every given file, whatever the manifest checks found;
/// - then, once, `S_MANIFEST_MISMATCH` if the number of files given is not
///   the number listed, or `log`, `size`, `head` or `checkpoint` disagree
///   with the lines.
pub fn verify_export(manifest: &[u8], segments: &[&[u8]], opts: VerifyOptions) -> StoreReport {
    let mut v = Verifier::new(VerifyOptions {
        source: Source::Export,
        ..opts
    });
    let manifest = Manifest::parse(manifest);
    match &manifest {
        Err(code) => v.add_finding(Finding::new(*code)),
        Ok(m) => {
            for (listed, bytes) in m.segments.iter().zip(segments) {
                if canon::sha256_hex(bytes) != listed.hash {
                    v.add_finding(Finding::segment_hash(&listed.file));
                }
            }
        }
    }
    for bytes in segments {
        v.push_segment(bytes);
    }
    if let Ok(m) = &manifest {
        if disagrees(m, segments.len(), &v) {
            v.add_finding(Finding::new(FindingCode::ManifestMismatch));
        }
    }
    v.finish()
}

/// The conditions of `S_MANIFEST_MISMATCH`, after the lines.
fn disagrees(m: &Manifest, given: usize, v: &Verifier) -> bool {
    let latest = v
        .checkpoints()
        .last()
        .map(|(_, id, checkpoint)| (id.as_str(), checkpoint.size));
    let listed = m.checkpoint.as_ref().map(|(id, size)| (id.as_str(), *size));
    given != m.segments.len()
        || v.log_id() != Some(m.log.as_str())
        || v.head() != Some((m.size, m.head.clone()))
        || latest != listed
}

/// SHA-256(0x01 ‖ left ‖ right): an RFC 9162 node hash, as in [`merkle`].
fn node_hash(left: &Digest, right: &Digest) -> Digest {
    let mut h = Sha256::new();
    h.update([0x01]);
    h.update(left);
    h.update(right);
    h.finalize().into()
}

/// The Merkle tree over the leaves pushed so far, kept so that the tree hash
/// of any prefix takes O(log n): `levels[k][i]` is the hash of the complete
/// subtree over leaves i × 2^k to (i + 1) × 2^k − 1. A log with many
/// checkpoints is then checked in O(n log n), not O(n²).
#[derive(Default)]
struct Tree {
    levels: Vec<Vec<Digest>>,
}

impl Tree {
    fn push(&mut self, leaf: Digest) {
        let mut node = leaf;
        let mut level = 0;
        loop {
            if self.levels.len() == level {
                self.levels.push(Vec::new());
            }
            let row = &mut self.levels[level];
            row.push(node);
            let len = row.len();
            if len % 2 == 1 {
                return;
            }
            node = node_hash(&row[len - 2], &row[len - 1]);
            level += 1;
        }
    }

    /// The RFC 9162 tree hash of the first `size` leaves, or None if `size`
    /// is zero or more than the leaves pushed.
    ///
    /// The tree of `size` leaves splits at the largest power of two below
    /// `size`, so it is the complete subtrees given by the binary digits of
    /// `size`, largest first, hashed together from the right.
    fn root(&self, size: usize) -> Option<Digest> {
        let leaves = self.levels.first().map_or(0, Vec::len);
        if size == 0 || size > leaves {
            return None;
        }
        let mut subtrees = Vec::new();
        let mut offset = 0usize;
        for level in (0..self.levels.len()).rev() {
            let width = 1usize.checked_shl(u32::try_from(level).ok()?)?;
            if size & width != 0 {
                subtrees.push(*self.levels[level].get(offset >> level)?);
                offset += width;
            }
        }
        let mut from_right = subtrees.into_iter().rev();
        let last = from_right.next()?;
        Some(from_right.fold(last, |right, left| node_hash(&left, &right)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::format_millis;
    use crate::entry::Kind;
    use crate::line::{attestation_ref, render_line};
    use crate::manifest::SegmentRef;
    use evidence_core::canon::canonicalize;
    use evidence_core::json;

    const SEED: [u8; 32] = [9u8; 32];
    const OTHER_SEED: [u8; 32] = [10u8; 32];

    fn h(c: char) -> String {
        format!("sha256:{}", c.to_string().repeat(64))
    }

    fn base() -> i64 {
        time::parse("2026-11-26T00:00:00Z").unwrap().secs
    }

    /// A timestamp `offset` seconds after 2026-11-26T00:00:00Z.
    fn ts(offset: i64) -> String {
        format_millis(Timestamp {
            secs: base() + offset,
            nanos: 0,
        })
    }

    fn s(t: &str) -> Value {
        Value::String(t.to_string())
    }

    fn obj(members: &[(&str, Value)]) -> Value {
        Value::Object(
            members
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
        )
    }

    /// A sealed evidence/0 object with inline content `{"note": note}`.
    fn object_of(
        op: &str,
        schema_ref: &str,
        (system, kind, key): (&str, &str, &str),
        note: &str,
        at: &str,
        prev: &[&str],
    ) -> Value {
        let inline = obj(&[("note", s(note))]);
        object::seal(&obj(&[
            ("v", s("evidence/0")),
            ("op", s(op)),
            (
                "schema",
                obj(&[("ref", s(schema_ref)), ("hash", s(&h('5')))]),
            ),
            (
                "subject",
                obj(&[("system", s(system)), ("type", s(kind)), ("key", s(key))]),
            ),
            (
                "content",
                obj(&[
                    ("media_type", s("application/json")),
                    ("hash", s(&canon::digest(&inline))),
                    ("inline", inline),
                ]),
            ),
            (
                "actor",
                obj(&[("id", s("service:test-registry")), ("kind", s("service"))]),
            ),
            ("at", s(at)),
            ("prev", Value::Array(prev.iter().map(|p| s(p)).collect())),
        ]))
    }

    /// A record change for subject `key` of a synthetic registry.
    fn thing(key: &str, op: &str, note: &str, at: &str, prev: &[&str]) -> Value {
        object_of(
            op,
            "test-registry/thing@1",
            ("test-registry", "thing", key),
            note,
            at,
            prev,
        )
    }

    /// The object with its inline content replaced and `content.hash`
    /// recomputed, but `id` left as it was.
    fn with_note(object: &Value, note: &str) -> Value {
        let inline = obj(&[("note", s(note))]);
        let content = object
            .get("content")
            .unwrap()
            .with("hash", s(&canon::digest(&inline)))
            .with("inline", inline);
        object.with("content", content)
    }

    fn id_of(v: &Value) -> String {
        v.get("id").and_then(Value::as_str).unwrap().to_string()
    }

    /// A log written one line at a time, as a writer would write it, with
    /// hooks for writing it wrongly.
    struct Log {
        genesis: Genesis,
        log_id: String,
        genesis_at: String,
        lines: Vec<String>,
        entries: Vec<String>,
        last: Option<String>,
        seq: u64,
        epoch: u64,
        clock: i64,
        checkpoint: Option<(String, u64)>,
    }

    impl Log {
        /// A log with no lines yet.
        fn bare() -> Log {
            let genesis = Genesis::new("test-store", &attest::public_key(&SEED)).unwrap();
            let genesis_at = ts(0);
            let log_id = id_of(&genesis.object(&genesis_at));
            Log {
                genesis,
                log_id,
                genesis_at,
                lines: Vec::new(),
                entries: Vec::new(),
                last: None,
                seq: 0,
                epoch: 1,
                clock: 0,
                checkpoint: None,
            }
        }

        /// A new log: the genesis and its author attestation.
        fn new() -> Log {
            let mut log = Log::bare();
            log.push_genesis();
            let author = log.author();
            log.push_attestation(&author);
            log
        }

        fn push_genesis(&mut self) {
            let genesis = self.genesis.object(&self.genesis_at);
            self.push_object(&genesis);
        }

        fn author(&self) -> Value {
            self.sign(&self.log_id, "author", &self.genesis_at)
        }

        /// The next entry, chained to the last one.
        fn entry(&mut self, kind: Kind, reference: &str) -> Entry {
            self.seq += 1;
            let entry = Entry {
                log: self.log_id.clone(),
                seq: self.seq,
                kind,
                reference: reference.to_string(),
                received_at: ts(self.clock),
                epoch: self.epoch,
                prev: self.last.clone().unwrap_or_else(|| self.log_id.clone()),
            };
            self.clock += 1;
            entry
        }

        fn push_entry(&mut self, entry: &Entry, body: &Value) {
            self.last = Some(entry.hash());
            self.entries.push(entry.canonical());
            self.lines.push(render_line(
                &entry.canonical(),
                entry.kind,
                &canonicalize(body),
            ));
        }

        fn push_object(&mut self, object: &Value) -> String {
            let id = id_of(object);
            let entry = self.entry(Kind::Object, &id);
            self.push_entry(&entry, object);
            id
        }

        fn push_attestation(&mut self, a: &Value) {
            let entry = self.entry(Kind::Attestation, &attestation_ref(a));
            self.push_entry(&entry, a);
        }

        fn sign(&self, object: &str, purpose: &str, at: &str) -> Value {
            attest::sign(object, purpose, &self.genesis.signer, at, &SEED).unwrap()
        }

        /// The content of a checkpoint over every entry so far.
        fn content(&self) -> Checkpoint {
            let leaves: Vec<Digest> = self
                .entries
                .iter()
                .map(|e| merkle::leaf_hash(e.as_bytes()))
                .collect();
            Checkpoint {
                log: self.log_id.clone(),
                size: self.seq,
                head: self.last.clone().unwrap(),
                root: merkle::to_text(&merkle::root(&leaves)),
            }
        }

        /// Append a checkpoint object built from `content`, with `prev` the
        /// last checkpoint, and its witness if `witness` gives a seed.
        fn push_checkpoint(&mut self, content: &Checkpoint, witness: Option<&[u8; 32]>) -> String {
            let at = ts(self.clock);
            let prev = self.checkpoint.as_ref().map(|(id, _)| id.clone());
            let object = content.object(&self.genesis, &at, prev.as_deref());
            let id = self.push_object(&object);
            if let Some(seed) = witness {
                let w = attest::sign(&id, "witness", &self.genesis.signer, &at, seed).unwrap();
                self.push_attestation(&w);
            }
            self.checkpoint = Some((id.clone(), content.size));
            id
        }

        /// A checkpoint over every entry so far, with its witness.
        fn checkpoint(&mut self) -> String {
            let content = self.content();
            self.push_checkpoint(&content, Some(&SEED))
        }

        /// Seq 1 genesis, 2 author, 3 record A, 4 update B (prev A),
        /// 5 approval of B, 6 checkpoint of size 5, 7 its witness.
        fn sample() -> Log {
            let mut log = Log::new();
            let a = thing("T-1", "create", "a", &ts(-100), &[]);
            let a_id = log.push_object(&a);
            let b = thing("T-1", "update", "b", &ts(-50), &[&a_id]);
            let b_id = log.push_object(&b);
            let approve =
                attest::sign(&b_id, "approve", "person:tester", &ts(-40), &OTHER_SEED).unwrap();
            log.push_attestation(&approve);
            log.checkpoint();
            log
        }

        fn segments(&self, segment_size: usize) -> Vec<Vec<u8>> {
            self.lines
                .chunks(segment_size)
                .map(|chunk| {
                    let mut bytes = Vec::new();
                    for line in chunk {
                        bytes.extend_from_slice(line.as_bytes());
                        bytes.push(b'\n');
                    }
                    bytes
                })
                .collect()
        }

        fn manifest(&self, segments: &[Vec<u8>], segment_size: u64) -> Manifest {
            Manifest {
                log: self.log_id.clone(),
                size: self.seq,
                head: self.last.clone().unwrap(),
                segment_size,
                segments: segments
                    .iter()
                    .enumerate()
                    .map(|(i, bytes)| {
                        let (first, last) =
                            Manifest::segment_range(i as u64 + 1, self.seq, segment_size).unwrap();
                        SegmentRef {
                            file: Manifest::file_name(i + 1),
                            first,
                            last,
                            hash: canon::sha256_hex(bytes),
                        }
                    })
                    .collect(),
                checkpoint: self.checkpoint.clone(),
            }
        }

        /// The export of this log: manifest bytes and segment files.
        fn export(&self, segment_size: usize) -> (Vec<u8>, Vec<Vec<u8>>) {
            let segments = self.segments(segment_size);
            let manifest = self.manifest(&segments, segment_size as u64);
            (manifest.canonical().into_bytes(), segments)
        }
    }

    /// The manifest with every segment hash recomputed from these files.
    fn rehash(manifest: &[u8], segments: &[Vec<u8>]) -> Vec<u8> {
        let mut m = Manifest::parse(manifest).unwrap();
        for (listed, bytes) in m.segments.iter_mut().zip(segments) {
            listed.hash = canon::sha256_hex(bytes);
        }
        m.canonical().into_bytes()
    }

    fn verify_lines(lines: &[String]) -> StoreReport {
        let mut v = Verifier::new(VerifyOptions {
            source: Source::Store,
            ..VerifyOptions::default()
        });
        for line in lines {
            v.push_line(line.as_bytes());
        }
        v.finish()
    }

    fn export(manifest: &[u8], segments: &[Vec<u8>], max_findings: usize) -> StoreReport {
        let segments: Vec<&[u8]> = segments.iter().map(Vec::as_slice).collect();
        let report = verify_export(
            manifest,
            &segments,
            VerifyOptions {
                max_findings,
                ..VerifyOptions::default()
            },
        );
        assert_consistent(&report);
        report
    }

    /// Each finding as `CODE`, `CODE@seq`, `CODE@seq:CORE` or `CODE:file`,
    /// in report order.
    fn codes(report: &StoreReport) -> Vec<String> {
        report
            .findings
            .iter()
            .map(|f| {
                let mut text = f.code.as_str().to_string();
                if let Some(seq) = f.seq {
                    text.push_str(&format!("@{seq}"));
                }
                if let Some(core) = f.core {
                    text.push_str(&format!(":{core}"));
                }
                if let Some(file) = &f.file {
                    text.push_str(&format!(":{file}"));
                }
                text
            })
            .collect()
    }

    /// The rules of the report schema that relate members to each other,
    /// and the sort order of findings.
    fn assert_consistent(report: &StoreReport) {
        use FindingCode::*;
        assert_eq!(report.ok(), report.findings.is_empty());
        assert_eq!(report.log.is_some(), report.head.is_some());
        if report.covered.is_some() {
            assert!(report.head.is_some());
        }
        let c = report.checked;
        assert_eq!(c.entries, c.objects + c.attestations + c.checkpoints);
        for pair in report.findings.windows(2) {
            assert!(
                (pair[0].seq, pair[0].code) <= (pair[1].seq, pair[1].code),
                "sorted: {:?}",
                codes(report)
            );
        }
        let no_seq = [
            ManifestInvalid,
            SegmentHash,
            ManifestMismatch,
            LineInvalid,
            EntryInvalid,
            ProjectionDrift,
            TriggersMissing,
            MetaMismatch,
        ];
        for f in &report.findings {
            assert_eq!(f.seq.is_some(), f.reference.is_some(), "{f:?}");
            assert_eq!(
                f.core.is_some(),
                matches!(f.code, ObjectInvalid | AttestationInvalid),
                "{f:?}"
            );
            assert_eq!(f.file.is_some(), f.code == SegmentHash, "{f:?}");
            if no_seq.contains(&f.code) {
                assert!(f.seq.is_none(), "{f:?}");
            } else if f.code != GenesisInvalid {
                assert!(f.seq.is_some(), "{f:?}");
            } else if let Some(seq) = f.seq {
                assert!(seq == 1 || seq == 2, "{f:?}");
            }
        }
        // The report is canonical JSON that parses back under strict rules.
        let text = canonicalize(&report.to_value());
        assert_eq!(canonicalize(&json::parse(text.as_bytes()).unwrap()), text);
    }

    fn check(lines: &[String]) -> Vec<String> {
        let report = verify_lines(lines);
        assert_consistent(&report);
        codes(&report)
    }

    const NONE: [&str; 0] = [];

    #[test]
    fn a_valid_log_has_no_findings() {
        let log = Log::sample();
        let report = verify_lines(&log.lines);
        assert_consistent(&report);
        assert!(report.ok(), "{:?}", codes(&report));
        assert_eq!(report.source, Source::Store);
        assert_eq!(report.log.as_deref(), Some(log.log_id.as_str()));
        assert_eq!(report.head, Some((7, log.last.clone().unwrap())));
        let (checkpoint, size) = log.checkpoint.clone().unwrap();
        assert_eq!(size, 5);
        assert_eq!(report.covered, Some((5, checkpoint)));
        assert_eq!(
            report.checked,
            Checked {
                entries: 7,
                objects: 3,
                attestations: 3,
                checkpoints: 1
            }
        );
        assert_eq!((report.forks, report.at_after_received), (0, 0));
        assert!(!report.truncated);

        // The same log as an export, in one segment or several.
        for segment_size in [1, 2, 3, 7, 10_000] {
            let (manifest, segments) = log.export(segment_size);
            let report = export(&manifest, &segments, 1000);
            assert!(report.ok(), "{segment_size}: {:?}", codes(&report));
            assert_eq!(report.source, Source::Export);
            assert_eq!(report.head, Some((7, log.last.clone().unwrap())));
        }

        // verify_export always reports an export.
        let (manifest, segments) = log.export(10_000);
        let report = verify_export(
            &manifest,
            &[segments[0].as_slice()],
            VerifyOptions {
                max_findings: 5,
                source: Source::Store,
            },
        );
        assert_eq!(report.source, Source::Export);
        assert!(report.ok());
    }

    #[test]
    fn the_verifier_exposes_heads_checkpoints_log_and_head() {
        let mut log = Log::sample();
        let c = thing("T-2", "create", "c", &ts(-10), &[]);
        let c_id = log.push_object(&c);
        let d = thing("T-2", "create", "d", &ts(-10), &[]);
        let d_id = log.push_object(&d);
        log.checkpoint();

        let mut v = Verifier::new(VerifyOptions::default());
        assert_eq!(v.log_id(), None);
        assert_eq!(v.head(), None);
        for line in &log.lines {
            v.push_line(line.as_bytes());
        }
        assert_eq!(v.log_id(), Some(log.log_id.as_str()));
        assert_eq!(v.head(), Some((log.seq, log.last.clone().unwrap())));
        let checkpoints: Vec<(u64, u64)> = v
            .checkpoints()
            .iter()
            .map(|(seq, _, c)| (*seq, c.size))
            .collect();
        assert_eq!(checkpoints, [(6, 5), (10, 9)]);
        let latest = log.checkpoint.clone().unwrap().0;
        assert_eq!(v.checkpoints()[1].1, latest);
        let mut both = vec![c_id, d_id];
        both.sort();
        assert_eq!(v.heads().get(&subject_of(&c), crate::lane::RECORD), both);
        assert_eq!(v.heads().record_forks(), 1);
        // The genesis and the checkpoints are in the fold too.
        let rows = v.heads().rows();
        assert!(rows
            .iter()
            .any(|(_, lane, id)| lane == "assert:evidence-store/genesis" && *id == log.log_id));
        let checkpoint_heads: Vec<&String> = rows
            .iter()
            .filter(|(_, lane, _)| lane == "assert:evidence-store/checkpoint")
            .map(|(_, _, id)| id)
            .collect();
        assert_eq!(checkpoint_heads, [&latest]);
        let report = v.finish();
        assert!(report.ok(), "{:?}", codes(&report));
        assert_eq!(report.forks, 1);
        assert_eq!(report.covered, Some((9, latest)));
    }

    #[test]
    fn line_invalid_has_no_seq_and_leaves_a_hole() {
        let log = Log::sample();
        let mut lines = log.lines.clone();
        lines[3] = lines[3].replacen("{\"entry\":", "{ \"entry\":", 1);
        // Line 4 (B) is unusable: it holds no object and is no leaf. The
        // next line's seq is unexpected and its chain check is skipped, the
        // approval of B finds no target, and the checkpoint counts only four
        // usable lines before it.
        assert_eq!(
            check(&lines),
            [
                "S_LINE_INVALID",
                "S_SEQ_GAP@5",
                "S_ATTESTATION_TARGET@5",
                "S_CHECKPOINT_MISMATCH@6"
            ]
        );
        // Not JSON, an empty line and a line with a trailing CR, in place of
        // the witness.
        let crlf = format!("{}\r", log.lines[6]);
        for bad in ["not json", "", crlf.as_str()] {
            let mut lines = log.lines.clone();
            lines[6] = bad.to_string();
            let report = verify_lines(&lines);
            assert_consistent(&report);
            assert_eq!(
                codes(&report),
                ["S_LINE_INVALID", "S_CHECKPOINT_UNSIGNED@6"]
            );
            // The head is the last usable line, and nothing is covered.
            assert_eq!(report.head.as_ref().map(|h| h.0), Some(6));
            assert_eq!(report.covered, None);
            assert_eq!(report.checked.entries, 6);
        }
    }

    #[test]
    fn entry_invalid_has_no_seq() {
        let mut log = Log::new();
        let a = thing("T-1", "create", "a", &ts(-100), &[]);
        let entry = log.entry(Kind::Object, &id_of(&a));
        let bad = entry.to_value().with("seq", Value::Number(0.0));
        log.lines.push(render_line(
            &canonicalize(&bad),
            Kind::Object,
            &canonicalize(&a),
        ));
        assert_eq!(check(&log.lines), ["S_ENTRY_INVALID"]);
    }

    #[test]
    fn wrong_log_has_seq_and_ref_and_makes_the_line_unusable() {
        let mut log = Log::new();
        let a = thing("T-1", "create", "a", &ts(-100), &[]);
        let mut entry = log.entry(Kind::Object, &id_of(&a));
        entry.log = h('e');
        log.push_entry(&entry, &a);
        let b = thing("T-2", "create", "b", &ts(-100), &[]);
        log.push_object(&b);
        let report = verify_lines(&log.lines);
        assert_consistent(&report);
        // The expected seq did not move past the unusable line, and the next
        // line's chain check is skipped.
        assert_eq!(codes(&report), ["S_WRONG_LOG@3", "S_SEQ_GAP@4"]);
        assert_eq!(
            report.findings[0].reference.as_deref(),
            Some(id_of(&a).as_str())
        );
        assert_eq!(report.checked.entries, 3);

        // An attestation naming the object on the wrong-log line finds nothing.
        let mut log = Log::new();
        let mut entry = log.entry(Kind::Object, &id_of(&a));
        entry.log = h('e');
        log.push_entry(&entry, &a);
        let approve =
            attest::sign(&id_of(&a), "approve", "person:tester", &ts(-1), &OTHER_SEED).unwrap();
        log.push_attestation(&approve);
        assert_eq!(
            check(&log.lines),
            ["S_WRONG_LOG@3", "S_SEQ_GAP@4", "S_ATTESTATION_TARGET@4"]
        );
    }

    #[test]
    fn seq_gap_and_chain_break() {
        // A line removed: the next line's seq and prev are both wrong.
        let log = Log::sample();
        let mut lines = log.lines.clone();
        lines.remove(4);
        assert_eq!(
            check(&lines),
            ["S_SEQ_GAP@6", "S_CHAIN_BREAK@6", "S_CHECKPOINT_MISMATCH@6"]
        );

        // Only the chain is wrong.
        let mut log = Log::new();
        let a = thing("T-1", "create", "a", &ts(-100), &[]);
        let mut entry = log.entry(Kind::Object, &id_of(&a));
        entry.prev = h('f');
        log.push_entry(&entry, &a);
        assert_eq!(check(&log.lines), ["S_CHAIN_BREAK@3"]);

        // Seq 1 chains to the log id.
        let mut log = Log::bare();
        let genesis = log.genesis.object(&log.genesis_at);
        let mut entry = log.entry(Kind::Object, &id_of(&genesis));
        entry.prev = h('f');
        log.push_entry(&entry, &genesis);
        let author = log.author();
        log.push_attestation(&author);
        assert_eq!(check(&log.lines), ["S_CHAIN_BREAK@1"]);

        // A seq that repeats: the expected seq follows the line read.
        let mut log = Log::new();
        let a = thing("T-1", "create", "a", &ts(-100), &[]);
        log.push_object(&a);
        log.seq -= 1;
        let b = thing("T-2", "create", "b", &ts(-100), &[]);
        log.push_object(&b);
        let c = thing("T-3", "create", "c", &ts(-100), &[]);
        log.push_object(&c);
        assert_eq!(check(&log.lines), ["S_SEQ_GAP@3"]);
    }

    #[test]
    fn regressions() {
        let mut log = Log::new();
        let a = thing("T-1", "create", "a", &ts(-100), &[]);
        let mut entry = log.entry(Kind::Object, &id_of(&a));
        entry.received_at = "2026-11-25T23:59:59.999999999Z".to_string();
        log.push_entry(&entry, &a);
        log.epoch = 3;
        let b = thing("T-2", "create", "b", &ts(-100), &[]);
        log.push_object(&b);
        log.epoch = 2;
        let c = thing("T-3", "create", "c", &ts(-100), &[]);
        log.push_object(&c);
        // An equal received_at and an equal epoch are not regressions.
        log.clock -= 1;
        let d = thing("T-4", "create", "d", &ts(-100), &[]);
        log.push_object(&d);
        assert_eq!(
            check(&log.lines),
            ["S_RECEIVED_REGRESSION@3", "S_EPOCH_REGRESSION@5"]
        );

        // Compared with the previous usable line, as instants.
        let mut log = Log::new();
        let a = thing("T-1", "create", "a", &ts(-100), &[]);
        let mut entry = log.entry(Kind::Object, &id_of(&a));
        entry.received_at = "2026-11-26T00:00:01Z".to_string();
        log.push_entry(&entry, &a);
        log.lines.push("garbage".to_string());
        let b = thing("T-2", "create", "b", &ts(-100), &[]);
        let mut entry = log.entry(Kind::Object, &id_of(&b));
        entry.received_at = "2026-11-26T00:00:00.999Z".to_string();
        log.push_entry(&entry, &b);
        assert_eq!(
            check(&log.lines),
            ["S_LINE_INVALID", "S_RECEIVED_REGRESSION@4"]
        );
    }

    #[test]
    fn ref_mismatch_and_duplicate_ref() {
        let mut log = Log::new();
        let a = thing("T-1", "create", "a", &ts(-100), &[]);
        let entry = log.entry(Kind::Object, &h('a'));
        log.push_entry(&entry, &a);
        // The object on a ref-mismatch line is not held.
        let b = thing("T-1", "update", "b", &ts(-50), &[&id_of(&a)]);
        log.push_object(&b);
        let approve =
            attest::sign(&id_of(&a), "approve", "person:tester", &ts(-1), &OTHER_SEED).unwrap();
        let entry = log.entry(Kind::Attestation, &h('b'));
        log.push_entry(&entry, &approve);
        // An object without a string id never matches.
        let entry = log.entry(Kind::Object, &h('c'));
        log.push_entry(&entry, &a.without("id"));
        let entry = log.entry(Kind::Object, &h('d'));
        log.push_entry(&entry, &a.with("id", Value::Number(1.0)));
        assert_eq!(
            check(&log.lines),
            [
                "S_REF_MISMATCH@3",
                "S_OBJECT_INVALID@4:E_PREV_MISSING",
                "S_REF_MISMATCH@5",
                "S_REF_MISMATCH@6",
                "S_REF_MISMATCH@7"
            ]
        );

        // The same object and the same attestation twice.
        let mut log = Log::new();
        let a = thing("T-1", "create", "a", &ts(-100), &[]);
        log.push_object(&a);
        let approve =
            attest::sign(&id_of(&a), "approve", "person:tester", &ts(-1), &OTHER_SEED).unwrap();
        log.push_attestation(&approve);
        log.push_object(&a);
        log.push_attestation(&approve);
        let report = verify_lines(&log.lines);
        assert_consistent(&report);
        assert_eq!(codes(&report), ["S_DUPLICATE_REF@5", "S_DUPLICATE_REF@6"]);
        // The duplicate is not folded twice: one head, no fork.
        assert_eq!(report.forks, 0);
        assert_eq!(report.checked.objects, 3);

        // A ref that a ref-mismatch line carried is a duplicate later: the
        // mismatch comes first on its own line.
        let mut log = Log::new();
        let a = thing("T-1", "create", "a", &ts(-100), &[]);
        let b = thing("T-2", "create", "b", &ts(-100), &[]);
        let entry = log.entry(Kind::Object, &id_of(&b));
        log.push_entry(&entry, &a);
        log.push_object(&b);
        assert_eq!(check(&log.lines), ["S_REF_MISMATCH@3", "S_DUPLICATE_REF@4"]);
    }

    #[test]
    fn genesis_rules() {
        // A log of one line, and of none.
        let log = Log::new();
        assert_eq!(check(&log.lines[..1]), ["S_GENESIS_INVALID"]);
        assert_eq!(
            check(&log.lines[1..]),
            ["S_GENESIS_INVALID", "S_SEQ_GAP@2", "S_ATTESTATION_TARGET@2"]
        );
        let report = verify_lines(&[]);
        assert_consistent(&report);
        assert_eq!(codes(&report), ["S_GENESIS_INVALID"]);
        assert_eq!((report.log.clone(), report.head.clone()), (None, None));
        assert_eq!(report.checked, Checked::default());

        // Seq 1 that is not a genesis.
        let mut log = Log::bare();
        let a = thing("T-1", "create", "a", &ts(-100), &[]);
        log.log_id = id_of(&a);
        log.push_object(&a);
        assert_eq!(
            check(&log.lines),
            ["S_GENESIS_INVALID", "S_GENESIS_INVALID@1"]
        );

        // A valid genesis whose id is not the log id of the lines.
        let mut log = Log::bare();
        log.log_id = h('7');
        log.push_genesis();
        assert_eq!(
            check(&log.lines),
            ["S_GENESIS_INVALID", "S_GENESIS_INVALID@1"]
        );

        // A genesis that is not exactly the object built from its content.
        let mut log = Log::bare();
        let genesis = log.genesis.object(&log.genesis_at);
        let changed = object::seal(&genesis.with("ext", obj(&[("test/0", Value::Null)])));
        log.log_id = id_of(&changed);
        log.push_object(&changed);
        assert_eq!(
            check(&log.lines),
            ["S_GENESIS_INVALID", "S_GENESIS_INVALID@1"]
        );

        // An object at seq 2, then an attestation at seq 1 whose target is held.
        let mut log = Log::bare();
        log.push_genesis();
        let a = thing("T-1", "create", "a", &ts(-100), &[]);
        log.push_object(&a);
        let author = log.author();
        log.seq = 0;
        log.push_attestation(&author);
        assert_eq!(
            check(&log.lines),
            [
                "S_SEQ_GAP@1",
                "S_CHAIN_BREAK@1",
                "S_GENESIS_INVALID@1",
                "S_GENESIS_INVALID@2"
            ]
        );

        // The author attestation: one millisecond late, the wrong purpose,
        // another key, another signer, another object.
        let later = format_millis(Timestamp {
            secs: base(),
            nanos: 1_000_000,
        });
        type Make = Box<dyn Fn(&Log) -> Value>;
        let cases: Vec<Make> = vec![
            Box::new(move |l: &Log| l.sign(&l.log_id, "author", &later)),
            Box::new(|l: &Log| l.sign(&l.log_id, "approve", &l.genesis_at)),
            Box::new(|l: &Log| {
                attest::sign(
                    &l.log_id,
                    "author",
                    &l.genesis.signer,
                    &l.genesis_at,
                    &OTHER_SEED,
                )
                .unwrap()
            }),
            Box::new(|l: &Log| {
                attest::sign(&l.log_id, "author", "service:other", &l.genesis_at, &SEED).unwrap()
            }),
        ];
        for make in cases {
            let mut log = Log::bare();
            log.push_genesis();
            let author = make(&log);
            log.push_attestation(&author);
            assert_eq!(check(&log.lines), ["S_GENESIS_INVALID@2"]);
        }

        // The author's `at` is compared as an instant.
        let mut log = Log::bare();
        log.push_genesis();
        let author = log.sign(&log.log_id, "author", "2026-11-26T00:00:00Z");
        log.push_attestation(&author);
        assert_eq!(check(&log.lines), NONE);

        // A bad signature on the author attestation is an attestation finding.
        let mut log = Log::bare();
        log.push_genesis();
        let forged = log.author().with("at", s(&ts(1)));
        log.push_attestation(&forged);
        assert_eq!(
            check(&log.lines),
            ["S_ATTESTATION_INVALID@2:E_SIGNATURE_INVALID"]
        );

        // The genesis line missing: no chain break on the first line read.
        let log = Log::sample();
        let report = verify_lines(&log.lines[1..]);
        assert_consistent(&report);
        assert_eq!(
            codes(&report),
            [
                "S_GENESIS_INVALID",
                "S_SEQ_GAP@2",
                "S_ATTESTATION_TARGET@2",
                "S_RESERVED_SUBJECT@6"
            ]
        );
        assert_eq!(report.log.as_deref(), Some(log.log_id.as_str()));
        assert_eq!(report.checked.checkpoints, 0);

        // The genesis line unusable: the log id is read from the next line.
        let mut lines = log.lines.clone();
        lines[0] = lines[0].replacen('{', "{ ", 1);
        let report = verify_lines(&lines);
        assert_consistent(&report);
        assert_eq!(report.log.as_deref(), Some(log.log_id.as_str()));
        assert_eq!(
            codes(&report),
            [
                "S_LINE_INVALID",
                "S_GENESIS_INVALID",
                "S_SEQ_GAP@2",
                "S_ATTESTATION_TARGET@2",
                "S_RESERVED_SUBJECT@6"
            ]
        );
        assert_eq!(report.checked.checkpoints, 0);
        assert_eq!(report.checked.objects, 3);
    }

    #[test]
    fn reserved_subjects_and_checkpoint_recognition() {
        let mut log = Log::sample();
        // An ordinary object with a store subject.
        let note = object_of(
            "assert",
            "test/note@1",
            (STORE_SYSTEM, "note", "x"),
            "n",
            &ts(-1),
            &[],
        );
        log.push_object(&note);
        // A checkpoint-shaped object by another actor, with a store-key witness.
        let content = log.content();
        let at = ts(log.clock);
        let other = Genesis::new("other", &attest::public_key(&SEED)).unwrap();
        let other_id = log.push_object(&content.object(&other, &at, None));
        let w = log.sign(&other_id, "witness", &at);
        log.push_attestation(&w);
        // A checkpoint by the genesis signer whose log is another log, with a
        // store-key witness.
        let mut wrong_log = log.content();
        wrong_log.log = h('9');
        log.push_checkpoint(&wrong_log, Some(&SEED));
        let report = verify_lines(&log.lines);
        assert_consistent(&report);
        assert_eq!(
            codes(&report),
            [
                "S_RESERVED_SUBJECT@8",
                "S_RESERVED_SUBJECT@9",
                "S_CHECKPOINT_INVALID@11"
            ]
        );
        assert_eq!(
            report.checked,
            Checked {
                entries: 12,
                objects: 5,
                attestations: 5,
                checkpoints: 2
            }
        );
        // covered, and the latest checkpoint, are still the first one.
        assert_eq!(report.covered.as_ref().map(|c| c.0), Some(5));

        // A store subject read before the genesis was found.
        let mut log = Log::bare();
        let genesis = log.genesis.object(&log.genesis_at);
        let mut entry = log.entry(Kind::Object, &id_of(&genesis));
        entry.seq = 3;
        log.push_entry(&entry, &genesis);
        assert_eq!(
            check(&log.lines),
            ["S_GENESIS_INVALID", "S_SEQ_GAP@3", "S_RESERVED_SUBJECT@3"]
        );
    }

    #[test]
    fn checkpoint_invalid() {
        // Two prevs: not the object built from its content.
        let mut log = Log::sample();
        let first = log.checkpoint.clone().unwrap().0;
        let content = log.content();
        let at = ts(log.clock);
        let built = content.object(&log.genesis, &at, Some(&first));
        let two = object::seal(&built.with(
            "prev",
            Value::Array(vec![s(&first), s(&log.log_id.clone())]),
        ));
        let id = log.push_object(&two);
        let w = log.sign(&id, "witness", &at);
        log.push_attestation(&w);
        let report = verify_lines(&log.lines);
        assert_consistent(&report);
        assert_eq!(codes(&report), ["S_CHECKPOINT_INVALID@8"]);
        assert_eq!(report.checked.checkpoints, 2);
        assert_eq!(report.covered.as_ref().map(|c| c.0), Some(5));

        // Content that is not checkpoint/0.
        let mut log = Log::sample();
        let content = log.content();
        let at = ts(log.clock);
        let built = content.object(&log.genesis, &at, None);
        let inline = built
            .get("content")
            .unwrap()
            .get("inline")
            .unwrap()
            .with("size", Value::Number(0.0));
        let changed = built.with(
            "content",
            built
                .get("content")
                .unwrap()
                .with("hash", s(&canon::digest(&inline)))
                .with("inline", inline),
        );
        let id = log.push_object(&object::seal(&changed));
        let w = log.sign(&id, "witness", &at);
        log.push_attestation(&w);
        assert_eq!(check(&log.lines), ["S_CHECKPOINT_INVALID@8"]);
    }

    #[test]
    fn checkpoint_mismatch() {
        // A wrong root, a wrong head, a size not below its own seq, a size
        // beyond the lines before it.
        type Change = Box<dyn Fn(&mut Checkpoint)>;
        let changes: Vec<Change> = vec![
            Box::new(|c| c.root = h('1')),
            Box::new(|c| c.head = h('2')),
            Box::new(|c| c.size += 1),
            Box::new(|c| c.size += 5),
        ];
        for change in changes {
            let mut log = Log::sample();
            let mut content = log.content();
            change(&mut content);
            log.push_checkpoint(&content, Some(&SEED));
            let report = verify_lines(&log.lines);
            assert_consistent(&report);
            assert_eq!(codes(&report), ["S_CHECKPOINT_MISMATCH@8"]);
            // Recognised, but not covered.
            assert_eq!(report.checked.checkpoints, 2);
            assert_eq!(report.covered.as_ref().map(|c| c.0), Some(5));
        }

        // A smaller size is fine: a checkpoint is checked against the first
        // `size` usable lines.
        let mut log = Log::sample();
        let leaves: Vec<Digest> = log.entries[..3]
            .iter()
            .map(|e| merkle::leaf_hash(e.as_bytes()))
            .collect();
        let content = Checkpoint {
            log: log.log_id.clone(),
            size: 3,
            head: Entry::parse(log.entries[2].as_bytes()).unwrap().hash(),
            root: merkle::to_text(&merkle::root(&leaves)),
        };
        log.push_checkpoint(&content, Some(&SEED));
        let report = verify_lines(&log.lines);
        assert!(report.ok(), "{:?}", codes(&report));
        // covered is the largest size, not the latest checkpoint.
        assert_eq!(report.covered.as_ref().map(|c| c.0), Some(5));

        // Two checkpoints of the same size: the first in reading order.
        let mut log = Log::sample();
        let first = log.checkpoint.clone().unwrap().0;
        let mut content = log.content();
        content.size = 5;
        content.head = Entry::parse(log.entries[4].as_bytes()).unwrap().hash();
        let leaves: Vec<Digest> = log.entries[..5]
            .iter()
            .map(|e| merkle::leaf_hash(e.as_bytes()))
            .collect();
        content.root = merkle::to_text(&merkle::root(&leaves));
        log.push_checkpoint(&content, Some(&SEED));
        let report = verify_lines(&log.lines);
        assert!(report.ok(), "{:?}", codes(&report));
        assert_eq!(report.covered, Some((5, first)));
    }

    #[test]
    fn checkpoint_unsigned_and_witnesses() {
        // No witness.
        let mut log = Log::sample();
        let content = log.content();
        log.push_checkpoint(&content, None);
        let report = verify_lines(&log.lines);
        assert_consistent(&report);
        assert_eq!(codes(&report), ["S_CHECKPOINT_UNSIGNED@8"]);
        assert_eq!(report.covered.as_ref().map(|c| c.0), Some(5));

        // A witness by another key.
        let mut log = Log::sample();
        let content = log.content();
        let id = log.push_checkpoint(&content, None);
        let w = attest::sign(&id, "witness", &log.genesis.signer, &ts(50), &OTHER_SEED).unwrap();
        log.push_attestation(&w);
        assert_eq!(check(&log.lines), ["S_CHECKPOINT_UNSIGNED@8"]);

        // A witness whose signature does not verify.
        let mut log = Log::sample();
        let content = log.content();
        let id = log.push_checkpoint(&content, None);
        let w = log.sign(&id, "witness", &ts(50)).with("at", s(&ts(51)));
        log.push_attestation(&w);
        assert_eq!(
            check(&log.lines),
            [
                "S_CHECKPOINT_UNSIGNED@8",
                "S_ATTESTATION_INVALID@9:E_SIGNATURE_INVALID"
            ]
        );

        // An attestation by the store key that is not a witness.
        let mut log = Log::sample();
        let content = log.content();
        let id = log.push_checkpoint(&content, None);
        let w = log.sign(&id, "approve", &ts(60));
        log.push_attestation(&w);
        assert_eq!(check(&log.lines), ["S_CHECKPOINT_UNSIGNED@8"]);

        // An unsigned checkpoint with a body finding is still unsigned.
        let mut log = Log::sample();
        let mut content = log.content();
        content.root = h('1');
        log.push_checkpoint(&content, None);
        assert_eq!(
            check(&log.lines),
            ["S_CHECKPOINT_MISMATCH@8", "S_CHECKPOINT_UNSIGNED@8"]
        );
    }

    #[test]
    fn object_invalid_carries_the_core_code() {
        let mut log = Log::new();
        let a = thing("T-1", "create", "a", &ts(-100), &[]);
        // evidence/0 steps 2 to 5.
        log.push_object(&with_note(&a, "A"));
        log.push_object(&object::seal(&a.with("at", s("2026-02-30T00:00:00Z"))));
        log.push_object(&object::seal(&a.with(
            "content",
            a.get("content").unwrap().with("hash", s(&h('4'))),
        )));
        log.push_object(&object::seal(&a.with("op", s("upsert"))));
        assert_eq!(
            check(&log.lines),
            [
                "S_OBJECT_INVALID@3:E_ID_MISMATCH",
                "S_OBJECT_INVALID@4:E_TIMESTAMP",
                "S_OBJECT_INVALID@5:E_CONTENT_HASH_MISMATCH",
                "S_OBJECT_INVALID@6:E_SHAPE"
            ]
        );
    }

    #[test]
    fn step_6_in_list_order() {
        // Case 126 of the export vectors.
        let t = -1000;
        let mut log = Log::new();
        let a1 = thing("S", "create", "a1", &ts(t), &[]);
        let a1_id = log.push_object(&a1);
        let a2 = thing("S", "create", "a2", &ts(t), &[]);
        let a2_id = log.push_object(&a2);
        let (x, y, z) = (h('a'), h('b'), h('c'));
        let m = thing("S", "update", "m", &ts(t - 5), &[&x, &a1_id, &a2_id]);
        let m_id = log.push_object(&m);
        let n = thing("S", "update", "n", &ts(t - 6), &[&m_id, &z]);
        log.push_object(&n);
        let check_of = |note: &str, prev: &[&str]| {
            object_of(
                "assert",
                "test/check@1",
                ("test-registry", "thing", "S"),
                note,
                &ts(t),
                prev,
            )
        };
        let l = check_of("l", &[]);
        let p = check_of("p", &[&id_of(&l), &y]);
        log.push_object(&p);
        log.push_object(&l);
        let report = verify_lines(&log.lines);
        assert_consistent(&report);
        assert_eq!(
            codes(&report),
            [
                "S_OBJECT_INVALID@5:E_PREV_MISSING",
                "S_OBJECT_INVALID@6:E_PREV_ORDER",
                "S_PREV_POSITION@7"
            ]
        );
        // M is held and merges A1 and A2, and N replaces M.
        assert_eq!(report.forks, 0);

        // A prev dated after, behind one that is fine.
        let mut log = Log::new();
        let a = thing("T-1", "create", "a", &ts(-10), &[]);
        let a_id = log.push_object(&a);
        let b = thing("T-2", "create", "b", &ts(-30), &[]);
        let b_id = log.push_object(&b);
        let c = thing("T-2", "update", "c", &ts(-20), &[&b_id, &a_id]);
        log.push_object(&c);
        assert_eq!(check(&log.lines), ["S_OBJECT_INVALID@5:E_PREV_ORDER"]);

        // A checkpoint with an open prev is not recognised: step j is skipped.
        let mut log = Log::sample();
        let content = log.content();
        let at = ts(log.clock);
        let missing = h('d');
        let object = content.object(&log.genesis, &at, Some(&missing));
        let id = log.push_object(&object);
        let w = log.sign(&id, "witness", &at);
        log.push_attestation(&w);
        let report = verify_lines(&log.lines);
        assert_eq!(codes(&report), ["S_OBJECT_INVALID@8:E_PREV_MISSING"]);
        assert_eq!(report.checked.checkpoints, 1);
    }

    #[test]
    fn invalid_objects_are_not_held() {
        // Case 125 of the export vectors: object A fails step 4.
        let mut log = Log::new();
        let a = thing("T-1", "create", "a", &ts(-100), &[]);
        let a_id = id_of(&a);
        log.push_object(&with_note(&a, "changed"));
        let b = thing("T-1", "update", "b", &ts(-50), &[&a_id]);
        log.push_object(&b);
        let approve =
            attest::sign(&a_id, "approve", "person:tester", &ts(-1), &OTHER_SEED).unwrap();
        log.push_attestation(&approve);
        log.checkpoint();
        let want = [
            "S_OBJECT_INVALID@3:E_ID_MISMATCH",
            "S_OBJECT_INVALID@4:E_PREV_MISSING",
            "S_ATTESTATION_TARGET@5",
        ];
        assert_eq!(check(&log.lines), want);

        // Case 129: truncation keeps the first findings in sort order. The
        // E_PREV_MISSING decided at the end displaces the target finding
        // found before it.
        let (manifest, segments) = log.export(10_000);
        for max in [1, 2, 3, 4] {
            let report = export(&manifest, &segments, max);
            let kept = want.len().min(max);
            assert_eq!(codes(&report), want[..kept], "max {max}");
            assert_eq!(report.truncated, max < want.len());
            assert!(!report.ok());
        }
        // 0 is taken as 1, and no limit is too large.
        assert_eq!(export(&manifest, &segments, 0).findings.len(), 1);
        assert_eq!(export(&manifest, &segments, usize::MAX).findings.len(), 3);
    }

    #[test]
    fn a_small_limit_survives_many_findings() {
        // An open prev at seq 3, then hundreds of findings on later lines,
        // found before the open prev is decided.
        let mut log = Log::new();
        let a = thing("T-1", "update", "a", &ts(-10), &[&h('a')]);
        log.push_object(&a);
        log.seq = 49;
        let b = thing("T-2", "create", "b", &ts(-10), &[]);
        log.push_object(&b);
        let repeated = log.lines[3].clone();
        let mut lines = log.lines.clone();
        lines.extend(std::iter::repeat_n(repeated, 300));
        for max in [1, 2, 3] {
            let mut v = Verifier::new(VerifyOptions {
                max_findings: max,
                source: Source::Store,
            });
            for line in &lines {
                v.push_line(line.as_bytes());
            }
            let report = v.finish();
            assert_consistent(&report);
            let want = [
                "S_OBJECT_INVALID@3:E_PREV_MISSING",
                "S_SEQ_GAP@50",
                "S_SEQ_GAP@50",
            ];
            assert_eq!(codes(&report), want[..max]);
            assert!(report.truncated);
        }
        let report = verify_lines(&lines);
        // One gap for the first line at seq 50, then a gap, a chain break and
        // a duplicate for each repeat.
        assert_eq!(report.findings.len(), 1 + 1 + 300 * 3);
        assert!(!report.truncated);
    }

    #[test]
    fn attestation_findings() {
        let mut log = Log::new();
        let a = thing("T-1", "create", "a", &ts(-100), &[]);
        let a_id = id_of(&a);
        // Before its target.
        let approve =
            attest::sign(&a_id, "approve", "person:tester", &ts(-1), &OTHER_SEED).unwrap();
        log.push_attestation(&approve);
        log.push_object(&a);
        // Not attestation/0: no sig, an unknown purpose, not an object.
        log.push_attestation(&approve.without("sig"));
        log.push_attestation(&approve.with("purpose", s("review")));
        // A signature that does not verify.
        log.push_attestation(&approve.with("signer", s("person:someone-else")));
        log.push_attestation(&s("text"));
        // Shape and signature come before the target.
        let unheld =
            attest::sign(&h('0'), "approve", "person:tester", &ts(-1), &OTHER_SEED).unwrap();
        log.push_attestation(&unheld.with("at", s(&ts(5))));
        assert_eq!(
            check(&log.lines),
            [
                "S_ATTESTATION_TARGET@3",
                "S_ATTESTATION_INVALID@5:E_ATTESTATION_SHAPE",
                "S_ATTESTATION_INVALID@6:E_ATTESTATION_SHAPE",
                "S_ATTESTATION_INVALID@7:E_SIGNATURE_INVALID",
                "S_ATTESTATION_INVALID@8:E_ATTESTATION_SHAPE",
                "S_ATTESTATION_INVALID@9:E_SIGNATURE_INVALID"
            ]
        );
    }

    #[test]
    fn late_claims_and_forks_are_counted() {
        let mut log = Log::new();
        // Line 3 is received at ts(2): an `at` exactly 300 s later is not
        // counted, one 300.001 s after its own line's is.
        let a = thing("T-1", "create", "a", &ts(2 + 300), &[]);
        log.push_object(&a);
        let late = format_millis(Timestamp {
            secs: base() + 3 + 300,
            nanos: 1_000_000,
        });
        let b = thing("T-2", "create", "b", &late, &[]);
        log.push_object(&b);
        let c = thing("T-2", "create", "c", &ts(-5), &[]);
        log.push_object(&c);
        // An object that is not held is not counted.
        let d = thing("T-3", "create", "d", &ts(86_400), &[]).with("id", s(&h('d')));
        let entry = log.entry(Kind::Object, &h('d'));
        log.push_entry(&entry, &d);
        let report = verify_lines(&log.lines);
        assert_consistent(&report);
        assert_eq!(codes(&report), ["S_OBJECT_INVALID@6:E_ID_MISMATCH"]);
        assert_eq!(report.at_after_received, 1);
        assert_eq!(report.forks, 1);
    }

    #[test]
    fn findings_sort_by_seq_then_code_then_line() {
        let log = Log::sample();
        let mut lines = log.lines.clone();
        // A and B swapped, the approval of B read again at the end, then an
        // unusable line.
        lines.swap(2, 3);
        lines.push(lines[4].clone());
        lines.push("x".to_string());
        let mut v = Verifier::new(VerifyOptions {
            source: Source::Store,
            ..VerifyOptions::default()
        });
        v.add_finding(Finding::new(FindingCode::TriggersMissing));
        for line in &lines {
            v.push_line(line.as_bytes());
        }
        v.add_finding(Finding::new(FindingCode::ProjectionDrift));
        v.add_finding(Finding::new(FindingCode::MetaMismatch));
        let report = v.finish();
        assert_consistent(&report);
        assert_eq!(
            codes(&report),
            [
                "S_LINE_INVALID",
                "S_PROJECTION_DRIFT",
                "S_TRIGGERS_MISSING",
                "S_META_MISMATCH",
                "S_SEQ_GAP@3",
                "S_CHAIN_BREAK@3",
                "S_RECEIVED_REGRESSION@3",
                "S_SEQ_GAP@4",
                "S_CHAIN_BREAK@4",
                "S_PREV_POSITION@4",
                "S_SEQ_GAP@5",
                "S_SEQ_GAP@5",
                "S_CHAIN_BREAK@5",
                "S_CHAIN_BREAK@5",
                "S_RECEIVED_REGRESSION@5",
                "S_DUPLICATE_REF@5",
                "S_CHECKPOINT_MISMATCH@6",
            ]
        );
        // The head is the last usable line: the repeated approval.
        assert_eq!(report.head.as_ref().map(|h| h.0), Some(5));
        assert_eq!(
            report.head.as_ref().map(|h| &h.1),
            log.entries
                .get(4)
                .map(|e| { Entry::parse(e.as_bytes()).unwrap().hash() })
                .as_ref()
        );
    }

    #[test]
    fn segment_files_are_split_at_every_lf() {
        // Case 119: the final LF missing, so the witness is unusable.
        let log = Log::sample();
        let (manifest, mut segments) = log.export(10_000);
        segments[0].pop();
        let manifest = rehash(&manifest, &segments);
        let report = export(&manifest, &segments, 1000);
        assert_eq!(
            codes(&report),
            [
                "S_MANIFEST_MISMATCH",
                "S_LINE_INVALID",
                "S_CHECKPOINT_UNSIGNED@6"
            ]
        );
        assert_eq!(report.head.as_ref().map(|h| h.0), Some(6));
        assert_eq!(report.covered, None);
        assert_eq!(
            report.checked,
            Checked {
                entries: 6,
                objects: 3,
                attestations: 2,
                checkpoints: 1
            }
        );

        // Case 120: one more LF, an empty last line. The head is still the
        // one the manifest names.
        let log = Log::new();
        let (manifest, mut segments) = log.export(10_000);
        segments[0].push(b'\n');
        let manifest = rehash(&manifest, &segments);
        let report = export(&manifest, &segments, 1000);
        assert_eq!(codes(&report), ["S_LINE_INVALID"]);
        assert_eq!(report.head.as_ref().map(|h| h.0), Some(2));

        // Case 121: every line ends in CR LF.
        let (manifest, segments) = log.export(10_000);
        let crlf: Vec<u8> = segments[0]
            .iter()
            .flat_map(|&b| {
                if b == b'\n' {
                    vec![b'\r', b'\n']
                } else {
                    vec![b]
                }
            })
            .collect();
        let segments = vec![crlf];
        let manifest = rehash(&manifest, &segments);
        let report = export(&manifest, &segments, 1000);
        assert_eq!(
            codes(&report),
            [
                "S_MANIFEST_MISMATCH",
                "S_LINE_INVALID",
                "S_LINE_INVALID",
                "S_GENESIS_INVALID"
            ]
        );
        assert_eq!((report.log.clone(), report.head.clone()), (None, None));
        assert_eq!(report.checked, Checked::default());

        // A line split across two files is two unusable lines; an empty file
        // has no lines.
        let log = Log::sample();
        let mut v = Verifier::new(VerifyOptions::default());
        for line in &log.lines[..3] {
            v.push_line(line.as_bytes());
        }
        let (front, back) = log.lines[3].split_at(20);
        v.push_segment(front.as_bytes());
        v.push_segment(b"");
        v.push_segment(format!("{back}\n").as_bytes());
        for line in &log.lines[4..] {
            v.push_segment(format!("{line}\n").as_bytes());
        }
        let report = v.finish();
        assert_consistent(&report);
        assert_eq!(
            codes(&report),
            [
                "S_LINE_INVALID",
                "S_LINE_INVALID",
                "S_SEQ_GAP@5",
                "S_ATTESTATION_TARGET@5",
                "S_CHECKPOINT_MISMATCH@6"
            ]
        );
    }

    #[test]
    fn manifest_findings() {
        let log = Log::sample();
        let (manifest, segments) = log.export(3);
        assert_eq!(segments.len(), 3);
        assert!(export(&manifest, &segments, 1000).ok());

        // Object B (line 4, in the second file) changed, keeping its line
        // canonical: case 101 with the hash recomputed, 116 without. B is
        // then not held, so its approval (line 5) has no target.
        let mut changed = segments.clone();
        let text = String::from_utf8(changed[1].clone()).unwrap();
        assert_eq!(text.matches("\"note\":\"b\"").count(), 1);
        changed[1] = text
            .replace("\"note\":\"b\"", "\"note\":\"c\"")
            .into_bytes();
        let report = export(&rehash(&manifest, &changed), &changed, 1000);
        assert_eq!(
            codes(&report),
            ["S_OBJECT_INVALID@4:E_ID_MISMATCH", "S_ATTESTATION_TARGET@5"]
        );
        let report = export(&manifest, &changed, 1000);
        assert_eq!(
            codes(&report),
            [
                "S_SEGMENT_HASH:000002.jsonl",
                "S_OBJECT_INVALID@4:E_ID_MISMATCH",
                "S_ATTESTATION_TARGET@5"
            ]
        );

        // Case 127: a manifest that is not canonical is not used further.
        let mut lf = manifest.clone();
        lf.push(b'\n');
        let report = export(&lf, &changed, 1000);
        assert_eq!(
            codes(&report),
            [
                "S_MANIFEST_INVALID",
                "S_OBJECT_INVALID@4:E_ID_MISMATCH",
                "S_ATTESTATION_TARGET@5"
            ]
        );
        // Not even JSON: the lines are still verified.
        let report = export(b"", &segments, 1000);
        assert_eq!(codes(&report), ["S_MANIFEST_INVALID"]);
        assert_eq!(report.head.as_ref().map(|h| h.0), Some(7));

        // Unreadable files, passed as empty bytes, in manifest order.
        let unread = vec![Vec::new(), segments[1].clone(), Vec::new()];
        let report = export(&manifest, &unread, 1000);
        assert_eq!(
            codes(&report)[..3],
            [
                "S_SEGMENT_HASH:000001.jsonl",
                "S_SEGMENT_HASH:000003.jsonl",
                "S_MANIFEST_MISMATCH"
            ]
        );

        // Fewer files than listed, and more.
        let report = export(&manifest, &segments[..2], 1000);
        assert_eq!(
            codes(&report),
            ["S_MANIFEST_MISMATCH", "S_CHECKPOINT_UNSIGNED@6"]
        );
        let mut more = segments.clone();
        more.push(Vec::new());
        assert_eq!(
            codes(&export(&manifest, &more, 1000)),
            ["S_MANIFEST_MISMATCH"]
        );

        // log, size and head, checkpoint: each disagreeing once.
        let (manifest, segments) = log.export(10_000);
        let good = Manifest::parse(&manifest).unwrap();
        let entry_6 = Entry::parse(log.entries[5].as_bytes()).unwrap().hash();
        let first = log.checkpoint.clone().unwrap().0;
        type Change = Box<dyn Fn(&mut Manifest)>;
        let changes: Vec<Change> = vec![
            Box::new(|m| m.log = h('1')),
            Box::new(|m| m.head = h('2')),
            Box::new(move |m| {
                m.size = 6;
                m.head = entry_6.clone();
                m.segments[0].last = 6;
            }),
            Box::new(|m| m.checkpoint = None),
            Box::new(|m| m.checkpoint = m.checkpoint.clone().map(|(id, _)| (id, 4))),
            Box::new(|m| m.checkpoint = Some((h('3'), 5))),
            Box::new(|m| {
                m.log = h('1');
                m.checkpoint = None;
            }),
        ];
        for change in changes {
            let mut m = good.clone();
            change(&mut m);
            let report = export(m.canonical().as_bytes(), &segments, 1000);
            assert_eq!(codes(&report), ["S_MANIFEST_MISMATCH"]);
        }
        assert_eq!(good.checkpoint, Some((first, 5)));
        // A checkpoint listed for lines that have none.
        let log = Log::new();
        let (manifest, segments) = log.export(10_000);
        let mut m = Manifest::parse(&manifest).unwrap();
        m.checkpoint = Some((h('4'), 1));
        let report = export(m.canonical().as_bytes(), &segments, 1000);
        assert_eq!(codes(&report), ["S_MANIFEST_MISMATCH"]);

        // Case 102: a line removed from the first of two files, hashes
        // recomputed. first and last are not compared with the lines.
        let log = Log::sample();
        let (manifest, mut segments) = log.export(4);
        let text = String::from_utf8(segments[0].clone()).unwrap();
        let kept: Vec<&str> = text.split_inclusive('\n').collect();
        segments[0] = [kept[0], kept[1], kept[2]].concat().into_bytes();
        let report = export(&rehash(&manifest, &segments), &segments, 1000);
        assert_eq!(
            codes(&report),
            [
                "S_SEQ_GAP@5",
                "S_CHAIN_BREAK@5",
                "S_ATTESTATION_TARGET@5",
                "S_CHECKPOINT_MISMATCH@6"
            ]
        );

        // Case 114: the genesis line removed, the manifest otherwise kept.
        let (manifest, mut segments) = log.export(10_000);
        let text = String::from_utf8(segments[0].clone()).unwrap();
        segments[0] = text.split_once('\n').unwrap().1.as_bytes().to_vec();
        let report = export(&rehash(&manifest, &segments), &segments, 1000);
        assert_eq!(
            codes(&report),
            [
                "S_MANIFEST_MISMATCH",
                "S_GENESIS_INVALID",
                "S_SEQ_GAP@2",
                "S_ATTESTATION_TARGET@2",
                "S_RESERVED_SUBJECT@6"
            ]
        );
    }

    #[test]
    fn prefix_roots_match_the_merkle_module() {
        let leaves: Vec<Digest> = (0..70)
            .map(|i| merkle::leaf_hash(format!("leaf-{i}").as_bytes()))
            .collect();
        let mut tree = Tree::default();
        assert_eq!(tree.root(0), None);
        assert_eq!(tree.root(1), None);
        for (i, leaf) in leaves.iter().enumerate() {
            tree.push(*leaf);
            for size in 1..=i + 1 {
                assert_eq!(
                    tree.root(size),
                    Some(merkle::root(&leaves[..size])),
                    "{size} of {}",
                    i + 1
                );
            }
            assert_eq!(tree.root(0), None);
            assert_eq!(tree.root(i + 2), None);
        }
    }

    /// A small deterministic generator, so the hostile-input test needs no
    /// randomness source.
    struct XorShift(u64);

    impl XorShift {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn below(&mut self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                (self.next() % n as u64) as usize
            }
        }
    }

    #[test]
    fn hostile_input_never_panics() {
        let mut log = Log::sample();
        let c = thing("T-1", "update", "c", &ts(-10), &[]);
        log.push_object(&c);
        log.checkpoint();
        let (manifest, segments) = log.export(3);
        let mut rng = XorShift(0x9e37_79b9_7f4a_7c15);
        let specials: &[&[u8]] = &[
            b"\n",
            b"\r",
            b"{",
            b"}",
            b"[",
            b"]",
            b"\"",
            b",",
            b":",
            b"0",
            b"\\",
            b"\xff",
            b"\xef\xbb\xbf",
        ];
        // Each round checks a few Ed25519 signatures, which is slow in debug
        // builds, so the number of rounds is kept modest.
        for round in 0..150 {
            let mut manifest = manifest.clone();
            let mut segments = segments.clone();
            for _ in 0..1 + rng.below(4) {
                let file = rng.below(segments.len() + 1);
                let bytes = if file == segments.len() {
                    &mut manifest
                } else {
                    &mut segments[file]
                };
                let at = rng.below(bytes.len() + 1);
                match rng.below(6) {
                    0 if at < bytes.len() => bytes[at] ^= 1 << rng.below(8),
                    1 if at < bytes.len() => {
                        bytes.remove(at);
                    }
                    2 => {
                        let ins = specials[rng.below(specials.len())];
                        bytes.splice(at..at, ins.iter().copied());
                    }
                    3 => bytes.truncate(at),
                    4 => {
                        // Copy a run of bytes from elsewhere in the file.
                        let from = rng.below(bytes.len() + 1);
                        let len = rng.below(200).min(bytes.len() - from);
                        let run: Vec<u8> = bytes[from..from + len].to_vec();
                        bytes.splice(at..at, run);
                    }
                    _ => {
                        // Swap two files or drop one.
                        if segments.len() > 1 {
                            let a = rng.below(segments.len());
                            let b = rng.below(segments.len());
                            segments.swap(a, b);
                            if round % 7 == 0 {
                                segments.remove(a);
                            }
                        }
                    }
                }
            }
            let max = 1 + rng.below(20);
            export(&manifest, &segments, max);
        }
    }

    #[test]
    fn hostile_lines_are_line_invalid() {
        // Deep nesting, huge numbers, lone surrogates and long garbage.
        let deep = "[".repeat(100_000);
        let hostile = [
            deep.clone(),
            format!("{{\"entry\":{deep}"),
            format!("{{\"attestation\":{deep},\"entry\":1}}"),
            "{\"entry\":{\"seq\":1e400},\"object\":{}}".to_string(),
            "{\"entry\":\"\\ud800\",\"object\":{}}".to_string(),
            "x".repeat(100_000),
        ];
        let report = verify_lines(&hostile);
        assert_consistent(&report);
        assert_eq!(report.findings.len(), hostile.len() + 1);
    }
}
