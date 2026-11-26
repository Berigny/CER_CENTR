//! T6: crash tests (`docs/m1-store-spec.md` section 11). The built
//! `evidence` binary is killed with SIGKILL at random moments while it
//! appends and while it imports, and what it leaves must verify. Unix only:
//! `Child::kill` sends SIGKILL there.
//!
//! - `EVIDENCE_CRASH_ITERS`: append iterations (default 10; CI sets 50,
//!   the nightly run 1,000). The import test runs ⌈iters / 5⌉ iterations.
//! - `EVIDENCE_CRASH_SEED`: the seed of the random choices (decimal u64).
//!   When unset it is taken from the system time; either way it is printed
//!   to standard error at the start, so a failure can be replayed (the
//!   choices repeat; the kill timing within them depends on the machine).
//!
//! The objects are 2,000 synthetic evidence/0 objects, generated once per
//! test binary into a temporary directory (`0000.json` to `1999.json`),
//! which is removed when the last test using it ends. They are
//! deterministic: object i is about subject `item-<i mod 20>` (system
//! `crash-test`, type `item`), a `create` with `prev` [] for the first of
//! each subject and an `update` whose `prev` is that subject's previous
//! object afterwards, `at` = 2026-11-26T00:00:00Z + i seconds, inline
//! content `{"n":i}`, sealed with `evidence_core::api::seal` after
//! `content_hash`, and checked with `object::check`. No other data.
//!
//! The tests run only CLI processes against their stores; the test process
//! never opens a store itself.

#![cfg(unix)]

use std::ffi::OsString;
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, Weak};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use evidence_core::json::{self, Value};
use evidence_core::{api, canon, object, time};
use tempfile::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_evidence");

/// How many objects each test appends.
const OBJECTS: usize = 2000;

/// How many subjects the objects are spread over.
const SUBJECTS: usize = 20;

// ---------------------------------------------------------------------------
// Settings and random choices

/// `EVIDENCE_CRASH_ITERS`, or 10.
fn iterations() -> u64 {
    match std::env::var("EVIDENCE_CRASH_ITERS") {
        Ok(text) => text
            .trim()
            .parse()
            .unwrap_or_else(|_| panic!("EVIDENCE_CRASH_ITERS is not a count: {text:?}")),
        Err(_) => 10,
    }
}

/// `EVIDENCE_CRASH_SEED`, or the system time; printed so a run can be
/// repeated.
fn seed(test: &str) -> u64 {
    let seed = match std::env::var("EVIDENCE_CRASH_SEED") {
        Ok(text) => text
            .trim()
            .parse()
            .unwrap_or_else(|_| panic!("EVIDENCE_CRASH_SEED is not a u64: {text:?}")),
        Err(_) => SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64),
    };
    eprintln!("{test}: EVIDENCE_CRASH_SEED={seed}");
    seed
}

/// SplitMix64: small, fast, and the same everywhere.
struct Rng(u64);

impl Rng {
    /// A generator for one test: the seed mixed with the test's name, so
    /// the two tests make different choices from one seed.
    fn new(seed: u64, test: &str) -> Rng {
        let salt = test
            .bytes()
            .fold(0u64, |h, b| h.rotate_left(5) ^ u64::from(b));
        Rng(seed ^ salt)
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number from 0 to `n` − 1 (`n` ≥ 1).
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

// ---------------------------------------------------------------------------
// Objects

/// The objects: their directory, file names and ids.
struct Objects {
    /// Removed when the last test holding these objects drops them.
    _tmp: TempDir,
    dir: PathBuf,
    names: Vec<String>,
    ids: Vec<String>,
}

/// The objects, written once while any test holds them: tests that run at
/// the same time share one directory.
fn objects() -> Arc<Objects> {
    static SHARED: Mutex<Weak<Objects>> = Mutex::new(Weak::new());
    let mut shared = SHARED.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(objects) = shared.upgrade() {
        return objects;
    }
    let objects = Arc::new(write_objects());
    *shared = Arc::downgrade(&objects);
    objects
}

/// Write the 2,000 objects into a new temporary directory.
fn write_objects() -> Objects {
    let tmp = tempdir();
    let dir = tmp.path().join("objects");
    fs::create_dir(&dir).unwrap();
    let start = time::parse("2026-11-26T00:00:00Z").unwrap().secs;
    let schema = "crash-test/item@1";
    let schema_hash = canon::sha256_hex(schema.as_bytes());
    let mut last: Vec<Option<String>> = vec![None; SUBJECTS];
    let (mut names, mut ids) = (Vec::new(), Vec::new());
    for i in 0..OBJECTS {
        let subject = i % SUBJECTS;
        let (op, prev) = match &last[subject] {
            None => ("create", String::new()),
            Some(id) => ("update", format!("\"{id}\"")),
        };
        let inline = format!("{{\"n\":{i}}}");
        let content_hash = api::content_hash(inline.as_bytes()).unwrap();
        let at = time::format_secs(start + i as i64);
        let unsealed = format!(
            "{{\"v\":\"evidence/0\",\"op\":\"{op}\",\
             \"schema\":{{\"ref\":\"{schema}\",\"hash\":\"{schema_hash}\"}},\
             \"subject\":{{\"system\":\"crash-test\",\"type\":\"item\",\"key\":\"item-{subject}\"}},\
             \"content\":{{\"media_type\":\"application/json\",\"hash\":\"{content_hash}\",\"inline\":{inline}}},\
             \"actor\":{{\"id\":\"service:crash-test\",\"kind\":\"service\"}},\
             \"at\":\"{at}\",\"prev\":[{prev}]}}"
        );
        let sealed = api::seal(unsealed.as_bytes()).unwrap();
        let id = object::check(sealed.as_bytes())
            .expect("a valid synthetic object")
            .id;
        let name = format!("{i:04}.json");
        fs::write(dir.join(&name), &sealed).unwrap();
        last[subject] = Some(id.clone());
        names.push(name);
        ids.push(id);
    }
    Objects {
        _tmp: tmp,
        dir,
        names,
        ids,
    }
}

// ---------------------------------------------------------------------------
// Running the binary

/// Command-line arguments from anything that is a path or a string.
macro_rules! args {
    ($($arg:expr),* $(,)?) => {
        vec![$(OsString::from(AsRef::<std::ffi::OsStr>::as_ref(&$arg))),*]
    };
}

struct Run {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

impl Run {
    fn expect(self, code: i32) -> Run {
        assert_eq!(
            self.code,
            Some(code),
            "exit status; standard output {:?}; standard error:\n{}",
            self.stdout,
            self.stderr
        );
        self
    }

    fn lines(&self) -> Vec<Value> {
        complete_lines(self.stdout.as_bytes())
            .iter()
            .map(|l| parse(l))
            .collect()
    }

    fn value(&self) -> Value {
        let mut lines = self.lines();
        assert_eq!(lines.len(), 1, "one line expected: {:?}", self.stdout);
        lines.remove(0)
    }
}

/// `evidence store <args>`, run to its end in `cwd`.
fn store(cwd: &Path, args: Vec<OsString>) -> Run {
    let output = Command::new(BIN)
        .arg("store")
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .output()
        .expect("run the evidence binary");
    Run {
        code: output.status.code(),
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
}

/// `store init <dir> --name crash` with the vectors' store key.
fn init(dir: &Path) {
    let seed =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-vectors/keys/store-demo.seed");
    store(
        Path::new("."),
        args!["init", dir, "--name", "crash", "--seed", seed],
    )
    .expect(0);
}

/// `store verify <dir>`: exit 0, ok. Gives the report.
fn verify_ok(dir: &Path) -> Value {
    let report = store(Path::new("."), args!["verify", dir])
        .expect(0)
        .value();
    assert_eq!(report.get("ok"), Some(&Value::Bool(true)));
    report
}

/// The lines of `bytes` that end in LF; a final piece without one is cut
/// off and ignored.
fn complete_lines(bytes: &[u8]) -> Vec<String> {
    let end = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
    std::str::from_utf8(&bytes[..end])
        .expect("UTF-8 output")
        .lines()
        .map(str::to_string)
        .collect()
}

fn parse(text: &str) -> Value {
    json::parse(text.as_bytes()).unwrap_or_else(|e| panic!("{text:?}: {e}"))
}

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

fn tempdir() -> TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

// ---------------------------------------------------------------------------
// The tests

/// Each iteration, in a new temporary directory:
/// 1. `store init s --name crash --seed test-vectors/keys/store-demo.seed`.
/// 2. Spawn `store append s 0000.json … 1999.json`, standard output piped,
///    standard error null.
/// 3. Choose k uniformly from 0 to 1,999; read k complete lines from the
///    child's standard output, sleep a random 0 to 999 µs, then
///    `child.kill()`; read the rest of standard output to its end and
///    `wait()`. A final piece without LF is ignored.
/// 4. Every complete line is a receipt `{"duplicate":false,"entry_hash",
///    "file","kind":"object","ok":true,"received_at","ref","seq"}` for the
///    files in order: file i at seq i + 3.
/// 5. `store verify s` exits 0 with `ok` true ("reopening" the store).
/// 6. `store log s --limit 3000`: for every printed receipt, the line at
///    its seq has `entry.ref` = `ref`, entry hash = `entry_hash` and
///    `entry.received_at` = `received_at`. The head's seq is the last
///    printed seq or one more (a commit whose receipt the kill cut off);
///    with no receipt printed, 2 or 3.
/// 7. `store append s <the next file not held>` (when one is left) lands
///    at head + 1 with `duplicate` false: the killed writer's lease is
///    gone. Its entry's epoch is 3 when the killed child printed a
///    receipt, 2 or 3 otherwise. `store verify s` is still ok.
#[test]
fn append_killed_at_random_keeps_every_printed_receipt() {
    let test = "append_killed_at_random_keeps_every_printed_receipt";
    let mut rng = Rng::new(seed(test), test);
    let iterations = iterations();
    let tmp = tempdir();
    let objects = objects();
    let mut killed_mid_run = 0;
    for iteration in 0..iterations {
        let work = tempfile::tempdir_in(tmp.path()).unwrap();
        let s = work.path().join("s");
        init(&s);

        // 2-3. Append everything; kill it after k receipts and a moment.
        let mut child = Command::new(BIN)
            .arg("store")
            .arg("append")
            .arg(&s)
            .args(&objects.names)
            .current_dir(&objects.dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn store append");
        let k = rng.below(OBJECTS as u64);
        let pause = Duration::from_micros(rng.below(1000));
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let mut bytes = Vec::new();
        for _ in 0..k {
            if stdout.read_until(b'\n', &mut bytes).unwrap() == 0 {
                break;
            }
        }
        thread::sleep(pause);
        // An error here only means the child has ended already.
        let _ = child.kill();
        stdout.read_to_end(&mut bytes).unwrap();
        let status = child.wait().unwrap();
        if status.code().is_none() {
            killed_mid_run += 1;
        }

        // 4. The printed receipts: file i at seq i + 3, in order.
        let printed: Vec<Value> = complete_lines(&bytes).iter().map(|l| parse(l)).collect();
        assert!(printed.len() <= OBJECTS);
        for (i, receipt) in printed.iter().enumerate() {
            let mut members: Vec<&str> = receipt
                .as_object()
                .unwrap()
                .iter()
                .map(|(name, _)| name.as_str())
                .collect();
            members.sort();
            assert_eq!(
                members,
                [
                    "duplicate",
                    "entry_hash",
                    "file",
                    "kind",
                    "ok",
                    "received_at",
                    "ref",
                    "seq"
                ],
                "iteration {iteration}"
            );
            assert_eq!(receipt.get("ok"), Some(&Value::Bool(true)));
            assert_eq!(receipt.get("duplicate"), Some(&Value::Bool(false)));
            assert_eq!(text(receipt, "kind"), "object");
            assert_eq!(text(receipt, "file"), objects.names[i]);
            assert_eq!(text(receipt, "ref"), objects.ids[i]);
            assert_eq!(num(receipt, "seq"), i as u64 + 3);
        }

        // 5. The store verifies.
        verify_ok(&s);

        // 6. Every printed receipt is in the log; the head is the last
        // printed seq or one more.
        let log = store(Path::new("."), args!["log", s, "--limit", "3000"])
            .expect(0)
            .lines();
        let head = log.len() as u64;
        for (i, line) in log.iter().enumerate() {
            assert_eq!(num(at(line, &["entry"]), "seq"), i as u64 + 1);
        }
        for receipt in &printed {
            let seq = num(receipt, "seq");
            let entry = at(&log[seq as usize - 1], &["entry"]);
            assert_eq!(text(entry, "ref"), text(receipt, "ref"));
            assert_eq!(text(entry, "received_at"), text(receipt, "received_at"));
            assert_eq!(
                canon::sha256_hex(canon::canonicalize(entry).as_bytes()),
                text(receipt, "entry_hash")
            );
        }
        let last = printed.len() as u64 + 2;
        assert!(
            head == last || head == last + 1,
            "iteration {iteration}: head {head}, last printed seq {last}"
        );
        for (i, line) in log.iter().enumerate().skip(2) {
            assert_eq!(text(at(line, &["entry"]), "ref"), objects.ids[i - 2]);
        }

        // 7. The next file lands at head + 1: the lease is free.
        let next = (head - 2) as usize;
        if next < OBJECTS {
            let receipt = store(&objects.dir, args!["append", s, objects.names[next]])
                .expect(0)
                .value();
            assert_eq!(num(&receipt, "seq"), head + 1);
            assert_eq!(receipt.get("duplicate"), Some(&Value::Bool(false)));
            let line = store(
                Path::new("."),
                args!["log", s, "--from", (head + 1).to_string(), "--limit", "1"],
            )
            .expect(0)
            .value();
            let epoch = num(at(&line, &["entry"]), "epoch");
            if printed.is_empty() {
                assert!(epoch == 2 || epoch == 3, "epoch {epoch}");
            } else {
                assert_eq!(epoch, 3);
            }
            verify_ok(&s);
        }
    }
    eprintln!("{test}: {iterations} iterations, {killed_mid_run} killed before the end");
}

/// Once: a store with all 2,000 objects (`store append` run to the end:
/// exit 0, 2,000 receipts, head seq 2,002), exported with `store export`.
/// One `store import` of it, timed: T. Then each iteration, in a new
/// temporary directory: spawn `store import <export> --into <tmp>/into`,
/// sleep a random time from 0 to 1.2 T, `child.kill()`, `wait()`. Then
/// either `<tmp>/into` does not exist, or the kill came after the rename
/// and `store verify <tmp>/into` exits 0 with the `head` of the export's
/// manifest (`size`, `head`). Partial directories `into.partial-*` may be
/// left. If `into` does not exist, `store import` into it again exits 0:
/// a partial directory never blocks a retry.
#[test]
fn import_killed_at_random_leaves_no_store_or_a_complete_one() {
    let test = "import_killed_at_random_leaves_no_store_or_a_complete_one";
    let mut rng = Rng::new(seed(test), test);
    let iterations = iterations().div_ceil(5);
    let tmp = tempdir();
    let objects = objects();

    let s = tmp.path().join("s");
    init(&s);
    let mut list = args!["append", s];
    list.extend(objects.names.iter().map(OsString::from));
    let receipts = store(&objects.dir, list).expect(0).lines();
    assert_eq!(receipts.len(), OBJECTS);
    let x = tmp.path().join("export");
    let manifest = store(Path::new("."), args!["export", s, "--out", x])
        .expect(0)
        .value();
    assert_eq!(num(&manifest, "size"), OBJECTS as u64 + 2);
    let head = (num(&manifest, "size"), text(&manifest, "head"));
    let head_of = |report: &Value| {
        let h = at(report, &["head"]);
        (num(h, "seq"), text(h, "entry_hash"))
    };

    let started = Instant::now();
    let report = store(
        Path::new("."),
        args!["import", x, "--into", tmp.path().join("timed")],
    )
    .expect(0)
    .value();
    let took = started.elapsed();
    assert_eq!(head_of(&report), head);

    let (mut absent, mut complete) = (0, 0);
    for iteration in 0..iterations {
        let work = tempfile::tempdir_in(tmp.path()).unwrap();
        let into = work.path().join("into");
        let wait = took.mul_f64(1.2).mul_f64(rng.below(1_000_001) as f64 / 1e6);
        let mut child = Command::new(BIN)
            .arg("store")
            .arg("import")
            .arg(&x)
            .arg("--into")
            .arg(&into)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn store import");
        thread::sleep(wait);
        // An error here only means the child has ended already.
        let _ = child.kill();
        child.wait().unwrap();

        for name in fs::read_dir(work.path()).unwrap() {
            let name = name.unwrap().file_name().to_string_lossy().into_owned();
            assert!(
                name == "into" || name.starts_with("into.partial-"),
                "iteration {iteration}: {name} left beside into"
            );
        }
        if fs::symlink_metadata(&into).is_ok() {
            complete += 1;
            assert_eq!(head_of(&verify_ok(&into)), head, "iteration {iteration}");
        } else {
            absent += 1;
            let report = store(Path::new("."), args!["import", x, "--into", into])
                .expect(0)
                .value();
            assert_eq!(head_of(&report), head, "iteration {iteration}");
            assert_eq!(head_of(&verify_ok(&into)), head);
        }
    }
    eprintln!(
        "{test}: {iterations} iterations ({absent} left no store, {complete} a complete one); \
         one import took {took:?}"
    );
}
