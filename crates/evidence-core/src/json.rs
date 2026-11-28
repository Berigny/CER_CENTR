//! Strict JSON (RFC 8259 / I-JSON RFC 7493) parser.
//!
//! Stricter than most parsers on purpose, because two parsers that read the
//! same bytes differently will hash them differently:
//! - input must be UTF-8 with no byte order mark;
//! - duplicate member names are rejected, not "last one wins";
//! - lone surrogates (`"\ud800"`) are rejected;
//! - numbers are IEEE 754 doubles, and any number with magnitude above
//!   2^53 - 1 is rejected (identifiers, big integers and money belong in strings);
//! - nesting is limited to [`MAX_DEPTH`].
//!
//! When the bytes have more than one fault, the code follows a fixed
//! precedence, whatever the faults' byte positions (evidence/0 step 1):
//! `E_PARSE` if they have any fault other than a number out of range or a
//! repeated name, otherwise `E_NUMBER_RANGE` if any number is out of range,
//! otherwise `E_DUPLICATE_KEY`.

use std::collections::HashSet;

use crate::error::{Code, Error, Result};

pub const MAX_DEPTH: usize = 128;
pub const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Value>),
    /// Members in input order. Canonicalisation sorts them.
    Object(Vec<(String, Value)>),
}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(members) => members.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(items) => Some(items),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&[(String, Value)]> {
        match self {
            Value::Object(members) => Some(members),
            _ => None,
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "boolean",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
        }
    }

    /// A copy of this object without `key`. Non-objects are returned unchanged.
    pub fn without(&self, key: &str) -> Value {
        match self {
            Value::Object(members) => {
                Value::Object(members.iter().filter(|(k, _)| k != key).cloned().collect())
            }
            other => other.clone(),
        }
    }

    /// A copy of this object with `key` set to `value` (replaced or appended).
    pub fn with(&self, key: &str, value: Value) -> Value {
        match self {
            Value::Object(members) => {
                let mut out: Vec<(String, Value)> = Vec::with_capacity(members.len() + 1);
                let mut replaced = false;
                for (k, v) in members {
                    if k == key {
                        out.push((k.clone(), value.clone()));
                        replaced = true;
                    } else {
                        out.push((k.clone(), v.clone()));
                    }
                }
                if !replaced {
                    out.push((key.to_string(), value));
                }
                Value::Object(out)
            }
            other => other.clone(),
        }
    }
}

/// Parse one JSON text under the strict rules above.
///
/// Every fault that is not a number out of range or a repeated member name is
/// `E_PARSE` (not UTF-8, a byte order mark, a syntax error, an unescaped
/// control character, a lone surrogate, bytes after the value, nesting deeper
/// than [`MAX_DEPTH`]), and the parser stops there. It notes the first number
/// out of range and the first repeated name and reads on, so one pass over
/// the bytes finds the code of the precedence: `E_PARSE`, then
/// `E_NUMBER_RANGE`, then `E_DUPLICATE_KEY`.
pub fn parse(bytes: &[u8]) -> Result<Value> {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return Err(Error::new(Code::Parse, "byte order mark is not allowed"));
    }
    if std::str::from_utf8(bytes).is_err() {
        return Err(Error::new(Code::Parse, "input is not valid UTF-8"));
    }
    let mut p = Parser {
        s: bytes,
        pos: 0,
        depth: 0,
        number_range: None,
        duplicate_key: None,
    };
    p.ws();
    let v = p.value()?;
    p.ws();
    if p.pos != p.s.len() {
        return Err(p.err("trailing characters after the JSON value"));
    }
    if let Some(e) = p.number_range {
        return Err(e);
    }
    if let Some(e) = p.duplicate_key {
        return Err(e);
    }
    Ok(v)
}

struct Parser<'a> {
    s: &'a [u8],
    pos: usize,
    depth: usize,
    /// The first number out of range, reported only if no `E_PARSE` fault follows.
    number_range: Option<Error>,
    /// The first repeated member name, reported only if there is no other fault.
    duplicate_key: Option<Error>,
}

impl Parser<'_> {
    fn err(&self, msg: &str) -> Error {
        Error::new(Code::Parse, format!("{msg} (byte {})", self.pos))
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.pos).copied()
    }

    fn ws(&mut self) {
        while let Some(b' ' | b'\t' | b'\n' | b'\r') = self.peek() {
            self.pos += 1;
        }
    }

    fn expect(&mut self, b: u8) -> Result<()> {
        if self.peek() == Some(b) {
            self.pos += 1;
            Ok(())
        } else {
            Err(self.err(&format!("expected '{}'", b as char)))
        }
    }

    fn value(&mut self) -> Result<Value> {
        match self.peek() {
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => Ok(Value::String(self.string()?)),
            Some(b't') => self.literal(b"true", Value::Bool(true)),
            Some(b'f') => self.literal(b"false", Value::Bool(false)),
            Some(b'n') => self.literal(b"null", Value::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => Err(self.err("unexpected character")),
            None => Err(self.err("unexpected end of input")),
        }
    }

    fn enter(&mut self) -> Result<()> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(self.err(&format!("nesting deeper than {MAX_DEPTH}")));
        }
        Ok(())
    }

    fn object(&mut self) -> Result<Value> {
        self.enter()?;
        self.expect(b'{')?;
        let mut members = Vec::new();
        let mut seen = HashSet::new();
        self.ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            self.depth -= 1;
            return Ok(Value::Object(members));
        }
        loop {
            self.ws();
            if self.peek() != Some(b'"') {
                return Err(self.err("expected a member name"));
            }
            let key = self.string()?;
            if !seen.insert(key.clone()) {
                self.duplicate_key.get_or_insert_with(|| {
                    Error::new(
                        Code::DuplicateKey,
                        format!("member name {key:?} appears more than once"),
                    )
                });
            }
            self.ws();
            self.expect(b':')?;
            self.ws();
            let v = self.value()?;
            members.push((key, v));
            self.ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    break;
                }
                _ => return Err(self.err("expected ',' or '}'")),
            }
        }
        self.depth -= 1;
        Ok(Value::Object(members))
    }

    fn array(&mut self) -> Result<Value> {
        self.enter()?;
        self.expect(b'[')?;
        let mut items = Vec::new();
        self.ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            self.depth -= 1;
            return Ok(Value::Array(items));
        }
        loop {
            self.ws();
            items.push(self.value()?);
            self.ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    break;
                }
                _ => return Err(self.err("expected ',' or ']'")),
            }
        }
        self.depth -= 1;
        Ok(Value::Array(items))
    }

    fn literal(&mut self, word: &[u8], v: Value) -> Result<Value> {
        if self.s[self.pos..].starts_with(word) {
            self.pos += word.len();
            Ok(v)
        } else {
            Err(self.err("invalid literal"))
        }
    }

    fn hex4(&mut self) -> Result<u32> {
        let end = self.pos + 4;
        let digits = self
            .s
            .get(self.pos..end)
            .ok_or_else(|| self.err("truncated \\u escape"))?;
        let text = std::str::from_utf8(digits).map_err(|_| self.err("invalid \\u escape"))?;
        if !text.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(self.err("invalid \\u escape"));
        }
        let v = u32::from_str_radix(text, 16).map_err(|_| self.err("invalid \\u escape"))?;
        self.pos = end;
        Ok(v)
    }

    fn string(&mut self) -> Result<String> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            // Copy the longest run of ordinary bytes in one go. Run boundaries are
            // ASCII bytes, so they are always UTF-8 character boundaries.
            let start = self.pos;
            while let Some(b) = self.peek() {
                if b == b'"' || b == b'\\' || b < 0x20 {
                    break;
                }
                self.pos += 1;
            }
            out.push_str(std::str::from_utf8(&self.s[start..self.pos]).expect("validated UTF-8"));
            match self.peek() {
                None => return Err(self.err("unterminated string")),
                Some(b'"') => {
                    self.pos += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.pos += 1;
                    let esc = self.peek().ok_or_else(|| self.err("unterminated escape"))?;
                    self.pos += 1;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hi = self.hex4()?;
                            let cp = if (0xD800..0xDC00).contains(&hi) {
                                if !self.s[self.pos..].starts_with(b"\\u") {
                                    return Err(self.err("lone high surrogate"));
                                }
                                self.pos += 2;
                                let lo = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&lo) {
                                    return Err(self.err("lone high surrogate"));
                                }
                                0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
                            } else if (0xDC00..0xE000).contains(&hi) {
                                return Err(self.err("lone low surrogate"));
                            } else {
                                hi
                            };
                            out.push(
                                char::from_u32(cp).ok_or_else(|| self.err("invalid code point"))?,
                            );
                        }
                        _ => return Err(self.err("invalid escape")),
                    }
                }
                Some(_) => return Err(self.err("unescaped control character in string")),
            }
        }
    }

    fn digits(&mut self) -> usize {
        let start = self.pos;
        while let Some(b'0'..=b'9') = self.peek() {
            self.pos += 1;
        }
        self.pos - start
    }

    fn number(&mut self) -> Result<Value> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        match self.peek() {
            Some(b'0') => {
                self.pos += 1;
                if let Some(b'0'..=b'9') = self.peek() {
                    return Err(self.err("leading zero in number"));
                }
            }
            Some(b'1'..=b'9') => {
                self.digits();
            }
            _ => return Err(self.err("invalid number")),
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            if self.digits() == 0 {
                return Err(self.err("digit expected after '.'"));
            }
        }
        if let Some(b'e' | b'E') = self.peek() {
            self.pos += 1;
            if let Some(b'+' | b'-') = self.peek() {
                self.pos += 1;
            }
            if self.digits() == 0 {
                return Err(self.err("digit expected in exponent"));
            }
        }
        let text = std::str::from_utf8(&self.s[start..self.pos]).expect("ASCII");
        // Rust's float parser is correctly rounded, like ECMAScript's.
        let v: f64 = text.parse().map_err(|_| self.err("invalid number"))?;
        if v.abs() > MAX_SAFE_INTEGER || !v.is_finite() {
            self.number_range.get_or_insert_with(|| {
                Error::new(
                    Code::NumberRange,
                    format!("number {text} has magnitude above 2^53 - 1; use a string"),
                )
            });
        }
        Ok(Value::Number(v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(input: &str) -> Code {
        parse(input.as_bytes()).unwrap_err().code
    }

    #[test]
    fn rejects_what_lenient_parsers_accept() {
        assert_eq!(code(r#"{"a":1,"a":2}"#), Code::DuplicateKey);
        assert_eq!(code(r#"{"a":1,"\u0061":2}"#), Code::DuplicateKey);
        assert_eq!(code("9007199254740992"), Code::NumberRange);
        assert_eq!(code("1e400"), Code::NumberRange);
        assert_eq!(code(r#""\ud800""#), Code::Parse);
        assert_eq!(code(r#""\udc00\ud800""#), Code::Parse);
        assert_eq!(code("01"), Code::Parse);
        assert_eq!(code("[1,]"), Code::Parse);
        assert_eq!(code("\u{feff}{}"), Code::Parse);
        assert_eq!(code(&"[".repeat(MAX_DEPTH + 1)), Code::Parse);
    }

    /// Step 1's precedence: `E_PARSE`, then `E_NUMBER_RANGE`, then
    /// `E_DUPLICATE_KEY`, in whichever byte order the faults come.
    #[test]
    fn step_one_precedence() {
        let big = "9007199254740993";
        for (input, want) in [
            // Each pair, both ways round.
            (format!(r#"{{"a":1,"a":2,"b":{big}}}"#), Code::NumberRange),
            (format!(r#"{{"b":{big},"a":1,"a":2}}"#), Code::NumberRange),
            (r#"{"a":1,"a":2,"b":}"#.to_string(), Code::Parse),
            (r#"{"b":,"a":1,"a":2}"#.to_string(), Code::Parse),
            (format!(r#"{{"b":{big},"c":01}}"#), Code::Parse),
            (format!(r#"{{"c":01,"b":{big}}}"#), Code::Parse),
            // All three.
            (format!(r#"{{"a":1,"a":2,"b":{big},"c":tru}}"#), Code::Parse),
            (
                format!(r#"{{"c":[1,],"b":{big},"a":1,"a":2}}"#),
                Code::Parse,
            ),
            (format!(r#"{{"b":{big},"a":1,"a":2"#), Code::Parse),
            // Faults in nested values.
            (
                format!(r#"[{{"a":{{"x":1,"x":2}}}},[[{big}]]]"#),
                Code::NumberRange,
            ),
            (
                r#"{"a":[[-1e400]],"b":{"x":1,"x":2}}"#.to_string(),
                Code::NumberRange,
            ),
            (r#"[{"x":1,"x":2},{"y":"\udc00"}]"#.to_string(), Code::Parse),
            // The number is the value of a repeated name, first or second.
            (format!(r#"{{"a":{big},"a":1}}"#), Code::NumberRange),
            (format!(r#"{{"a":1,"a":{big}}}"#), Code::NumberRange),
            // A lone surrogate is E_PARSE, before or after the other faults.
            (format!(r#"{{"a":{big},"b":"\ud800"}}"#), Code::Parse),
            (r#"{"a":"\ud800","a":1}"#.to_string(), Code::Parse),
            // Bytes after a document that only repeats a name.
            (r#"{"a":1,"a":2} x"#.to_string(), Code::Parse),
            (r#"{"a":1,"a":2}{}"#.to_string(), Code::Parse),
            // A name spelled with an escape still repeats it.
            (
                format!(r#"{{"a":1,"\u0061":2,"b":{big}}}"#),
                Code::NumberRange,
            ),
            (r#"{"a":1,"\u0061":2}"#.to_string(), Code::DuplicateKey),
            // Nesting past the limit, after the other two faults.
            (
                format!(
                    r#"{{"a":1,"a":{big},"b":{}{}}}"#,
                    "[".repeat(MAX_DEPTH),
                    "]".repeat(MAX_DEPTH)
                ),
                Code::Parse,
            ),
        ] {
            assert_eq!(code(&input), want, "{input}");
        }
        // Not UTF-8, or a byte order mark, after or before the other faults.
        let mut bytes = format!(r#"{{"a":1,"a":{big},"b":""#).into_bytes();
        bytes.extend_from_slice(b"\xff\"}");
        assert_eq!(parse(&bytes).unwrap_err().code, Code::Parse);
        let mut bytes = b"\xEF\xBB\xBF".to_vec();
        bytes.extend_from_slice(format!(r#"{{"a":1,"a":{big}}}"#).as_bytes());
        assert_eq!(parse(&bytes).unwrap_err().code, Code::Parse);
    }

    /// The detail names the first fault of the reported kind.
    #[test]
    fn step_one_detail_is_the_first_of_its_kind() {
        let e = parse(br#"{"a":1,"b":1e400,"a":2,"b":2,"c":-1e400}"#).unwrap_err();
        assert_eq!(e.code, Code::NumberRange);
        assert!(e.detail.contains("1e400") && !e.detail.contains("-1e400"));
        let e = parse(br#"{"a":1,"b":1,"b":2,"a":2}"#).unwrap_err();
        assert_eq!(e.code, Code::DuplicateKey);
        assert!(e.detail.contains("\"b\""));
    }

    #[test]
    fn accepts_edges() {
        assert_eq!(
            parse(b"9007199254740991").unwrap(),
            Value::Number(MAX_SAFE_INTEGER)
        );
        assert_eq!(parse(b"1e-400").unwrap(), Value::Number(0.0));
        assert_eq!(
            parse(br#""\ud83d\ude00""#).unwrap(),
            Value::String("\u{1F600}".into())
        );
    }
}
