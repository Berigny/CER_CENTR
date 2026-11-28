//! Helpers shared by the integration tests: the spec's dictionaries, built
//! here from the spec's text rather than read from the vectors, and a small
//! deterministic generator.

#![allow(dead_code)]

use evidence_coordinate::Dictionary;
use evidence_core::canon::canonicalize;
use evidence_core::json::Value;

/// `catalogue-demo`, exactly as spec section Dictionary writes it.
pub const CATALOGUE_DEMO: &str = r#"{"v": "coordinate-dictionary/0",
 "profile": "catalogue-demo/0",
 "families": [
   {"name": "jurisdiction", "values": "one"},
   {"name": "product", "values": "one"},
   {"name": "record", "values": "one"},
   {"name": "standard", "values": "set"}],
 "facets": [
   {"id": "record.product-listing", "prime": 2},
   {"id": "record.installation", "prime": 3},
   {"id": "product.inverter", "prime": 5},
   {"id": "standard.as-nzs-4777-2", "prime": 7},
   {"id": "jurisdiction.vic", "prime": 11},
   {"id": "product.battery", "prime": 13},
   {"id": "product.pv-module", "prime": 17},
   {"id": "standard.as-nzs-5033", "prime": 19},
   {"id": "standard.as-nzs-3000", "prime": 23},
   {"id": "standard.as-nzs-5139", "prime": 29},
   {"id": "jurisdiction.nsw", "prime": 31},
   {"id": "jurisdiction.qld", "prime": 37},
   {"id": "jurisdiction.sa", "prime": 41},
   {"id": "jurisdiction.wa", "prime": 43},
   {"id": "jurisdiction.tas", "prime": 47},
   {"id": "jurisdiction.act", "prime": 53},
   {"id": "jurisdiction.nt", "prime": 59},
   {"id": "record.inspection", "prime": 61},
   {"id": "standard.as-nzs-4777-1", "prime": 67}]}"#;

/// The ids the spec states.
pub const CATALOGUE_DEMO_ID: &str =
    "sha256:005aad78a1aaae4ef493f0f8b20facec76f5dadddefc8de05903754998ca6278";
pub const KERNEL_2_ID: &str =
    "sha256:28ec5d10ee5e6e5fd8effc7515e9adec7f049f75beeccce5064d738d922cac71";

/// Kernel/2 section 2.2: the blocks in block order, and the positions.
pub const KERNEL_BLOCKS: [&str; 7] = [
    "universal",
    "measurement",
    "emissions",
    "reporting",
    "inputs",
    "infrastructure",
    "registry",
];
pub const KERNEL_POSITIONS: [&str; 10] = [
    "entry",
    "exit",
    "unclassified",
    "quantified",
    "transfer",
    "locked",
    "committed",
    "unit",
    "linked",
    "executed",
];

/// The primes below `limit`, by a sieve.
pub fn primes_below(limit: usize) -> Vec<u32> {
    let mut composite = vec![false; limit];
    let mut out = Vec::new();
    for n in 2..limit {
        if !composite[n] {
            out.push(n as u32);
            let mut m = n * n;
            while m < limit {
                composite[m] = true;
                m += n;
            }
        }
    }
    out
}

/// The first `count` primes.
pub fn first_primes(count: usize) -> Vec<u32> {
    let primes = primes_below(40_000);
    assert!(primes.len() >= count);
    primes[..count].to_vec()
}

fn s(text: &str) -> Value {
    Value::String(text.to_string())
}

/// A dictionary value from its parts. `families` must already be by name
/// and `facets` by prime: this builds what the spec describes, it does not
/// sort.
pub fn dictionary_value(
    profile: &str,
    families: &[(&str, &str)],
    facets: &[(String, u32)],
) -> Value {
    Value::Object(vec![
        ("v".to_string(), s("coordinate-dictionary/0")),
        ("profile".to_string(), s(profile)),
        (
            "families".to_string(),
            Value::Array(
                families
                    .iter()
                    .map(|(name, values)| {
                        Value::Object(vec![
                            ("name".to_string(), s(name)),
                            ("values".to_string(), s(values)),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "facets".to_string(),
            Value::Array(
                facets
                    .iter()
                    .map(|(id, prime)| {
                        Value::Object(vec![
                            ("id".to_string(), s(id)),
                            ("prime".to_string(), Value::Number(f64::from(*prime))),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}

/// Loads a dictionary value through its canonical bytes, as a reader of a
/// published file would.
pub fn load(value: &Value) -> Dictionary {
    Dictionary::parse(canonicalize(value).as_bytes()).expect("a valid dictionary")
}

pub fn catalogue_demo() -> Dictionary {
    Dictionary::parse(CATALOGUE_DEMO.as_bytes()).expect("catalogue-demo loads")
}

/// The Kernel/2 facet ids in prime order: block b, position i gets the
/// prime P[10b + i] (Kernel/2 section 2.1).
pub fn kernel_facets() -> Vec<(String, u32)> {
    let primes = first_primes(70);
    let mut out = Vec::new();
    for (b, block) in KERNEL_BLOCKS.iter().enumerate() {
        for (i, position) in KERNEL_POSITIONS.iter().enumerate() {
            out.push((format!("{block}.{position}"), primes[10 * b + i]));
        }
    }
    out
}

/// Kernel/2's dictionary, built from its allocation rule.
pub fn kernel_2_value() -> Value {
    let mut blocks = KERNEL_BLOCKS.to_vec();
    blocks.sort_unstable();
    let families: Vec<(&str, &str)> = blocks.iter().map(|b| (*b, "multiset")).collect();
    dictionary_value("kernel/2", &families, &kernel_facets())
}

/// `wide-demo`: the first 4,096 primes, `multi.p<prime>` up to 53 and
/// `flag.p<prime>` for the rest.
pub fn wide_demo_value() -> Value {
    let facets: Vec<(String, u32)> = first_primes(4096)
        .into_iter()
        .map(|p| {
            let family = if p <= 53 { "multi" } else { "flag" };
            (format!("{family}.p{p}"), p)
        })
        .collect();
    dictionary_value(
        "wide-demo/0",
        &[("flag", "set"), ("multi", "multiset")],
        &facets,
    )
}

/// `max-prime-demo`: one facet at 2^31 - 1, and a family with no facets.
pub fn max_prime_demo_value() -> Value {
    dictionary_value(
        "max-prime-demo/0",
        &[("big", "multiset"), ("unused", "one")],
        &[("big.p2147483647".to_string(), 2_147_483_647)],
    )
}

/// splitmix64: deterministic, so every run draws the same cases.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number in `0..n`.
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    pub fn chance(&mut self, one_in: usize) -> bool {
        self.below(one_in) == 0
    }
}

/// The decimal string of a product of primes, by u128 arithmetic where it
/// fits. None when it does not.
pub fn product_u128(primes: &[u32]) -> Option<u128> {
    primes
        .iter()
        .try_fold(1u128, |acc, &p| acc.checked_mul(u128::from(p)))
}
