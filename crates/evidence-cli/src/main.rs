//! `evidence`: command-line tool for evidence/0 objects and store/0 stores.
//!
//! The object commands are here; `evidence store …` is in [`store_cmd`]
//! (`docs/m1-store-spec.md` section 8).

mod store_cmd;

use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use evidence_core::api::{self, VerifyArgs};
use evidence_core::{canon, object, time};

const USAGE: &str = "\
evidence: create and verify evidence/0 objects, and keep them in a store/0 log

USAGE:
  evidence canon <file>                  print canonical JSON (RFC 8785)
  evidence content-hash <file>           print sha256 of a JSON record's canonical form
  evidence file-hash <file>              print sha256 of a file's bytes (schemas, external content)
  evidence id <file>                     print the id an object should have
  evidence seal <file>                   print the object with its id filled in
  evidence verify <file> [--attestations <file.jsonl>] [--store <dir>] [--require <purpose>]...
                                         print a JSON report; exit 0 if valid, 1 if not.
                                         --store: a folder of .json objects, or a store
                                         (a directory holding log.db), read only
  evidence sign <file> --seed <file> --signer <id> --purpose <author|approve|witness> [--at <time>]
                                         print one attestation line for a valid object
  evidence keygen <seed-file>            write a new random seed (hex) and print its public key

STORE (store/0: an append-only log in a directory holding log.db):
  evidence store init <dir> --name <name> --seed <seed-file>
                                         create a store whose key is the seed's
  evidence store append <dir> <object.json>... [--if-heads <id>[,<id>...] | --if-heads none]
                                         append each file in its own transaction
  evidence store attest <dir> <attestations.jsonl>...
                                         append each line that is not blank in its own transaction
  evidence store checkpoint <dir> --seed <seed-file>
                                         sign a checkpoint over every entry so far
  evidence store get <dir> <id> [--require <purpose>]...
                                         print a held object, its entry, attestations and report
  evidence store heads <dir> <system> <type> <key> [--lane <lane>]
                                         print a subject's heads in a lane (default: record)
  evidence store log <dir> [--from <seq>] [--limit <n>]
                                         print log lines (default: from seq 1, at most 100)
  evidence store verify <dir> [--max-findings <n>]
                                         verify a store or an export; print store-report/0
  evidence store export <dir> --out <dir> [--segment-size <n>]
                                         write an export to a new directory; print its manifest
  evidence store import <export-dir> --into <dir>
                                         build a new store from an export that verifies
  evidence store prove <dir> --seq <n> [--size <m>]
                                         inclusion proof (default size: the latest checkpoint's)
  evidence store prove <dir> --from <old-size> --to <new-size>
                                         consistency proof

Store commands print canonical JSON on standard output, one value per line,
and details on standard error. When they exit with status 2, the last line
of standard output is {\"code\":...,\"ok\":false}.

Seed files hold an Ed25519 private key in hex. They are for development and
test vectors only; production keys belong in an HSM or a key service.

Exit status: 0 success; 1 verification failed, or a store item was rejected;
2 usage, I/O or store operation error.";

fn main() -> ExitCode {
    let raw: Vec<OsString> = std::env::args_os().skip(1).collect();
    let is_store = raw.first().is_some_and(|command| command == "store");
    // Every argument is read as UTF-8 text; one that is not (a path with
    // such a name) is a usage error, never a panic.
    let args: Vec<String> = match raw.into_iter().map(OsString::into_string).collect() {
        Ok(args) => args,
        Err(arg) if is_store => return store_cmd::not_utf8(&arg),
        Err(arg) => {
            to_stderr(&format!(
                "evidence: usage: argument {arg:?} is not UTF-8\n\n{USAGE}"
            ));
            return ExitCode::from(2);
        }
    };
    if let Some(("store", rest)) = args.split_first().map(|(c, r)| (c.as_str(), r)) {
        return store_cmd::run(rest);
    }
    match run(&args) {
        Ok(code) => code,
        Err(msg) => {
            to_stderr(&format!("evidence: {msg}"));
            if msg.starts_with("usage") {
                to_stderr(&format!("\n{USAGE}"));
            }
            ExitCode::from(2)
        }
    }
}

/// Print `text` and an LF on standard output, flushed. A write that fails
/// (a closed pipe, a full disk) is an error, exit 2, never a panic.
fn print(text: &str) -> Result<(), String> {
    let mut out = io::stdout().lock();
    writeln!(out, "{text}")
        .and_then(|()| out.flush())
        .map_err(|e| format!("standard output: {e}"))
}

/// Write `text` and an LF on standard error. A write that fails is
/// ignored, as in the store commands: it must not make a command panic or
/// change its exit status.
fn to_stderr(text: &str) {
    let _ = writeln!(io::stderr().lock(), "{text}");
}

fn run(args: &[String]) -> Result<ExitCode, String> {
    let Some((cmd, rest)) = args.split_first() else {
        return Err("usage: missing command".into());
    };
    let flags = Flags::parse(rest)?;
    let text = |r: evidence_core::Result<String>| r.map_err(|e| e.to_string());
    match cmd.as_str() {
        "canon" => print(&text(api::canonicalize(&read(flags.one_path()?)?))?)?,
        "content-hash" => print(&text(api::content_hash(&read(flags.one_path()?)?))?)?,
        "file-hash" => print(&api::file_hash(&read(flags.one_path()?)?))?,
        "id" => print(&text(api::compute_id(&read(flags.one_path()?)?))?)?,
        "seal" => {
            let sealed = text(api::seal(&read(flags.one_path()?)?))?;
            print(&sealed)?;
            if let Err(e) = object::check(sealed.as_bytes()) {
                to_stderr(&format!(
                    "evidence: sealed, but the object is not valid: {e}"
                ));
                return Ok(ExitCode::from(1));
            }
        }
        "verify" => {
            let bytes = read(flags.one_path()?)?;
            let attestations = flags.value("--attestations").map(read).transpose()?;
            let require = flags.values("--require");
            let report = match flags.value("--store") {
                // An empty value names no directory: the folder form
                // refuses it, as it always has.
                Some(dir) if !dir.is_empty() && holds_store(&dir)? => store_cmd::verify_with_store(
                    &bytes,
                    attestations.as_deref(),
                    Path::new(&dir),
                    &require,
                )?,
                dir => {
                    let store = dir.map(load_store).transpose()?;
                    api::verify(
                        &bytes,
                        &VerifyArgs {
                            attestations: attestations.as_deref(),
                            store: store.as_deref(),
                            require: &require,
                        },
                    )
                    .map_err(|e| format!("usage: {}", e.detail))?
                }
            };
            print(&canon::canonicalize(&report.to_value()))?;
            return Ok(if report.ok() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            });
        }
        "sign" => {
            let bytes = read(flags.one_path()?)?;
            let seed_path = flags.required("--seed")?;
            let seed = String::from_utf8(read(&seed_path)?)
                .map_err(|_| format!("{seed_path}: not text"))?;
            let at = flags.value("--at").unwrap_or_else(time::now);
            let line = api::sign(
                &bytes,
                &seed,
                &flags.required("--signer")?,
                &flags.required("--purpose")?,
                &at,
            )
            .map_err(|e| format!("refusing to sign: {e}"))?;
            print(&line)?;
        }
        "keygen" => {
            let path = flags.one_path()?;
            if path.exists() {
                return Err(format!("{} already exists", path.display()));
            }
            let mut seed = [0u8; 32];
            getrandom::fill(&mut seed).map_err(|e| format!("no system randomness: {e}"))?;
            let hex: String = seed.iter().map(|b| format!("{b:02x}")).collect();
            write_private(&path, &format!("{hex}\n"))?;
            print(&text(api::public_key(&hex))?)?;
        }
        "help" | "-h" | "--help" => print(USAGE)?,
        other => return Err(format!("usage: unknown command {other:?}")),
    }
    Ok(ExitCode::SUCCESS)
}

/// Positional arguments plus `--name value` options (repeatable).
struct Flags {
    positional: Vec<String>,
    options: Vec<(String, String)>,
}

impl Flags {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut positional = Vec::new();
        let mut options = Vec::new();
        let mut it = args.iter();
        while let Some(a) = it.next() {
            if a.starts_with("--") {
                let v = it
                    .next()
                    .ok_or_else(|| format!("usage: {a} needs a value"))?;
                options.push((a.clone(), v.clone()));
            } else {
                positional.push(a.clone());
            }
        }
        Ok(Flags {
            positional,
            options,
        })
    }

    fn one_path(&self) -> Result<PathBuf, String> {
        match self.positional.as_slice() {
            [p] => Ok(PathBuf::from(p)),
            _ => Err("usage: expected exactly one file argument".into()),
        }
    }

    fn value(&self, name: &str) -> Option<String> {
        self.options
            .iter()
            .rev()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
    }

    fn values(&self, name: &str) -> Vec<String> {
        self.options
            .iter()
            .filter(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
            .collect()
    }

    fn required(&self, name: &str) -> Result<String, String> {
        self.value(name)
            .ok_or_else(|| format!("usage: {name} is required"))
    }
}

/// Whether `--store <dir>` is a store rather than a folder
/// ([`store_cmd::holds_store`]). When that cannot be told, the error of the
/// store form: `<dir>: S_IO: <detail>`.
fn holds_store(dir: &str) -> Result<bool, String> {
    store_cmd::holds_store(Path::new(dir)).map_err(|e| format!("{dir}: {e}"))
}

fn read(path: impl AsRef<Path>) -> Result<Vec<u8>, String> {
    let path = path.as_ref();
    fs::read(path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Every `*.json` file in `dir`, in name order.
fn load_store(dir: String) -> Result<Vec<Vec<u8>>, String> {
    let mut paths: Vec<PathBuf> = fs::read_dir(&dir)
        .map_err(|e| format!("{dir}: {e}"))?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    paths.sort();
    paths.iter().map(read).collect()
}

fn write_private(path: &Path, text: &str) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        f.write_all(text.as_bytes())
            .map_err(|e| format!("{}: {e}", path.display()))
    }
    #[cfg(not(unix))]
    {
        fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
    }
}
