//! The spec's own examples and case tables, built here from the text of
//! `spec/coordinate-0.md` rather than read from the vectors. `vectors.rs`
//! runs the generated vectors; this file checks the same rules without
//! them, and checks the ids and values the spec states.

mod common;

use common::*;
use evidence_coordinate::{
    extends, factors_value, Dictionary, DictionaryDetail, Error, ExtensionDetail,
};
use evidence_core::canon::canonicalize;
use evidence_core::json::Value;

fn with(base: &str, from: &str, to: &str) -> String {
    assert_eq!(base.matches(from).count(), 1, "{from:?} must occur once");
    base.replacen(from, to, 1)
}

/// `catalogue-demo` with a facet appended as the last one.
fn appended(facet: &str) -> String {
    with(
        CATALOGUE_DEMO,
        r#"{"id": "standard.as-nzs-4777-1", "prime": 67}]}"#,
        &format!(r#"{{"id": "standard.as-nzs-4777-1", "prime": 67}}, {facet}]}}"#),
    )
}

fn detail(bytes: &[u8]) -> Error {
    Dictionary::parse(bytes).expect_err("an invalid dictionary")
}

fn d(detail: DictionaryDetail) -> Error {
    Error::Dictionary { detail, core: None }
}

fn core(code: evidence_core::Code) -> Error {
    Error::Dictionary {
        detail: DictionaryDetail::Shape,
        core: Some(code),
    }
}

fn cardinality(family: &str) -> Error {
    Error::Cardinality {
        family: family.to_string(),
    }
}

fn unknown_facet(facet: &str) -> Error {
    Error::UnknownFacet {
        facet: facet.to_string(),
    }
}

fn ids(dictionary: &Dictionary, coordinate: &str) -> Vec<(String, u32)> {
    dictionary
        .decode(coordinate)
        .unwrap_or_else(|e| panic!("{coordinate}: {e}"))
        .into_iter()
        .map(|f| (f.id, f.exponent))
        .collect()
}

fn pow2(k: u32) -> String {
    let mut digits = vec![1u32];
    for _ in 0..k {
        let mut carry = 0;
        for d in &mut digits {
            let t = *d * 2 + carry;
            *d = t % 10;
            carry = t / 10;
        }
        if carry > 0 {
            digits.push(carry);
        }
    }
    digits
        .iter()
        .rev()
        .map(|d| char::from(b'0' + *d as u8))
        .collect()
}

/// Adds or subtracts a small number from a decimal string.
fn offset(n: &str, delta: i64) -> String {
    let mut digits: Vec<i64> = n.bytes().rev().map(|b| i64::from(b - b'0')).collect();
    digits[0] += delta;
    for i in 0..digits.len() {
        while digits[i] < 0 {
            digits[i] += 10;
            digits[i + 1] -= 1;
        }
        while digits[i] > 9 {
            digits[i] -= 10;
            if i + 1 == digits.len() {
                digits.push(0);
            }
            digits[i + 1] += 1;
        }
    }
    while digits.len() > 1 && digits.last() == Some(&0) {
        digits.pop();
    }
    digits
        .iter()
        .rev()
        .map(|d| char::from(b'0' + *d as u8))
        .collect()
}

#[test]
fn stated_ids() {
    assert_eq!(catalogue_demo().id(), CATALOGUE_DEMO_ID);
    assert_eq!(load(&kernel_2_value()).id(), KERNEL_2_ID);
}

#[test]
fn worked_example() {
    let demo = catalogue_demo();
    let four = [
        "record.installation",
        "product.inverter",
        "standard.as-nzs-4777-2",
        "jurisdiction.vic",
    ];
    assert_eq!(demo.encode(four).unwrap(), "1155");
    let mut reversed = four;
    reversed.reverse();
    assert_eq!(demo.encode(reversed).unwrap(), "1155");
    assert_eq!(
        ids(&demo, "1155"),
        four.iter()
            .map(|id| (id.to_string(), 1))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        demo.encode([
            "record.installation",
            "standard.as-nzs-4777-2",
            "jurisdiction.vic"
        ])
        .unwrap(),
        "231"
    );
    assert_eq!(demo.divides("231", "1155"), Ok(true));
    assert_eq!(demo.divides("231", "3255"), Ok(false));
    assert_eq!(
        demo.encode(["jurisdiction.vic", "jurisdiction.nsw"]),
        Err(cardinality("jurisdiction"))
    );
    assert_eq!(demo.decode("341"), Err(cardinality("jurisdiction")));
    assert_eq!(demo.decode("97"), Err(Error::UnknownFactor));
}

#[test]
fn error_objects() {
    let cases = [
        (
            core(evidence_core::Code::DuplicateKey),
            r#"{"code":"P_COORDINATE_DICTIONARY","core":"E_DUPLICATE_KEY","detail":"shape"}"#,
        ),
        (
            d(DictionaryDetail::PrimeOrder),
            r#"{"code":"P_COORDINATE_DICTIONARY","detail":"prime-order"}"#,
        ),
        (
            unknown_facet("product.heat-pump"),
            r#"{"code":"P_COORDINATE_UNKNOWN_FACET","facet":"product.heat-pump"}"#,
        ),
        (
            cardinality("jurisdiction"),
            r#"{"code":"P_COORDINATE_CARDINALITY","family":"jurisdiction"}"#,
        ),
        (
            Error::NotExtension {
                detail: ExtensionDetail::Facet,
            },
            r#"{"code":"P_COORDINATE_NOT_EXTENSION","detail":"facet"}"#,
        ),
        (Error::Syntax, r#"{"code":"P_COORDINATE_SYNTAX"}"#),
        (Error::Range, r#"{"code":"P_COORDINATE_RANGE"}"#),
        (
            Error::UnknownFactor,
            r#"{"code":"P_COORDINATE_UNKNOWN_FACTOR"}"#,
        ),
        (Error::Drift, r#"{"code":"P_COORDINATE_DRIFT"}"#),
    ];
    for (error, want) in cases {
        assert_eq!(canonicalize(&error.to_value()), want);
    }
}

#[test]
fn valid_dictionaries() {
    // catalogue-demo-reordered: members in another order, other whitespace,
    // \u escapes, 7 written 7.0 and 59 written 58.99999999999999999.
    let reordered = r#"{
  "facets":[{"prime":2,"id":"record.product-listing"},{"prime":3,"id":"record.installation"},
  {"prime":5,"id":"product.inverter"},{"prime":7.0,"id":"standard.as-nzs-4777-2"},
  {"prime":11,"id":"jurisdiction.vic"},{"prime":13,"id":"product.battery"},
  {"prime":17,"id":"product.pv-module"},{"prime":19,"id":"standard.as-nzs-5033"},
  {"prime":23,"id":"standard.as-nzs-3000"},{"prime":29,"id":"standard.as-nzs-5139"},
  {"prime":31,"id":"jurisdiction.nsw"},{"prime":37,"id":"jurisdiction.qld"},
  {"prime":41,"id":"jurisdiction.sa"},{"prime":43,"id":"jurisdiction.wa"},
  {"prime":47,"id":"jurisdiction.tas"},{"prime":53,"id":"jurisdiction.act"},
  {"prime":58.99999999999999999,"id":"jurisdiction.nt"},{"prime":61,"id":"record.inspection"},
  {"prime":67,"id":"standard.as-nzs-4777-1"}],
	"profile" : "catalogue-demo/0",
  "families":[{"values":"one","name":"jurisdiction"},{"values":"one","name":"product"},
  {"values":"one","name":"record"},{"values":"set","name":"standard"}],
  "v":"coordinate-dictionary/0"}"#;
    assert_eq!(
        Dictionary::parse(reordered.as_bytes()).unwrap().id(),
        CATALOGUE_DEMO_ID
    );

    let wide = load(&wide_demo_value());
    assert_eq!(wide.facets().len(), 4096);
    assert_eq!(wide.facets().last().unwrap().prime(), 38_873);
    let max = load(&max_prime_demo_value());
    assert_eq!(max.facets().len(), 1);
    assert_eq!(max.families()[1].name(), "unused");
}

/// `patterns-demo`: facet names that start with a digit, family names with
/// a hyphen or a digit, families in code point order, and a two-digit
/// profile version.
#[test]
fn patterns_demo() {
    let text = r#"{"v": "coordinate-dictionary/0", "profile": "patterns-demo/10",
 "families": [{"name": "zone-b", "values": "set"}, {"name": "zone2", "values": "multiset"},
   {"name": "zonea", "values": "one"}],
 "facets": [{"id": "zone-b.4777-3", "prime": 2}, {"id": "zonea.0", "prime": 3},
   {"id": "zone-b.x-1", "prime": 5}, {"id": "zone2.7", "prime": 7}]}"#;
    let patterns = Dictionary::parse(text.as_bytes()).unwrap();
    assert_eq!(
        patterns.encode([
            "zone2.7",
            "zone-b.x-1",
            "zone2.7",
            "zonea.0",
            "zone-b.4777-3"
        ]),
        Ok("1470".to_string())
    );
    assert_eq!(
        ids(&patterns, "1470"),
        [
            ("zone-b.4777-3", 1),
            ("zonea.0", 1),
            ("zone-b.x-1", 1),
            ("zone2.7", 2)
        ]
        .map(|(id, e)| (id.to_string(), e))
    );
    // Ignoring the hyphen would sort zonea before zone-b.
    let reordered = with(
        text,
        r#"{"name": "zone-b", "values": "set"}, {"name": "zone2", "values": "multiset"},
   {"name": "zonea", "values": "one"}"#,
        r#"{"name": "zone2", "values": "multiset"}, {"name": "zonea", "values": "one"},
   {"name": "zone-b", "values": "set"}"#,
    );
    assert_eq!(
        detail(reordered.as_bytes()),
        d(DictionaryDetail::FamilyOrder)
    );
}

/// An implementation that takes an already parsed value starts at step 2.
/// A NaN or an infinity, which only a value built in code can hold, fails
/// step 8d (`not-integer`), and a number above 2^53 - 1 fails step 8e
/// (`prime-range`).
#[test]
fn parsed_values() {
    use DictionaryDetail::*;

    let parsed = evidence_core::json::parse(CATALOGUE_DEMO.as_bytes()).unwrap();
    assert_eq!(
        Dictionary::from_value(&parsed).unwrap().id(),
        catalogue_demo().id()
    );
    let one_facet = |prime: f64| {
        let s = |x: &str| Value::String(x.to_string());
        Value::Object(vec![
            ("v".to_string(), s("coordinate-dictionary/0")),
            ("profile".to_string(), s("m-demo/0")),
            (
                "families".to_string(),
                Value::Array(vec![Value::Object(vec![
                    ("name".to_string(), s("m")),
                    ("values".to_string(), s("one")),
                ])]),
            ),
            (
                "facets".to_string(),
                Value::Array(vec![Value::Object(vec![
                    ("id".to_string(), s("m.a")),
                    ("prime".to_string(), Value::Number(prime)),
                ])]),
            ),
        ])
    };
    let cases = [
        (f64::NAN, NotInteger),
        (f64::INFINITY, NotInteger),
        (f64::NEG_INFINITY, NotInteger),
        (-0.0, PrimeRange),
        (2.5, NotInteger),
        (9_007_199_254_740_992.0, PrimeRange),
        (1e300, PrimeRange),
    ];
    for (prime, want) in cases {
        assert_eq!(
            Dictionary::from_value(&one_facet(prime)),
            Err(d(want)),
            "{prime}"
        );
    }
    assert!(Dictionary::from_value(&one_facet(2.0)).is_ok());
}

#[test]
fn invalid_dictionaries() {
    use evidence_core::Code;
    use DictionaryDetail::*;

    let base = CATALOGUE_DEMO;
    let product_first = with(
        base,
        r#"{"name": "jurisdiction", "values": "one"},
   {"name": "product", "values": "one"},"#,
        r#"{"name": "product", "values": "one"},
   {"name": "jurisdiction", "values": "one"},"#,
    );
    let second_product = |values: &str| {
        with(
            base,
            r#"{"name": "standard", "values": "set"}]"#,
            &format!(
                r#"{{"name": "standard", "values": "set"}}, {{"name": "product", "values": "{values}"}}]"#
            ),
        )
    };
    let no_facets = |text: &str| {
        let start = text.find(r#""facets": ["#).unwrap();
        format!("{}\"facets\": []}}", &text[..start])
    };

    let mut wide = wide_demo_value();
    if let Value::Object(members) = &mut wide {
        let facets = members.iter_mut().find(|(k, _)| k == "facets").unwrap();
        if let Value::Array(list) = &mut facets.1 {
            list[0] = Value::Object(vec![
                ("id".to_string(), Value::String("multi.P2".to_string())),
                ("prime".to_string(), Value::Number(2.0)),
            ]);
            list.push(Value::Object(vec![
                ("id".to_string(), Value::String("flag.p38891".to_string())),
                ("prime".to_string(), Value::Number(38_891.0)),
            ]));
        }
    }
    let too_many = canonicalize(&wide);

    // wide-demo with the family `multi` named `Multi` and flag.p38891
    // appended: the families are checked before the count.
    let mut wide = wide_demo_value();
    if let Value::Object(members) = &mut wide {
        for (key, value) in members.iter_mut() {
            match (key.as_str(), value) {
                ("families", Value::Array(list)) => {
                    list[1] = Value::Object(vec![
                        ("name".to_string(), Value::String("Multi".to_string())),
                        ("values".to_string(), Value::String("multiset".to_string())),
                    ]);
                }
                ("facets", Value::Array(list)) => list.push(Value::Object(vec![
                    ("id".to_string(), Value::String("flag.p38891".to_string())),
                    ("prime".to_string(), Value::Number(38_891.0)),
                ])),
                _ => {}
            }
        }
    }
    let families_before_too_many = canonicalize(&wide);
    let big_j = |text: &str| {
        with(
            text,
            r#""name": "jurisdiction""#,
            r#""name": "Jurisdiction""#,
        )
    };

    let start = base.find(r#""facets": "#).unwrap() + r#""facets": "#.len();
    let facets_alone = &base[start..base.len() - 1];

    let cases: Vec<(&str, String, Error)> = vec![
        (
            "not-json",
            with(base, r#""prime": 67}]}"#, r#""prime": 67},]}"#),
            core(Code::Parse),
        ),
        (
            "duplicate-member",
            with(
                base,
                r#""profile": "catalogue-demo/0","#,
                r#""profile": "catalogue-demo/0", "profile": "catalogue-demo/0","#,
            ),
            core(Code::DuplicateKey),
        ),
        (
            "number-range",
            appended(r#"{"id": "record.extra", "prime": 9007199254740992}"#),
            core(Code::NumberRange),
        ),
        ("not-an-object", facets_alone.to_string(), d(Shape)),
        (
            "member-missing",
            format!("{}}}", &base[..base.find(",\n \"facets\"").unwrap()]),
            d(Shape),
        ),
        (
            "member-unknown",
            with(
                base,
                r#"{"id": "product.inverter", "prime": 5}"#,
                r#"{"id": "product.inverter", "prime": 5, "label": "Inverter"}"#,
            ),
            d(Shape),
        ),
        (
            "prime-not-a-number",
            with(base, r#""prime": 7}"#, r#""prime": "7"}"#),
            d(Shape),
        ),
        (
            "v-not-a-string",
            with(base, r#""coordinate-dictionary/0""#, "0"),
            d(Shape),
        ),
        (
            "profile-not-a-string",
            with(base, r#""catalogue-demo/0""#, r#"["catalogue-demo/0"]"#),
            d(Shape),
        ),
        (
            "family-member-unknown",
            with(
                base,
                r#"{"name": "product", "values": "one"}"#,
                r#"{"name": "product", "values": "one", "label": "Product"}"#,
            ),
            d(Shape),
        ),
        (
            "family-name-not-a-string",
            with(base, r#""name": "record""#, r#""name": ["record"]"#),
            d(Shape),
        ),
        (
            "family-values-not-a-string",
            with(
                base,
                r#"{"name": "standard", "values": "set"}"#,
                r#"{"name": "standard", "values": 1}"#,
            ),
            d(Shape),
        ),
        (
            "id-not-a-string",
            with(base, r#""id": "jurisdiction.vic""#, r#""id": 11"#),
            d(Shape),
        ),
        (
            "shape-in-family-before-family-name",
            with(
                &big_j(base),
                r#"{"name": "standard", "values": "set"}"#,
                r#"{"name": "standard", "values": "set", "label": "Standard"}"#,
            ),
            d(Shape),
        ),
        (
            "shape-in-facet-before-family",
            with(
                &big_j(base),
                r#"{"id": "standard.as-nzs-4777-1", "prime": 67}"#,
                r#"{"id": "standard.as-nzs-4777-1", "prime": 67, "label": "x"}"#,
            ),
            d(Shape),
        ),
        (
            "shape-before-version",
            with(
                &with(base, "coordinate-dictionary/0", "coordinate-dictionary/1"),
                r#""profile""#,
                r#""note": "x", "profile""#,
            ),
            d(Shape),
        ),
        (
            "version",
            with(base, "coordinate-dictionary/0", "coordinate-dictionary/1"),
            d(Version),
        ),
        (
            "version-before-profile",
            with(
                &with(base, "coordinate-dictionary/0", "coordinate-dictionary/1"),
                r#""catalogue-demo/0""#,
                r#""catalogue-demo""#,
            ),
            d(Version),
        ),
        (
            "profile",
            with(base, r#""catalogue-demo/0""#, r#""catalogue-demo""#),
            d(Profile),
        ),
        (
            "profile-before-families",
            big_j(&with(base, r#""catalogue-demo/0""#, r#""catalogue-demo""#)),
            d(Profile),
        ),
        (
            "profile-leading-zero",
            with(base, r#""catalogue-demo/0""#, r#""catalogue-demo/00""#),
            d(Profile),
        ),
        (
            "profile-trailing-newline",
            with(base, r#""catalogue-demo/0""#, r#""catalogue-demo/0\n""#),
            d(Profile),
        ),
        (
            "family-name",
            with(
                base,
                r#""name": "jurisdiction""#,
                r#""name": "Jurisdiction""#,
            ),
            d(FamilyName),
        ),
        (
            "family-name-trailing-newline",
            with(
                base,
                r#""name": "jurisdiction""#,
                r#""name": "jurisdiction\n""#,
            ),
            d(FamilyName),
        ),
        (
            "name-before-values",
            with(
                base,
                r#"{"name": "standard", "values": "set"}"#,
                r#"{"name": "Standard", "values": "many"}"#,
            ),
            d(FamilyName),
        ),
        (
            "values",
            with(
                base,
                r#"{"name": "standard", "values": "set"}"#,
                r#"{"name": "standard", "values": "many"}"#,
            ),
            d(Values),
        ),
        (
            "family-document-order",
            with(
                &with(
                    base,
                    r#"{"name": "jurisdiction", "values": "one"}"#,
                    r#"{"name": "jurisdiction", "values": "many"}"#,
                ),
                r#""name": "product""#,
                r#""name": "Product""#,
            ),
            d(Values),
        ),
        ("values-before-duplicate", second_product("many"), d(Values)),
        (
            "family-duplicate",
            second_product("one"),
            d(FamilyDuplicate),
        ),
        ("family-order", product_first.clone(), d(FamilyOrder)),
        (
            "families-before-facets",
            with(&product_first, r#""prime": 59}"#, r#""prime": 57}"#),
            d(FamilyOrder),
        ),
        (
            "families-before-empty",
            no_facets(&product_first),
            d(FamilyOrder),
        ),
        ("empty", no_facets(base), d(Empty)),
        ("too-many", too_many, d(TooMany)),
        (
            "families-before-too-many",
            families_before_too_many,
            d(FamilyName),
        ),
        (
            "id-form",
            with(base, "product.inverter", "product.Inverter"),
            d(IdForm),
        ),
        (
            "id-form-trailing-newline",
            with(base, r#""jurisdiction.vic""#, r#""jurisdiction.vic\n""#),
            d(IdForm),
        ),
        (
            "checks-within-a-facet",
            with(
                base,
                r#"{"id": "product.inverter", "prime": 5}"#,
                r#"{"id": "product.Inverter", "prime": 5.5}"#,
            ),
            d(IdForm),
        ),
        (
            "form-before-family",
            appended(r#"{"id": "Phase.single", "prime": 71}"#),
            d(IdForm),
        ),
        (
            "unknown-family",
            appended(r#"{"id": "phase.single", "prime": 71}"#),
            d(UnknownFamily),
        ),
        (
            "family-before-prime",
            appended(r#"{"id": "phase.single", "prime": 70}"#),
            d(UnknownFamily),
        ),
        (
            "id-duplicate",
            appended(r#"{"id": "jurisdiction.vic", "prime": 71}"#),
            d(IdDuplicate),
        ),
        (
            "id-before-prime",
            appended(r#"{"id": "jurisdiction.vic", "prime": 11}"#),
            d(IdDuplicate),
        ),
        (
            "not-integer",
            with(base, r#""prime": 59}"#, r#""prime": 59.5}"#),
            d(NotInteger),
        ),
        (
            "integer-before-range",
            appended(r#"{"id": "record.extra", "prime": 2147483648.5}"#),
            d(NotInteger),
        ),
        (
            "prime-range-low",
            with(
                base,
                r#"{"id": "record.product-listing", "prime": 2}"#,
                r#"{"id": "record.product-listing", "prime": 1}"#,
            ),
            d(PrimeRange),
        ),
        (
            "prime-range-high",
            appended(r#"{"id": "record.extra", "prime": 2147483659}"#),
            d(PrimeRange),
        ),
        (
            "range-before-primality",
            appended(r#"{"id": "record.extra", "prime": 2147483648}"#),
            d(PrimeRange),
        ),
        (
            "range-before-order",
            appended(r#"{"id": "record.extra", "prime": 1}"#),
            d(PrimeRange),
        ),
        (
            "not-prime",
            with(base, r#""prime": 59}"#, r#""prime": 57}"#),
            d(NotPrime),
        ),
        (
            "primality-before-order",
            appended(r#"{"id": "record.extra", "prime": 9}"#),
            d(NotPrime),
        ),
        (
            "document-order",
            with(
                &with(base, r#""prime": 59}"#, r#""prime": 57}"#),
                "record.inspection",
                "record.Inspection",
            ),
            d(NotPrime),
        ),
        (
            "prime-duplicate",
            with(
                base,
                r#"{"id": "record.inspection", "prime": 61}"#,
                r#"{"id": "record.inspection", "prime": 59}"#,
            ),
            d(PrimeDuplicate),
        ),
        (
            "prime-order",
            with(
                &with(base, r#"{"id": "jurisdiction.vic", "prime": 11}"#, "SWAP"),
                r#"{"id": "product.battery", "prime": 13}"#,
                r#"{"id": "jurisdiction.vic", "prime": 11}"#,
            )
            .replacen("SWAP", r#"{"id": "product.battery", "prime": 13}"#, 1),
            d(PrimeOrder),
        ),
    ];
    let mut failures = Vec::new();
    for (name, bytes, want) in &cases {
        let got = detail(bytes.as_bytes());
        if got != *want {
            failures.push(format!("{name}: got {got}, want {want}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert_eq!(cases.len(), 54);
}

/// `catalogue-demo-v2`: `product.heat-pump` (71) and a family `phase` with
/// `phase.single` (73) and `phase.three` (79).
fn catalogue_demo_v2() -> Dictionary {
    let text = with(
        CATALOGUE_DEMO,
        r#"{"name": "jurisdiction", "values": "one"},"#,
        r#"{"name": "jurisdiction", "values": "one"}, {"name": "phase", "values": "one"},"#,
    );
    let text = with(
        &text,
        r#"{"id": "standard.as-nzs-4777-1", "prime": 67}]}"#,
        r#"{"id": "standard.as-nzs-4777-1", "prime": 67}, {"id": "product.heat-pump", "prime": 71},
   {"id": "phase.single", "prime": 73}, {"id": "phase.three", "prime": 79}]}"#,
    );
    Dictionary::parse(text.as_bytes()).unwrap()
}

#[test]
fn encode_cases() {
    let demo = catalogue_demo();
    let kernel = load(&kernel_2_value());
    let wide = load(&wide_demo_value());
    let max = load(&max_prime_demo_value());

    assert_eq!(demo.encode(Vec::<String>::new()).unwrap(), "1");
    assert_eq!(
        demo.encode([
            "standard.as-nzs-4777-2",
            "standard.as-nzs-5033",
            "standard.as-nzs-3000"
        ]),
        Ok("3059".to_string())
    );
    assert_eq!(
        demo.encode(["product.heat-pump"]),
        Err(unknown_facet("product.heat-pump"))
    );
    assert_eq!(
        demo.encode(["jurisdiction.vic\n"]),
        Err(unknown_facet("jurisdiction.vic\n"))
    );
    assert_eq!(
        demo.encode(["jurisdiction.vic", "product.heat-pump", "phase.single"]),
        Err(unknown_facet("product.heat-pump"))
    );
    assert_eq!(
        demo.encode(["jurisdiction.vic", "jurisdiction.nsw", "product.heat-pump"]),
        Err(unknown_facet("product.heat-pump"))
    );
    assert_eq!(
        demo.encode(["jurisdiction.vic"; 2]),
        Err(cardinality("jurisdiction"))
    );
    assert_eq!(
        demo.encode(["standard.as-nzs-5033"; 2]),
        Err(cardinality("standard"))
    );
    assert_eq!(
        demo.encode([
            "record.product-listing",
            "record.installation",
            "jurisdiction.vic",
            "jurisdiction.nsw"
        ]),
        Err(cardinality("jurisdiction"))
    );
    assert_eq!(
        demo.encode([
            "standard.as-nzs-4777-2",
            "standard.as-nzs-4777-2",
            "product.inverter",
            "product.battery"
        ]),
        Err(cardinality("product"))
    );
    assert_eq!(
        catalogue_demo_v2().encode(["product.heat-pump", "phase.single"]),
        Ok("5183".to_string())
    );
    assert_eq!(kernel.encode(["universal.entry"; 2]), Ok("4".to_string()));

    let first_16: Vec<String> = KERNEL_POSITIONS
        .iter()
        .map(|p| format!("universal.{p}"))
        .chain(
            [
                "entry",
                "exit",
                "unclassified",
                "quantified",
                "transfer",
                "locked",
            ]
            .map(|p| format!("measurement.{p}")),
        )
        .collect();
    assert_eq!(
        kernel.encode(&first_16),
        Ok("32589158477190044730".to_string())
    );
    let mut first_20 = first_16.clone();
    first_20
        .extend(["committed", "unit", "linked", "executed"].map(|p| format!("measurement.{p}")));
    assert_eq!(
        kernel.encode(&first_20),
        Ok("557940830126698960967415390".to_string())
    );
    let all: Vec<String> = kernel_facets().into_iter().map(|(id, _)| id).collect();
    let all_70 = kernel.encode(&all).unwrap();
    assert_eq!(ids(&kernel, &all_70).len(), 70);

    assert_eq!(wide.encode(vec!["multi.p2"; 1023]), Ok(pow2(1023)));
    assert_eq!(wide.encode(vec!["multi.p2"; 1024]), Err(Error::Range));
    let flags = |last: u32| -> Vec<String> {
        primes_below(800)
            .into_iter()
            .filter(|&p| (59..=last).contains(&p))
            .map(|p| format!("flag.p{p}"))
            .collect()
    };
    let below = flags(787);
    assert_eq!(below.len(), 122);
    let product = wide.encode(&below).unwrap();
    assert!(product.len() >= 307 && product.len() <= 309);
    let over = flags(797);
    assert_eq!(over.len(), 123);
    assert_eq!(wide.encode(&over), Err(Error::Range));
    let mut both = vec!["multi.p2"; 1024];
    both.extend(["flag.p59", "flag.p59"]);
    assert_eq!(wide.encode(&both), Err(cardinality("flag")));
    assert_eq!(
        max.encode(["big.p2147483647"]),
        Ok("2147483647".to_string())
    );
}

#[test]
fn decode_cases() {
    let demo = catalogue_demo();
    let kernel = load(&kernel_2_value());
    let wide = load(&wide_demo_value());
    let max = load(&max_prime_demo_value());

    assert_eq!(demo.decode("1"), Ok(vec![]));
    for bad in [
        "0", "01", "+5", "-5", " 5", "5 ", "5\n", "5.0", "1e3", "", "\u{FF15}",
    ] {
        assert_eq!(demo.decode(bad), Err(Error::Syntax), "{bad:?}");
    }
    // A value that is not a string is never converted to one: ["5"] is not "5".
    for not_a_string in [
        Value::Number(5.0),
        Value::Null,
        Value::Array(vec![Value::String("5".to_string())]),
    ] {
        assert_eq!(demo.decode_value(&not_a_string), Err(Error::Syntax));
    }
    assert_eq!(
        demo.decode(&format!("0{}", "1".repeat(399))),
        Err(Error::Syntax)
    );
    assert_eq!(
        demo.decode(&format!("1{}x", "0".repeat(400))),
        Err(Error::Syntax)
    );
    assert_eq!(demo.decode("71"), Err(Error::UnknownFactor));
    assert_eq!(demo.decode("33077"), Err(Error::UnknownFactor));
    assert_eq!(demo.decode("121"), Err(cardinality("jurisdiction")));
    assert_eq!(demo.decode("49"), Err(cardinality("standard")));
    assert_eq!(demo.decode("2046"), Err(cardinality("jurisdiction")));
    assert_eq!(demo.decode("3185"), Err(cardinality("product")));
    let v2 = catalogue_demo_v2();
    assert_eq!(ids(&v2, "71"), vec![("product.heat-pump".to_string(), 1)]);
    assert_eq!(ids(&v2, "1155"), ids(&demo, "1155"));

    // REQ-KRN-011: "1" and "5" decode differently.
    assert_eq!(kernel.decode("1"), Ok(vec![]));
    assert_eq!(
        ids(&kernel, "5"),
        vec![("universal.unclassified".to_string(), 1)]
    );
    assert_eq!(ids(&kernel, "4"), vec![("universal.entry".to_string(), 2)]);
    assert_eq!(ids(&kernel, "32589158477190044730").len(), 16);
    assert_eq!(ids(&kernel, "557940830126698960967415390").len(), 20);
    assert_eq!(kernel.decode("353"), Err(Error::UnknownFactor));

    let multi_2 = |e: u32| vec![("multi.p2".to_string(), e)];
    for k in [53, 63, 64] {
        assert_eq!(ids(&wide, &pow2(k)), multi_2(k));
        assert_eq!(
            wide.decode(&offset(&pow2(k), -1)),
            Err(Error::UnknownFactor),
            "2^{k}-1"
        );
        assert_eq!(
            wide.decode(&offset(&pow2(k), 1)),
            Err(Error::UnknownFactor),
            "2^{k}+1"
        );
    }
    // The nearest coordinates below and above 2^k that decode.
    let nearest = [
        (53, "9007199254740990", "9007199254741096"),
        (63, "9223372036854775800", "9223372036854776017"),
        (64, "18446744073709551600", "18446744073709552034"),
    ];
    for (k, below, above) in nearest {
        let p = pow2(k);
        let mut d = 1;
        while wide.decode(&offset(&p, -d)).is_err() {
            d += 1;
        }
        assert_eq!(offset(&p, -d), below, "below 2^{k}");
        let mut d = 1;
        while wide.decode(&offset(&p, d)).is_err() {
            d += 1;
        }
        assert_eq!(offset(&p, d), above, "above 2^{k}");
    }
    assert_eq!(ids(&wide, &pow2(1023)), multi_2(1023));
    assert_eq!(
        wide.decode(&offset(&pow2(1024), -1)),
        Err(Error::UnknownFactor)
    );
    assert_eq!(offset(&pow2(1024), -1).len(), 309);
    assert_eq!(wide.decode(&pow2(1024)), Err(Error::Range));
    assert_eq!(wide.decode(&offset(&pow2(1024), 1)), Err(Error::Range));
    let ten_308 = format!("1{}", "0".repeat(308));
    assert_eq!(
        ids(&wide, &ten_308),
        vec![("multi.p2".to_string(), 308), ("multi.p5".to_string(), 308)]
    );
    assert_eq!(
        wide.decode(&format!("2{}", "0".repeat(308))),
        Err(Error::Range)
    );
    assert_eq!(wide.decode(&"9".repeat(309)), Err(Error::Range));
    assert_eq!(
        wide.decode(&format!("1{}", "0".repeat(309))),
        Err(Error::Range)
    );
    assert_eq!(wide.decode("3481"), Err(cardinality("flag")));

    assert_eq!(
        ids(&max, "2147483647"),
        vec![("big.p2147483647".to_string(), 1)]
    );
    assert_eq!(
        ids(&max, "4611686014132420609"),
        vec![("big.p2147483647".to_string(), 2)]
    );
    assert_eq!(max.decode("2"), Err(Error::UnknownFactor));

    let decoded = demo.decode("1155").unwrap();
    assert_eq!(
        canonicalize(&factors_value(&decoded)),
        r#"[{"exponent":1,"id":"record.installation"},{"exponent":1,"id":"product.inverter"},{"exponent":1,"id":"standard.as-nzs-4777-2"},{"exponent":1,"id":"jurisdiction.vic"}]"#
    );
}

#[test]
fn divides_cases() {
    let demo = catalogue_demo();
    let kernel = load(&kernel_2_value());
    let wide = load(&wide_demo_value());
    let n = |x: f64| Value::Number(x);
    let s = |x: &str| Value::String(x.to_string());

    assert_eq!(demo.divides("1", "1"), Ok(true));
    assert_eq!(demo.divides("1", "1155"), Ok(true));
    assert_eq!(demo.divides("1155", "1155"), Ok(true));
    assert_eq!(demo.divides("97", "1155"), Err(Error::UnknownFactor));
    assert_eq!(
        demo.divides("341", "1155"),
        Err(cardinality("jurisdiction"))
    );
    assert_eq!(demo.divides("01", "1155"), Err(Error::Syntax));
    assert_eq!(demo.divides("231\n", "1155"), Err(Error::Syntax));
    assert_eq!(demo.divides("1", "97"), Err(Error::UnknownFactor));
    assert_eq!(demo.divides("1", "341"), Err(cardinality("jurisdiction")));
    assert_eq!(
        demo.divides_value(&s("231"), &n(1155.0)),
        Err(Error::Syntax)
    );
    assert_eq!(
        demo.divides("1", &format!("1{}x", "0".repeat(400))),
        Err(Error::Syntax)
    );
    assert_eq!(
        demo.divides("-5", &format!("1{}", "0".repeat(400))),
        Err(Error::Syntax)
    );
    assert_eq!(kernel.divides("53", "32589158477190044730"), Ok(true));
    assert_eq!(
        kernel.divides("53", "557940830126698960967415390"),
        Ok(true)
    );
    // SQLite finds 73 in p16 and 127 in p20, which hold neither.
    assert_eq!(kernel.divides("73", "32589158477190044730"), Ok(false));
    assert_eq!(
        kernel.divides("127", "557940830126698960967415390"),
        Ok(false)
    );
    assert_eq!(kernel.divides("4", "2"), Ok(false));
    assert_eq!(kernel.divides("4", "8"), Ok(true));
    assert_eq!(wide.divides(&pow2(63), &pow2(64)), Ok(true));
    assert_eq!(wide.divides(&pow2(64), &pow2(63)), Ok(false));
    assert_eq!(wide.divides("2", &pow2(1023)), Ok(true));
}

#[test]
fn check_cases() {
    let demo = catalogue_demo();
    let wide = load(&wide_demo_value());
    let four = [
        "record.installation",
        "product.inverter",
        "standard.as-nzs-4777-2",
        "jurisdiction.vic",
    ];
    assert_eq!(demo.check(four, "1155"), Ok(()));
    assert_eq!(demo.check(four.iter().rev(), "1155"), Ok(()));
    assert_eq!(demo.check(Vec::<&str>::new(), "1"), Ok(()));
    assert_eq!(demo.check(four, "1156"), Err(Error::Drift));
    assert_eq!(demo.check(four, "01155"), Err(Error::Drift));
    assert_eq!(
        demo.check_value(four, &Value::Number(1155.0)),
        Err(Error::Drift)
    );
    assert_eq!(demo.check(["jurisdiction.vic"], "1"), Err(Error::Drift));
    assert_eq!(
        demo.check(["product.heat-pump"], "71"),
        Err(unknown_facet("product.heat-pump"))
    );
    assert_eq!(
        demo.check(["jurisdiction.vic", "jurisdiction.nsw"], "341"),
        Err(cardinality("jurisdiction"))
    );
    assert_eq!(wide.check(vec!["multi.p2"; 1024], "1"), Err(Error::Range));
}

#[test]
fn extends_cases() {
    let demo = catalogue_demo();
    let v2 = catalogue_demo_v2();
    let kernel = load(&kernel_2_value());
    let not = |detail| Err(Error::NotExtension { detail });
    let variant = |from: &str, to: &str| {
        Dictionary::parse(with(CATALOGUE_DEMO, from, to).as_bytes()).unwrap()
    };
    let values_changed = variant(
        r#"{"name": "standard", "values": "set"}"#,
        r#"{"name": "standard", "values": "multiset"}"#,
    );
    let prime_changed = variant(
        r#"{"id": "jurisdiction.nt", "prime": 59},
   {"id": "record.inspection", "prime": 61},
   {"id": "standard.as-nzs-4777-1", "prime": 67}]}"#,
        r#"{"id": "record.inspection", "prime": 61},
   {"id": "standard.as-nzs-4777-1", "prime": 67},
   {"id": "jurisdiction.nt", "prime": 71}]}"#,
    );
    let facet_removed = variant(
        r#"
   {"id": "record.inspection", "prime": 61},"#,
        "",
    );

    assert_eq!(extends(&demo, &demo), Ok(()));
    assert_eq!(extends(&demo, &v2), Ok(()));
    assert_eq!(extends(&v2, &demo), not(ExtensionDetail::Family));
    assert_eq!(extends(&demo, &kernel), not(ExtensionDetail::Profile));
    assert_eq!(
        extends(&demo, &values_changed),
        not(ExtensionDetail::Family)
    );
    assert_eq!(extends(&demo, &prime_changed), not(ExtensionDetail::Facet));
    assert_eq!(extends(&demo, &facet_removed), not(ExtensionDetail::Facet));

    // A coordinate that decodes under old decodes the same under new.
    for c in ["1", "1155", "3059", "2", "61"] {
        assert_eq!(demo.decode(c), v2.decode(c));
    }
}
