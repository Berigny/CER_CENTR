//! RFC 8785 JSON Canonicalization Scheme (JCS) and SHA-256 digests.
//!
//! Output matches ECMAScript `JSON.stringify` for primitives, with object
//! members sorted by UTF-16 code units. The number formatter is checked against
//! V8's output in `test-vectors/numbers.json`.

use sha2::{Digest, Sha256};

use crate::json::Value;

/// Canonical JSON text for `v`.
pub fn canonicalize(v: &Value) -> String {
    let mut out = String::new();
    write_value(&mut out, v);
    out
}

/// `"sha256:" + hex(sha256(canonical bytes))`.
pub fn digest(v: &Value) -> String {
    sha256_hex(canonicalize(v).as_bytes())
}

/// `"sha256:" + hex(sha256(bytes))`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let hash = Sha256::digest(bytes);
    let mut out = String::with_capacity(7 + 64);
    out.push_str("sha256:");
    for b in hash.iter() {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

fn write_value(out: &mut String, v: &Value) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(n) => out.push_str(&format_number(*n)),
        Value::String(s) => write_string(out, s),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_value(out, item);
            }
            out.push(']');
        }
        Value::Object(members) => {
            let mut sorted: Vec<&(String, Value)> = members.iter().collect();
            sorted.sort_by(|a, b| a.0.encode_utf16().cmp(b.0.encode_utf16()));
            out.push('{');
            for (i, (k, val)) in sorted.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_string(out, k);
                out.push(':');
                write_value(out, val);
            }
            out.push('}');
        }
    }
}

fn write_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// ECMAScript `Number.prototype.toString()` for a finite double.
///
/// Rust's `{:e}` gives the shortest digits that round-trip; the ECMAScript
/// rules (ECMA-262, Number::toString) then decide where the point goes.
///
/// # Panics
/// If `n` is not finite. The parser never produces such a value.
pub fn format_number(n: f64) -> String {
    assert!(n.is_finite(), "JCS cannot represent {n}");
    if n == 0.0 {
        return "0".to_string(); // also -0
    }
    let (digits, exp) = shortest_digits(n.abs());
    let k = digits.len() as i32;
    let point = exp + 1; // ECMAScript's n: digits * 10^(point - k)

    let mut out = String::new();
    if n < 0.0 {
        out.push('-');
    }
    if k <= point && point <= 21 {
        out.push_str(&digits);
        out.push_str(&"0".repeat((point - k) as usize));
    } else if 0 < point && point <= 21 {
        out.push_str(&digits[..point as usize]);
        out.push('.');
        out.push_str(&digits[point as usize..]);
    } else if -6 < point && point <= 0 {
        out.push_str("0.");
        out.push_str(&"0".repeat((-point) as usize));
        out.push_str(&digits);
    } else {
        let e = point - 1;
        out.push_str(&digits[..1]);
        if k > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        out.push('e');
        out.push(if e < 0 { '-' } else { '+' });
        out.push_str(&e.abs().to_string());
    }
    out
}

/// Shortest round-tripping significant digits of `x > 0` and the decimal
/// exponent of the first digit, with ECMAScript's tie-break.
///
/// Rust's `{:e}` already gives the shortest, closest digits, but when the exact
/// value lies exactly halfway between two candidates it rounds up. ECMA-262
/// chooses the even candidate (e.g. 1424953923781206.25 -> "...206.2").
fn shortest_digits(x: f64) -> (String, i32) {
    let (digits, exp) = split_sci(&format!("{x:e}"));
    // Every double has a finite decimal expansion of at most 767 significant
    // digits, so 800 digits of precision print it exactly.
    let (exact, exact_exp) = split_sci(&format!("{x:.800e}"));
    let k = digits.len();
    if exact_exp == exp && exact.len() > k {
        let (head, tail) = exact.split_at(k);
        let is_tie = tail.starts_with('5') && tail[1..].bytes().all(|b| b == b'0');
        let head_is_even = head.as_bytes()[k - 1] % 2 == 0;
        if is_tie && head != digits && head_is_even {
            let candidate = format!("{}.{}e{exp}", &head[..1], &head[1..]);
            if candidate.parse::<f64>() == Ok(x) {
                let trimmed = head.trim_end_matches('0');
                return (trimmed.to_string(), exp);
            }
        }
    }
    (digits, exp)
}

/// `"d.ddde±x"` -> (`"dddd"` without trailing zeros, x).
fn split_sci(s: &str) -> (String, i32) {
    let (mantissa, exp) = s.split_once('e').expect("LowerExp output has an exponent");
    let mut digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    while digits.len() > 1 && digits.ends_with('0') {
        digits.pop();
    }
    (digits, exp.parse().expect("integer exponent"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::parse;

    #[test]
    fn rfc8785_appendix_b_samples() {
        for (bits, want) in [
            (0x0000000000000000u64, "0"),
            (0x8000000000000000, "0"),
            (0x0000000000000001, "5e-324"),
            (0x7fefffffffffffff, "1.7976931348623157e+308"),
            (0x4340000000000000, "9007199254740992"),
            (0x4430000000000000, "295147905179352830000"),
            (0x44b52d02c7e14af6, "1e+23"),
            (0x444b1ae4d6e2ef50, "1e+21"),
            (0x3eb0c6f7a0b5ed8d, "0.000001"),
            (0x3eb0c6f7a0b5ed8c, "9.999999999999997e-7"),
            (0x41b3de4355555553, "333333333.3333332"),
            (0xbecbf647612f3696, "-0.0000033333333333333333"),
            (0x43143ff3c1cb0959, "1424953923781206.2"),
        ] {
            assert_eq!(
                format_number(f64::from_bits(bits)),
                want,
                "bits {bits:016x}"
            );
        }
    }

    #[test]
    fn rfc8785_key_order_is_utf16() {
        let v = parse(
            r#"{"\u20ac":"Euro","\r":"CR","\ufb33":"Hebrew","1":"One","\ud83d\ude00":"Smiley","\u0080":"Control","\u00f6":"Latin"}"#
                .as_bytes(),
        )
        .unwrap();
        let order: Vec<String> = match crate::json::parse(canonicalize(&v).as_bytes()).unwrap() {
            Value::Object(m) => m
                .into_iter()
                .map(|(_, v)| v.as_str().unwrap().to_string())
                .collect(),
            _ => unreachable!(),
        };
        assert_eq!(
            order,
            ["CR", "One", "Control", "Latin", "Euro", "Smiley", "Hebrew"]
        );
    }
}
