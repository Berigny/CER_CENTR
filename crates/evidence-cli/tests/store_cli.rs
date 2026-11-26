//! The `evidence store` commands and `evidence verify --store`, run as a
//! user runs them: the built binary (`CARGO_BIN_EXE_evidence`), on stores
//! in temporary directories, over the test vectors. Covers T3, the CLI
//! parts of T7, T10 and T11, T12, and the command-line contract of section
//! 8 of `docs/m1-store-spec.md`.
//!
//! Every assertion on standard output compares exact bytes: canonical JSON
//! values, one per line. Standard error is never compared, except that its
//! lines start `evidence: ` and that an operation error names its code
//! first ([`check_contract`], run on every store command).
//!
//! Data: the vectors in `test-vectors/` and synthetic objects built here.
//! Received times come from the system clock, so no assertion depends on
//! them.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use evidence_core::json::{self, Value};
use evidence_core::{api, canon, object};
use evidence_log::merkle::{self, Digest};
use evidence_store::{LogWrite, SqliteLog, SystemClock};
use tempfile::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_evidence");

/// Command-line arguments from anything that is a path or a string.
macro_rules! args {
    ($($arg:expr),* $(,)?) => {
        vec![$(OsString::from(AsRef::<std::ffi::OsStr>::as_ref(&$arg))),*]
    };
}

// ---------------------------------------------------------------------------
// Running the binary

/// One finished run of the binary.
#[derive(Debug)]
struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Run {
    /// Assert the exit status; give the run back.
    fn expect(self, code: i32) -> Run {
        assert_eq!(self.code, code, "exit status of {self:#?}");
        self
    }

    /// Standard output, one JSON value per line.
    fn lines(&self) -> Vec<Value> {
        stdout_lines(&self.stdout)
            .iter()
            .map(|line| parse(line))
            .collect()
    }

    /// The one value on standard output.
    fn value(&self) -> Value {
        let mut lines = self.lines();
        assert_eq!(lines.len(), 1, "one line expected: {self:#?}");
        lines.remove(0)
    }
}

/// `evidence <args>`, from the test's working directory.
fn evidence(args: Vec<OsString>) -> Run {
    run_in(None, args)
}

/// `evidence store <args>`, checked against section 8.1.
fn store(args: Vec<OsString>) -> Run {
    store_in(None, args)
}

/// `evidence store <args>` with this working directory.
fn store_in(cwd: Option<&Path>, args: Vec<OsString>) -> Run {
    let mut full = args!["store"];
    full.extend(args);
    let run = run_in(cwd, full);
    check_contract(&run);
    run
}

fn run_in(cwd: Option<&Path>, args: Vec<OsString>) -> Run {
    let mut command = Command::new(BIN);
    command.args(&args).stdin(Stdio::null());
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let output = command.output().expect("run the evidence binary");
    Run {
        code: output.status.code().expect("an exit status, not a signal"),
        stdout: String::from_utf8(output.stdout).expect("standard output is UTF-8"),
        stderr: String::from_utf8(output.stderr).expect("standard error is UTF-8"),
    }
}

/// The lines of standard output, each of which ended in LF.
fn stdout_lines(stdout: &str) -> Vec<&str> {
    if stdout.is_empty() {
        return Vec::new();
    }
    let body = stdout
        .strip_suffix('\n')
        .unwrap_or_else(|| panic!("standard output does not end in LF: {stdout:?}"));
    body.split('\n').collect()
}

/// Section 8.1, checked on every store command: exit status 0, 1 or 2;
/// standard output is canonical JSON values, one per line, each ending in
/// LF; standard error lines start `evidence: ` (up to the usage text that
/// follows a usage error); on exit 2 the last line is `{"code","ok":false}`
/// with at most `file` and `line` besides, and standard error starts
/// `evidence: [<file>[:<line>]: ]<code>`.
fn check_contract(run: &Run) {
    assert!(matches!(run.code, 0..=2), "exit status: {run:#?}");
    for line in stdout_lines(&run.stdout) {
        assert_eq!(canon::canonicalize(&parse(line)), line, "not canonical");
    }
    for line in run.stderr.split('\n').take_while(|l| !l.is_empty()) {
        assert!(line.starts_with("evidence: "), "standard error: {run:#?}");
    }
    if run.code != 2 {
        return;
    }
    let lines = run.lines();
    let last = lines
        .last()
        .unwrap_or_else(|| panic!("no code line: {run:#?}"));
    for (name, _) in last.as_object().expect("an object") {
        assert!(
            matches!(name.as_str(), "code" | "file" | "line" | "ok"),
            "{name} in the code line: {run:#?}"
        );
    }
    assert_eq!(last.get("ok"), Some(&Value::Bool(false)));
    let code = text(last, "code");
    let place = match (last.get("file"), last.get("line")) {
        (Some(file), Some(_)) => format!("{}:{}: ", file.as_str().unwrap(), num(last, "line")),
        (Some(file), None) => format!("{}: ", file.as_str().unwrap()),
        _ => String::new(),
    };
    let prefix = format!("evidence: {place}{code}");
    let first = run.stderr.lines().next().unwrap_or("");
    let rejected_before = lines[..lines.len() - 1]
        .iter()
        .any(|l| l.get("ok") == Some(&Value::Bool(false)));
    if rejected_before {
        assert!(
            run.stderr.lines().any(|l| l.starts_with(&prefix)),
            "{prefix}: {run:#?}"
        );
    } else {
        assert!(first.starts_with(&prefix), "{prefix}: {run:#?}");
    }
}

/// The exact standard output of an operation error with no earlier line.
fn code_line(code: &str) -> String {
    format!("{{\"code\":\"{code}\",\"ok\":false}}\n")
}

// ---------------------------------------------------------------------------
// Values and files

fn parse(text: &str) -> Value {
    json::parse(text.as_bytes()).unwrap_or_else(|e| panic!("{text:?}: {e}"))
}

fn read_json(path: &Path) -> Value {
    parse(&fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display())))
}

/// The member at this path of names.
fn at<'a>(v: &'a Value, path: &[&str]) -> &'a Value {
    path.iter().fold(v, |v, name| {
        v.get(name)
            .unwrap_or_else(|| panic!("no {name} in {}", canon::canonicalize(v)))
    })
}

fn num(v: &Value, name: &str) -> u64 {
    match v.get(name) {
        Some(Value::Number(n)) if n.fract() == 0.0 && *n >= 0.0 => *n as u64,
        other => panic!("{name} is not a count: {other:?}"),
    }
}

fn text(v: &Value, name: &str) -> String {
    v.get(name)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{name} is not a string in {}", canon::canonicalize(v)))
        .to_string()
}

fn string(text: &str) -> Value {
    Value::String(text.to_string())
}

fn number(n: u64) -> Value {
    Value::Number(n as f64)
}

/// A JSON object of these members, in canonical order (the names here are
/// ASCII), so that it equals the value parsed from canonical output.
fn obj(members: &[(&str, Value)]) -> Value {
    let mut members: Vec<(String, Value)> = members
        .iter()
        .map(|(name, value)| (name.to_string(), value.clone()))
        .collect();
    members.sort_by(|a, b| a.0.cmp(&b.0));
    Value::Object(members)
}

/// Canonical JSON and an LF: one line of standard output.
fn line_of(v: &Value) -> String {
    format!("{}\n", canon::canonicalize(v))
}

fn path_text(path: &Path) -> &str {
    path.to_str().expect("a UTF-8 path")
}

fn tempdir() -> TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

fn write(path: &Path, bytes: impl AsRef<[u8]>) {
    fs::write(path, bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
}

/// The names in a directory, sorted, with each file's length and
/// modification time: enough to see that nothing was created, removed or
/// written.
fn snapshot(dir: &Path) -> Vec<(String, u64, Option<SystemTime>)> {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|entry| {
            let entry = entry.unwrap();
            let meta = entry.metadata().unwrap();
            (
                entry.file_name().to_string_lossy().into_owned(),
                meta.len(),
                meta.modified().ok(),
            )
        })
        .collect();
    entries.sort();
    entries
}

fn names(dir: &Path) -> Vec<String> {
    snapshot(dir).into_iter().map(|(name, _, _)| name).collect()
}

// ---------------------------------------------------------------------------
// Test vectors

fn vectors() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-vectors")
}

/// The store key of the vectors (test-only).
fn seed() -> PathBuf {
    vectors().join("keys/store-demo.seed")
}

/// Another test key: not the store key of any store here.
fn other_seed() -> PathBuf {
    vectors().join("keys/assessor-01.seed")
}

/// The directory of object vector `number` (`"003"`).
fn case(number: &str) -> PathBuf {
    let dir = vectors().join("objects");
    let mut found: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(&format!("{number}-")))
        })
        .collect();
    assert_eq!(found.len(), 1, "object vector {number}");
    found.remove(0)
}

fn object_file(number: &str) -> PathBuf {
    case(number).join("object.json")
}

fn attestation_file(number: &str) -> PathBuf {
    case(number).join("attestations.jsonl")
}

/// `expect` of an object vector's `case.json`.
fn expected(number: &str) -> Value {
    at(&read_json(&case(number).join("case.json")), &["expect"]).clone()
}

fn id_of(number: &str) -> String {
    text(&expected(number), "id")
}

/// The canonical object of a vector, as a value.
fn canonical_object(number: &str) -> Value {
    parse(&api::canonicalize(&fs::read(object_file(number)).unwrap()).unwrap())
}

/// The lines of a vector's attestation file that are not blank.
fn attestation_lines(number: &str) -> Vec<String> {
    fs::read_to_string(attestation_file(number))
        .unwrap()
        .split('\n')
        .filter(|l| !l.trim().is_empty())
        .map(str::to_string)
        .collect()
}

/// An attestation's ref: the hash of its canonical line.
fn attestation_ref(line: &str) -> String {
    canon::sha256_hex(api::canonicalize(line.as_bytes()).unwrap().as_bytes())
}

/// The export cases, in name order.
fn export_cases() -> Vec<PathBuf> {
    let mut cases: Vec<PathBuf> = fs::read_dir(vectors().join("store/exports"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .collect();
    cases.sort();
    cases
}

fn case_name(dir: &Path) -> &str {
    dir.file_name().and_then(|n| n.to_str()).unwrap()
}

// ---------------------------------------------------------------------------
// Stores and objects

/// `store init <dir> --name <name>` with the vectors' store key: exit 0.
fn init(dir: &Path, name: &str) -> Value {
    store(args!["init", dir, "--name", name, "--seed", seed()])
        .expect(0)
        .value()
}

/// `store append <dir> <file>` of one file that must be accepted.
fn append_one(dir: &Path, file: &Path) -> Value {
    store(args!["append", dir, file]).expect(0).value()
}

/// The head seq, from `store verify` (which must be ok).
fn head_seq(dir: &Path) -> u64 {
    let report = store(args!["verify", dir]).expect(0).value();
    num(at(&report, &["head"]), "seq")
}

/// Every line of the log, as `store log` prints it.
fn whole_log(dir: &Path) -> Run {
    store(args!["log", dir, "--limit", "9007199254740991"]).expect(0)
}

/// What a receipt line must hold.
struct Receipt<'a> {
    file: &'a str,
    kind: &'a str,
    reference: &'a str,
    seq: u64,
    duplicate: bool,
    line: Option<u64>,
}

impl Receipt<'_> {
    /// Assert that `v` is this receipt: exactly these members, plus an
    /// entry hash and a received time (not compared).
    fn check(&self, v: &Value) {
        let mut members: Vec<&str> = v
            .as_object()
            .expect("an object")
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        members.sort();
        let mut want = vec![
            "duplicate",
            "entry_hash",
            "file",
            "kind",
            "ok",
            "received_at",
            "ref",
            "seq",
        ];
        if self.line.is_some() {
            want.push("line");
            want.sort();
        }
        assert_eq!(members, want, "{}", canon::canonicalize(v));
        assert_eq!(v.get("ok"), Some(&Value::Bool(true)));
        assert_eq!(v.get("duplicate"), Some(&Value::Bool(self.duplicate)));
        assert_eq!(text(v, "file"), self.file);
        assert_eq!(text(v, "kind"), self.kind);
        assert_eq!(text(v, "ref"), self.reference);
        assert_eq!(num(v, "seq"), self.seq, "{}", canon::canonicalize(v));
        if let Some(line) = self.line {
            assert_eq!(num(v, "line"), line);
        }
    }
}

/// The rejected line `{"code","file",["line",]"ok":false}`.
fn rejected_line(code: &str, file: &str, line: Option<u64>) -> Value {
    let mut members = vec![
        ("code", string(code)),
        ("file", string(file)),
        ("ok", Value::Bool(false)),
    ];
    if let Some(n) = line {
        members.push(("line", number(n)));
    }
    obj(&members)
}

/// A sealed synthetic object in canonical form, and its id: subject
/// (`cli-test`, `item`, `key`), schema `cli-test/<schema>@1`, inline
/// `{"n":n}`, `at` 2026-11-26T00:00:00Z plus `n` seconds.
fn synthetic(op: &str, schema: &str, key: &str, n: u64, prev: &[&str]) -> (String, String) {
    sealed_object("cli-test", op, schema, key, n, prev)
}

/// As [`synthetic`], about a subject of another system.
fn sealed_object(
    system: &str,
    op: &str,
    schema: &str,
    key: &str,
    n: u64,
    prev: &[&str],
) -> (String, String) {
    let schema_ref = format!("cli-test/{schema}@1");
    let inline = format!("{{\"n\":{n}}}");
    let at = evidence_core::time::format_secs(
        evidence_core::time::parse("2026-11-26T00:00:00Z")
            .unwrap()
            .secs
            + n as i64,
    );
    let prev: Vec<String> = prev.iter().map(|p| format!("\"{p}\"")).collect();
    let unsealed = format!(
        "{{\"v\":\"evidence/0\",\"op\":\"{op}\",\
         \"schema\":{{\"ref\":\"{schema_ref}\",\"hash\":\"{}\"}},\
         \"subject\":{{\"system\":\"{system}\",\"type\":\"item\",\"key\":\"{key}\"}},\
         \"content\":{{\"media_type\":\"application/json\",\"hash\":\"{}\",\"inline\":{inline}}},\
         \"actor\":{{\"id\":\"service:cli-test\",\"kind\":\"service\"}},\
         \"at\":\"{at}\",\"prev\":[{}]}}",
        canon::sha256_hex(schema_ref.as_bytes()),
        api::content_hash(inline.as_bytes()).unwrap(),
        prev.join(",")
    );
    let sealed = api::seal(unsealed.as_bytes()).unwrap();
    let checked = object::check(sealed.as_bytes()).expect("a valid synthetic object");
    (sealed, checked.id)
}

/// Write a synthetic object to `<dir>/<name>`; give its path and id.
fn synthetic_file(
    dir: &Path,
    name: &str,
    op: &str,
    key: &str,
    n: u64,
    prev: &[&str],
) -> (PathBuf, String) {
    let (bytes, id) = synthetic(op, "item", key, n, prev);
    let path = dir.join(name);
    write(&path, bytes);
    (path, id)
}

/// Replace the last hex digit of a digest's text: `0` by `1`, any other by
/// `0`.
fn changed(digest: &str) -> String {
    let (head, last) = digest.split_at(digest.len() - 1);
    format!("{head}{}", if last == "0" { "1" } else { "0" })
}

fn digest(text: &str) -> Digest {
    merkle::from_text(text).unwrap_or_else(|| panic!("not a digest: {text}"))
}

fn digests(v: &Value) -> Vec<Digest> {
    v.as_array()
        .expect("an array")
        .iter()
        .map(|d| digest(d.as_str().unwrap()))
        .collect()
}

// ---------------------------------------------------------------------------
// T3

/// T3, steps 1-8, on a store made by `store init <dir> --name t3 --seed
/// test-vectors/keys/store-demo.seed`.
#[test]
fn t3_object_vectors_through_a_cli_store() {
    let tmp = tempdir();
    let s = tmp.path().join("s");
    let genesis = init(&s, "t3");
    assert_eq!(text(&genesis, "name"), "t3");
    assert_eq!(num(at(&genesis, &["head"]), "seq"), 2);
    let keys = read_json(&vectors().join("keys/public.json"));
    assert_eq!(text(&genesis, "key"), text(&keys, "store-demo"));
    let id003 = id_of("003");
    let id001 = id_of("001");

    // 1. 001-005 at seqs 3-7.
    for (i, number) in ["001", "002", "003", "004", "005"].iter().enumerate() {
        let file = object_file(number);
        Receipt {
            file: path_text(&file),
            kind: "object",
            reference: &id_of(number),
            seq: i as u64 + 3,
            duplicate: false,
            line: None,
        }
        .check(&append_one(&s, &file));
    }

    // 2. No attestation yet: case 113's code.
    let got = store(args!["get", s, id003, "--require", "approve"])
        .expect(1)
        .value();
    assert_eq!(
        at(&got, &["report", "code"]),
        at(&expected("113"), &["code"])
    );
    assert_eq!(text(at(&got, &["report"]), "code"), "E_ATTESTATION_MISSING");
    assert_eq!(at(&got, &["report", "ok"]), &Value::Bool(false));
    assert_eq!(at(&got, &["attestations"]), &Value::Array(Vec::new()));

    // 3. 003's attestation file: seqs 8-10.
    let file = attestation_file("003");
    let lines = store(args!["attest", s, file]).expect(0).lines();
    let written = attestation_lines("003");
    assert_eq!(lines.len(), 3);
    for (i, line) in lines.iter().enumerate() {
        Receipt {
            file: path_text(&file),
            kind: "attestation",
            reference: &attestation_ref(&written[i]),
            seq: i as u64 + 8,
            duplicate: false,
            line: Some(i as u64 + 1),
        }
        .check(line);
    }
    let attested = store(args!["log", s, "--from", "8", "--limit", "3"])
        .expect(0)
        .lines();
    let attested: Vec<(String, String)> = attested
        .iter()
        .map(|l| {
            let a = at(l, &["attestation"]);
            (text(a, "object"), text(a, "purpose"))
        })
        .collect();
    assert_eq!(
        attested,
        [
            (id003.clone(), "author".to_string()),
            (id003.clone(), "approve".to_string()),
            (id001.clone(), "author".to_string()),
        ]
    );

    // 4. Every 1xx object.
    for n in 101..=115 {
        let number = n.to_string();
        let file = object_file(&number);
        let run = store(args!["append", s, file]);
        match n {
            112 | 113 | 115 => Receipt {
                file: path_text(&file),
                kind: "object",
                reference: &id003,
                seq: 5,
                duplicate: true,
                line: None,
            }
            .check(&run.expect(0).value()),
            _ => {
                let code = text(&expected(&number), "code");
                match n {
                    110 => assert_eq!(code, "E_PREV_MISSING"),
                    111 => assert_eq!(code, "E_PREV_ORDER"),
                    _ => {}
                }
                let run = run.expect(1);
                assert_eq!(
                    run.stdout,
                    line_of(&rejected_line(&code, path_text(&file), None)),
                    "{number}"
                );
            }
        }
    }

    // 5. The attestation files of 112, 113 and 115.
    let file = attestation_file("112");
    let lines = store(args!["attest", s, file]).expect(1).lines();
    assert_eq!(lines.len(), 2);
    Receipt {
        file: path_text(&file),
        kind: "attestation",
        reference: &attestation_ref(&written[0]),
        seq: 8,
        duplicate: true,
        line: Some(1),
    }
    .check(&lines[0]);
    assert_eq!(
        lines[1],
        rejected_line("E_SIGNATURE_INVALID", path_text(&file), Some(2))
    );
    let file = attestation_file("113");
    Receipt {
        file: path_text(&file),
        kind: "attestation",
        reference: &attestation_ref(&written[0]),
        seq: 8,
        duplicate: true,
        line: Some(1),
    }
    .check(&store(args!["attest", s, file]).expect(0).value());
    let file = attestation_file("115");
    let run = store(args!["attest", s, file]).expect(1);
    assert_eq!(
        run.stdout,
        line_of(&rejected_line(
            "E_ATTESTATION_SHAPE",
            path_text(&file),
            Some(1)
        ))
    );

    // 6. Approved now.
    let got = store(args!["get", s, id003, "--require", "approve"])
        .expect(0)
        .value();
    let report = at(&got, &["report"]);
    assert_eq!(report.get("ok"), Some(&Value::Bool(true)));
    assert_eq!(
        at(report, &["attestations"]),
        &obj(&[("approve", number(1)), ("author", number(1))])
    );
    assert_eq!(at(&got, &["object"]), &canonical_object("003"));
    assert_eq!(num(at(&got, &["entry"]), "seq"), 5);
    assert_eq!(text(at(&got, &["entry"]), "ref"), id003);
    let log = whole_log(&s).lines();
    assert_eq!(at(&got, &["entry"]), at(&log[4], &["entry"]));
    let held = at(&got, &["attestations"]).as_array().unwrap();
    assert_eq!(held.len(), 2);
    for (i, a) in held.iter().enumerate() {
        assert_eq!(a, &log[7 + i], "the log line of seq {}", 8 + i);
        assert_eq!(at(a, &["attestation"]), &parse(&written[i]));
    }
    // The same report as `evidence verify` of the files.
    let direct = evidence(args![
        "verify",
        object_file("003"),
        "--attestations",
        attestation_file("003"),
        "--require",
        "approve"
    ])
    .expect(0)
    .value();
    assert_eq!(report, &direct);

    // 7. Verified.
    let report = store(args!["verify", s]).expect(0).value();
    assert_eq!(report.get("ok"), Some(&Value::Bool(true)));
    assert_eq!(text(&report, "source"), "store");
    assert_eq!(
        at(&report, &["checked"]),
        &obj(&[
            ("attestations", number(4)),
            ("checkpoints", number(0)),
            ("entries", number(10)),
            ("objects", number(6)),
        ])
    );
    assert_eq!(num(at(&report, &["head"]), "seq"), 10);

    // 8. A checkpoint over seqs 1-10.
    let cp = store(args!["checkpoint", s, "--seed", seed()])
        .expect(0)
        .value();
    assert_eq!(num(&cp, "size"), 10);
    assert_eq!(num(&cp, "seq"), 11);
    let report = store(args!["verify", s]).expect(0).value();
    assert_eq!(num(at(&report, &["covered"]), "size"), 10);
    assert_eq!(
        text(at(&report, &["covered"]), "checkpoint"),
        text(&cp, "checkpoint")
    );
    assert_eq!(num(at(&report, &["checked"]), "checkpoints"), 1);
    assert_eq!(num(at(&report, &["head"]), "seq"), 12);
}

// ---------------------------------------------------------------------------
// Section 8: the command line

/// Command lines that are usage errors, for a store (or path) `d`. `t` is
/// a path that must not be created, `f` a file name that need not exist.
fn usage_errors(d: &Path, t: &Path, f: &Path) -> Vec<Vec<OsString>> {
    let id = id_of("001");
    let bad_id = id.to_uppercase();
    vec![
        // An unknown command.
        args!["frobnicate", d],
        // An option missing its value.
        args!["log", d, "--limit"],
        args!["init", d, "--name", "x", "--seed"],
        // An option the command does not take, for every command.
        args!["init", d, "--name", "x", "--seed", seed(), "--bogus", "1"],
        args!["append", d, f, "--lane", "record"],
        args!["attest", d, f, "--if-heads", "none"],
        args!["checkpoint", d, "--seed", seed(), "--name", "x"],
        args!["get", d, id, "--lane", "record"],
        args!["heads", d, "a", "b", "c", "--from", "1"],
        args!["log", d, "--lane", "record"],
        args!["verify", d, "--verbose", "1"],
        args!["export", d, "--out", t, "--seq", "1"],
        args!["import", d, "--into", t, "--out", t],
        args!["prove", d, "--seq", "1", "--lane", "record"],
        // An option given twice.
        args!["log", d, "--limit", "2", "--limit", "3"],
        args!["init", d, "--name", "a", "--name", "b", "--seed", seed()],
        args!["export", d, "--out", t, "--out", t],
        // A missing required option.
        args!["init", d, "--name", "x"],
        args!["init", d, "--seed", seed()],
        args!["checkpoint", d],
        args!["export", d],
        args!["import", d],
        // Too few or too many positional arguments.
        args!["init", "--name", "x", "--seed", seed()],
        args!["append", d],
        args!["attest", d],
        args!["get", d],
        args!["get", d, id, "extra"],
        args!["heads", d, "a", "b"],
        args!["heads", d, "a", "b", "c", "e"],
        args!["log"],
        args!["log", d, "extra"],
        args!["verify", d, "extra"],
        args!["checkpoint", d, "extra", "--seed", seed()],
        args!["export", d, "extra", "--out", t],
        args!["import", d, "extra", "--into", t],
        args!["prove", d, "extra", "--seq", "1"],
        // Numbers.
        args!["log", d, "--limit", "0"],
        args!["log", d, "--from", "x"],
        args!["log", d, "--from", "-1"],
        args!["log", d, "--from", "9007199254740992"],
        args!["log", d, "--limit", "12345678901234567"],
        args!["verify", d, "--max-findings", "-1"],
        args!["verify", d, "--max-findings", "1.5"],
        args!["export", d, "--out", t, "--segment-size", "0"],
        args!["prove", d, "--seq", "+1"],
        // --if-heads.
        args!["append", d, f, f, "--if-heads", "none"],
        args!["append", d, f, "--if-heads", "nothing"],
        args!["append", d, f, "--if-heads", ""],
        args!["append", d, f, "--if-heads", format!("{id},")],
        args!["append", d, f, "--if-heads", bad_id],
        // Ids and purposes.
        args!["get", d, "not-a-hash"],
        args!["get", d, bad_id],
        args!["get", d, id, "--require", "sponsor"],
        args!["get", d, id, "--require", "approve", "--require", ""],
        // prove: exactly one form.
        args!["prove", d],
        args!["prove", d, "--seq", "1", "--from", "1"],
        args!["prove", d, "--size", "3"],
        args!["prove", d, "--from", "1"],
        args!["prove", d, "--to", "2"],
        args!["prove", d, "--seq", "1", "--size", "2", "--to", "3"],
        args!["prove", d, "--from", "1", "--to", "2", "--size", "2"],
    ]
}

/// Section 8, usage errors: exit 2, exactly `{"code":"E_USAGE","ok":false}`
/// on standard output, the usage text on standard error, and nothing
/// created or changed, against a store and against a path that does not
/// exist. `evidence store help` prints the usage text and exits 0.
#[test]
fn usage_errors_exit_2_and_touch_nothing() {
    let tmp = tempdir();
    let s = tmp.path().join("s");
    init(&s, "usage");
    append_one(&s, &object_file("001"));
    // No writer holds the lease now; without LOCK, one that appears shows
    // that a command took the lease.
    fs::remove_file(s.join("LOCK")).unwrap();
    let log = whole_log(&s).stdout;
    let missing = tmp.path().join("missing");
    let target = tmp.path().join("target");
    let file = tmp.path().join("object.json");
    let before = (snapshot(tmp.path()), snapshot(&s));

    let alone = store(args![]).expect(2);
    assert_eq!(alone.stdout, code_line("E_USAGE"));
    let mut count = 0;
    for d in [&s, &missing] {
        for args in usage_errors(d, &target, &file) {
            let run = store(args.clone()).expect(2);
            assert_eq!(run.stdout, code_line("E_USAGE"), "{args:?}");
            assert!(run.stderr.contains("\nUSAGE:\n"), "{args:?}: {run:#?}");
            count += 1;
        }
    }
    assert!(count > 100);
    assert_eq!((snapshot(tmp.path()), snapshot(&s)), before);
    assert!(!missing.exists() && !target.exists());
    assert!(!s.join("LOCK").exists());
    assert_eq!(whole_log(&s).stdout, log);

    for help in ["help", "-h", "--help"] {
        let run = evidence(args!["store", help]).expect(0);
        assert!(run.stdout.starts_with("evidence: "), "{run:#?}");
        assert!(run.stdout.ends_with('\n'));
        assert!(run.stderr.is_empty());
        for command in [
            "init",
            "append",
            "attest",
            "checkpoint",
            "get",
            "heads",
            "log",
            "verify",
            "export",
            "import",
            "prove",
        ] {
            assert!(
                run.stdout.contains(&format!("evidence store {command} ")),
                "{command}"
            );
        }
    }
}

/// Section 8, operation errors: exit 2 with the code line last on standard
/// output. `S_NOT_A_STORE`, `S_EXISTS`, `S_STORE_KEY`, `S_NOT_HELD`,
/// `S_IO` (a file that cannot be read, mid-run; a seed file that is
/// missing), and `E_USAGE` from the library (a store name, a seed's
/// content, a proof position).
#[test]
fn operation_errors_print_their_code_last() {
    let tmp = tempdir();
    let t = tmp.path();
    let s = t.join("s");
    init(&s, "errors");

    // S_NOT_A_STORE: an empty directory, for every command that opens one.
    let empty = t.join("empty");
    fs::create_dir(&empty).unwrap();
    let out = t.join("out");
    let x = object_file("001");
    for args in [
        args!["append", empty, x],
        args!["attest", empty, attestation_file("003")],
        args!["checkpoint", empty, "--seed", seed()],
        args!["get", empty, id_of("001")],
        args!["heads", empty, "a", "b", "c"],
        args!["log", empty],
        args!["verify", empty],
        args!["export", empty, "--out", out],
        args!["prove", empty, "--seq", "1", "--size", "1"],
        args!["import", empty, "--into", out],
        args!["verify", t.join("absent")],
    ] {
        let run = store(args.clone()).expect(2);
        assert_eq!(run.stdout, code_line("S_NOT_A_STORE"), "{args:?}");
    }
    assert!(names(&empty).is_empty(), "nothing created in a non-store");
    assert!(!out.exists());

    // A log.db that is not a store wins over a manifest.json beside it.
    let both = t.join("both");
    fs::create_dir(&both).unwrap();
    let export = export_cases().remove(0);
    for name in ["manifest.json", "000001.jsonl"] {
        fs::copy(export.join(name), both.join(name)).unwrap();
    }
    write(&both.join("log.db"), "this is not a database, only text\n");
    let run = store(args!["verify", both]).expect(2);
    assert_eq!(run.stdout, code_line("S_NOT_A_STORE"));
    fs::remove_file(both.join("log.db")).unwrap();
    let run = store(args!["verify", both]).expect(0);
    assert_eq!(
        run.stdout,
        line_of(at(&read_json(&export.join("case.json")), &["expect"]))
    );

    // A manifest.json that is not JSON: a report that is not ok (exit 1),
    // and an import that writes nothing.
    let not_json = t.join("not-json");
    fs::create_dir(&not_json).unwrap();
    write(&not_json.join("manifest.json"), "{");
    let report = store(args!["verify", not_json]).expect(1).value();
    assert_eq!(
        text(&at(&report, &["findings"]).as_array().unwrap()[0], "code"),
        "S_MANIFEST_INVALID"
    );
    let run = store(args!["import", not_json, "--into", out]).expect(1);
    assert_eq!(run.value(), report);
    assert!(!out.exists());

    // S_EXISTS.
    let run = store(args!["init", s, "--name", "again", "--seed", seed()]).expect(2);
    assert_eq!(run.stdout, code_line("S_EXISTS"));
    let file = t.join("file");
    write(&file, "x");
    let run = store(args!["init", file, "--name", "x", "--seed", seed()]).expect(2);
    assert_eq!(run.stdout, code_line("S_EXISTS"));
    let run = store(args!["export", s, "--out", empty]).expect(2);
    assert_eq!(run.stdout, code_line("S_EXISTS"));
    let run = store(args!["export", s, "--out", file]).expect(2);
    assert_eq!(run.stdout, code_line("S_EXISTS"));
    let run = store(args!["import", export, "--into", empty]).expect(2);
    assert_eq!(run.stdout, code_line("S_EXISTS"));
    assert!(names(&empty).is_empty());

    // Seeds: S_IO when missing, E_USAGE when not 64 hex digits, S_STORE_KEY
    // when another key. Nothing is created or written.
    let fresh = t.join("fresh");
    let bad_seed = t.join("bad.seed");
    write(&bad_seed, "not a seed\n");
    let short_seed = t.join("short.seed");
    write(&short_seed, "abcd\n");
    for (seed_file, code) in [
        (t.join("no.seed"), "S_IO"),
        (bad_seed.clone(), "E_USAGE"),
        (short_seed, "E_USAGE"),
    ] {
        let run = store(args!["init", fresh, "--name", "x", "--seed", seed_file]).expect(2);
        assert_eq!(run.stdout, code_line(code));
        let run = store(args!["checkpoint", s, "--seed", seed_file]).expect(2);
        assert_eq!(run.stdout, code_line(code));
    }
    assert!(!fresh.exists());
    let before = whole_log(&s).stdout;
    let run = store(args!["checkpoint", s, "--seed", other_seed()]).expect(2);
    assert_eq!(run.stdout, code_line("S_STORE_KEY"));
    assert_eq!(whole_log(&s).stdout, before, "nothing written");

    // E_USAGE from the library: a name that is not a store name.
    let long = "x".repeat(64);
    for name in ["Bad", "bad name", "", "-x", long.as_str()] {
        let run = store(args!["init", fresh, "--name", name, "--seed", seed()]).expect(2);
        assert_eq!(run.stdout, code_line("E_USAGE"), "{name:?}");
    }
    assert!(!fresh.exists());

    // S_IO mid-run: a's receipt, then the code line naming the file; b is
    // not read.
    let missing = t.join("missing.json");
    let run = store(args![
        "append",
        s,
        object_file("001"),
        missing,
        object_file("002")
    ])
    .expect(2);
    let lines = run.lines();
    assert_eq!(lines.len(), 2);
    Receipt {
        file: path_text(&object_file("001")),
        kind: "object",
        reference: &id_of("001"),
        seq: 3,
        duplicate: false,
        line: None,
    }
    .check(&lines[0]);
    assert_eq!(
        lines[1],
        obj(&[
            ("code", string("S_IO")),
            ("file", string(path_text(&missing))),
            ("ok", Value::Bool(false)),
        ])
    );
    assert_eq!(head_seq(&s), 3, "a stays, b is not appended");
    let run = store(args!["attest", s, missing]).expect(2);
    assert_eq!(
        run.stdout,
        line_of(&obj(&[
            ("code", string("S_IO")),
            ("file", string(path_text(&missing))),
            ("ok", Value::Bool(false)),
        ]))
    );
    // A directory is not a file that can be read.
    let run = store(args!["append", s, empty]).expect(2);
    assert_eq!(text(&run.value(), "code"), "S_IO");

    // S_NOT_HELD: a well-formed id the store holds no object for, an
    // attestation's ref included.
    let none = format!("sha256:{}", "0".repeat(64));
    let run = store(args!["get", s, none]).expect(2);
    assert_eq!(run.stdout, code_line("S_NOT_HELD"));
    let genesis_author = whole_log(&s).lines().remove(1);
    let reference = text(at(&genesis_author, &["entry"]), "ref");
    let run = store(args!["get", s, reference]).expect(2);
    assert_eq!(run.stdout, code_line("S_NOT_HELD"));

    // E_USAGE from the library: proof positions outside the log, and no
    // checkpoint for the default size.
    for args in [
        args!["prove", s, "--seq", "1"],
        args!["prove", s, "--seq", "4", "--size", "3"],
        args!["prove", s, "--seq", "1", "--size", "4"],
        args!["prove", s, "--from", "3", "--to", "3"],
        args!["prove", s, "--from", "3", "--to", "2"],
        args!["prove", s, "--from", "1", "--to", "4"],
    ] {
        let run = store(args.clone()).expect(2);
        assert_eq!(run.stdout, code_line("E_USAGE"), "{args:?}");
    }
    store(args!["prove", s, "--seq", "3", "--size", "3"]).expect(0);
    store(args!["prove", s, "--from", "2", "--to", "3"]).expect(0);
}

/// Section 8, mixed runs: a rejection does not stop a run; the run exits
/// 1 and every accepted item is held at consecutive seqs. Text that is not
/// JSON is a rejection too.
#[test]
fn rejections_do_not_stop_a_run_and_exit_1() {
    let tmp = tempdir();
    let t = tmp.path();
    let s = t.join("s");
    init(&s, "mixed");

    let (a, bad, b) = (object_file("001"), object_file("101"), object_file("004"));
    let lines = store(args!["append", s, a, bad, b]).expect(1).lines();
    assert_eq!(lines.len(), 3);
    Receipt {
        file: path_text(&a),
        kind: "object",
        reference: &id_of("001"),
        seq: 3,
        duplicate: false,
        line: None,
    }
    .check(&lines[0]);
    assert_eq!(
        lines[1],
        rejected_line("E_ID_MISMATCH", path_text(&bad), None)
    );
    Receipt {
        file: path_text(&b),
        kind: "object",
        reference: &id_of("004"),
        seq: 4,
        duplicate: false,
        line: None,
    }
    .check(&lines[2]);
    assert_eq!(head_seq(&s), 4);

    // Lines [valid, invalid, blank, valid with CR LF], numbered as in the
    // file.
    let author001 = attestation_lines("003").remove(2);
    let signed = evidence(args![
        "sign",
        object_file("004"),
        "--seed",
        vectors().join("keys/rec-registry-cdc.seed"),
        "--signer",
        "service:rec-registry-cdc",
        "--purpose",
        "author",
        "--at",
        "2026-11-26T00:00:00Z"
    ])
    .expect(0)
    .stdout;
    let signed = signed.trim_end();
    let file = t.join("mixed.jsonl");
    write(
        &file,
        format!("{author001}\n{{\"v\":\"attestation/0\"}}\n  \t\n{signed}\r\n"),
    );
    let lines = store(args!["attest", s, file]).expect(1).lines();
    assert_eq!(lines.len(), 3);
    Receipt {
        file: path_text(&file),
        kind: "attestation",
        reference: &attestation_ref(&author001),
        seq: 5,
        duplicate: false,
        line: Some(1),
    }
    .check(&lines[0]);
    assert_eq!(
        lines[1],
        rejected_line("E_ATTESTATION_SHAPE", path_text(&file), Some(2))
    );
    Receipt {
        file: path_text(&file),
        kind: "attestation",
        reference: &attestation_ref(signed),
        seq: 6,
        duplicate: false,
        line: Some(4),
    }
    .check(&lines[2]);
    assert_eq!(head_seq(&s), 6);

    // An attestation whose object is not held.
    let file = attestation_file("003");
    let lines = store(args!["attest", s, file]).expect(1).lines();
    assert_eq!(
        lines[..2],
        [
            rejected_line("S_ATTESTATION_TARGET", path_text(&file), Some(1)),
            rejected_line("S_ATTESTATION_TARGET", path_text(&file), Some(2)),
        ]
    );
    assert_eq!(text(&lines[2], "ref"), attestation_ref(&author001));
    assert_eq!(lines[2].get("duplicate"), Some(&Value::Bool(true)));

    // Text that is not JSON.
    let broken = t.join("broken.json");
    write(&broken, "{\"v\":");
    let run = store(args!["append", s, broken]).expect(1);
    assert_eq!(
        run.stdout,
        line_of(&rejected_line("E_PARSE", path_text(&broken), None))
    );
    let broken = t.join("broken.jsonl");
    write(&broken, "not json\n");
    let run = store(args!["attest", s, broken]).expect(1);
    assert_eq!(
        run.stdout,
        line_of(&rejected_line(
            "E_ATTESTATION_SHAPE",
            path_text(&broken),
            Some(1)
        ))
    );
    // A subject only the store may write about, even the genesis it holds:
    // that check comes before the duplicate check.
    let reserved = t.join("reserved.json");
    write(
        &reserved,
        sealed_object("evidence-store", "create", "item", "x", 7, &[]).0,
    );
    let genesis = t.join("genesis.json");
    let first = whole_log(&s).lines().remove(0);
    write(&genesis, canon::canonicalize(at(&first, &["object"])));
    for file in [&reserved, &genesis] {
        let run = store(args!["append", s, file]).expect(1);
        assert_eq!(
            run.stdout,
            line_of(&rejected_line("S_RESERVED_SUBJECT", path_text(file), None))
        );
    }

    // A file with no line that is not blank prints nothing.
    let blank = t.join("blank.jsonl");
    write(&blank, "\n \r\n\t\n");
    let run = store(args!["attest", s, blank]).expect(0);
    assert!(run.stdout.is_empty());
    let empty = t.join("empty.jsonl");
    write(&empty, "");
    store(args!["attest", s, empty, blank]).expect(0);

    assert_eq!(head_seq(&s), 6, "a rejected item adds no entry");
}

/// Section 8, `--if-heads` and `store heads`.
#[test]
fn if_heads_and_heads() {
    let tmp = tempdir();
    let t = tmp.path();
    let s = t.join("s");
    init(&s, "heads");
    let heads = |key: &str, lane: Option<&str>| {
        let mut list = args!["heads", s, "cli-test", "item", key];
        if let Some(lane) = lane {
            list.extend(args!["--lane", lane]);
        }
        store(list).expect(0).stdout
    };
    let heads_line = |ids: &[&str], lane: &str| {
        line_of(&obj(&[
            (
                "heads",
                Value::Array(ids.iter().map(|id| string(id)).collect()),
            ),
            ("lane", string(lane)),
        ]))
    };
    assert_eq!(heads("k1", None), heads_line(&[], "record"));

    // A new subject: `none` lands, and then is stale.
    let (first, first_id) = synthetic_file(t, "first.json", "create", "k1", 1, &[]);
    let receipt = store(args!["append", s, first, "--if-heads", "none"])
        .expect(0)
        .value();
    assert_eq!(num(&receipt, "seq"), 3);
    let (second, _) = synthetic_file(t, "second.json", "create", "k1", 2, &[]);
    let run = store(args!["append", s, second, "--if-heads", "none"]).expect(1);
    assert_eq!(
        run.stdout,
        line_of(&obj(&[
            ("code", string("S_STALE_HEADS")),
            ("file", string(path_text(&second))),
            ("heads", Value::Array(vec![string(&first_id)])),
            ("ok", Value::Bool(false)),
        ]))
    );

    // An update with the current heads lands.
    let (update, update_id) = synthetic_file(t, "update.json", "update", "k1", 3, &[&first_id]);
    let receipt = store(args!["append", s, update, "--if-heads", first_id])
        .expect(0)
        .value();
    assert_eq!(num(&receipt, "seq"), 4);
    assert_eq!(heads("k1", None), heads_line(&[&update_id], "record"));

    // The same update again, with heads that are stale now: a duplicate,
    // since if_heads is not evaluated for a held item.
    let again = store(args!["append", s, update, "--if-heads", first_id])
        .expect(0)
        .value();
    assert_eq!(again.get("duplicate"), Some(&Value::Bool(true)));
    assert_eq!(num(&again, "seq"), 4);

    // Ids compared as a set: repeats allowed.
    let (next, next_id) = synthetic_file(t, "next.json", "update", "k1", 4, &[&update_id]);
    let repeated = format!("{update_id},{update_id}");
    let receipt = store(args!["append", s, next, "--if-heads", repeated])
        .expect(0)
        .value();
    assert_eq!(num(&receipt, "seq"), 5);
    assert_eq!(heads("k1", None), heads_line(&[&next_id], "record"));

    // Stale against a subject with no heads: `heads` is [].
    let (other, _) = synthetic_file(t, "other.json", "create", "k2", 5, &[]);
    let run = store(args!["append", s, other, "--if-heads", next_id]).expect(1);
    assert_eq!(
        run.stdout,
        line_of(&obj(&[
            ("code", string("S_STALE_HEADS")),
            ("file", string(path_text(&other))),
            ("heads", Value::Array(Vec::new())),
            ("ok", Value::Bool(false)),
        ]))
    );

    // An assert lane.
    let (note, note_id) = synthetic("assert", "note", "k1", 6, &[]);
    let note_file = t.join("note.json");
    write(&note_file, note);
    store(args!["append", s, note_file, "--if-heads", "none"]).expect(0);
    assert_eq!(
        heads("k1", Some("assert:cli-test/note")),
        heads_line(&[&note_id], "assert:cli-test/note")
    );
    assert_eq!(heads("k1", None), heads_line(&[&next_id], "record"));
    assert_eq!(
        heads("k1", Some("assert:cli-test/other")),
        heads_line(&[], "assert:cli-test/other")
    );

    // Any strings, and an unknown subject.
    assert_eq!(heads("unknown", None), heads_line(&[], "record"));
    let run = store(args!["heads", s, "", "", "", "--lane", ""]).expect(0);
    assert_eq!(run.stdout, heads_line(&[], ""));
    let genesis = store(args![
        "heads",
        s,
        "evidence-store",
        "log",
        "heads",
        "--lane",
        "assert:evidence-store/genesis"
    ])
    .expect(0)
    .value();
    assert_eq!(at(&genesis, &["heads"]).as_array().unwrap().len(), 1);
}

/// Section 8, `store log`: defaults, ranges, more than one chunk, and the
/// whole log equal to an export's segment files.
#[test]
fn log_prints_segment_lines() {
    let tmp = tempdir();
    let t = tmp.path();
    let s = t.join("s");
    init(&s, "log");
    let objects = t.join("objects");
    fs::create_dir(&objects).unwrap();
    let mut files = Vec::new();
    for n in 0..1100 {
        let name = format!("{n:04}.json");
        synthetic_file(&objects, &name, "create", &format!("item-{n}"), n, &[]);
        files.push(name);
    }
    let mut list = args!["append", s];
    list.extend(files.iter().map(OsString::from));
    let receipts = store_in(Some(&objects), list).expect(0).lines();
    assert_eq!(receipts.len(), 1100);
    assert_eq!(num(&receipts[1099], "seq"), 1102);

    // Defaults: from seq 1, at most 100.
    let first = store(args!["log", s]).expect(0);
    let lines = stdout_lines(&first.stdout);
    assert_eq!(lines.len(), 100);
    for (i, line) in first.lines().iter().enumerate() {
        assert_eq!(num(at(line, &["entry"]), "seq"), i as u64 + 1);
    }
    let same = store(args!["log", s, "--from", "001", "--limit", "100"]).expect(0);
    assert_eq!(same.stdout, first.stdout);
    let two = store(args!["log", s, "--from", "3", "--limit", "2"]).expect(0);
    assert_eq!(stdout_lines(&two.stdout), lines[2..4]);

    // Past the head: nothing.
    for from in ["1103", "9007199254740991"] {
        let run = store(args!["log", s, "--from", from]).expect(0);
        assert!(run.stdout.is_empty());
    }
    let run = store(args!["log", s, "--from", "1102", "--limit", "5"]).expect(0);
    assert_eq!(num(at(&run.value(), &["entry"]), "seq"), 1102);

    // More than one chunk of 1024.
    let run = store(args!["log", s, "--limit", "1100"]).expect(0);
    let many = run.lines();
    assert_eq!(many.len(), 1100);
    for (i, line) in many.iter().enumerate() {
        assert_eq!(num(at(line, &["entry"]), "seq"), i as u64 + 1);
    }
    let across = store(args!["log", s, "--from", "1023", "--limit", "4"]).expect(0);
    let seqs: Vec<u64> = across
        .lines()
        .iter()
        .map(|l| num(at(l, &["entry"]), "seq"))
        .collect();
    assert_eq!(seqs, [1023, 1024, 1025, 1026]);

    // The whole log is the concatenation of an export's segments.
    let whole = store(args!["log", s, "--limit", "1102"]).expect(0).stdout;
    assert_eq!(whole, whole_log(&s).stdout);
    let out = t.join("export");
    let manifest = store(args!["export", s, "--out", out, "--segment-size", "4"])
        .expect(0)
        .value();
    let segments = at(&manifest, &["segments"]).as_array().unwrap();
    assert_eq!(segments.len(), 276);
    let mut joined = String::new();
    for segment in segments {
        joined.push_str(&fs::read_to_string(out.join(text(segment, "file"))).unwrap());
    }
    assert_eq!(whole, joined);
}

// ---------------------------------------------------------------------------
// T7

/// T7, CLI part: while the test process holds the lease, writers get
/// `S_LOCKED` at once and readers work; once it is released, a writer
/// appends at the next epoch.
#[test]
fn lease_held_by_another_process_gives_s_locked() {
    let tmp = tempdir();
    let t = tmp.path();
    let s = t.join("s");
    init(&s, "lease");
    append_one(&s, &object_file("001"));
    store(args!["checkpoint", s, "--seed", seed()]).expect(0);
    let id001 = id_of("001");

    let writer = SqliteLog::open(&s, Box::new(SystemClock)).expect("the lease is free");
    let epoch = writer.epoch();
    for args in [
        args!["append", s, object_file("002")],
        args!["attest", s, attestation_file("003")],
        args!["checkpoint", s, "--seed", seed()],
    ] {
        let started = Instant::now();
        let run = store(args.clone()).expect(2);
        assert_eq!(run.stdout, code_line("S_LOCKED"), "{args:?}");
        assert!(run.stderr.starts_with("evidence: S_LOCKED"), "{run:#?}");
        assert!(started.elapsed() < Duration::from_secs(5), "no waiting");
    }
    // Readers take no lease.
    store(args!["get", s, id001]).expect(0);
    store(args![
        "heads",
        s,
        "rec-registry",
        "installation",
        "INS-TEST-000001"
    ])
    .expect(0);
    store(args!["log", s]).expect(0);
    store(args!["verify", s]).expect(0);
    store(args!["export", s, "--out", t.join("export")]).expect(0);
    store(args!["prove", s, "--seq", "3"]).expect(0);
    store(args!["prove", s, "--from", "2", "--to", "3"]).expect(0);
    evidence(args!["verify", object_file("002"), "--store", s]).expect(0);
    drop(writer);

    // A child the test harness is starting may hold a copy of the lease
    // until it calls exec, so retry for a while.
    let started = Instant::now();
    let receipt = loop {
        let run = store(args!["append", s, object_file("002")]);
        if run.code == 2 && run.stdout == code_line("S_LOCKED") {
            assert!(
                started.elapsed() < Duration::from_secs(2),
                "the lease is still held: {run:#?}"
            );
            thread::sleep(Duration::from_millis(20));
            continue;
        }
        break run.expect(0).value();
    };
    let seq = num(&receipt, "seq");
    let line = store(args!["log", s, "--from", seq.to_string(), "--limit", "1"])
        .expect(0)
        .value();
    assert_eq!(num(at(&line, &["entry"]), "epoch"), epoch + 1);
}

// ---------------------------------------------------------------------------
// T10

/// T10, round trip: export, verify the export, import, and the new store
/// has the old one's head and lines.
#[test]
fn export_import_round_trip_keeps_the_head() {
    let tmp = tempdir();
    let t = tmp.path();
    let s = t.join("s");
    init(&s, "round-trip");
    for number in ["001", "002", "003", "004", "005"] {
        append_one(&s, &object_file(number));
    }
    store(args!["attest", s, attestation_file("003")]).expect(0);
    store(args!["checkpoint", s, "--seed", seed()]).expect(0);
    append_one(
        &s,
        &synthetic_file(t, "late.json", "create", "late", 1, &[]).0,
    );

    let x = t.join("x");
    let run = store(args!["export", s, "--out", x]).expect(0);
    let manifest = fs::read_to_string(x.join("manifest.json")).unwrap();
    assert_eq!(run.stdout, format!("{manifest}\n"));
    let manifest = parse(&manifest);
    assert_eq!(num(&manifest, "segment_size"), 10_000);

    let old = store(args!["verify", s]).expect(0).value();
    let exported = store(args!["verify", x]).expect(0).value();
    assert_eq!(text(&exported, "source"), "export");
    assert_eq!(exported.without("source"), old.without("source"));
    assert_eq!(
        num(at(&old, &["head"]), "seq"),
        num(&manifest, "size"),
        "the export holds the whole log"
    );

    let y = t.join("y");
    let run = store(args!["import", x, "--into", y]).expect(0);
    let imported = run.value();
    assert_eq!(text(&imported, "source"), "store");
    assert_eq!(imported, old);
    let new = store(args!["verify", y]).expect(0);
    assert_eq!(new.stdout, run.stdout);
    assert_eq!(at(&new.value(), &["head"]), at(&old, &["head"]));
    assert_eq!(whole_log(&y).stdout, whole_log(&s).stdout);

    let run = store(args!["import", x, "--into", y]).expect(2);
    assert_eq!(run.stdout, code_line("S_EXISTS"));

    // The imported store takes appends at the next epoch.
    let max_epoch = whole_log(&s)
        .lines()
        .iter()
        .map(|l| num(at(l, &["entry"]), "epoch"))
        .max()
        .unwrap();
    let receipt = append_one(
        &y,
        &synthetic_file(t, "after.json", "create", "after", 2, &[]).0,
    );
    let line = store(args![
        "log",
        y,
        "--from",
        num(&receipt, "seq").to_string(),
        "--limit",
        "1"
    ])
    .expect(0)
    .value();
    assert_eq!(num(at(&line, &["entry"]), "epoch"), max_epoch + 1);

    // --out takes the next argument whatever it is: a directory `--x`.
    let manifest = store_in(
        Some(t),
        args!["export", s, "--out", "--x", "--segment-size", "3"],
    )
    .expect(0)
    .value();
    assert!(t.join("--x").join("manifest.json").is_file());
    assert_eq!(num(&manifest, "segment_size"), 3);
    assert_eq!(
        at(&manifest, &["segments"]).as_array().unwrap().len() as u64,
        num(&manifest, "size").div_ceil(3)
    );
}

/// T10, every export case: `store verify <case dir>` prints exactly
/// canonical(`expect`) + LF, and exits 0 for every 0xx case and 1 for every
/// 1xx case.
#[test]
fn verify_every_export_case() {
    let cases = export_cases();
    assert_eq!(cases.len(), 42, "7 valid and 35 tampered export cases");
    let (mut valid, mut tampered) = (0, 0);
    for dir in &cases {
        let case = read_json(&dir.join("case.json"));
        let mut list = args!["verify", dir];
        if case.get("max_findings").is_some() {
            list.extend(args![
                "--max-findings",
                num(&case, "max_findings").to_string()
            ]);
        }
        let run = store(list);
        let expect = at(&case, &["expect"]);
        assert_eq!(run.stdout, line_of(expect), "{}", case_name(dir));
        if case_name(dir).starts_with('0') {
            run.expect(0);
            valid += 1;
        } else {
            assert!(case_name(dir).starts_with('1'));
            run.expect(1);
            tampered += 1;
        }
    }
    assert_eq!((valid, tampered), (7, 35));
}

/// T10, imports that must not happen: every 1xx case exits 1 and leaves
/// nothing; every 0xx case imports, and its store exports the case's
/// files byte for byte.
#[test]
fn import_of_a_tampered_export_writes_nothing() {
    for dir in export_cases() {
        let name = case_name(&dir);
        let case = read_json(&dir.join("case.json"));
        let tmp = tempdir();
        let into = tmp.path().join("into");
        let run = store(args!["import", dir, "--into", into]);
        if name.starts_with('1') {
            let run = run.expect(1);
            let report = run.value();
            assert_eq!(report.get("ok"), Some(&Value::Bool(false)), "{name}");
            assert_eq!(text(&report, "source"), "export");
            if case.get("max_findings").is_none() {
                assert_eq!(run.stdout, line_of(at(&case, &["expect"])), "{name}");
            }
            assert!(names(tmp.path()).is_empty(), "{name} left files");
            continue;
        }
        let report = run.expect(0).value();
        assert_eq!(text(&report, "source"), "store");
        assert_eq!(
            canon::canonicalize(at(&report, &["head"])),
            canon::canonicalize(at(&case, &["expect", "head"]))
        );
        let manifest = read_json(&dir.join("manifest.json"));
        let again = tmp.path().join("again");
        store(args![
            "export",
            into,
            "--out",
            again,
            "--segment-size",
            num(&manifest, "segment_size").to_string()
        ])
        .expect(0);
        let mut files = names(&again);
        let mut want: Vec<String> = names(&dir)
            .into_iter()
            .filter(|n| n == "manifest.json" || n.ends_with(".jsonl"))
            .collect();
        files.sort();
        want.sort();
        assert_eq!(files, want, "{name}");
        for file in &files {
            assert_eq!(
                fs::read(again.join(file)).unwrap(),
                fs::read(dir.join(file)).unwrap(),
                "{name}/{file}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// T11

/// T11, Rust side: the fixed store of section 11; every inclusion proof up
/// to the latest checkpoint's size and up to the first's, and every
/// consistency proof, verifies with `evidence_log::merkle`, with the roots
/// of the entries `store log` prints and of the checkpoints; one changed
/// hex digit in a proof element, the leaf hash or a root fails.
#[test]
fn prove_output_verifies_and_a_changed_byte_does_not() {
    let tmp = tempdir();
    let s = tmp.path().join("s");
    init(&s, "proofs");
    for number in ["001", "002", "003", "004", "005"] {
        append_one(&s, &object_file(number));
    }
    store(args!["attest", s, attestation_file("003")]).expect(0);
    let checkpoints: Vec<Value> = (0..3)
        .map(|_| {
            store(args!["checkpoint", s, "--seed", seed()])
                .expect(0)
                .value()
        })
        .collect();
    let sizes: Vec<u64> = checkpoints.iter().map(|c| num(c, "size")).collect();
    assert_eq!(sizes, [10, 12, 14]);
    let log = whole_log(&s).lines();
    assert_eq!(log.len(), 16);
    let entries: Vec<String> = log
        .iter()
        .map(|l| canon::canonicalize(at(l, &["entry"])))
        .collect();
    let leaves: Vec<Digest> = entries
        .iter()
        .map(|e| merkle::leaf_hash(e.as_bytes()))
        .collect();
    let root = |n: u64| merkle::to_text(&merkle::root(&leaves[..n as usize]));
    for c in &checkpoints {
        assert_eq!(text(c, "root"), root(num(c, "size")));
    }

    // Inclusion: at the latest checkpoint's size (the default) and at the
    // first's.
    let mut checked = 0;
    for (size, c, upto) in [(None, &checkpoints[2], 14), (Some(10), &checkpoints[0], 10)] {
        for seq in 1..=upto {
            let mut list = args!["prove", s, "--seq", seq.to_string()];
            if let Some(size) = size {
                list.extend(args!["--size", size.to_string()]);
            }
            let proof = store(list).expect(0).value();
            let size = num(c, "size");
            assert_eq!(num(&proof, "seq"), seq);
            assert_eq!(num(&proof, "size"), size);
            assert_eq!(text(&proof, "root"), text(c, "root"));
            assert_eq!(text(&proof, "root"), root(size));
            assert_eq!(text(&proof, "checkpoint"), text(c, "checkpoint"));
            assert_eq!(
                at(&proof, &["entry"]),
                at(&log[seq as usize - 1], &["entry"])
            );
            let leaf = text(&proof, "leaf_hash");
            assert_eq!(
                leaf,
                merkle::to_text(&merkle::leaf_hash(
                    canon::canonicalize(at(&proof, &["entry"])).as_bytes()
                ))
            );
            let path = digests(at(&proof, &["proof"]));
            let root_text = text(&proof, "root");
            let verify = |leaf: &str, path: &[Digest], root: &str| {
                merkle::verify_inclusion(&digest(leaf), seq - 1, size, path, &digest(root))
            };
            assert!(verify(&leaf, &path, &root_text), "seq {seq} size {size}");
            assert!(!path.is_empty());
            let mut bent = path.clone();
            bent[0] = digest(&changed(&merkle::to_text(&path[0])));
            assert!(!verify(&leaf, &bent, &root_text));
            assert!(!verify(&changed(&leaf), &path, &root_text));
            assert!(!verify(&leaf, &path, &changed(&root_text)));
            checked += 1;
        }
    }
    assert_eq!(checked, 24);

    // Consistency: every 1 <= a < b <= 16.
    let mut checked = 0;
    for a in 1..=16u64 {
        for b in a + 1..=16 {
            let proof = store(args![
                "prove",
                s,
                "--from",
                a.to_string(),
                "--to",
                b.to_string()
            ])
            .expect(0)
            .value();
            assert_eq!((num(&proof, "old_size"), num(&proof, "new_size")), (a, b));
            let (old, new) = (text(&proof, "old_root"), text(&proof, "new_root"));
            assert_eq!(old, root(a));
            assert_eq!(new, root(b));
            let path = digests(at(&proof, &["proof"]));
            let verify = |old: &str, new: &str, path: &[Digest]| {
                merkle::verify_consistency(a, b, &digest(old), &digest(new), path)
            };
            assert!(verify(&old, &new, &path), "{a} to {b}");
            let mut bent = path.clone();
            bent[0] = digest(&changed(&merkle::to_text(&path[0])));
            assert!(!verify(&old, &new, &bent));
            assert!(!verify(&changed(&old), &new, &path));
            assert!(!verify(&old, &changed(&new), &path));
            checked += 1;
        }
    }
    assert_eq!(checked, 120);

    // With no checkpoint, --size is required.
    let bare = tmp.path().join("bare");
    init(&bare, "bare");
    let run = store(args!["prove", bare, "--seq", "1"]).expect(2);
    assert_eq!(run.stdout, code_line("E_USAGE"));
    let proof = store(args!["prove", bare, "--seq", "1", "--size", "2"])
        .expect(0)
        .value();
    assert!(proof.get("checkpoint").is_none());
}

// ---------------------------------------------------------------------------
// T12

/// T12: `evidence verify --store` gives the same bytes and status for a
/// store directory as for a folder holding the same objects; stored
/// attestations are not used; errors go to standard error only.
#[test]
fn verify_with_a_store_matches_the_folder_of_objects() {
    let tmp = tempdir();
    let t = tmp.path();
    let s = t.join("s");
    init(&s, "t12");
    append_one(&s, &object_file("001"));

    for (number, status) in [("002", 0), ("110", 1), ("111", 1)] {
        let file = object_file(number);
        let folder =
            evidence(args!["verify", file, "--store", case(number).join("store")]).expect(status);
        let with_store = evidence(args!["verify", file, "--store", s]).expect(status);
        assert_eq!(with_store.stdout, folder.stdout, "{number}");
        assert!(with_store.stderr.is_empty());
        let report = with_store.value();
        assert_eq!(
            canon::canonicalize(&report),
            stdout_lines(&folder.stdout)[0]
        );
        for (name, value) in expected(number).as_object().unwrap() {
            assert_eq!(report.get(name), Some(value), "{number}: {name}");
        }
    }

    // 003 with its attestations: the bytes it gives with no --store.
    let file = object_file("003");
    let attestations = attestation_file("003");
    let alone = evidence(args![
        "verify",
        file,
        "--attestations",
        attestations,
        "--require",
        "approve"
    ])
    .expect(0);
    let with_store = evidence(args![
        "verify",
        file,
        "--attestations",
        attestations,
        "--require",
        "approve",
        "--store",
        s
    ])
    .expect(0);
    assert_eq!(with_store.stdout, alone.stdout);

    // Stored attestations are not used: a store holding 003 and its
    // attestations, and no --attestations, gives case 113's code.
    append_one(&s, &object_file("003"));
    store(args!["attest", s, attestations]).expect(0);
    let run = evidence(args!["verify", file, "--require", "approve", "--store", s]).expect(1);
    assert_eq!(at(&run.value(), &["code"]), at(&expected("113"), &["code"]));

    // An unknown purpose: the folder form's message, exit 2, nothing on
    // standard output.
    let folder = evidence(args![
        "verify",
        object_file("002"),
        "--store",
        case("002").join("store"),
        "--require",
        "sponsor"
    ])
    .expect(2);
    let with_store = evidence(args![
        "verify",
        object_file("002"),
        "--store",
        s,
        "--require",
        "sponsor"
    ])
    .expect(2);
    assert!(folder.stdout.is_empty() && with_store.stdout.is_empty());
    assert_eq!(with_store.stderr, folder.stderr);
    assert!(with_store
        .stderr
        .starts_with("evidence: usage: purpose \"sponsor\" must be one of"));

    // A log.db that is not a store: exit 2, nothing on standard output.
    let broken = t.join("broken");
    fs::create_dir(&broken).unwrap();
    write(&broken.join("log.db"), "not a database\n");
    fs::copy(case("002").join("store/0.json"), broken.join("0.json")).unwrap();
    let run = evidence(args!["verify", object_file("002"), "--store", broken]).expect(2);
    assert!(run.stdout.is_empty());
    assert!(
        run.stderr
            .starts_with(&format!("evidence: {}: S_NOT_A_STORE", broken.display())),
        "{run:#?}"
    );
}
