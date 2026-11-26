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
    };
    p.ws();
    let v = p.value()?;
    p.ws();
    if p.pos != p.s.len() {
        return Err(p.err("trailing characters after the JSON value"));
    }
    Ok(v)
}

struct Parser<'a> {
    s: &'a [u8],
    pos: usize,
    depth: usize,
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
                return Err(Error::new(
                    Code::DuplicateKey,
                    format!("member name {key:?} appears more than once"),
                ));
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
            return Err(Error::new(
                Code::NumberRange,
                format!("number {text} has magnitude above 2^53 - 1; use a string"),
            ));
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
        assert_eq!(code(r#"{"a":1,"a":2}"#), Code::DuplicateKey);
        assert_eq!(code("9007199254740992"), Code::NumberRange);
        assert_eq!(code("1e400"), Code::NumberRange);
        assert_eq!(code(r#""\ud800""#), Code::Parse);
        assert_eq!(code(r#""\udc00\ud800""#), Code::Parse);
        assert_eq!(code("01"), Code::Parse);
        assert_eq!(code("[1,]"), Code::Parse);
        assert_eq!(code("\u{feff}{}"), Code::Parse);
        assert_eq!(code(&"[".repeat(MAX_DEPTH + 1)), Code::Parse);
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
