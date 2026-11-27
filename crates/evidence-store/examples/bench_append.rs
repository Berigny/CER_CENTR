//! Append throughput and verification time of a store/0 log: the
//! informational benchmark of the nightly run (`docs/m1-store-spec.md`
//! section 11). It is not a gate.
//!
//! ```text
//! cargo run --release -p evidence-store --example bench_append -- [N] [--mem] [--batch B]
//! ```
//!
//! 1. Creates a store keyed by the test store key
//!    (`test-vectors/keys/store-demo.seed`): a `SqliteLog` in a new
//!    temporary directory, or a `MemLog` with `--mem`. The writer reads the
//!    system clock.
//! 2. Appends N synthetic objects (default 10,000), B per transaction
//!    (default 1, as `evidence store append` commits each file on its own):
//!    a create for each of 1,000 subjects, then updates that chain to the
//!    subject's previous object. Only the commits are timed, not building
//!    the objects.
//! 3. Writes one checkpoint.
//! 4. Verifies the log: `verify_store` for a store directory, `verify_log`
//!    for a `MemLog`.
//!
//! It prints one canonical JSON line, for example:
//!
//! ```text
//! {"append_seconds":…,"appends_per_second":…,"backend":"sqlite","batch":1,"checkpoint_seconds":…,"entries":10004,"objects":10000,"ok":true,"verify_seconds":…}
//! ```
//!
//! and exits 0 when the log verifies, 1 when it does not or a store
//! operation fails, and 2 for a usage error. `entries` is N + 4: the genesis
//! and its attestation, the N objects, and the checkpoint and its witness
//! attestation. The objects are synthetic.

use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use evidence_core::canon;
use evidence_core::json::Value;
use evidence_core::object;
use evidence_core::time;
use evidence_log::{StoreReport, VerifyOptions};
use evidence_store::{
    verify_log, verify_store, Item, LogWrite, MemLog, SeedSigner, SqliteLog, StoreError,
    SystemClock,
};

/// The number of subjects the objects are about.
const SUBJECTS: u64 = 1_000;
/// Every object's `at` is this, plus its number in seconds.
const BASE: &str = "2026-01-01T00:00:00Z";

const USAGE: &str = "usage: bench_append [N] [--mem] [--batch B]";

/// What was asked for.
struct Args {
    objects: u64,
    mem: bool,
    batch: usize,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        objects: 10_000,
        mem: false,
        batch: 1,
    };
    let mut given = std::env::args().skip(1);
    while let Some(arg) = given.next() {
        match arg.as_str() {
            "--mem" => args.mem = true,
            "--batch" => {
                args.batch = given
                    .next()
                    .and_then(|b| b.parse().ok())
                    .filter(|b| *b >= 1)
                    .ok_or("--batch takes a number of objects, 1 or more")?;
            }
            n => {
                args.objects = n
                    .parse()
                    .map_err(|_| format!("{n:?} is not a number of objects"))?;
            }
        }
    }
    Ok(args)
}

/// Builds the synthetic objects in order.
struct Objects {
    base: i64,
    /// The id of each subject's latest object.
    latest: Vec<Option<String>>,
}

impl Objects {
    fn new() -> Objects {
        Objects {
            base: time::parse(BASE).map(|t| t.secs).unwrap_or(0),
            latest: vec![None; SUBJECTS as usize],
        }
    }

    /// Object `n`: about subject `n` mod 1,000, a create the first time,
    /// then an update over the subject's previous object.
    fn next(&mut self, n: u64) -> Vec<u8> {
        let s = (n % SUBJECTS) as usize;
        let text = |t: &str| Value::String(t.to_string());
        let inline = Value::Object(vec![("n".into(), Value::Number(n as f64))]);
        let prev = self.latest[s].iter().map(|p| text(p)).collect();
        let op = if self.latest[s].is_some() {
            "update"
        } else {
            "create"
        };
        let schema = "bench/thing@1";
        let unsealed = Value::Object(vec![
            ("v".into(), text("evidence/0")),
            ("op".into(), text(op)),
            (
                "schema".into(),
                Value::Object(vec![
                    ("ref".into(), text(schema)),
                    ("hash".into(), text(&canon::sha256_hex(schema.as_bytes()))),
                ]),
            ),
            (
                "subject".into(),
                Value::Object(vec![
                    ("system".into(), text("bench")),
                    ("type".into(), text("thing")),
                    ("key".into(), text(&format!("T{s:04}"))),
                ]),
            ),
            (
                "content".into(),
                Value::Object(vec![
                    ("media_type".into(), text("application/json")),
                    ("hash".into(), text(&canon::digest(&inline))),
                    ("inline".into(), inline),
                ]),
            ),
            (
                "actor".into(),
                Value::Object(vec![
                    ("id".into(), text("service:bench")),
                    ("kind".into(), text("service")),
                ]),
            ),
            ("at".into(), text(&time::format_secs(self.base + n as i64))),
            ("prev".into(), Value::Array(prev)),
        ]);
        let sealed = object::seal(&unsealed);
        self.latest[s] = sealed.get("id").and_then(Value::as_str).map(str::to_string);
        canon::canonicalize(&sealed).into_bytes()
    }
}

/// What the run measured.
struct Measured {
    append: Duration,
    checkpoint: Duration,
    verify: Duration,
    report: StoreReport,
}

/// Append `args.objects` objects to `log`, `args.batch` per transaction,
/// then write a checkpoint. Returns the time spent committing and the time
/// of the checkpoint.
fn fill(
    log: &mut dyn LogWrite,
    signer: &SeedSigner,
    args: &Args,
) -> Result<(Duration, Duration), StoreError> {
    let mut objects = Objects::new();
    let mut append = Duration::ZERO;
    let mut n = 0;
    while n < args.objects {
        let end = n.saturating_add(args.batch as u64).min(args.objects);
        let batch: Vec<Vec<u8>> = (n..end).map(|i| objects.next(i)).collect();
        let items: Vec<Item<'_>> = batch
            .iter()
            .map(|bytes| Item::Object {
                bytes,
                if_heads: None,
            })
            .collect();
        let started = Instant::now();
        log.commit(&items)?;
        append += started.elapsed();
        n = end;
    }
    let started = Instant::now();
    log.checkpoint(signer)?;
    Ok((append, started.elapsed()))
}

fn bench(args: &Args, signer: &SeedSigner) -> Result<Measured, StoreError> {
    let name = "bench";
    if args.mem {
        let mut log = MemLog::init(name, signer, Box::new(SystemClock))?;
        let (append, checkpoint) = fill(&mut log, signer, args)?;
        let started = Instant::now();
        let report = verify_log(&log, VerifyOptions::default())?;
        return Ok(Measured {
            append,
            checkpoint,
            verify: started.elapsed(),
            report,
        });
    }
    let tmp = tempfile::tempdir()?;
    let dir = tmp.path().join("store");
    let (append, checkpoint) = {
        let mut log = SqliteLog::init(&dir, name, signer, Box::new(SystemClock))?;
        fill(&mut log, signer, args)?
    };
    let started = Instant::now();
    let report = verify_store(&dir, VerifyOptions::default())?;
    Ok(Measured {
        append,
        checkpoint,
        verify: started.elapsed(),
        report,
    })
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(e) => {
            eprintln!("{e}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let seed =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-vectors/keys/store-demo.seed");
    let measured = SeedSigner::from_file(&seed).and_then(|signer| bench(&args, &signer));
    let m = match measured {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(1);
        }
    };
    let secs = |d: Duration| Value::Number(d.as_secs_f64());
    let rate = if m.append.is_zero() {
        0.0
    } else {
        args.objects as f64 / m.append.as_secs_f64()
    };
    let out = Value::Object(vec![
        (
            "backend".into(),
            Value::String(if args.mem { "mem" } else { "sqlite" }.into()),
        ),
        ("objects".into(), Value::Number(args.objects as f64)),
        ("batch".into(), Value::Number(args.batch as f64)),
        (
            "entries".into(),
            Value::Number(m.report.checked.entries as f64),
        ),
        ("append_seconds".into(), secs(m.append)),
        ("appends_per_second".into(), Value::Number(rate.round())),
        ("checkpoint_seconds".into(), secs(m.checkpoint)),
        ("verify_seconds".into(), secs(m.verify)),
        ("ok".into(), Value::Bool(m.report.ok())),
    ]);
    println!("{}", canon::canonicalize(&out));
    if m.report.ok() {
        ExitCode::SUCCESS
    } else {
        eprintln!("findings: {:?}", m.report.findings);
        ExitCode::from(1)
    }
}
