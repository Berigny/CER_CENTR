//! `evidence`: command-line tool for evidence/0 objects.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use evidence_core::verify::{self, Options, Store};
use evidence_core::{attest, canon, json, object, time, Value};

const USAGE: &str = "\
evidence: create and verify evidence/0 objects

USAGE:
  evidence canon <file>                  print canonical JSON (RFC 8785)
  evidence content-hash <file>           print sha256 of a JSON record's canonical form
  evidence file-hash <file>              print sha256 of a file's bytes (schemas, external content)
  evidence id <file>                     print the id an object should have
  evidence seal <file>                   print the object with its id filled in
  evidence verify <file> [--attestations <file.jsonl>] [--store <dir>] [--require <purpose>]...
                                         print a JSON report; exit 0 if valid, 1 if not
  evidence sign <file> --seed <file> --signer <id> --purpose <author|approve|witness> [--at <time>]
                                         print one attestation line for a valid object
  evidence keygen <seed-file>            write a new random seed (hex) and print its public key

Seed files hold an Ed25519 private key in hex. They are for development and
test vectors only; production keys belong in an HSM or a key service.

Exit status: 0 success, 1 verification failed, 2 usage or I/O error.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(code) => code,
        Err(msg) => {
            eprintln!("evidence: {msg}");
            if msg.starts_with("usage") {
                eprintln!("\n{USAGE}");
            }
            ExitCode::from(2)
        }
    }
}

fn run(args: &[String]) -> Result<ExitCode, String> {
    let Some((cmd, rest)) = args.split_first() else {
        return Err("usage: missing command".into());
    };
    let flags = Flags::parse(rest)?;
    match cmd.as_str() {
        "canon" => {
            println!("{}", canon::canonicalize(&read_json(flags.one_path()?)?));
        }
        "content-hash" => {
            println!("{}", canon::digest(&read_json(flags.one_path()?)?));
        }
        "file-hash" => {
            println!("{}", canon::sha256_hex(&read(flags.one_path()?)?));
        }
        "id" => {
            println!("{}", object::compute_id(&read_json(flags.one_path()?)?));
        }
        "seal" => {
            let sealed = object::seal(&read_json(flags.one_path()?)?);
            let text = canon::canonicalize(&sealed);
            if let Err(e) = object::check(text.as_bytes()) {
                eprintln!("evidence: sealed, but the object is not valid: {e}");
                println!("{text}");
                return Ok(ExitCode::from(1));
            }
            println!("{text}");
        }
        "verify" => {
            let bytes = read(flags.one_path()?)?;
            let attestations = flags.value("--attestations").map(read).transpose()?;
            let store = flags.value("--store").map(load_store).transpose()?;
            let require = flags.values("--require");
            for p in &require {
                if !attest::PURPOSES.contains(&p.as_str()) {
                    return Err(format!(
                        "usage: --require must be one of {:?}",
                        attest::PURPOSES
                    ));
                }
            }
            let report = verify::verify(
                &bytes,
                &Options {
                    attestations: attestations.as_deref(),
                    store: store.as_ref(),
                    require: &require,
                },
            );
            println!("{}", canon::canonicalize(&report.to_value()));
            return Ok(if report.ok() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            });
        }
        "sign" => {
            let bytes = read(flags.one_path()?)?;
            let checked = object::check(&bytes).map_err(|e| format!("refusing to sign: {e}"))?;
            let seed = read_seed(&flags.required("--seed")?)?;
            let purpose = flags.required("--purpose")?;
            if !attest::PURPOSES.contains(&purpose.as_str()) {
                return Err(format!(
                    "usage: --purpose must be one of {:?}",
                    attest::PURPOSES
                ));
            }
            let at = flags.value("--at").unwrap_or_else(time::now);
            let att = attest::sign(
                &checked.id,
                &purpose,
                &flags.required("--signer")?,
                &at,
                &seed,
            )
            .map_err(|e| e.to_string())?;
            println!("{}", canon::canonicalize(&att));
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
            println!("{}", attest::public_key(&seed));
        }
        "help" | "-h" | "--help" => println!("{USAGE}"),
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

fn read(path: impl AsRef<Path>) -> Result<Vec<u8>, String> {
    let path = path.as_ref();
    fs::read(path).map_err(|e| format!("{}: {e}", path.display()))
}

fn read_json(path: PathBuf) -> Result<Value, String> {
    json::parse(&read(&path)?).map_err(|e| format!("{}: {e}", path.display()))
}

fn read_seed(path: &str) -> Result<[u8; 32], String> {
    let text = String::from_utf8(read(path)?).map_err(|_| format!("{path}: not text"))?;
    let hex = text.trim();
    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("{path}: expected 64 hex digits"));
    }
    let mut seed = [0u8; 32];
    for (i, byte) in seed.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).expect("hex checked");
    }
    Ok(seed)
}

fn load_store(dir: String) -> Result<Store, String> {
    let mut paths: Vec<PathBuf> = fs::read_dir(&dir)
        .map_err(|e| format!("{dir}: {e}"))?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    paths.sort();
    let mut store = Store::new();
    for p in paths {
        store
            .insert(&read(&p)?)
            .map_err(|e| format!("store object {} is not valid: {e}", p.display()))?;
    }
    Ok(store)
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
