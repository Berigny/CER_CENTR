//! The evidence/0 core as a WebAssembly module.
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
//! Byte inputs (`input`, `attestations`, `store` entries) are passed either as
//! a string under their name, or as standard base64 under `<name>_base64` when
//! the exact bytes matter (for example, a file that may not be valid UTF-8).
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

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use evidence_core::api::{self, VerifyArgs};
use evidence_core::json::{self, Value};
use evidence_core::{canon, Code, Error, Result};

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
