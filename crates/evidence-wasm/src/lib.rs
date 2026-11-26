//! The evidence/0 core and the store/0 verifier as a WebAssembly module.
//!
//! The ABI is deliberately tiny, so any WebAssembly host can use the same
//! `.wasm` file without generated glue: browsers and Node (see `js/`), and
//! Wasmtime or Chicory hosts in .NET, Java, Go or Python.
//!
//! ```text
//! evidence_alloc(len) -> ptr       reserve len bytes and write a request there
//! evidence_call(ptr, len) -> out   run the request; `out` points at a 4-byte
//!                                  little-endian length followed by that many
//!                                  bytes of JSON
//! evidence_free(ptr, len)          release a request (len) or a result (4 + length)
//! ```
//!
//! Request: `{"fn": "<name>", "args": {...}}`.
//! Response: `{"result": ...}` or `{"error": {"code": "E_...", "detail": "..."}}`.
//!
//! Byte inputs (`input`, `attestations`, `store` entries, `leaves`, `manifest`,
//! `segments`) are passed either as a string under their name, or as standard
//! base64 under `<name>_base64` when the exact bytes matter (for example, a
//! file that may not be valid UTF-8). If both are given, `<name>` is used.
//! Hashes (`leaf_hash`, roots, proof elements) are always text, `"sha256:"`
//! and 64 lowercase hex digits. Sizes and indexes are integers from 0 to
//! 2^53 − 1, and `max_findings` from 1 to 2^53 − 1.
//!
//! | fn | args | result |
//! |---|---|---|
//! | `version` | | `{"spec", "crate"}` |
//! | `canonicalize` | `input` | string |
//! | `content_hash` | `input` | string |
//! | `file_hash` | `input` | string |
//! | `compute_id` | `input` | string |
//! | `seal` | `input` | string |
//! | `verify` | `input`, `attestations`?, `store`?, `require`? | report object |
//! | `sign` | `input`, `seed_hex`, `signer`, `purpose`, `at` | attestation line |
//! | `public_key` | `seed_hex` | string |
//! | `entry_hash` | `input` (log-entry/0 JSON) | string |
//! | `merkle_root` | `leaves` (list of leaf bytes) | string |
//! | `verify_inclusion` | `leaf_hash`, `index`, `size`, `proof`, `root` | bool |
//! | `verify_consistency` | `old_size`, `new_size`, `old_root`, `new_root`, `proof` | bool |
//! | `verify_export` | `manifest`, `segments` (manifest order), `max_findings`? | store-report/0 object |
//!
//! `E_USAGE` means the request itself was invalid; for `entry_hash`, its
//! detail begins with `S_ENTRY_INVALID` when the input is not a valid entry.
//! Verification outcomes are never errors: proof checks answer `false`, and
//! `verify_export` returns a report however damaged the export is. See
//! `spec/store-0.md`, "Programming interfaces".

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use evidence_core::api::{self, VerifyArgs};
use evidence_core::json::{self, Value, MAX_SAFE_INTEGER};
use evidence_core::{canon, Code, Error, Result};
use evidence_log::merkle::{self, Digest};
use evidence_log::verify::DEFAULT_MAX_FINDINGS;
use evidence_log::{Entry, VerifyOptions};

/// Reserve `len` bytes for a request.
#[no_mangle]
pub extern "C" fn evidence_alloc(len: usize) -> *mut u8 {
    Box::into_raw(vec![0u8; len].into_boxed_slice()).cast::<u8>()
}

/// Release a buffer from [`evidence_alloc`] or [`evidence_call`].
///
/// # Safety
/// `ptr` must come from `evidence_alloc(len)`, or from `evidence_call` with
/// `len` equal to 4 plus the length stored in its first four bytes, and must
/// not have been freed already.
#[no_mangle]
pub unsafe extern "C" fn evidence_free(ptr: *mut u8, len: usize) {
    drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len)));
}

/// Run one JSON request and return a length-prefixed JSON response.
///
/// # Safety
/// `ptr..ptr + len` must be readable, for example a buffer from
/// [`evidence_alloc`]. The result must be released with [`evidence_free`].
#[no_mangle]
pub unsafe extern "C" fn evidence_call(ptr: *const u8, len: usize) -> *mut u8 {
    let request = if len == 0 {
        &[][..]
    } else {
        std::slice::from_raw_parts(ptr, len)
    };
    let response = handle(request).into_bytes();
    let mut out = Vec::with_capacity(4 + response.len());
    out.extend_from_slice(&(response.len() as u32).to_le_bytes());
    out.extend_from_slice(&response);
    Box::into_raw(out.into_boxed_slice()).cast::<u8>()
}

/// The ABI without the memory handling: request bytes in, response JSON out.
pub fn handle(request: &[u8]) -> String {
    let response = match dispatch(request) {
        Ok(result) => obj(vec![("result", result)]),
        Err(e) => obj(vec![(
            "error",
            obj(vec![
                ("code", Value::String(e.code.as_str().into())),
                ("detail", Value::String(e.detail)),
            ]),
        )]),
    };
    canon::canonicalize(&response)
}

fn dispatch(request: &[u8]) -> Result<Value> {
    let request = json::parse(request).map_err(|e| usage(format!("request: {e}")))?;
    let name = request
        .get("fn")
        .and_then(Value::as_str)
        .ok_or_else(|| usage("request needs a string \"fn\""))?;
    let empty = Value::Object(Vec::new());
    let args = request.get("args").unwrap_or(&empty);
    if args.as_object().is_none() {
        return Err(usage("\"args\" must be an object"));
    }
    let text = |s: String| Value::String(s);
    Ok(match name {
        "version" => obj(vec![
            ("spec", text(api::SPEC_VERSION.into())),
            ("crate", text(env!("CARGO_PKG_VERSION").into())),
        ]),
        "canonicalize" => text(api::canonicalize(&input(args)?)?),
        "content_hash" => text(api::content_hash(&input(args)?)?),
        "file_hash" => text(api::file_hash(&input(args)?)),
        "compute_id" => text(api::compute_id(&input(args)?)?),
        "seal" => text(api::seal(&input(args)?)?),
        "verify" => {
            let attestations = bytes(args, "attestations")?;
            let store = byte_list(args, "store")?;
            let require = strings(args, "require")?;
            let report = api::verify(
                &input(args)?,
                &VerifyArgs {
                    attestations: attestations.as_deref(),
                    store: store.as_deref(),
                    require: &require,
                },
            )?;
            report.to_value()
        }
        "sign" => text(api::sign(
            &input(args)?,
            &string(args, "seed_hex")?,
            &string(args, "signer")?,
            &string(args, "purpose")?,
            &string(args, "at")?,
        )?),
        "public_key" => text(api::public_key(&string(args, "seed_hex")?)?),
        "entry_hash" => text(
            Entry::parse(&input(args)?)
                .map_err(|code| usage(format!("{code}: \"input\" is not valid log-entry/0")))?
                .hash(),
        ),
        "merkle_root" => {
            let leaves = byte_list(args, "leaves")?
                .ok_or_else(|| usage("\"leaves\" or \"leaves_base64\" is required"))?;
            let hashes: Vec<Digest> = leaves.iter().map(|l| merkle::leaf_hash(l)).collect();
            text(merkle::to_text(&merkle::root(&hashes)))
        }
        "verify_inclusion" => Value::Bool(merkle::verify_inclusion(
            &digest(args, "leaf_hash")?,
            integer(args, "index", 0)?,
            integer(args, "size", 0)?,
            &digests(args, "proof")?,
            &digest(args, "root")?,
        )),
        "verify_consistency" => Value::Bool(merkle::verify_consistency(
            integer(args, "old_size", 0)?,
            integer(args, "new_size", 0)?,
            &digest(args, "old_root")?,
            &digest(args, "new_root")?,
            &digests(args, "proof")?,
        )),
        "verify_export" => {
            let manifest = bytes(args, "manifest")?
                .ok_or_else(|| usage("\"manifest\" or \"manifest_base64\" is required"))?;
            let segments = byte_list(args, "segments")?
                .ok_or_else(|| usage("\"segments\" or \"segments_base64\" is required"))?;
            let max_findings = match args.get("max_findings") {
                None => DEFAULT_MAX_FINDINGS,
                // A limit above what this target can count is no limit.
                Some(_) => usize::try_from(integer(args, "max_findings", 1)?).unwrap_or(usize::MAX),
            };
            let segments: Vec<&[u8]> = segments.iter().map(Vec::as_slice).collect();
            evidence_log::verify_export(
                &manifest,
                &segments,
                VerifyOptions {
                    max_findings,
                    ..VerifyOptions::default()
                },
            )
            .to_value()
        }
        other => return Err(usage(format!("unknown fn {other:?}"))),
    })
}

fn obj(members: Vec<(&str, Value)>) -> Value {
    Value::Object(
        members
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    )
}

fn usage(detail: impl Into<String>) -> Error {
    Error::new(Code::Usage, detail)
}

fn input(args: &Value) -> Result<Vec<u8>> {
    bytes(args, "input")?.ok_or_else(|| usage("\"input\" or \"input_base64\" is required"))
}

/// `name` as a UTF-8 string, or `name_base64` as standard base64.
fn bytes(args: &Value, name: &str) -> Result<Option<Vec<u8>>> {
    if let Some(v) = args.get(name) {
        let s = v
            .as_str()
            .ok_or_else(|| usage(format!("\"{name}\" must be a string")))?;
        return Ok(Some(s.as_bytes().to_vec()));
    }
    let key = format!("{name}_base64");
    match args.get(&key) {
        Some(v) => decode(v, &key).map(Some),
        None => Ok(None),
    }
}

/// `name` as an array of strings, or `name_base64` as an array of base64.
fn byte_list(args: &Value, name: &str) -> Result<Option<Vec<Vec<u8>>>> {
    let key = format!("{name}_base64");
    let (items, base64) = match (args.get(name), args.get(&key)) {
        (Some(v), _) => (v, false),
        (None, Some(v)) => (v, true),
        (None, None) => return Ok(None),
    };
    let label = if base64 { key.as_str() } else { name };
    let items = items
        .as_array()
        .ok_or_else(|| usage(format!("\"{label}\" must be an array")))?;
    items
        .iter()
        .map(|v| {
            if base64 {
                decode(v, label)
            } else {
                v.as_str()
                    .map(|s| s.as_bytes().to_vec())
                    .ok_or_else(|| usage(format!("\"{label}\" must hold strings")))
            }
        })
        .collect::<Result<Vec<_>>>()
        .map(Some)
}

fn decode(v: &Value, label: &str) -> Result<Vec<u8>> {
    let s = v
        .as_str()
        .ok_or_else(|| usage(format!("\"{label}\" must be base64 text")))?;
    STANDARD
        .decode(s)
        .map_err(|e| usage(format!("\"{label}\" is not valid base64: {e}")))
}

fn string(args: &Value, name: &str) -> Result<String> {
    args.get(name)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| usage(format!("\"{name}\" must be a string")))
}

const HASH_TEXT: &str = "\"sha256:\" and 64 lowercase hex digits";

/// `name` as a hash written `"sha256:"` + 64 lowercase hex digits.
fn digest(args: &Value, name: &str) -> Result<Digest> {
    args.get(name)
        .and_then(Value::as_str)
        .and_then(merkle::from_text)
        .ok_or_else(|| usage(format!("\"{name}\" must be {HASH_TEXT}")))
}

/// `name` as an array of hashes, each written as for [`digest`].
fn digests(args: &Value, name: &str) -> Result<Vec<Digest>> {
    let bad = || usage(format!("\"{name}\" must be an array of {HASH_TEXT}"));
    args.get(name)
        .and_then(Value::as_array)
        .ok_or_else(bad)?
        .iter()
        .map(|v| v.as_str().and_then(merkle::from_text).ok_or_else(bad))
        .collect()
}

/// `name` as an integer from `min` to 2^53 − 1, written as a JSON number
/// with no fractional part.
fn integer(args: &Value, name: &str, min: u64) -> Result<u64> {
    match args.get(name) {
        Some(Value::Number(n))
            if n.fract() == 0.0 && *n >= min as f64 && *n <= MAX_SAFE_INTEGER =>
        {
            Ok(*n as u64)
        }
        _ => Err(usage(format!(
            "\"{name}\" must be an integer from {min} to 2^53 - 1"
        ))),
    }
}

fn strings(args: &Value, name: &str) -> Result<Vec<String>> {
    match args.get(name) {
        None => Ok(Vec::new()),
        Some(v) => v
            .as_array()
            .ok_or_else(|| usage(format!("\"{name}\" must be an array of strings")))?
            .iter()
            .map(|s| {
                s.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| usage(format!("\"{name}\" must be an array of strings")))
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(request: &str) -> Value {
        json::parse(handle(request.as_bytes()).as_bytes()).unwrap()
    }

    fn error_code(v: &Value) -> Option<&str> {
        v.get("error")?.get("code")?.as_str()
    }

    #[test]
    fn dispatches_and_reports_errors() {
        let v = call(r#"{"fn":"canonicalize","args":{"input":"{ \"b\":1, \"a\":2 }"}}"#);
        assert_eq!(v.get("result").unwrap().as_str(), Some(r#"{"a":2,"b":1}"#));

        // "{}" in base64 is "e30=".
        let v = call(r#"{"fn":"content_hash","args":{"input_base64":"e30="}}"#);
        assert!(v
            .get("result")
            .unwrap()
            .as_str()
            .unwrap()
            .starts_with("sha256:"));

        let v = call(r#"{"fn":"canonicalize","args":{"input":"{\"a\":1,\"a\":2}"}}"#);
        assert_eq!(error_code(&v), Some("E_DUPLICATE_KEY"));

        for bad in [
            "not json",
            r#"{"fn":"nope"}"#,
            r#"{"fn":"canonicalize","args":{}}"#,
            r#"{"fn":"verify","args":{"input":"{}","require":["bless"]}}"#,
        ] {
            assert_eq!(error_code(&call(bad)), Some("E_USAGE"), "{bad}");
        }
    }

    #[test]
    fn verify_returns_a_report_not_an_error() {
        let v = call(r#"{"fn":"verify","args":{"input":"{}"}}"#);
        let report = v.get("result").unwrap();
        assert_eq!(report.get("ok"), Some(&Value::Bool(false)));
        assert_eq!(report.get("code").unwrap().as_str(), Some("E_SHAPE"));
    }

    fn h(c: char) -> String {
        format!("sha256:{}", c.to_string().repeat(64))
    }

    #[test]
    fn store_functions() {
        // The empty tree is SHA-256 of no bytes; one leaf is its leaf hash.
        let empty = "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        let v = call(r#"{"fn":"merkle_root","args":{"leaves":[]}}"#);
        assert_eq!(v.get("result").unwrap().as_str(), Some(empty));
        let one = merkle::to_text(&merkle::leaf_hash(b"leaf-0"));
        // "leaf-0" in base64 is "bGVhZi0w"; the plain form wins when both are given.
        for args in [
            r#"{"leaves":["leaf-0"]}"#,
            r#"{"leaves_base64":["bGVhZi0w"]}"#,
            r#"{"leaves":["leaf-0"],"leaves_base64":["AA=="]}"#,
        ] {
            let v = call(&format!(r#"{{"fn":"merkle_root","args":{args}}}"#));
            assert_eq!(
                v.get("result").unwrap().as_str(),
                Some(one.as_str()),
                "{args}"
            );
        }

        // A one-leaf tree: the leaf hash is the root, with an empty proof.
        let inclusion = |index: &str, size: &str, root: &str| {
            call(&format!(
                r#"{{"fn":"verify_inclusion","args":{{"leaf_hash":"{one}","index":{index},"size":{size},"proof":[],"root":"{root}"}}}}"#
            ))
        };
        assert_eq!(
            inclusion("0", "1", &one).get("result"),
            Some(&Value::Bool(true))
        );
        assert_eq!(
            inclusion("1", "1", &one).get("result"),
            Some(&Value::Bool(false))
        );
        assert_eq!(
            inclusion("0", "1", &h('a')).get("result"),
            Some(&Value::Bool(false))
        );
        assert_eq!(
            inclusion("9007199254740991", "9007199254740991", &one).get("result"),
            Some(&Value::Bool(false))
        );
        for (index, size) in [
            ("-1", "1"),
            ("0.5", "1"),
            ("0", "9007199254740992"),
            ("true", "1"),
            ("\"0\"", "1"),
            ("null", "1"),
        ] {
            assert_eq!(
                error_code(&inclusion(index, size, &one)),
                Some("E_USAGE"),
                "{index} {size}"
            );
        }
        let upper = one.to_uppercase().replace("SHA256", "sha256");
        assert_eq!(error_code(&inclusion("0", "1", &upper)), Some("E_USAGE"));

        let consistency = |proof: &str| {
            call(&format!(
                r#"{{"fn":"verify_consistency","args":{{"old_size":1,"new_size":2,"old_root":"{one}","new_root":"{}","proof":{proof}}}}}"#,
                h('b')
            ))
        };
        assert_eq!(
            consistency(&format!(r#"["{}"]"#, h('c'))).get("result"),
            Some(&Value::Bool(false))
        );
        for proof in ["null", r#""x""#, r#"["sha256:00"]"#, "[1]"] {
            assert_eq!(error_code(&consistency(proof)), Some("E_USAGE"), "{proof}");
        }

        let v = call(r#"{"fn":"entry_hash","args":{"input":"{}"}}"#);
        assert_eq!(error_code(&v), Some("E_USAGE"));
        let detail = v
            .get("error")
            .unwrap()
            .get("detail")
            .unwrap()
            .as_str()
            .unwrap();
        assert!(detail.starts_with("S_ENTRY_INVALID"), "{detail}");

        for bad in [
            r#"{"fn":"entry_hash","args":{}}"#,
            r#"{"fn":"merkle_root","args":{}}"#,
            r#"{"fn":"merkle_root","args":{"leaves":"leaf-0"}}"#,
            r#"{"fn":"merkle_root","args":{"leaves_base64":["!"]}}"#,
            r#"{"fn":"verify_inclusion","args":{}}"#,
            r#"{"fn":"verify_export","args":{"segments":[]}}"#,
            r#"{"fn":"verify_export","args":{"manifest":""}}"#,
            r#"{"fn":"verify_export","args":{"manifest":"","segments":[],"max_findings":0}}"#,
            r#"{"fn":"verify_export","args":{"manifest":"","segments":[],"max_findings":null}}"#,
        ] {
            assert_eq!(error_code(&call(bad)), Some("E_USAGE"), "{bad}");
        }
    }

    #[test]
    fn verify_export_returns_a_report_not_an_error() {
        let v =
            call(r#"{"fn":"verify_export","args":{"manifest":"","segments":[],"max_findings":1}}"#);
        let report = v.get("result").unwrap();
        assert_eq!(report.get("v").unwrap().as_str(), Some("store-report/0"));
        assert_eq!(report.get("source").unwrap().as_str(), Some("export"));
        assert_eq!(report.get("ok"), Some(&Value::Bool(false)));
        assert_eq!(report.get("truncated"), Some(&Value::Bool(true)));
        let findings = report.get("findings").unwrap().as_array().unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(
            findings[0].get("code").unwrap().as_str(),
            Some("S_MANIFEST_INVALID")
        );
    }

    /// Every export case through `handle`, as a host would call it, compared
    /// with the golden report in canonical form (the WebAssembly and Python
    /// tests do the same through the built module and package).
    #[test]
    fn export_vectors_through_the_abi() {
        use std::fs;
        let exports = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-vectors/store/exports");
        let mut cases: Vec<_> = fs::read_dir(&exports)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        cases.sort();
        assert!(!cases.is_empty());
        for dir in cases {
            let case = json::parse(&fs::read(dir.join("case.json")).unwrap()).unwrap();
            let mut names: Vec<String> = fs::read_dir(&dir)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .filter(|n| n.ends_with(".jsonl"))
                .collect();
            names.sort();
            let segments: Vec<Value> = names
                .iter()
                .map(|n| Value::String(STANDARD.encode(fs::read(dir.join(n)).unwrap())))
                .collect();
            let mut args = vec![
                (
                    "manifest_base64",
                    Value::String(STANDARD.encode(fs::read(dir.join("manifest.json")).unwrap())),
                ),
                ("segments_base64", Value::Array(segments)),
            ];
            if let Some(n) = case.get("max_findings") {
                args.push(("max_findings", n.clone()));
            }
            let request = obj(vec![
                ("fn", Value::String("verify_export".into())),
                ("args", obj(args)),
            ]);
            let v = call(&canon::canonicalize(&request));
            assert_eq!(
                canon::canonicalize(v.get("result").unwrap()),
                canon::canonicalize(case.get("expect").unwrap()),
                "{}",
                dir.display()
            );
        }
    }

    #[test]
    fn raw_abi_round_trip() {
        let request = br#"{"fn":"version"}"#;
        let ptr = evidence_alloc(request.len());
        unsafe {
            std::ptr::copy_nonoverlapping(request.as_ptr(), ptr, request.len());
            let out = evidence_call(ptr, request.len());
            evidence_free(ptr, request.len());
            let len =
                u32::from_le_bytes(std::slice::from_raw_parts(out, 4).try_into().unwrap()) as usize;
            let body = std::slice::from_raw_parts(out.add(4), len).to_vec();
            evidence_free(out, 4 + len);
            let v = json::parse(&body).unwrap();
            let spec = v.get("result").unwrap().get("spec").unwrap().as_str();
            assert_eq!(spec, Some("evidence/0"));
        }
    }
}
