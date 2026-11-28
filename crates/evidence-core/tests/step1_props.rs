//! Property test for the order of evidence/0's step-1 codes, with a fixed
//! seed, so every run draws the same cases and no dependency is needed.
//!
//! Each case is a generated document that is valid strict JSON, with faults
//! of one, two or all three kinds put in at random places: a parse fault
//! (bad syntax, a lone surrogate, bytes that are not UTF-8, nesting past this
//! parser's `MAX_DEPTH`, a byte order mark, bytes after the value), a number
//! out of range, or a member name used twice in one object (sometimes spelled
//! with an escape). Whatever the positions, the code must be `E_PARSE` if there is a
//! parse fault, otherwise `E_NUMBER_RANGE` if there is a number out of
//! range, otherwise `E_DUPLICATE_KEY`. Documents with no fault must parse.

use std::time::{Duration, Instant};

use evidence_core::json::{self, MAX_DEPTH};
use evidence_core::Code;

const CASES: usize = 3000;

/// splitmix64.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number in `0..n`.
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

/// A document as a tree, so a fault can go anywhere in it. Scalars and member
/// names are kept as the bytes of their tokens, valid or not.
enum Node {
    Token(Vec<u8>),
    Array(Vec<Node>),
    Object(Vec<(Vec<u8>, Node)>),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Fault {
    Parse,
    Range,
    Duplicate,
}

const SAFE_NUMBERS: &[&str] = &[
    "0",
    "-0",
    "1",
    "-1",
    "6.6",
    "0.5",
    "1e2",
    "1E-7",
    "2.5e+10",
    "123456789.125",
    "9007199254740991",
    "-9007199254740991",
    "9007199254740991.4",
    "5e-324",
    "1e-400",
];

/// Out of range: magnitude above 2^53 - 1 once rounded to a double, or
/// overflow to infinity.
const RANGE_FAULTS: &[&str] = &[
    "9007199254740992",
    "-9007199254740992",
    "9007199254740993",
    "9007199254740991.5",
    "1e16",
    "1e400",
    "-1e400",
    "1E309",
    "18446744073709551616",
    "123456789012345678901234567890",
];

/// Values that are not strict JSON wherever they stand.
const PARSE_FAULTS: &[&[u8]] = &[
    b"01",
    b"-",
    b"1.",
    b"1e",
    b".5",
    b"+1",
    b"tru",
    b"nul",
    b"NaN",
    b"Infinity",
    b"'a'",
    b"[1,]",
    b"[1 2]",
    b"{\"a\" 1}",
    b"{\"a\":1,}",
    b"\"\\ud800\"",
    b"\"x\\udc00\"",
    b"\"\\ud800\\u0041\"",
    b"\"a\x01b\"",
    b"\"\\x\"",
    b"\"\xff\"",
    b"\"\xed\xa0\x80\"",
    b"\"\xc0\xaf\"",
    b"\"unterminated",
];

/// Member names that are not strict JSON.
const NAME_FAULTS: &[&[u8]] = &[b"\"\\udc00\"", b"abc", b"\"a\x02\"", b"'a'"];

/// Bytes after a complete value.
const TRAILING_FAULTS: &[&[u8]] = &[b" x", b" {}", b"1", b",", b"\x00", b" \"\""];

const WHITESPACE: &[&[u8]] = &[b"", b"", b"", b" ", b"\n", b"\t", b"\r\n  "];

fn valid_string(rng: &mut Rng) -> Vec<u8> {
    let mut out = b"\"".to_vec();
    for _ in 0..rng.below(6) {
        let part: &[u8] = rng.pick(&[
            &b"a"[..],
            b"Z",
            b"7",
            b" ",
            b"\\n",
            b"\\\"",
            b"\\\\",
            b"\\/",
            b"\\u00e9",
            b"\\ud83d\\ude00",
            "\u{e9}".as_bytes(),
            "\u{20ac}".as_bytes(),
            "\u{1F600}".as_bytes(),
        ]);
        out.extend_from_slice(part);
    }
    out.push(b'"');
    out
}

fn safe_number(rng: &mut Rng) -> Vec<u8> {
    if rng.below(2) == 0 {
        rng.pick(SAFE_NUMBERS).as_bytes().to_vec()
    } else {
        // Below 2^53.
        (rng.next() >> 11).to_string().into_bytes()
    }
}

/// The name `letters` + `n`, as a token; sometimes its first letter is
/// written as a `\u` escape. Names in one object differ in `n`.
fn name(rng: &mut Rng, n: usize) -> Vec<u8> {
    let first = b"abcdefgh"[rng.below(8)];
    let mut out = b"\"".to_vec();
    if rng.below(4) == 0 {
        out.extend_from_slice(format!("\\u{first:04x}").as_bytes());
    } else {
        out.push(first);
    }
    out.extend_from_slice(format!("{n}\"").as_bytes());
    out
}

/// The same name with its first letter spelled the other way. A name that
/// [`name`] did not make (one of [`NAME_FAULTS`]) is kept as it is.
fn respelled(token: &[u8]) -> Vec<u8> {
    let escaped = token
        .strip_prefix(b"\"\\u")
        .and_then(|rest| Some((std::str::from_utf8(rest.get(..4)?).ok()?, &rest[4..])))
        .and_then(|(hex, rest)| Some((u8::from_str_radix(hex, 16).ok()?, rest)));
    match (escaped, token) {
        (Some((letter, rest)), _) => [&[b'"', letter][..], rest].concat(),
        (None, [b'"', letter @ b'a'..=b'h', rest @ ..]) => {
            [format!("\"\\u{letter:04X}").as_bytes(), rest].concat()
        }
        _ => token.to_vec(),
    }
}

fn valid_value(rng: &mut Rng, depth: usize) -> Node {
    let kinds = if depth >= 5 { 4 } else { 6 };
    match rng.below(kinds) {
        0 => Node::Token(rng.pick(&[&b"null"[..], b"true", b"false"]).to_vec()),
        1 | 2 => Node::Token(safe_number(rng)),
        3 => Node::Token(valid_string(rng)),
        4 => Node::Array(
            (0..rng.below(5))
                .map(|_| valid_value(rng, depth + 1))
                .collect(),
        ),
        _ => Node::Object(
            (0..rng.below(5))
                .map(|n| (name(rng, n), valid_value(rng, depth + 1)))
                .collect(),
        ),
    }
}

/// A valid document whose top level is an array or an object.
fn document(rng: &mut Rng) -> Node {
    loop {
        let node = valid_value(rng, 0);
        if matches!(node, Node::Array(_) | Node::Object(_)) {
            return node;
        }
    }
}

/// The number of arrays and objects in `node`, itself included.
fn containers(node: &Node) -> usize {
    match node {
        Node::Token(_) => 0,
        Node::Array(items) => 1 + items.iter().map(containers).sum::<usize>(),
        Node::Object(members) => 1 + members.iter().map(|(_, v)| containers(v)).sum::<usize>(),
    }
}

/// The `k`-th container of `node` in pre-order, counting from 0.
fn container<'a>(node: &'a mut Node, k: &mut usize) -> Option<&'a mut Node> {
    if matches!(node, Node::Token(_)) {
        return None;
    }
    if *k == 0 {
        return Some(node);
    }
    *k -= 1;
    match node {
        Node::Token(_) => None,
        Node::Array(items) => items.iter_mut().find_map(|v| container(v, k)),
        Node::Object(members) => members.iter_mut().find_map(|(_, v)| container(v, k)),
    }
}

fn random_container<'a>(rng: &mut Rng, doc: &'a mut Node) -> &'a mut Node {
    let mut k = rng.below(containers(doc));
    container(doc, &mut k).unwrap()
}

/// A fresh name for `members`: one no member of them has.
fn fresh_name(rng: &mut Rng, members: &[(Vec<u8>, Node)]) -> Vec<u8> {
    name(rng, members.len() + 1000)
}

/// Puts `value` in a random container of `doc`, at a random position.
fn insert_value(rng: &mut Rng, doc: &mut Node, value: Node) {
    match random_container(rng, doc) {
        Node::Array(items) => {
            let at = rng.below(items.len() + 1);
            items.insert(at, value);
        }
        Node::Object(members) => {
            let key = fresh_name(rng, members);
            let at = rng.below(members.len() + 1);
            members.insert(at, (key, value));
        }
        Node::Token(_) => unreachable!(),
    }
}

/// What goes before and after the rendered document.
#[derive(Default)]
struct Frame {
    bom: bool,
    trailing: Vec<u8>,
    truncate: bool,
}

fn inject(rng: &mut Rng, doc: &mut Node, frame: &mut Frame, fault: Fault) {
    match fault {
        Fault::Range => {
            let token = rng.pick(RANGE_FAULTS).as_bytes().to_vec();
            insert_value(rng, doc, Node::Token(token));
        }
        Fault::Duplicate => match random_container(rng, doc) {
            Node::Object(members) if !members.is_empty() => {
                let original = rng.below(members.len());
                let mut key = members[original].0.clone();
                if rng.below(2) == 0 {
                    key = respelled(&key);
                }
                let value = valid_value(rng, 4);
                let at = rng.below(members.len() + 1);
                members.insert(at, (key, value));
            }
            other => {
                // An empty object, or an array: put in an object that holds
                // the repeated name.
                let key = name(rng, 0);
                let pair = vec![
                    (key.clone(), valid_value(rng, 4)),
                    (respelled(&key), valid_value(rng, 4)),
                ];
                match other {
                    Node::Array(items) => {
                        let at = rng.below(items.len() + 1);
                        items.insert(at, Node::Object(pair));
                    }
                    Node::Object(members) => members.extend(pair),
                    Node::Token(_) => unreachable!(),
                }
            }
        },
        Fault::Parse => match rng.below(10) {
            0..=4 => {
                let token = rng.pick(PARSE_FAULTS).to_vec();
                insert_value(rng, doc, Node::Token(token));
            }
            5 => {
                let deep = MAX_DEPTH + 1;
                let token = [vec![b'['; deep], vec![b']'; deep]].concat();
                insert_value(rng, doc, Node::Token(token));
            }
            6 => match random_container(rng, doc) {
                Node::Object(members) => {
                    let at = rng.below(members.len() + 1);
                    let value = valid_value(rng, 4);
                    members.insert(at, (rng.pick(NAME_FAULTS).to_vec(), value));
                }
                Node::Array(items) => {
                    let bad = vec![(rng.pick(NAME_FAULTS).to_vec(), Node::Token(b"1".to_vec()))];
                    let at = rng.below(items.len() + 1);
                    items.insert(at, Node::Object(bad));
                }
                Node::Token(_) => unreachable!(),
            },
            7 => frame.bom = true,
            8 => frame.trailing = rng.pick(TRAILING_FAULTS).to_vec(),
            _ => frame.truncate = true,
        },
    }
}

fn ws(rng: &mut Rng, out: &mut Vec<u8>) {
    out.extend_from_slice(rng.pick(WHITESPACE));
}

fn render(rng: &mut Rng, node: &Node, out: &mut Vec<u8>) {
    match node {
        Node::Token(bytes) => out.extend_from_slice(bytes),
        Node::Array(items) => {
            out.push(b'[');
            for (i, v) in items.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                ws(rng, out);
                render(rng, v, out);
                ws(rng, out);
            }
            out.push(b']');
        }
        Node::Object(members) => {
            out.push(b'{');
            for (i, (k, v)) in members.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                ws(rng, out);
                out.extend_from_slice(k);
                ws(rng, out);
                out.push(b':');
                ws(rng, out);
                render(rng, v, out);
                ws(rng, out);
            }
            out.push(b'}');
        }
    }
}

fn want(faults: &[Fault]) -> Option<Code> {
    if faults.contains(&Fault::Parse) {
        Some(Code::Parse)
    } else if faults.contains(&Fault::Range) {
        Some(Code::NumberRange)
    } else if faults.contains(&Fault::Duplicate) {
        Some(Code::DuplicateKey)
    } else {
        None
    }
}

#[test]
fn step_one_codes_follow_the_precedence_wherever_the_faults_are() {
    let mut rng = Rng(0x5354_4550_315F_5052); // fixed seed
    let kinds = [Fault::Parse, Fault::Range, Fault::Duplicate];
    let mut seen = std::collections::BTreeMap::new();
    for case in 0..CASES {
        let mut doc = document(&mut rng);
        let mut frame = Frame::default();
        // Case 0 of every 8 has no fault; the others use subsets 1-7 of the
        // three kinds, with one to three faults of each kind chosen.
        let subset = case % 8;
        let mut faults = Vec::new();
        for (bit, kind) in kinds.iter().enumerate() {
            if subset & (1 << bit) != 0 {
                for _ in 0..1 + rng.below(3) {
                    faults.push(*kind);
                }
            }
        }
        // Inject in a random order, so no kind always lands first.
        for i in (1..faults.len()).rev() {
            let j = rng.below(i + 1);
            faults.swap(i, j);
        }
        for fault in &faults {
            inject(&mut rng, &mut doc, &mut frame, *fault);
        }
        let mut bytes = Vec::new();
        if frame.bom {
            bytes.extend_from_slice(b"\xEF\xBB\xBF");
        }
        bytes.extend_from_slice(rng.pick(WHITESPACE));
        render(&mut rng, &doc, &mut bytes);
        if frame.truncate {
            bytes.pop();
        }
        bytes.extend_from_slice(&frame.trailing);
        bytes.extend_from_slice(rng.pick(WHITESPACE));

        let got = json::parse(&bytes).err().map(|e| e.code);
        let expected = want(&faults);
        assert_eq!(
            got,
            expected,
            "case {case}, faults {faults:?}: {}",
            String::from_utf8_lossy(&bytes)
        );
        *seen.entry(format!("{expected:?}")).or_insert(0usize) += 1;
    }
    // Every outcome was drawn many times.
    assert_eq!(seen.len(), 4, "{seen:?}");
    assert!(seen.values().all(|&n| n >= CASES / 10), "{seen:?}");
}

/// The parser reads on past repeated names and numbers out of range, so a
/// long document full of both, with one syntax error at the very end, still
/// takes one linear pass: well under a second even in a debug build.
#[test]
fn reading_on_past_faults_stays_linear() {
    let n = 200_000;
    let mut text = String::from("{");
    for i in 0..n {
        text.push_str(&format!("\"a\":1e400,\"b{i}\":[9007199254740993],"));
    }
    text.push_str("\"end\":}");
    let start = Instant::now();
    assert_eq!(json::parse(text.as_bytes()).unwrap_err().code, Code::Parse);
    let mut fixed = text.clone();
    fixed.truncate(fixed.len() - 1);
    fixed.push_str("0}");
    assert_eq!(
        json::parse(fixed.as_bytes()).unwrap_err().code,
        Code::NumberRange
    );
    let elapsed = start.elapsed();
    assert!(elapsed < Duration::from_secs(10), "took {elapsed:?}");
}
