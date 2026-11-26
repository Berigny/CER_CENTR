//! `evidence store …`: the store/0 commands, and the store side of
//! `evidence verify --store`. The contract is section 8 of
//! `docs/m1-store-spec.md`; this module follows it exactly.
//!
//! - **Arguments.** The syntax of the object commands: an argument that
//!   starts with `--` names an option and takes the next argument as its
//!   value, whatever it is; every other argument is positional. Unlike the
//!   object commands, each store command accepts only its own options, each
//!   at most once (`--require` may repeat), and a fixed number of
//!   positional arguments ([`COMMANDS`]). [`Args::check`] enforces this
//!   before anything is read; a command body then parses its values
//!   (numbers, ids, purposes, `--if-heads`) before it opens anything.
//! - **Standard output.** Canonical JSON (RFC 8785), one value per line,
//!   each line written whole and flushed at once ([`Out`]). Nothing else.
//! - **Standard error.** Human-readable detail, each line starting
//!   `evidence: `. Never part of the contract.
//! - **Exit status.** 0: success ([`Status::Ok`]). 1: an item was rejected
//!   at intake, or a report is not ok ([`Status::Refused`]). 2: an
//!   operation error ([`Failure`]); the last line of standard output is
//!   then `{"code":…,"ok":false}`.
//!
//! Writers read [`SystemClock`] and sign with a [`SeedSigner`] read from
//! `--seed`. There is no other clock, no hidden option and no environment
//! variable.

use std::cell::RefCell;
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::process::ExitCode;

use evidence_core::attest;
use evidence_core::canon;
use evidence_core::json::Value;
use evidence_core::time::Timestamp;
use evidence_core::verify::{verify_lookup, LookupOptions, PrevLookup};
use evidence_log::manifest::MANIFEST_FILE;
use evidence_log::merkle::MAX_SIZE;
use evidence_log::{StoreReport, SubjectKey, VerifyOptions};
use evidence_store::sqlite::DB_FILE;
use evidence_store::{
    verify_export_dir, verify_store, LogRead, LogWrite, Receipt, RejectCode, Rejected, SeedSigner,
    SqliteLog, SqliteReader, StoreError, SystemClock, DEFAULT_SEGMENT_SIZE,
};

/// The operation code `store get` gives for a well-formed id that the store
/// does not hold. The CLI's own addition to the codes of `StoreError`
/// (`spec/store-0.md`, "Intake and operation codes").
pub(crate) const S_NOT_HELD: &str = "S_NOT_HELD";

/// evidence/0's code for an invalid argument.
const E_USAGE: &str = "E_USAGE";

/// The code of a storage or file error.
const S_IO: &str = "S_IO";

/// `--max-findings` when it is not given, as in every verifier.
pub(crate) const DEFAULT_MAX_FINDINGS: u64 = 1000;

/// `store log --limit` when it is not given.
pub(crate) const DEFAULT_LOG_LIMIT: u64 = 100;

/// How many lines `store log` asks the store for at a time, so that its
/// memory stays bounded whatever `--limit` is.
pub(crate) const LOG_CHUNK: usize = 1024;

/// `store heads --lane` when it is not given.
pub(crate) const DEFAULT_LANE: &str = evidence_log::RECORD;

// ---------------------------------------------------------------------------
// Entry point and dispatch

/// Run `evidence store <args>`: everything after the word `store`. Prints
/// the command's output, or its failure, and returns the exit status.
pub(crate) fn run(args: &[String]) -> ExitCode {
    let mut out = Out::new();
    let result = match args.split_first() {
        Some((command, _)) if matches!(command.as_str(), "help" | "-h" | "--help") => {
            out.line(crate::USAGE).map(|()| Status::Ok)
        }
        Some((command, rest)) => dispatch(command, rest, &mut out),
        None => Err(Failure::usage("missing store command")),
    };
    match result {
        Ok(Status::Ok) => ExitCode::SUCCESS,
        Ok(Status::Refused) => ExitCode::from(1),
        Err(failure) => {
            failure.report(&mut out);
            ExitCode::from(2)
        }
    }
}

/// Find the command, parse and check its arguments, run its body.
fn dispatch(name: &str, rest: &[String], out: &mut Out) -> Result<Status, Failure> {
    let command = COMMANDS
        .iter()
        .find(|c| c.name == name)
        .ok_or_else(|| Failure::usage(format!("unknown store command {name:?}")))?;
    let args = Args::parse(rest)?;
    args.check(command)?;
    (command.body)(&args, out)
}

/// One store command: what it accepts, and its body.
pub(crate) struct Command {
    pub(crate) name: &'static str,
    /// The usage line after `evidence store `, for usage errors.
    pub(crate) synopsis: &'static str,
    /// Positional arguments after the command name: at least `min`.
    pub(crate) min: usize,
    /// At most this many; `None` for no limit.
    pub(crate) max: Option<usize>,
    /// The only options it accepts.
    pub(crate) options: &'static [&'static str],
    /// Options that must be given.
    pub(crate) required: &'static [&'static str],
    /// Options that may be given more than once. Any other option given
    /// twice is `E_USAGE`.
    pub(crate) repeat: &'static [&'static str],
    pub(crate) body: fn(&Args, &mut Out) -> Result<Status, Failure>,
}

/// Every store command, in the order of the usage text.
pub(crate) const COMMANDS: [Command; 11] = [
    Command {
        name: "init",
        synopsis: "init <dir> --name <name> --seed <seed-file>",
        min: 1,
        max: Some(1),
        options: &["--name", "--seed"],
        required: &["--name", "--seed"],
        repeat: &[],
        body: init,
    },
    Command {
        name: "append",
        synopsis: "append <dir> <object.json>... [--if-heads <id>[,<id>...] | --if-heads none]",
        min: 2,
        max: None,
        options: &["--if-heads"],
        required: &[],
        repeat: &[],
        body: append,
    },
    Command {
        name: "attest",
        synopsis: "attest <dir> <attestations.jsonl>...",
        min: 2,
        max: None,
        options: &[],
        required: &[],
        repeat: &[],
        body: attest,
    },
    Command {
        name: "checkpoint",
        synopsis: "checkpoint <dir> --seed <seed-file>",
        min: 1,
        max: Some(1),
        options: &["--seed"],
        required: &["--seed"],
        repeat: &[],
        body: checkpoint,
    },
    Command {
        name: "get",
        synopsis: "get <dir> <id> [--require <purpose>]...",
        min: 2,
        max: Some(2),
        options: &["--require"],
        required: &[],
        repeat: &["--require"],
        body: get,
    },
    Command {
        name: "heads",
        synopsis: "heads <dir> <system> <type> <key> [--lane <lane>]",
        min: 4,
        max: Some(4),
        options: &["--lane"],
        required: &[],
        repeat: &[],
        body: heads,
    },
    Command {
        name: "log",
        synopsis: "log <dir> [--from <seq>] [--limit <n>]",
        min: 1,
        max: Some(1),
        options: &["--from", "--limit"],
        required: &[],
        repeat: &[],
        body: log,
    },
    Command {
        name: "verify",
        synopsis: "verify <dir> [--max-findings <n>]",
        min: 1,
        max: Some(1),
        options: &["--max-findings"],
        required: &[],
        repeat: &[],
        body: verify,
    },
    Command {
        name: "export",
        synopsis: "export <dir> --out <dir> [--segment-size <n>]",
        min: 1,
        max: Some(1),
        options: &["--out", "--segment-size"],
        required: &["--out"],
        repeat: &[],
        body: export,
    },
    Command {
        name: "import",
        synopsis: "import <export-dir> --into <dir>",
        min: 1,
        max: Some(1),
        options: &["--into"],
        required: &["--into"],
        repeat: &[],
        body: import,
    },
    Command {
        name: "prove",
        synopsis:
            "prove <dir> --seq <n> [--size <m>]  or  prove <dir> --from <old-size> --to <new-size>",
        min: 1,
        max: Some(1),
        options: &["--seq", "--size", "--from", "--to"],
        required: &[],
        repeat: &[],
        body: prove,
    },
];

// ---------------------------------------------------------------------------
// Command bodies. Each is called with arguments that passed `Args::check`
// for its `Command`. A body parses its values first, then opens the store,
// so an `E_USAGE` never touches a store.

/// `evidence store init <dir> --name <name> --seed <seed-file>`
///
/// 1. [`seed_signer`] (`S_IO`, `E_USAGE`).
/// 2. `SqliteLog::init(dir, name, &signer, Box::new(SystemClock))`:
///    `E_USAGE` for a name that is not a store name, `S_EXISTS`,
///    `S_LOCKED`, `S_IO` as it says. The writer it returns is dropped at
///    once, releasing the lease.
/// 3. Print `{"head":{"entry_hash":…,"seq":2},"key":…,"log":…,"name":…}`:
///    `log` the log id, `name` and `key` the genesis's, `head` from
///    `LogRead::head()` (`Head::to_value`). [`Status::Ok`].
fn init(args: &Args, out: &mut Out) -> Result<Status, Failure> {
    let name = args.required("--name")?;
    let signer = seed_signer(args)?;
    let log = SqliteLog::init(args.dir(), name, &signer, Box::new(SystemClock))?;
    let head = log.head();
    let genesis = log.genesis().clone();
    let log_id = log.log_id().to_string();
    drop(log);
    out.value(&object_value(vec![
        ("head", head?.to_value()),
        ("key", Value::String(genesis.key)),
        ("log", Value::String(log_id)),
        ("name", Value::String(genesis.name)),
    ]))?;
    Ok(Status::Ok)
}

/// `evidence store append <dir> <object.json>... [--if-heads <ids> | --if-heads none]`
///
/// 1. `--if-heads`, when given: [`parse_if_heads`], and exactly one file
///    (`E_USAGE` otherwise).
/// 2. [`open_writer`] (`S_NOT_A_STORE`, `S_LOCKED`, …), before any file is
///    read.
/// 3. For each file, in argument order: [`read_input`] (an error stops the
///    run: `S_IO` with `file`), then one `commit` of one
///    `Item::Object { bytes, if_heads }`, so one transaction per file:
///    - accepted (a duplicate included): print [`accepted`]`(receipt, file,
///      None)`;
///    - `StoreError::Rejected`: print [`rejected`]`(r, file, None)`, write
///      [`note_rejection`], go on with the next file;
///    - any other error: stop, and fail with it, `file` set ([`Failure::at`]).
///
///    Each line is printed after its commit and before the next file is
///    read.
/// 4. [`Status::Refused`] if any file was rejected, else [`Status::Ok`].
fn append(args: &Args, out: &mut Out) -> Result<Status, Failure> {
    let files = args.positional().get(1..).unwrap_or_default();
    let if_heads = args.value("--if-heads").map(parse_if_heads).transpose()?;
    if if_heads.is_some() && files.len() != 1 {
        return Err(Failure::usage(format!(
            "--if-heads needs exactly one object file, not {}",
            files.len()
        )));
    }
    let mut log = open_writer(args.dir())?;
    let mut status = Status::Ok;
    for file in files {
        let bytes = read_input(file)?;
        let result = match &if_heads {
            Some(expected) => log.append_if_heads(&bytes, expected),
            None => log.append(&bytes),
        };
        status = status.and(intake(out, result, file, None)?);
    }
    Ok(status)
}

/// `evidence store attest <dir> <attestations.jsonl>...`
///
/// As [`append`], with `Item::Attestation`: [`open_writer`] first; then for
/// each file in argument order, [`read_input`] (an error stops the run) and,
/// for each of its [`attestation_lines`] in order, one `commit` of one item:
/// a line [`accepted`]`(receipt, file, Some(n))` or
/// [`rejected`]`(r, file, Some(n))`; any other error stops the run with
/// `file` and `line` set. [`Status::Refused`] if any line was rejected. A
/// file with no line that is not blank prints nothing.
fn attest(args: &Args, out: &mut Out) -> Result<Status, Failure> {
    let files = args.positional().get(1..).unwrap_or_default();
    let mut log = open_writer(args.dir())?;
    let mut status = Status::Ok;
    for file in files {
        let bytes = read_input(file)?;
        for (n, line) in attestation_lines(&bytes) {
            status = status.and(intake(out, log.attest(line), file, Some(n))?);
        }
    }
    Ok(status)
}

/// Print the line for one item a writer was given: [`accepted`], or
/// [`rejected`] (with [`note_rejection`] on standard error), and say which.
/// Any other error is the run's failure, about this file and line.
fn intake(
    out: &mut Out,
    result: Result<Receipt, StoreError>,
    file: &str,
    line: Option<u64>,
) -> Result<Status, Failure> {
    match result {
        Ok(receipt) => {
            out.value(&accepted(&receipt, file, line))?;
            Ok(Status::Ok)
        }
        Err(StoreError::Rejected(r)) => {
            out.value(&rejected(&r, file, line))?;
            note_rejection(&r, file, line);
            Ok(Status::Refused)
        }
        Err(e) => Err(Failure::from(e).at(file, line)),
    }
}

/// `evidence store checkpoint <dir> --seed <seed-file>`
///
/// [`seed_signer`], then [`open_writer`], then `LogWrite::checkpoint`
/// (`S_STORE_KEY` when the seed is not the store key; nothing is written).
/// Print `CheckpointInfo::to_value()`:
/// `{"checkpoint":…,"head":…,"root":…,"seq":…,"size":…}`. [`Status::Ok`].
fn checkpoint(args: &Args, out: &mut Out) -> Result<Status, Failure> {
    let signer = seed_signer(args)?;
    let mut log = open_writer(args.dir())?;
    let info = log.checkpoint(&signer)?;
    drop(log);
    out.value(&info.to_value())?;
    Ok(Status::Ok)
}

/// `evidence store get <dir> <id> [--require <purpose>]...`
///
/// 1. `E_USAGE` unless `<id>` [`is_hash`] and every purpose passes
///    [`check_purposes`].
/// 2. [`open_reader`]; then, in one `SqliteReader::snapshot`:
///    - `LogRead::object(id)`: `None` is [`Failure::not_held`];
///    - the object's line, `lines(seq, 1)`, parsed as JSON: its `entry`
///      and `object` members (`S_IO` if there is no line at that seq, or
///      its entry's `seq` or `ref` is not the object's);
///    - `LogRead::attestations(id)`, and for each its line,
///      `lines(seq, 1)`, parsed: `{"attestation":A,"entry":E}`;
///    - the report: `evidence_core::verify::verify_lookup` over the stored
///      canonical bytes, with `attestations` the stored canonical
///      attestation lines joined by LF, `prev` a [`StoreLookup`] over the
///      same reader, and `require` the purposes; then
///      [`StoreLookup::finish`] (a storage error fails with `S_IO`).
/// 3. Print `{"attestations":[{"attestation":A,"entry":E},…],"entry":E,
///    "object":O,"report":R}`, attestations in seq order, `R` the core
///    report's `to_value()`. [`Status::Refused`] if the report is not ok.
fn get(args: &Args, out: &mut Out) -> Result<Status, Failure> {
    let [_, id] = args.positional() else {
        return Err(Failure::usage("evidence store get takes <dir> <id>"));
    };
    if !is_hash(id) {
        return Err(Failure::usage(format!(
            "an id is sha256: and 64 lowercase hex digits, not {id:?}"
        )));
    }
    let require = args.values("--require");
    check_purposes(&require).map_err(Failure::usage)?;
    let reader = open_reader(args.dir())?;
    let found = reader.snapshot(|r| {
        let Some(held) = r.object(id)? else {
            return Ok(None);
        };
        let (entry, object) = stored_line(r, held.seq, id, "object")?;
        let mut attestations = Vec::new();
        let mut lines = Vec::new();
        for a in r.attestations(id)? {
            let (entry, attestation) = stored_line(r, a.seq, &a.hash, "attestation")?;
            attestations.push(Value::Object(vec![
                ("attestation".into(), attestation),
                ("entry".into(), entry),
            ]));
            lines.push(a.line);
        }
        let lines = lines.join("\n");
        let lookup = StoreLookup::new(r);
        let report = verify_lookup(
            held.bytes.as_bytes(),
            &LookupOptions {
                attestations: Some(lines.as_bytes()),
                prev: Some(&lookup),
                require: &require,
            },
        );
        lookup.finish()?;
        Ok(Some((
            object_value(vec![
                ("attestations", Value::Array(attestations)),
                ("entry", entry),
                ("object", object),
                ("report", report.to_value()),
            ]),
            report.ok(),
        )))
    })?;
    let Some((value, ok)) = found else {
        return Err(Failure::not_held(id));
    };
    out.value(&value)?;
    Ok(Status::of(ok))
}

/// The line at `seq`, parsed: its `entry` member and its `body` member
/// (`object` or `attestation`). `S_IO` unless there is a line at `seq`, it
/// is JSON with both members, and its entry's `seq` and `ref` are `seq`
/// and `reference`: a store whose tables disagree is damaged.
fn stored_line(
    r: &SqliteReader,
    seq: u64,
    reference: &str,
    body: &str,
) -> Result<(Value, Value), StoreError> {
    let damaged =
        |what: &str| StoreError::Io(format!("the line at seq {seq} {what}: verify the store"));
    let lines = r.lines(seq, 1)?;
    let text = match lines.as_slice() {
        [(got, text)] if *got == seq => text,
        _ => return Err(damaged("is missing")),
    };
    let line = evidence_core::json::parse(text.as_bytes())
        .map_err(|e| damaged(&format!("does not parse ({e})")))?;
    let (Some(entry), Some(item)) = (line.get("entry"), line.get(body)) else {
        return Err(damaged(&format!("has no entry or no {body}")));
    };
    let entry_seq = match entry.get("seq") {
        Some(Value::Number(n)) => Some(*n),
        _ => None,
    };
    if entry_seq != Some(seq as f64) || entry.get("ref").and_then(Value::as_str) != Some(reference)
    {
        return Err(damaged(&format!("is not the entry of {reference}")));
    }
    Ok((entry.clone(), item.clone()))
}

/// `evidence store heads <dir> <system> <type> <key> [--lane <lane>]`
///
/// [`open_reader`], then `LogRead::heads(&SubjectKey { system, type, key },
/// lane)` with lane [`DEFAULT_LANE`] when not given. Any strings are
/// accepted; a subject or lane with no objects has no heads. Print
/// `{"heads":[…],"lane":…}`, the heads sorted as `heads` gives them.
/// [`Status::Ok`].
fn heads(args: &Args, out: &mut Out) -> Result<Status, Failure> {
    let [_, system, kind, key] = args.positional() else {
        return Err(Failure::usage(
            "evidence store heads takes <dir> <system> <type> <key>",
        ));
    };
    let subject = SubjectKey {
        system: system.clone(),
        r#type: kind.clone(),
        key: key.clone(),
    };
    let lane = args.value("--lane").unwrap_or(DEFAULT_LANE);
    let reader = open_reader(args.dir())?;
    let heads = reader.heads(&subject, lane)?;
    out.value(&object_value(vec![
        (
            "heads",
            Value::Array(heads.into_iter().map(Value::String).collect()),
        ),
        ("lane", Value::String(lane.to_string())),
    ]))?;
    Ok(Status::Ok)
}

/// `evidence store log <dir> [--from <seq>] [--limit <n>]`
///
/// `--from` (default 1) and `--limit` (default [`DEFAULT_LOG_LIMIT`]), each
/// from 1 to 2^53 − 1 ([`Args::number`]). [`open_reader`], then print the
/// lines of seqs `from` onwards, at most `limit`, in seq order, each with
/// [`Out::line`]: exactly the lines of an export's segment files. Ask
/// `LogRead::lines` for at most [`LOG_CHUNK`] at a time, each next chunk
/// from the seq after the last one given, until `limit` lines are printed
/// or a chunk comes back empty. Nothing is printed when `from` is past the
/// head. [`Status::Ok`].
fn log(args: &Args, out: &mut Out) -> Result<Status, Failure> {
    let from = args.number("--from", 1)?.unwrap_or(1);
    let limit = args.number("--limit", 1)?.unwrap_or(DEFAULT_LOG_LIMIT);
    let reader = open_reader(args.dir())?;
    let (mut next, mut left) = (from, limit);
    while left > 0 {
        let ask = usize::try_from(left).map_or(LOG_CHUNK, |left| left.min(LOG_CHUNK));
        let chunk = reader.lines(next, ask)?;
        let Some(&(last, _)) = chunk.last() else {
            break;
        };
        if last < next || chunk.len() > ask {
            return Err(Failure::io(format!(
                "the store gave lines out of order from seq {next}: verify the store"
            )));
        }
        for (seq, line) in &chunk {
            // A line is one line of output, as in a segment file.
            if line.contains('\n') {
                return Err(Failure::io(format!(
                    "the line at seq {seq} holds an LF: verify the store"
                )));
            }
            out.line(line)?;
        }
        left -= chunk.len() as u64;
        match last.checked_add(1) {
            Some(after) => next = after,
            None => break,
        }
    }
    Ok(Status::Ok)
}

/// `evidence store verify <dir> [--max-findings <n>]`
///
/// [`verify_options`], then: `verify_store(dir, opts)` if [`holds_store`],
/// else `verify_export_dir(dir, opts)` if [`holds_export`], else
/// `S_NOT_A_STORE`. Print the report (`StoreReport::to_value`); a summary
/// on standard error is allowed. [`Status::Refused`] if it is not ok.
fn verify(args: &Args, out: &mut Out) -> Result<Status, Failure> {
    let opts = verify_options(args)?;
    let dir = args.dir();
    let report = if holds_store(dir) {
        verify_store(dir, opts)?
    } else if holds_export(dir) {
        verify_export_dir(dir, opts)?
    } else {
        return Err(StoreError::NotAStore(dir.to_path_buf()).into());
    };
    out.value(&report.to_value())?;
    note_report(dir, &report);
    Ok(Status::of(report.ok()))
}

/// `evidence store export <dir> --out <dir> [--segment-size <n>]`
///
/// `--segment-size` from 1 to 2^53 − 1, default
/// `evidence_store::DEFAULT_SEGMENT_SIZE`. [`open_reader`] (no lease: an
/// export runs while a writer appends), then `evidence_store::export(&reader,
/// out, segment_size)` (`S_EXISTS` if `--out` exists). Print the manifest
/// (`Manifest::to_value`): the same bytes as `manifest.json`, plus an LF.
/// [`Status::Ok`].
fn export(args: &Args, out: &mut Out) -> Result<Status, Failure> {
    let target = args.required("--out")?;
    let segment_size = args
        .number("--segment-size", 1)?
        .unwrap_or(DEFAULT_SEGMENT_SIZE);
    let reader = open_reader(args.dir())?;
    let manifest = evidence_store::export(&reader, Path::new(target), segment_size)?;
    out.line(&manifest.canonical())?;
    Ok(Status::Ok)
}

/// `evidence store import <export-dir> --into <dir>`
///
/// `evidence_store::import(export_dir, into)`. `Ok(report)`: print it;
/// [`Status::Ok`] if it is ok (the `verify_store` report of the new store,
/// source `store`), [`Status::Refused`] if not (nothing was written). An
/// error (`S_EXISTS`, `S_NOT_A_STORE`, `S_IO`, …) is a [`Failure`].
fn import(args: &Args, out: &mut Out) -> Result<Status, Failure> {
    let into = args.required("--into")?;
    let report = evidence_store::import(args.dir(), Path::new(into))?;
    out.value(&report.to_value())?;
    note_report(args.dir(), &report);
    Ok(Status::of(report.ok()))
}

/// `evidence store prove <dir> --seq <n> [--size <m>]` and
/// `evidence store prove <dir> --from <old-size> --to <new-size>`
///
/// 1. Exactly one form: `--seq` with an optional `--size`, or both `--from`
///    and `--to`; any other combination is `E_USAGE`. Each number from 1
///    to 2^53 − 1 ([`Args::number`]).
/// 2. [`open_reader`].
/// 3. Inclusion: the size is `--size`, or else the `size` of the latest
///    checkpoint (the last of `LogRead::checkpoints()`); `E_USAGE` if the
///    store has none. `evidence_store::inclusion(&reader, seq, size)`
///    (`E_USAGE` unless 1 ≤ seq ≤ size ≤ head). Print
///    `InclusionProof::to_value()`.
/// 4. Consistency: `evidence_store::consistency(&reader, from, to)`
///    (`E_USAGE` unless 1 ≤ from < to ≤ head). Print
///    `ConsistencyProof::to_value()`.
///
/// Everything a proof reads is read in one snapshot. [`Status::Ok`].
fn prove(args: &Args, out: &mut Out) -> Result<Status, Failure> {
    let seq = args.number("--seq", 1)?;
    let size = args.number("--size", 1)?;
    let from = args.number("--from", 1)?;
    let to = args.number("--to", 1)?;
    let inclusion = match (seq, size, from, to) {
        (Some(seq), size, None, None) => Ok((seq, size)),
        (None, None, Some(from), Some(to)) => Err((from, to)),
        _ => {
            return Err(Failure::usage(
                "evidence store prove takes --seq <n> [--size <m>], or --from <old-size> --to <new-size>",
            ))
        }
    };
    let reader = open_reader(args.dir())?;
    let proof = reader.snapshot(|r| match inclusion {
        Ok((seq, size)) => {
            let size = match size {
                Some(size) => size,
                None => r.checkpoints()?.last().map(|c| c.size).ok_or_else(|| {
                    StoreError::Usage(
                        "the store has no checkpoint, so give the tree size with --size".into(),
                    )
                })?,
            };
            Ok(evidence_store::inclusion(r, seq, size)?.to_value())
        }
        Err((from, to)) => Ok(evidence_store::consistency(r, from, to)?.to_value()),
    })?;
    out.value(&proof)?;
    Ok(Status::Ok)
}

/// `evidence verify <file> [--attestations <file>] --store <dir>
/// [--require <purpose>]...` when `<dir>` [`holds_store`]. `main` has read
/// the object file and the attestations file; it prints errors as the
/// object commands do (`Err(message)`: standard error, exit 2, nothing on
/// standard output; a message starting `usage` adds the usage text).
///
/// 1. [`check_purposes`]: on failure, `Err(format!("usage: {detail}"))`,
///    the message `evidence verify` gives for a folder.
/// 2. `SqliteReader::open(dir)`: on failure, `Err(format!("{dir}: {e}"))`.
/// 3. In one `SqliteReader::snapshot`: `evidence_core::verify::verify_lookup`
///    with `attestations`, `prev` a [`StoreLookup`] over the reader, and
///    `require`; then [`StoreLookup::finish`] (an error is
///    `Err(format!("{dir}: {e}"))`, never a report). Stored attestations
///    are not used: `--store` gives only the objects `prev` may name, as it
///    does for a folder.
/// 4. Print the report as canonical JSON, as for a folder; exit 0 if ok, 1
///    if not. For a store holding the objects of a folder, the bytes are
///    the folder's (T12).
pub(crate) fn verify_with_store(
    bytes: &[u8],
    attestations: Option<&[u8]>,
    dir: &Path,
    require: &[String],
) -> Result<ExitCode, String> {
    check_purposes(require).map_err(|detail| format!("usage: {detail}"))?;
    let failed = |e: StoreError| format!("{}: {e}", dir.display());
    let reader = SqliteReader::open(dir).map_err(failed)?;
    let report = reader
        .snapshot(|r| {
            let lookup = StoreLookup::new(r);
            let report = verify_lookup(
                bytes,
                &LookupOptions {
                    attestations,
                    prev: Some(&lookup),
                    require,
                },
            );
            lookup.finish()?;
            Ok(report)
        })
        .map_err(failed)?;
    Out::new()
        .value(&report.to_value())
        .map_err(|e| e.message)?;
    Ok(if report.ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

// ---------------------------------------------------------------------------
// Arguments

/// Parsed arguments of one store command: positional arguments in order,
/// and `--name value` options in order.
#[derive(Debug, Default)]
pub(crate) struct Args {
    positional: Vec<String>,
    options: Vec<(String, String)>,
}

impl Args {
    /// The syntax of the object commands: an argument starting with `--`
    /// names an option and takes the next argument as its value, whatever
    /// it is (`--if-heads none`, `--out --x`); every other argument is
    /// positional. An option with no argument after it is `E_USAGE`.
    pub(crate) fn parse(args: &[String]) -> Result<Args, Failure> {
        let mut parsed = Args::default();
        let mut it = args.iter();
        while let Some(arg) = it.next() {
            if arg.starts_with("--") {
                let value = it
                    .next()
                    .ok_or_else(|| Failure::usage(format!("{arg} needs a value")))?;
                parsed.options.push((arg.clone(), value.clone()));
            } else {
                parsed.positional.push(arg.clone());
            }
        }
        Ok(parsed)
    }

    /// `E_USAGE` unless every option is one `command` accepts, none but its
    /// `repeat` options is given twice, its `required` options are given,
    /// and the number of positional arguments is within its bounds.
    pub(crate) fn check(&self, command: &Command) -> Result<(), Failure> {
        let synopsis = || format!("usage: evidence store {}", command.synopsis);
        for (i, (name, _)) in self.options.iter().enumerate() {
            if !command.options.contains(&name.as_str()) {
                return Err(Failure::usage(format!(
                    "evidence store {} takes no option {name}; {}",
                    command.name,
                    synopsis()
                )));
            }
            let again = self.options[..i].iter().any(|(earlier, _)| earlier == name);
            if again && !command.repeat.contains(&name.as_str()) {
                return Err(Failure::usage(format!("{name} is given more than once")));
            }
        }
        for name in command.required {
            if self.value(name).is_none() {
                return Err(Failure::usage(format!(
                    "{name} is required; {}",
                    synopsis()
                )));
            }
        }
        let n = self.positional.len();
        if n < command.min || command.max.is_some_and(|max| n > max) {
            return Err(Failure::usage(synopsis()));
        }
        Ok(())
    }

    /// The positional arguments, in order.
    pub(crate) fn positional(&self) -> &[String] {
        &self.positional
    }

    /// The first positional argument: the store, export or source directory
    /// of every command. `check` has made sure there is one.
    pub(crate) fn dir(&self) -> &Path {
        Path::new(self.positional.first().map_or("", String::as_str))
    }

    /// The value of an option given at most once, or None.
    pub(crate) fn value(&self, name: &str) -> Option<&str> {
        self.options
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// The value of a required option (`check` has made sure it is given).
    pub(crate) fn required(&self, name: &str) -> Result<&str, Failure> {
        self.value(name)
            .ok_or_else(|| Failure::usage(format!("{name} is required")))
    }

    /// Every value of a repeatable option, in order.
    pub(crate) fn values(&self, name: &str) -> Vec<String> {
        self.options
            .iter()
            .filter(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
            .collect()
    }

    /// A numeric option: None when it is not given; otherwise its value,
    /// which must be 1 to 16 ASCII digits (leading zeros allowed, no sign)
    /// giving an integer from `min` to 2^53 − 1. `E_USAGE` otherwise.
    pub(crate) fn number(&self, name: &str, min: u64) -> Result<Option<u64>, Failure> {
        self.value(name)
            .map(|text| {
                let digits = !text.is_empty()
                    && text.len() <= 16
                    && text.bytes().all(|b| b.is_ascii_digit());
                digits
                    .then(|| text.parse::<u64>().ok())
                    .flatten()
                    .filter(|n| (min..=MAX_SIZE).contains(n))
                    .ok_or_else(|| {
                        Failure::usage(format!(
                            "{name} takes an integer from {min} to 2^53 - 1, not {text:?}"
                        ))
                    })
            })
            .transpose()
    }
}

/// `--if-heads`: `none` is the empty set (the subject and lane must have no
/// heads); otherwise one or more ids separated by commas, each a hash
/// ([`is_hash`]), repeats allowed (it is compared as a set). `E_USAGE`
/// otherwise, an empty value or an empty id included.
pub(crate) fn parse_if_heads(text: &str) -> Result<Vec<String>, Failure> {
    if text == "none" {
        return Ok(Vec::new());
    }
    text.split(',')
        .map(|id| {
            if is_hash(id) {
                Ok(id.to_string())
            } else {
                Err(Failure::usage(format!(
                    "--if-heads takes \"none\" or ids separated by commas, not {text:?}"
                )))
            }
        })
        .collect()
}

/// `sha256:` and 64 lowercase hex digits: an evidence/0 id or hash.
pub(crate) fn is_hash(text: &str) -> bool {
    text.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    })
}

/// `--require` purposes: each must be one of evidence/0's. On failure, the
/// detail `evidence verify` gives (evidence-core's `api::verify`).
pub(crate) fn check_purposes(require: &[String]) -> Result<(), String> {
    match require
        .iter()
        .find(|p| !attest::PURPOSES.contains(&p.as_str()))
    {
        Some(p) => Err(format!(
            "purpose {p:?} must be one of {:?}",
            attest::PURPOSES
        )),
        None => Ok(()),
    }
}

/// `--max-findings` (default [`DEFAULT_MAX_FINDINGS`], from 1 to 2^53 − 1)
/// as verifier options.
pub(crate) fn verify_options(args: &Args) -> Result<VerifyOptions, Failure> {
    let max = args
        .number("--max-findings", 1)?
        .unwrap_or(DEFAULT_MAX_FINDINGS);
    Ok(VerifyOptions {
        max_findings: usize::try_from(max).unwrap_or(usize::MAX),
        ..VerifyOptions::default()
    })
}

/// The lines of an attestation file that are not blank, each with its
/// number: lines are split at LF, numbered from 1 with blank lines counted,
/// and a line is blank when every byte is ASCII whitespace (an empty line
/// included). The rule and the numbers of `evidence verify --attestations`.
pub(crate) fn attestation_lines(bytes: &[u8]) -> Vec<(u64, &[u8])> {
    bytes
        .split(|b| *b == b'\n')
        .zip(1u64..)
        .filter(|(line, _)| !line.iter().all(u8::is_ascii_whitespace))
        .map(|(line, n)| (n, line))
        .collect()
}

// ---------------------------------------------------------------------------
// Stores, files and keys

/// Whether `dir` holds a store: `<dir>/log.db` exists, as any kind of file
/// (a symlink is not followed). A `log.db` that is not a store is then an
/// error when it is opened, never a reason to read `dir` another way.
pub(crate) fn holds_store(dir: &Path) -> bool {
    fs::symlink_metadata(dir.join(DB_FILE)).is_ok()
}

/// Whether `dir` holds an export: `<dir>/manifest.json` exists, as any kind
/// of file. [`holds_store`] is asked first.
pub(crate) fn holds_export(dir: &Path) -> bool {
    fs::symlink_metadata(dir.join(MANIFEST_FILE)).is_ok()
}

/// The writer of the store in `dir`, reading the system clock. Takes the
/// lease: `S_LOCKED` while another writer holds it.
pub(crate) fn open_writer(dir: &Path) -> Result<SqliteLog, Failure> {
    Ok(SqliteLog::open(dir, Box::new(SystemClock))?)
}

/// A reader of the store in `dir`. Takes no lease.
pub(crate) fn open_reader(dir: &Path) -> Result<SqliteReader, Failure> {
    Ok(SqliteReader::open(dir)?)
}

/// The store key from `--seed`: `S_IO` if the file cannot be read,
/// `E_USAGE` if it does not hold 64 hex digits.
pub(crate) fn seed_signer(args: &Args) -> Result<SeedSigner, Failure> {
    let path = args.required("--seed")?;
    Ok(SeedSigner::from_file(Path::new(path))?)
}

/// The bytes of an input file of `append` or `attest`: `S_IO`, with `file`
/// set, if it cannot be read.
pub(crate) fn read_input(file: &str) -> Result<Vec<u8>, Failure> {
    fs::read(file).map_err(|e| Failure::io(e.to_string()).at(file, None))
}

/// A prev lookup over a store that tells "not held" from a read that
/// failed. `SqliteReader`'s own `prev_at` reads both as None, which would
/// turn a storage error into `E_PREV_MISSING`; this asks again through
/// `LogRead::object`, which reports errors, and keeps the first error for
/// [`finish`](Self::finish).
///
/// Use it inside one `SqliteReader::snapshot`, so that an object a writer
/// appends meanwhile cannot appear between the two reads.
pub(crate) struct StoreLookup<'a> {
    reader: &'a SqliteReader,
    error: RefCell<Option<StoreError>>,
}

impl<'a> StoreLookup<'a> {
    pub(crate) fn new(reader: &'a SqliteReader) -> StoreLookup<'a> {
        StoreLookup {
            reader,
            error: RefCell::new(None),
        }
    }

    /// The first storage error a lookup met, if any. A report made with
    /// this lookup must not be printed when there was one.
    pub(crate) fn finish(self) -> Result<(), StoreError> {
        match self.error.into_inner() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    fn fail(&self, e: StoreError) {
        let mut slot = self.error.borrow_mut();
        if slot.is_none() {
            *slot = Some(e);
        }
    }
}

impl PrevLookup for StoreLookup<'_> {
    fn prev_at(&self, id: &str) -> Option<Timestamp> {
        if let Some(at) = self.reader.prev_at(id) {
            return Some(at);
        }
        match self.reader.object(id) {
            Ok(None) => None,
            Ok(Some(_)) => {
                self.fail(StoreError::Io(format!(
                    "object {id} is held, but its time could not be read: verify the store"
                )));
                None
            }
            Err(e) => {
                self.fail(e);
                None
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Output

/// How a command that did not fail ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Status {
    /// Exit 0.
    Ok,
    /// Exit 1: an item was rejected, or a report is not ok.
    Refused,
}

impl Status {
    /// [`Status::Ok`] when `ok`, else [`Status::Refused`].
    pub(crate) fn of(ok: bool) -> Status {
        if ok {
            Status::Ok
        } else {
            Status::Refused
        }
    }

    /// [`Status::Ok`] when both are, else [`Status::Refused`]: a run with
    /// one rejected item is refused.
    pub(crate) fn and(self, other: Status) -> Status {
        Status::of(self == Status::Ok && other == Status::Ok)
    }
}

/// Standard output. Each line is one canonical JSON value, written in one
/// call with its LF and flushed at once, so that a line printed after a
/// commit has left the process before the next commit starts.
pub(crate) struct Out {
    stdout: io::Stdout,
}

impl Out {
    fn new() -> Out {
        Out {
            stdout: io::stdout(),
        }
    }

    /// Print `value` as canonical JSON and an LF.
    pub(crate) fn value(&mut self, value: &Value) -> Result<(), Failure> {
        self.line(&canon::canonicalize(value))
    }

    /// Print `text`, which is already one canonical JSON value (a segment
    /// line), and an LF.
    pub(crate) fn line(&mut self, text: &str) -> Result<(), Failure> {
        let mut bytes = Vec::with_capacity(text.len() + 1);
        bytes.extend_from_slice(text.as_bytes());
        bytes.push(b'\n');
        let mut lock = self.stdout.lock();
        lock.write_all(&bytes)
            .and_then(|()| lock.flush())
            .map_err(|e| Failure::io(format!("standard output: {e}")))
    }
}

/// The output line for an item a writer accepted (a duplicate included):
/// the receipt's members (`Receipt::to_value`), plus `"ok":true`, `file`,
/// and for an attestation line its `line` number.
pub(crate) fn accepted(receipt: &Receipt, file: &str, line: Option<u64>) -> Value {
    let mut members = vec![
        ("ok".to_string(), Value::Bool(true)),
        ("file".to_string(), Value::String(file.to_string())),
    ];
    if let Some(n) = line {
        members.push(("line".to_string(), Value::Number(n as f64)));
    }
    if let Value::Object(receipt) = receipt.to_value() {
        members.extend(receipt);
    }
    Value::Object(members)
}

/// The output line for an item intake rejected: `"ok":false`, `file`,
/// `code` (the `E_` code, or `S_STALE_HEADS`, `S_RESERVED_SUBJECT` or
/// `S_ATTESTATION_TARGET`), `line` for an attestation line, and `heads`
/// (the current heads, sorted) for `S_STALE_HEADS` only, even when empty.
pub(crate) fn rejected(r: &Rejected, file: &str, line: Option<u64>) -> Value {
    let mut members = vec![
        ("ok".to_string(), Value::Bool(false)),
        ("file".to_string(), Value::String(file.to_string())),
        ("code".to_string(), Value::String(r.code_str().to_string())),
    ];
    if let Some(n) = line {
        members.push(("line".to_string(), Value::Number(n as f64)));
    }
    if r.code == RejectCode::StaleHeads {
        let heads = r.heads.iter().cloned().map(Value::String).collect();
        members.push(("heads".to_string(), Value::Array(heads)));
    }
    Value::Object(members)
}

/// Standard error for a rejected item: `evidence: <file>[:<line>]: <code>:
/// <detail>`.
pub(crate) fn note_rejection(r: &Rejected, file: &str, line: Option<u64>) {
    note(format_args!(
        "{}: {}: {}",
        place(Some(file), line),
        r.code_str(),
        r.detail
    ));
}

/// Standard error for a report that is not ok: how many findings, and the
/// first one's code. Nothing for a report that is ok.
fn note_report(dir: &Path, report: &StoreReport) {
    if let Some(first) = report.findings.first() {
        note(format_args!(
            "{}: not ok: {} finding(s){}, the first {}",
            dir.display(),
            report.findings.len(),
            if report.truncated { " and more" } else { "" },
            first.code.as_str()
        ));
    }
}

/// Write `evidence: <text>` and an LF on standard error. An error writing
/// it is ignored: standard error is not part of the contract, and a closed
/// one must not stop a command or make it panic.
fn note(text: fmt::Arguments<'_>) {
    let _ = writeln!(io::stderr().lock(), "evidence: {text}");
}

/// A JSON object of these members. Canonical output sorts them.
fn object_value(members: Vec<(&str, Value)>) -> Value {
    Value::Object(
        members
            .into_iter()
            .map(|(name, value)| (name.to_string(), value))
            .collect(),
    )
}

/// `file`, `file:line`, or nothing.
fn place(file: Option<&str>, line: Option<u64>) -> String {
    match (file, line) {
        (Some(file), Some(line)) => format!("{file}:{line}"),
        (Some(file), None) => file.to_string(),
        _ => String::new(),
    }
}

// ---------------------------------------------------------------------------
// Failures

/// An operation error: exit status 2. Printed as one line on standard
/// output, `{"code":…,"ok":false}` with `file` (and `line`) when it
/// concerns one input of `append` or `attest`, and its detail on standard
/// error.
#[derive(Debug)]
pub(crate) struct Failure {
    /// `E_USAGE`, [`S_NOT_HELD`], or a `StoreError::code_str()`.
    code: &'static str,
    /// For standard error only; starts with the code.
    message: String,
    file: Option<String>,
    line: Option<u64>,
    /// Print the usage text after the message.
    show_usage: bool,
}

impl Failure {
    /// `E_USAGE` for an invalid command line; the usage text follows the
    /// message on standard error.
    pub(crate) fn usage(detail: impl Into<String>) -> Failure {
        Failure {
            code: E_USAGE,
            message: format!("{E_USAGE}: {}", detail.into()),
            file: None,
            line: None,
            show_usage: true,
        }
    }

    /// [`S_NOT_HELD`]: `store get` names an object the store does not hold.
    pub(crate) fn not_held(id: &str) -> Failure {
        Failure {
            code: S_NOT_HELD,
            message: format!("{S_NOT_HELD}: the store holds no object {id}"),
            file: None,
            line: None,
            show_usage: false,
        }
    }

    /// `S_IO`: a file or output error the CLI meets itself.
    pub(crate) fn io(detail: impl Into<String>) -> Failure {
        Failure {
            code: S_IO,
            message: format!("{S_IO}: {}", detail.into()),
            file: None,
            line: None,
            show_usage: false,
        }
    }

    /// The same failure, about this input file and line.
    pub(crate) fn at(mut self, file: &str, line: Option<u64>) -> Failure {
        self.file = Some(file.to_string());
        self.line = line;
        self
    }

    /// The code printed on standard output.
    #[cfg(test)]
    pub(crate) fn code(&self) -> &'static str {
        self.code
    }

    /// `{"code":…,"ok":false}`, plus `file` and `line` when set.
    pub(crate) fn to_value(&self) -> Value {
        let mut members = vec![
            ("ok".to_string(), Value::Bool(false)),
            ("code".to_string(), Value::String(self.code.to_string())),
        ];
        if let Some(file) = &self.file {
            members.push(("file".to_string(), Value::String(file.clone())));
        }
        if let Some(n) = self.line {
            members.push(("line".to_string(), Value::Number(n as f64)));
        }
        Value::Object(members)
    }

    /// Print the failure: its line on standard output (an error writing it
    /// is ignored; the exit status still says 2), then `evidence: [<place>:
    /// ]<message>` on standard error, and the usage text for `E_USAGE`.
    fn report(&self, out: &mut Out) {
        let _ = out.value(&self.to_value());
        let place = place(self.file.as_deref(), self.line);
        if place.is_empty() {
            note(format_args!("{}", self.message));
        } else {
            note(format_args!("{place}: {}", self.message));
        }
        if self.show_usage {
            let _ = writeln!(io::stderr().lock(), "\n{}", crate::USAGE);
        }
    }
}

/// Every `StoreError` but `Rejected` is an operation error with its code
/// (`S_LOCKED`, `S_STORE_KEY`, `S_NOT_A_STORE`, `S_EXISTS`, `S_FORMAT`,
/// `E_USAGE`, `S_IO`). Commands handle `Rejected` themselves; one that
/// reaches here keeps its code.
impl From<StoreError> for Failure {
    fn from(e: StoreError) -> Failure {
        Failure {
            code: e.code_str(),
            message: e.to_string(),
            file: None,
            line: None,
            show_usage: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn command(name: &str) -> &'static Command {
        COMMANDS.iter().find(|c| c.name == name).unwrap()
    }

    fn checked(name: &str, list: &[&str]) -> Result<Args, Failure> {
        let parsed = Args::parse(&args(list))?;
        parsed.check(command(name))?;
        Ok(parsed)
    }

    #[test]
    fn every_command_is_in_the_usage_text() {
        for c in &COMMANDS {
            assert!(
                crate::USAGE.contains(&format!("evidence store {} ", c.name)),
                "{}",
                c.name
            );
            assert!(c.synopsis.starts_with(c.name));
            for name in c.required.iter().chain(c.repeat) {
                assert!(c.options.contains(name), "{}: {name}", c.name);
            }
        }
    }

    #[test]
    fn options_take_the_next_argument_whatever_it_is() {
        let a = checked("append", &["s", "--if-heads", "none", "x.json"]).unwrap();
        assert_eq!(a.positional(), ["s", "x.json"]);
        assert_eq!(a.value("--if-heads"), Some("none"));
        let a = checked("export", &["s", "--out", "--x"]).unwrap();
        assert_eq!(a.value("--out"), Some("--x"));
        assert_eq!(a.dir(), Path::new("s"));
    }

    #[test]
    fn command_lines_that_are_usage_errors() {
        let cases: &[(&str, &[&str])] = &[
            ("init", &["s", "--name", "demo"]), // --seed missing
            ("init", &["s", "--name", "a", "--name", "b", "--seed", "k"]), // twice
            ("init", &["--name", "demo", "--seed", "k"]), // no dir
            ("append", &["s"]),                 // no file
            ("append", &["s", "x.json", "--if-heads"]), // no value
            ("append", &["s", "x.json", "--lane", "record"]), // not its option
            ("get", &["s"]),                    // no id
            ("get", &["s", "id", "extra"]),     // one too many
            ("heads", &["s", "sys", "type"]),   // key missing
            ("verify", &["s", "--verbose", "1"]), // unknown option
        ];
        for (name, list) in cases {
            let e = checked(name, list).unwrap_err();
            assert_eq!(e.code(), E_USAGE, "{name} {list:?}");
            assert!(e.show_usage);
        }
        assert!(checked(
            "get",
            &["s", "id", "--require", "author", "--require", "approve"]
        )
        .is_ok());
    }

    #[test]
    fn numbers_are_plain_decimal_integers_in_range() {
        let n = |text: &str, min| {
            Args::parse(&args(&["s", "--limit", text]))
                .unwrap()
                .number("--limit", min)
        };
        assert_eq!(n("100", 1).unwrap(), Some(100));
        assert_eq!(n("007", 1).unwrap(), Some(7));
        assert_eq!(n("9007199254740991", 1).unwrap(), Some(MAX_SIZE));
        for bad in [
            "0",
            "",
            "-1",
            "+1",
            "1.0",
            "1e3",
            " 1",
            "9007199254740992",
            "12345678901234567",
        ] {
            assert_eq!(n(bad, 1).unwrap_err().code(), E_USAGE, "{bad:?}");
        }
        assert_eq!(Args::default().number("--limit", 1).unwrap(), None);
    }

    #[test]
    fn if_heads_is_none_or_hashes() {
        let h = format!("sha256:{}", "ab".repeat(32));
        assert_eq!(parse_if_heads("none").unwrap(), Vec::<String>::new());
        assert_eq!(parse_if_heads(&h).unwrap(), vec![h.clone()]);
        assert_eq!(parse_if_heads(&format!("{h},{h}")).unwrap().len(), 2);
        for bad in [
            "",
            ",",
            "none,",
            &format!("{h},"),
            &h.to_uppercase(),
            "sha256:ab",
        ] {
            assert_eq!(parse_if_heads(bad).unwrap_err().code(), E_USAGE, "{bad:?}");
        }
    }

    #[test]
    fn attestation_lines_skip_blank_lines_and_keep_their_numbers() {
        let lines = attestation_lines(b"a\n\n  \nb\r\nc");
        assert_eq!(
            lines,
            vec![(1, &b"a"[..]), (4, &b"b\r"[..]), (5, &b"c"[..])]
        );
        assert!(attestation_lines(b"").is_empty());
        assert!(attestation_lines(b"\n \t\n").is_empty());
    }

    #[test]
    fn one_refusal_refuses_the_run() {
        assert_eq!(Status::Ok.and(Status::Ok), Status::Ok);
        assert_eq!(Status::Ok.and(Status::Refused), Status::Refused);
        assert_eq!(Status::Refused.and(Status::Ok), Status::Refused);
        assert_eq!(Status::of(false), Status::Refused);
    }

    #[test]
    fn purposes_are_checked_with_the_message_of_evidence_verify() {
        assert!(check_purposes(&["approve".into(), "author".into()]).is_ok());
        assert_eq!(
            check_purposes(&["sponsor".into()]).unwrap_err(),
            "purpose \"sponsor\" must be one of [\"author\", \"approve\", \"witness\"]"
        );
    }

    #[test]
    fn output_lines_have_their_contract_shapes() {
        let h = format!("sha256:{}", "11".repeat(32));
        let receipt = Receipt {
            kind: evidence_log::Kind::Object,
            reference: h.clone(),
            seq: 3,
            entry_hash: h.clone(),
            received_at: "2026-11-26T00:00:00.000Z".into(),
            duplicate: false,
        };
        assert_eq!(
            canon::canonicalize(&accepted(&receipt, "a.json", None)),
            format!(
                "{{\"duplicate\":false,\"entry_hash\":\"{h}\",\"file\":\"a.json\",\"kind\":\"object\",\
                 \"ok\":true,\"received_at\":\"2026-11-26T00:00:00.000Z\",\"ref\":\"{h}\",\"seq\":3}}"
            )
        );
        let mut stale = Rejected::new(0, RejectCode::StaleHeads, "x");
        stale.heads = vec![h.clone()];
        assert_eq!(
            canon::canonicalize(&rejected(&stale, "a.json", None)),
            format!("{{\"code\":\"S_STALE_HEADS\",\"file\":\"a.json\",\"heads\":[\"{h}\"],\"ok\":false}}")
        );
        let shape = Rejected::new(
            0,
            RejectCode::Core(evidence_core::Code::AttestationShape),
            "x",
        );
        assert_eq!(
            canon::canonicalize(&rejected(&shape, "a.jsonl", Some(4))),
            "{\"code\":\"E_ATTESTATION_SHAPE\",\"file\":\"a.jsonl\",\"line\":4,\"ok\":false}"
        );
        assert_eq!(
            canon::canonicalize(
                &Failure::from(StoreError::Locked {
                    holder: String::new()
                })
                .to_value()
            ),
            "{\"code\":\"S_LOCKED\",\"ok\":false}"
        );
        assert_eq!(
            canon::canonicalize(&Failure::io("gone").at("b.jsonl", Some(2)).to_value()),
            "{\"code\":\"S_IO\",\"file\":\"b.jsonl\",\"line\":2,\"ok\":false}"
        );
        assert_eq!(Failure::not_held(&h).code(), S_NOT_HELD);
    }
}
