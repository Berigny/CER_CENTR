//! The five operations (spec section Operations): encode, decode, divides,
//! check and extends. Each gives a result or one error; where more than one
//! error applies, the first in the spec's order is the result.

use std::collections::BTreeMap;

use evidence_core::json::Value;

use crate::big::Big;
use crate::dictionary::{Dictionary, Values};
use crate::error::{Error, ExtensionDetail};

/// A coordinate is below 2^1024: it has at most this many bits.
pub const MAX_BITS: u64 = 1024;

/// 2^1024 has 309 decimal digits, so a coordinate has at most 309.
pub const MAX_DIGITS: usize = 309;

/// One facet of a decoded coordinate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Factor {
    /// The facet id.
    pub id: String,
    /// The facet's prime.
    pub prime: u32,
    /// How many times the prime divides the coordinate, 1 or more.
    pub exponent: u32,
}

impl Factor {
    /// `{"id", "exponent"}`, as the vectors write a decoded facet.
    pub fn to_value(&self) -> Value {
        Value::Object(vec![
            ("id".to_string(), Value::String(self.id.clone())),
            (
                "exponent".to_string(),
                Value::Number(f64::from(self.exponent)),
            ),
        ])
    }
}

/// A decode result as the vectors write it: an array of `{"id", "exponent"}`
/// in ascending prime order.
pub fn factors_value(factors: &[Factor]) -> Value {
    Value::Array(factors.iter().map(Factor::to_value).collect())
}

/// Whether `s` is a coordinate's canonical decimal form, `^[1-9][0-9]*$`,
/// matched against the whole string. This is the syntax only, not the range.
pub fn is_canonical(s: &str) -> bool {
    match s.as_bytes() {
        [b'1'..=b'9', rest @ ..] => rest.iter().all(u8::is_ascii_digit),
        _ => false,
    }
}

impl Dictionary {
    /// `encode(dictionary, facets)`: the coordinate of a list of facet ids.
    ///
    /// An id that appears more than once counts once each time, and the
    /// order of the list does not matter. Errors, first applicable first:
    /// `P_COORDINATE_UNKNOWN_FACET` (the first unknown id in list order),
    /// `P_COORDINATE_CARDINALITY` (the first broken family in the order of
    /// `families`), `P_COORDINATE_RANGE` (the product is 2^1024 or more).
    pub fn encode<I>(&self, facets: I) -> Result<String, Error>
    where
        I: IntoIterator,
        I::Item: AsRef<str>,
    {
        let mut counts: BTreeMap<usize, u64> = BTreeMap::new();
        for id in facets {
            let id = id.as_ref();
            let &i = self.index.get(id).ok_or_else(|| Error::UnknownFacet {
                facet: id.to_string(),
            })?;
            *counts.entry(i).or_insert(0) += 1;
        }
        self.cardinality(counts.iter().map(|(&i, &n)| (i, n)))?;
        let mut product = Big::one();
        for (&i, &n) in &counts {
            let prime = self.facets[i].prime;
            for _ in 0..n {
                product.mul(prime);
                // Stop as soon as the product reaches 2^1024.
                if product.bits() > MAX_BITS {
                    return Err(Error::Range);
                }
            }
        }
        Ok(product.to_decimal())
    }

    /// `decode(dictionary, coordinate)`: the facets of a coordinate, each
    /// with its exponent, in ascending prime order. `"1"` decodes to the
    /// empty list.
    ///
    /// Errors, first applicable first: `P_COORDINATE_SYNTAX`,
    /// `P_COORDINATE_RANGE`, `P_COORDINATE_UNKNOWN_FACTOR`,
    /// `P_COORDINATE_CARDINALITY`.
    pub fn decode(&self, coordinate: &str) -> Result<Vec<Factor>, Error> {
        Ok(self
            .exponents(coordinate)?
            .into_iter()
            .map(|(i, exponent)| {
                let facet = &self.facets[i];
                Factor {
                    id: facet.id.clone(),
                    prime: facet.prime,
                    exponent,
                }
            })
            .collect())
    }

    /// [`decode`](Dictionary::decode) of a JSON value as it is: anything but
    /// a string, a JSON number included, is `P_COORDINATE_SYNTAX`.
    pub fn decode_value(&self, coordinate: &Value) -> Result<Vec<Factor>, Error> {
        self.decode(as_coordinate(coordinate)?)
    }

    /// `divides(dictionary, query, coordinate)`: whether every facet of the
    /// query has an exponent in the coordinate at least as large as its
    /// exponent in the query, which is "the query divides the coordinate".
    ///
    /// The query is decoded, then the coordinate, and the first error is the
    /// result. Both are always decoded: a query of `"1"` does not excuse a
    /// coordinate that fails to decode.
    pub fn divides(&self, query: &str, coordinate: &str) -> Result<bool, Error> {
        let query = self.exponents(query)?;
        let coordinate = self.exponents(coordinate)?;
        Ok(holds_all(&query, &coordinate))
    }

    /// [`divides`](Dictionary::divides) of JSON values as they are.
    pub fn divides_value(&self, query: &Value, coordinate: &Value) -> Result<bool, Error> {
        let query = self.exponents(as_coordinate(query)?)?;
        let coordinate = self.exponents(as_coordinate(coordinate)?)?;
        Ok(holds_all(&query, &coordinate))
    }

    /// `check(dictionary, facets, claimed)`: ok when `claimed` is exactly
    /// the string the facets encode to. Encode's errors come first, then
    /// `P_COORDINATE_DRIFT`. The comparison is exact, so a claimed
    /// coordinate that is not canonical is drift.
    pub fn check<I>(&self, facets: I, claimed: &str) -> Result<(), Error>
    where
        I: IntoIterator,
        I::Item: AsRef<str>,
    {
        if self.encode(facets)? == claimed {
            Ok(())
        } else {
            Err(Error::Drift)
        }
    }

    /// [`check`](Dictionary::check) of a claimed JSON value as it is: a
    /// claimed value that is not a string, a JSON number included, is
    /// `P_COORDINATE_DRIFT` once the facets encode.
    pub fn check_value<I>(&self, facets: I, claimed: &Value) -> Result<(), Error>
    where
        I: IntoIterator,
        I::Item: AsRef<str>,
    {
        let encoded = self.encode(facets)?;
        match claimed {
            Value::String(s) if *s == encoded => Ok(()),
            _ => Err(Error::Drift),
        }
    }

    /// The exponents of a coordinate, as (facet index, exponent) in
    /// ascending prime order, after every check of decode.
    fn exponents(&self, coordinate: &str) -> Result<Vec<(usize, u32)>, Error> {
        // Step 1 reads the whole string before step 2 looks at its length.
        if !is_canonical(coordinate) {
            return Err(Error::Syntax);
        }
        if coordinate.len() > MAX_DIGITS {
            return Err(Error::Range);
        }
        let mut residue = Big::from_digits(coordinate.as_bytes());
        if residue.bits() > MAX_BITS {
            return Err(Error::Range);
        }

        let mut found = Vec::new();
        for group in &self.groups {
            if residue.is_one() {
                break;
            }
            // A residue above 1 but below the next prime has only factors
            // below it, and every dictionary prime below it is divided out:
            // what is left is outside the dictionary.
            if residue
                .to_u64()
                .is_some_and(|r| r < u64::from(self.facets[group.start].prime))
            {
                break;
            }
            let r = residue.rem(group.modulus);
            for (i, facet) in (group.start..).zip(&self.facets[group.start..group.end]) {
                let prime = facet.prime;
                if !r.is_multiple_of(prime) {
                    continue;
                }
                // Dividing out other primes does not change whether this one
                // divides, so the group's remainder still answers for it.
                let mut exponent = 0;
                loop {
                    residue.div_rem(prime);
                    exponent += 1;
                    if residue.rem(prime) != 0 {
                        break;
                    }
                }
                found.push((i, exponent));
            }
        }
        if !residue.is_one() {
            return Err(Error::UnknownFactor);
        }
        self.cardinality(found.iter().map(|&(i, e)| (i, u64::from(e))))?;
        Ok(found)
    }

    /// The first family, in the order of `families`, whose rule the
    /// exponents break. `exponents` gives each facet at most once.
    fn cardinality(&self, exponents: impl Iterator<Item = (usize, u64)>) -> Result<(), Error> {
        let mut one_totals: BTreeMap<usize, u64> = BTreeMap::new();
        let mut first: Option<usize> = None;
        for (i, exponent) in exponents {
            let family = self.facets[i].family;
            let broken = match self.families[family].values {
                Values::One => {
                    let total = one_totals.entry(family).or_insert(0);
                    *total = total.saturating_add(exponent);
                    *total > 1
                }
                Values::Set => exponent > 1,
                Values::Multiset => false,
            };
            if broken {
                first = Some(first.map_or(family, |f| f.min(family)));
            }
        }
        match first {
            Some(family) => Err(Error::Cardinality {
                family: self.families[family].name.clone(),
            }),
            None => Ok(()),
        }
    }
}

/// `extends(old, new)`: ok when `new` only adds families and facets to `old`.
/// Otherwise `P_COORDINATE_NOT_EXTENSION` with the first check that fails:
/// `profile`, then `family` (in `old`'s order), then `facet` (in `old`'s
/// order). Every dictionary extends itself.
pub fn extends(old: &Dictionary, new: &Dictionary) -> Result<(), Error> {
    let fail = |detail| Err(Error::NotExtension { detail });
    if old.profile != new.profile {
        return fail(ExtensionDetail::Profile);
    }
    for family in &old.families {
        match new.family(family.name()) {
            Some(f) if f.values() == family.values() => {}
            _ => return fail(ExtensionDetail::Family),
        }
    }
    for facet in &old.facets {
        match new.facet(facet.id()) {
            Some(f) if f.prime() == facet.prime() => {}
            _ => return fail(ExtensionDetail::Facet),
        }
    }
    Ok(())
}

fn as_coordinate(v: &Value) -> Result<&str, Error> {
    v.as_str().ok_or(Error::Syntax)
}

/// Whether every (facet, exponent) of `query` is in `coordinate` with at
/// least that exponent. Both lists are in ascending facet order.
fn holds_all(query: &[(usize, u32)], coordinate: &[(usize, u32)]) -> bool {
    query.iter().all(
        |&(i, e)| match coordinate.binary_search_by_key(&i, |&(j, _)| j) {
            Ok(k) => coordinate[k].1 >= e,
            Err(_) => false,
        },
    )
}
