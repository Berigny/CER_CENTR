//! The dictionary, `coordinate-dictionary/0`, and loading one (spec sections
//! Dictionary and Loading a dictionary).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use evidence_core::canon;
use evidence_core::json::{self, Value};

use crate::error::{DictionaryDetail as D, Error};

/// `v` of every dictionary.
pub const DICTIONARY_V: &str = "coordinate-dictionary/0";

/// The most facets a dictionary may hold.
pub const MAX_FACETS: usize = 4096;

/// Every prime is below this bound, 2^31, so it fits a signed 32-bit integer.
pub const PRIME_BOUND: u32 = 1 << 31;

/// A family's rule for how many of its facets a coordinate may hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Values {
    /// At most one facet of the family, with exponent 1.
    One,
    /// Any number of distinct facets of the family, each with exponent 1.
    Set,
    /// Any facets of the family, repeated any number of times.
    Multiset,
}

impl Values {
    /// The rule as the dictionary writes it: `one`, `set` or `multiset`.
    pub fn as_str(self) -> &'static str {
        match self {
            Values::One => "one",
            Values::Set => "set",
            Values::Multiset => "multiset",
        }
    }

    fn parse(s: &str) -> Option<Values> {
        match s {
            "one" => Some(Values::One),
            "set" => Some(Values::Set),
            "multiset" => Some(Values::Multiset),
            _ => None,
        }
    }
}

impl fmt::Display for Values {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A family of a loaded dictionary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Family {
    pub(crate) name: String,
    pub(crate) values: Values,
}

impl Family {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn values(&self) -> Values {
        self.values
    }
}

/// A facet of a loaded dictionary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Facet {
    pub(crate) id: String,
    pub(crate) prime: u32,
    pub(crate) family: usize,
}

impl Facet {
    /// `<family>.<name>`, such as `jurisdiction.vic`.
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn prime(&self) -> u32 {
        self.prime
    }

    /// The position of the facet's family in [`Dictionary::families`].
    pub fn family(&self) -> usize {
        self.family
    }
}

/// Consecutive facets whose primes multiply to less than 2^32. Decoding
/// takes one remainder by the product and tests each prime against it, which
/// reads the coordinate once per group rather than once per prime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Group {
    pub(crate) modulus: u32,
    pub(crate) start: usize,
    pub(crate) end: usize,
}

/// A valid, loaded dictionary.
///
/// Families are in the order of the dictionary's `families` array, which is
/// by name; facets are in the order of its `facets` array, which is by
/// ascending prime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dictionary {
    pub(crate) profile: String,
    pub(crate) families: Vec<Family>,
    pub(crate) facets: Vec<Facet>,
    pub(crate) index: BTreeMap<String, usize>,
    pub(crate) groups: Vec<Group>,
    id: String,
}

impl Dictionary {
    /// Loads a dictionary from its bytes, running the checks of "Loading a
    /// dictionary" in order and stopping at the first failure, which is
    /// `P_COORDINATE_DICTIONARY` with that check's `detail`.
    pub fn parse(bytes: &[u8]) -> Result<Dictionary, Error> {
        let value = json::parse(bytes).map_err(|e| Error::Dictionary {
            detail: D::Shape,
            core: Some(e.code),
        })?;
        Dictionary::from_value(&value)
    }

    /// Loads a dictionary from an already parsed value, starting at step 2.
    pub fn from_value(value: &Value) -> Result<Dictionary, Error> {
        let raw = Raw::read(value).ok_or(Error::dictionary(D::Shape))?;
        if raw.v != DICTIONARY_V {
            return Err(Error::dictionary(D::Version));
        }
        if !is_profile(raw.profile) {
            return Err(Error::dictionary(D::Profile));
        }

        let mut families: Vec<Family> = Vec::with_capacity(raw.families.len());
        let mut names = BTreeSet::new();
        for &(name, values) in &raw.families {
            if !is_family_name(name.as_bytes()) {
                return Err(Error::dictionary(D::FamilyName));
            }
            let values = Values::parse(values).ok_or(Error::dictionary(D::Values))?;
            if !names.insert(name) {
                return Err(Error::dictionary(D::FamilyDuplicate));
            }
            if families
                .last()
                .is_some_and(|prev| name <= prev.name.as_str())
            {
                return Err(Error::dictionary(D::FamilyOrder));
            }
            families.push(Family {
                name: name.to_string(),
                values,
            });
        }

        if raw.facets.is_empty() {
            return Err(Error::dictionary(D::Empty));
        }
        if raw.facets.len() > MAX_FACETS {
            return Err(Error::dictionary(D::TooMany));
        }

        let mut facets: Vec<Facet> = Vec::with_capacity(raw.facets.len());
        let mut index = BTreeMap::new();
        let mut primes = BTreeSet::new();
        for &(id, prime) in &raw.facets {
            let family_name = id_family(id).ok_or(Error::dictionary(D::IdForm))?;
            // Step 5 has passed, so `families` is sorted by name.
            let family = families
                .binary_search_by(|f| f.name.as_str().cmp(family_name))
                .map_err(|_| Error::dictionary(D::UnknownFamily))?;
            if index.contains_key(id) {
                return Err(Error::dictionary(D::IdDuplicate));
            }
            // The value, the double evidence/0 parsed, not how it was written.
            // A NaN or an infinity, which only a value built in code can hold,
            // has no integer part either.
            if prime.fract() != 0.0 {
                return Err(Error::dictionary(D::NotInteger));
            }
            if !(2.0..f64::from(PRIME_BOUND)).contains(&prime) {
                return Err(Error::dictionary(D::PrimeRange));
            }
            let prime = prime as u32;
            if !is_prime(prime) {
                return Err(Error::dictionary(D::NotPrime));
            }
            if !primes.insert(prime) {
                return Err(Error::dictionary(D::PrimeDuplicate));
            }
            if facets.last().is_some_and(|prev| prime <= prev.prime) {
                return Err(Error::dictionary(D::PrimeOrder));
            }
            index.insert(id.to_string(), facets.len());
            facets.push(Facet {
                id: id.to_string(),
                prime,
                family,
            });
        }

        let groups = groups(&facets);
        Ok(Dictionary {
            profile: raw.profile.to_string(),
            families,
            facets,
            index,
            groups,
            id: canon::digest(value),
        })
    }

    /// The dictionary id: `"sha256:"` and the lowercase hex SHA-256 of the
    /// dictionary's canonical form.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The id of the profile that publishes the dictionary, such as `kernel/2`.
    pub fn profile(&self) -> &str {
        &self.profile
    }

    /// The families, by name.
    pub fn families(&self) -> &[Family] {
        &self.families
    }

    /// The facets, by ascending prime.
    pub fn facets(&self) -> &[Facet] {
        &self.facets
    }

    /// The family with this name.
    pub fn family(&self, name: &str) -> Option<&Family> {
        self.families
            .binary_search_by(|f| f.name.as_str().cmp(name))
            .ok()
            .map(|i| &self.families[i])
    }

    /// The facet with this id.
    pub fn facet(&self, id: &str) -> Option<&Facet> {
        self.index.get(id).map(|&i| &self.facets[i])
    }
}

/// The members of a value with the dictionary's shape (step 2).
struct Raw<'a> {
    v: &'a str,
    profile: &'a str,
    families: Vec<(&'a str, &'a str)>,
    facets: Vec<(&'a str, f64)>,
}

impl<'a> Raw<'a> {
    fn read(value: &'a Value) -> Option<Raw<'a>> {
        let [v, profile, families, facets] =
            exactly(value, ["v", "profile", "families", "facets"])?;
        let families = families
            .as_array()?
            .iter()
            .map(|f| {
                let [name, values] = exactly(f, ["name", "values"])?;
                Some((name.as_str()?, values.as_str()?))
            })
            .collect::<Option<Vec<_>>>()?;
        let facets = facets
            .as_array()?
            .iter()
            .map(|f| {
                let [id, prime] = exactly(f, ["id", "prime"])?;
                match prime {
                    Value::Number(n) => Some((id.as_str()?, *n)),
                    _ => None,
                }
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Raw {
            v: v.as_str()?,
            profile: profile.as_str()?,
            families,
            facets,
        })
    }
}

/// The members of an object that has exactly these member names. With as
/// many members as names and every name present, each name appears once,
/// even in a value built in code that repeats a name.
fn exactly<'a, const N: usize>(value: &'a Value, names: [&str; N]) -> Option<[&'a Value; N]> {
    if value.as_object()?.len() != N {
        return None;
    }
    let mut out = [value; N];
    for (slot, name) in out.iter_mut().zip(names) {
        *slot = value.get(name)?;
    }
    Some(out)
}

/// `^[a-z][a-z0-9-]*/(0|[1-9][0-9]*)$`, matched against the whole string.
fn is_profile(s: &str) -> bool {
    let Some((name, version)) = s.split_once('/') else {
        return false;
    };
    let version = version.as_bytes();
    is_family_name(name.as_bytes())
        && match version {
            [b'0'] => true,
            [b'1'..=b'9', rest @ ..] => rest.iter().all(u8::is_ascii_digit),
            _ => false,
        }
}

/// `^[a-z][a-z0-9-]*$`: a family name, and the part of a profile id before
/// the slash.
fn is_family_name(s: &[u8]) -> bool {
    match s {
        [first, rest @ ..] => first.is_ascii_lowercase() && rest.iter().all(|&b| is_name_byte(b)),
        [] => false,
    }
}

/// `^[a-z0-9][a-z0-9-]*$`: the part of a facet id after the dot.
fn is_facet_name(s: &[u8]) -> bool {
    match s {
        [first, rest @ ..] => {
            (first.is_ascii_lowercase() || first.is_ascii_digit())
                && rest.iter().all(|&b| is_name_byte(b))
        }
        [] => false,
    }
}

fn is_name_byte(b: u8) -> bool {
    b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'
}

/// The family part of a facet id that matches
/// `^[a-z][a-z0-9-]*\.[a-z0-9][a-z0-9-]*$` (step 8a). Neither part holds a
/// dot, so the id splits at its first.
fn id_family(id: &str) -> Option<&str> {
    let (family, name) = id.split_once('.')?;
    (is_family_name(family.as_bytes()) && is_facet_name(name.as_bytes())).then_some(family)
}

/// Whether `n`, below 2^31, is prime.
///
/// Miller-Rabin with the bases 2, 7 and 61 decides primality exactly for
/// every n below 4,759,123,141 (Jaeschke, 1993), so for every n this needs.
/// The tests check it against trial division.
pub(crate) fn is_prime(n: u32) -> bool {
    if n < 2 {
        return false;
    }
    for p in [2, 3, 5, 7, 11, 13, 61] {
        if n.is_multiple_of(p) {
            return n == p;
        }
    }
    let n = u64::from(n);
    let mut d = n - 1;
    let mut s = 0;
    while d % 2 == 0 {
        d /= 2;
        s += 1;
    }
    [2u64, 7, 61].iter().all(|&a| {
        let mut x = pow_mod(a, d, n);
        if x == 1 || x == n - 1 {
            return true;
        }
        for _ in 1..s {
            x = x * x % n;
            if x == n - 1 {
                return true;
            }
        }
        false
    })
}

/// `base^exp mod m`, for `m` below 2^32 so that products fit 64 bits.
fn pow_mod(base: u64, mut exp: u64, m: u64) -> u64 {
    let mut result = 1;
    let mut b = base % m;
    while exp > 0 {
        if exp & 1 == 1 {
            result = result * b % m;
        }
        b = b * b % m;
        exp >>= 1;
    }
    result
}

fn groups(facets: &[Facet]) -> Vec<Group> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut modulus: u64 = 1;
    for (i, facet) in facets.iter().enumerate() {
        let p = u64::from(facet.prime);
        // modulus < 2^32 and p < 2^31, so the product fits 64 bits.
        if modulus * p > u64::from(u32::MAX) {
            out.push(Group {
                modulus: modulus as u32,
                start,
                end: i,
            });
            start = i;
            modulus = 1;
        }
        modulus *= p;
    }
    out.push(Group {
        modulus: modulus as u32,
        start,
        end: facets.len(),
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trial_division(n: u32) -> bool {
        n >= 2
            && (2..)
                .take_while(|d: &u64| d * d <= u64::from(n))
                .all(|d| !u64::from(n).is_multiple_of(d))
    }

    #[test]
    fn primality_agrees_with_trial_division() {
        for n in 0..200_000 {
            assert_eq!(is_prime(n), trial_division(n), "{n}");
        }
        // Strong pseudoprimes to some of the bases, and numbers near 2^31.
        for n in [
            2047,
            1_373_653,
            25_326_001,
            3_215_031_751,
            2_147_483_647,
            2_147_483_629,
        ] {
            assert_eq!(is_prime(n), trial_division(n), "{n}");
        }
        let mut x: u64 = 12345;
        for _ in 0..20_000 {
            x = x
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let n = ((x >> 33) as u32) | 1;
            assert_eq!(is_prime(n), trial_division(n), "{n}");
        }
        for n in (PRIME_BOUND - 2000)..PRIME_BOUND {
            assert_eq!(is_prime(n), trial_division(n), "{n}");
        }
    }

    #[test]
    fn patterns() {
        assert!(is_profile("kernel/2"));
        assert!(is_profile("catalogue-demo/0"));
        assert!(is_profile("a/10"));
        for bad in [
            "kernel/02",
            "kernel/00",
            "kernel",
            "kernel/",
            "/2",
            "Kernel/2",
            "2kernel/2",
            "kernel/2\n",
            "kernel/2/3",
            "ker.nel/2",
            "kernel/+2",
            "kernel/２",
            "kérnel/2",
        ] {
            assert!(!is_profile(bad), "{bad:?}");
        }
        assert_eq!(id_family("jurisdiction.vic"), Some("jurisdiction"));
        assert_eq!(id_family("multi.p2"), Some("multi"));
        assert_eq!(id_family("a.0-"), Some("a"));
        for bad in [
            "jurisdiction",
            "jurisdiction.",
            ".vic",
            "a.b.c",
            "a.-b",
            "A.b",
            "a.B",
            "1a.b",
            "a.b\n",
            "a .b",
            "a.b ",
        ] {
            assert_eq!(id_family(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn groups_cover_every_facet_once() {
        let facets: Vec<Facet> = (2..20_000u32)
            .filter(|&n| is_prime(n))
            .chain([PRIME_BOUND - 1])
            .map(|p| Facet {
                id: format!("f.p{p}"),
                prime: p,
                family: 0,
            })
            .collect();
        let groups = groups(&facets);
        let mut next = 0;
        for g in &groups {
            assert_eq!(g.start, next);
            assert!(g.end > g.start);
            let product: u64 = facets[g.start..g.end]
                .iter()
                .map(|f| u64::from(f.prime))
                .product();
            assert_eq!(product, u64::from(g.modulus));
            next = g.end;
        }
        assert_eq!(next, facets.len());
    }
}
