//! Property tests with a fixed seed, so every run draws the same cases and
//! no dependency is needed. Each property is checked against an oracle that
//! does not use the crate's arithmetic: the multiset that built a
//! coordinate, u128 arithmetic where it fits, and trial division. The
//! factor-table test also builds its rows from those multisets, not from
//! decode, so a wrong decode cannot give the same wrong rows on both sides.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::*;
use evidence_coordinate::{Dictionary, Error, Values};

/// Facet index → exponent, the oracle for a coordinate.
type Exponents = BTreeMap<usize, u32>;

/// A random list of facets, as ids, that meets every family rule, and its
/// exponents. `max` is the most facets drawn.
fn valid_list(rng: &mut Rng, d: &Dictionary, max: usize) -> (Vec<String>, Exponents) {
    let facets = d.facets().len();
    valid_list_from(rng, d, max, |rng| rng.below(facets))
}

/// As [`valid_list`], with `pick` choosing each facet drawn.
fn valid_list_from(
    rng: &mut Rng,
    d: &Dictionary,
    max: usize,
    mut pick: impl FnMut(&mut Rng) -> usize,
) -> (Vec<String>, Exponents) {
    let mut exponents = Exponents::new();
    let mut used_one = BTreeSet::new();
    let count = rng.below(max + 1);
    for _ in 0..count {
        let i = pick(rng);
        let family = d.facets()[i].family();
        let allowed = match d.families()[family].values() {
            Values::One => !used_one.contains(&family),
            Values::Set => !exponents.contains_key(&i),
            Values::Multiset => true,
        };
        if allowed {
            used_one.insert(family);
            *exponents.entry(i).or_insert(0) += 1;
        }
    }
    let mut list: Vec<String> = exponents
        .iter()
        .flat_map(|(&i, &e)| std::iter::repeat_n(d.facets()[i].id().to_string(), e as usize))
        .collect();
    // The order of the list does not matter: shuffle it.
    for k in (1..list.len()).rev() {
        list.swap(k, rng.below(k + 1));
    }
    (list, exponents)
}

fn primes_of(d: &Dictionary, exponents: &Exponents) -> Vec<u32> {
    exponents
        .iter()
        .flat_map(|(&i, &e)| std::iter::repeat_n(d.facets()[i].prime(), e as usize))
        .collect()
}

fn decoded(d: &Dictionary, coordinate: &str) -> Exponents {
    d.decode(coordinate)
        .unwrap_or_else(|e| panic!("{coordinate}: {e}"))
        .into_iter()
        .map(|f| {
            (
                d.facets().iter().position(|x| x.id() == f.id).unwrap(),
                f.exponent,
            )
        })
        .collect()
}

/// REQ-KRN-006: 1,000 random multisets of up to 40 facets round-trip.
#[test]
fn round_trip_kernel_2() {
    let k = load(&kernel_2_value());
    let mut rng = Rng(2026);
    let (mut past_2_63, mut past_u128) = (0, 0);
    for _ in 0..1000 {
        let (list, want) = valid_list(&mut rng, &k, 40);
        let coordinate = k.encode(&list).unwrap();
        assert!(
            evidence_coordinate::is_canonical(&coordinate),
            "{coordinate}"
        );
        assert_eq!(decoded(&k, &coordinate), want, "{coordinate}");
        match product_u128(&primes_of(&k, &want)) {
            Some(p) => assert_eq!(coordinate, p.to_string()),
            None => past_u128 += 1,
        }
        if coordinate.len() > 19 {
            past_2_63 += 1;
        }
        // Encoding the decoded facets gives the coordinate back.
        let again: Vec<String> = k
            .decode(&coordinate)
            .unwrap()
            .into_iter()
            .flat_map(|f| std::iter::repeat_n(f.id, f.exponent as usize))
            .collect();
        assert_eq!(k.encode(&again).unwrap(), coordinate);
        assert_eq!(k.check(&list, &coordinate), Ok(()));
    }
    assert!(
        past_2_63 > 500 && past_u128 > 100,
        "{past_2_63} past 2^63, {past_u128} past 2^128"
    );
}

#[test]
fn round_trip_with_family_rules() {
    let demo = catalogue_demo();
    let wide = load(&wide_demo_value());
    let mut rng = Rng(4777);
    for _ in 0..2000 {
        let (list, want) = valid_list(&mut rng, &demo, 12);
        let coordinate = demo.encode(&list).unwrap();
        assert_eq!(decoded(&demo, &coordinate), want);
        assert_eq!(
            coordinate,
            product_u128(&primes_of(&demo, &want)).unwrap().to_string()
        );
    }
    // Near the 1,024-bit limit: a product whose log2 is clearly below 1024
    // encodes and round-trips; one clearly above is P_COORDINATE_RANGE.
    // About half the draws come from the 16 `multi` facets, so that repeated
    // facets are common; drawn uniformly, 16 of 4,096 facets rarely repeat.
    let multi = wide
        .facets()
        .iter()
        .take_while(|f| f.id().starts_with("multi."))
        .count();
    assert_eq!(multi, 16);
    let facets = wide.facets().len();
    let (mut below, mut above, mut repeated) = (0, 0, 0);
    for _ in 0..300 {
        let (list, want) = valid_list_from(&mut rng, &wide, 160, |rng| {
            if rng.chance(2) {
                rng.below(multi)
            } else {
                rng.below(facets)
            }
        });
        if want.values().any(|&e| e > 1) {
            repeated += 1;
        }
        let log2: f64 = primes_of(&wide, &want)
            .iter()
            .map(|&p| f64::from(p).log2())
            .sum();
        match wide.encode(&list) {
            Ok(coordinate) => {
                assert!(log2 < 1024.001, "{log2}");
                assert_eq!(decoded(&wide, &coordinate), want);
                below += 1;
            }
            Err(Error::Range) => {
                assert!(log2 > 1023.999, "{log2}");
                above += 1;
            }
            Err(e) => panic!("{e}"),
        }
    }
    assert!(
        below > 20 && above > 20 && repeated > 200,
        "below {below}, above {above}, {repeated} with a repeated facet"
    );
}

/// Decode against trial division on random 64-bit numbers, smooth ones
/// included, under Kernel/2's 70 primes.
#[test]
fn decode_agrees_with_trial_division() {
    let k = load(&kernel_2_value());
    let primes = first_primes(70);
    let mut rng = Rng(349);
    for round in 0..20_000 {
        let n: u64 = if round % 2 == 0 {
            rng.next() >> rng.below(64)
        } else {
            // A product of a few dictionary primes, sometimes times another.
            let mut n: u64 = 1;
            for _ in 0..rng.below(12) {
                let p = u64::from(primes[rng.below(70)]);
                match n.checked_mul(p) {
                    Some(m) => n = m,
                    None => break,
                }
            }
            if rng.chance(4) {
                n = n.saturating_mul([353, 359, 367, 4093][rng.below(4)]);
            }
            n
        };
        if n == 0 {
            assert_eq!(k.decode("0"), Err(Error::Syntax));
            continue;
        }
        let mut residue = n;
        let mut want = Exponents::new();
        for (i, &p) in primes.iter().enumerate() {
            while residue.is_multiple_of(u64::from(p)) {
                residue /= u64::from(p);
                *want.entry(i).or_insert(0) += 1;
            }
        }
        let got = k.decode(&n.to_string());
        if residue == 1 {
            let got: Exponents = got
                .unwrap()
                .into_iter()
                .map(|f| {
                    (
                        primes.iter().position(|&p| p == f.prime).unwrap(),
                        f.exponent,
                    )
                })
                .collect();
            assert_eq!(got, want, "{n}");
        } else {
            assert_eq!(got, Err(Error::UnknownFactor), "{n}");
        }
    }
}

/// `divides` agrees with comparing the exponents of the multisets that
/// built the two coordinates, and with u128 remainders where both fit.
#[test]
fn divides_agrees_with_exponents() {
    let k = load(&kernel_2_value());
    let demo = catalogue_demo();
    let mut rng = Rng(53);
    let (mut yes, mut no) = (0, 0);
    for d in [&k, &demo] {
        for _ in 0..3000 {
            let (c_list, c) = valid_list(&mut rng, d, 30);
            // A query drawn on its own, or part of the coordinate, so that
            // both answers are common.
            let (q_list, q) = if rng.chance(2) {
                valid_list(&mut rng, d, 4)
            } else {
                let mut q = Exponents::new();
                for (&i, &e) in &c {
                    if rng.chance(3) {
                        q.insert(i, 1 + rng.below(e as usize) as u32);
                    }
                }
                if rng.chance(5) {
                    if let Some((_, e)) = q.iter_mut().next() {
                        *e += 1;
                    }
                }
                let list: Vec<String> = q
                    .iter()
                    .flat_map(|(&i, &e)| {
                        std::iter::repeat_n(d.facets()[i].id().to_string(), e as usize)
                    })
                    .collect();
                (list, q)
            };
            // Raising an exponent can break a family rule; such a query
            // does not encode, and is left out here.
            let Ok(query) = d.encode(&q_list) else {
                continue;
            };
            let coordinate = d.encode(&c_list).unwrap();
            let want = q.iter().all(|(i, e)| c.get(i).is_some_and(|ce| ce >= e));
            assert_eq!(
                d.divides(&query, &coordinate),
                Ok(want),
                "{query} | {coordinate}"
            );
            if let (Some(qn), Some(cn)) = (
                product_u128(&primes_of(d, &q)),
                product_u128(&primes_of(d, &c)),
            ) {
                assert_eq!(cn % qn == 0, want);
            }
            if want {
                yes += 1;
            } else {
                no += 1;
            }
            // The query "1" divides every coordinate.
            assert_eq!(d.divides("1", &coordinate), Ok(true));
        }
    }
    assert!(yes > 1000 && no > 1000, "yes {yes}, no {no}");
}

/// An index of factor rows (object, prime, exponent) and a per-object row,
/// queried as "has every prime of the query with at least its exponent",
/// returns the objects that the meaning of divisibility gives, the same as
/// testing `divides` on every stored coordinate; and the same error for a
/// query that does not decode.
///
/// The rows come from the multisets that built each coordinate, and the
/// reference answer from those multisets alone, with u128 remainders where
/// both numbers fit. The index decodes each query first, as spec section
/// Indexing requires, so a decode that loses exponents gives rows that no
/// longer match.
#[test]
fn factor_rows_agree_with_divides() {
    let k = load(&kernel_2_value());
    let mut rng = Rng(16);
    // Each object: its coordinate and the exponents that built it.
    let objects: Vec<(String, Exponents)> = (0..400)
        .map(|_| {
            let max = if rng.chance(10) { 0 } else { 40 };
            let (list, exponents) = valid_list(&mut rng, &k, max);
            (k.encode(&list).unwrap(), exponents)
        })
        .collect();
    assert!(objects.iter().any(|(c, _)| c == "1"));
    assert!(
        objects.iter().any(|(c, _)| c.len() > 19),
        "no coordinate past 2^63"
    );

    // The index: one row per (object, prime), from the generating multiset,
    // and one per object.
    let mut rows: Vec<(usize, u32, u32)> = Vec::new();
    for (object, (_, exponents)) in objects.iter().enumerate() {
        for (&i, &e) in exponents {
            rows.push((object, k.facets()[i].prime(), e));
        }
    }
    let index_answer = |query: &str| -> Result<BTreeSet<usize>, Error> {
        // Decode the query first, and return its error.
        let q = k.decode(query)?;
        if q.is_empty() {
            return Ok((0..objects.len()).collect());
        }
        let mut hits: BTreeMap<usize, usize> = BTreeMap::new();
        for &(object, prime, exponent) in &rows {
            if q.iter().any(|f| f.prime == prime && exponent >= f.exponent) {
                *hits.entry(object).or_insert(0) += 1;
            }
        }
        Ok(hits
            .into_iter()
            .filter(|&(_, n)| n == q.len())
            .map(|(o, _)| o)
            .collect())
    };
    let scan_answer = |query: &str| -> Result<BTreeSet<usize>, Error> {
        let mut out = BTreeSet::new();
        for (object, (coordinate, _)) in objects.iter().enumerate() {
            if k.divides(query, coordinate)? {
                out.insert(object);
            }
        }
        Ok(out)
    };
    // The meaning of divisibility, from the multisets alone.
    let reference = |q: &Exponents| -> BTreeSet<usize> {
        objects
            .iter()
            .enumerate()
            .filter(|(_, (_, c))| q.iter().all(|(i, e)| c.get(i).is_some_and(|ce| ce >= e)))
            .map(|(o, _)| o)
            .collect()
    };

    // Each query with the multiset that built it, or None for one that does
    // not decode.
    assert_eq!(k.facets()[0].prime(), 2);
    let mut queries: Vec<(String, Option<Exponents>)> = vec![
        ("1".to_string(), Some(Exponents::new())),
        ("2".to_string(), Some(Exponents::from([(0, 1)]))),
        ("353".to_string(), None),
        ("01".to_string(), None),
    ];
    // Part of a stored object's multiset, which that object matches; and,
    // one time in four, with one exponent raised by one, which it may not.
    while queries.len() < 304 {
        let (_, base) = &objects[rng.below(objects.len())];
        if base.is_empty() {
            continue;
        }
        let mut q = Exponents::new();
        for (n, (&i, &e)) in base.iter().enumerate() {
            if n == 0 || rng.chance(3) {
                q.insert(i, 1 + rng.below(e as usize) as u32);
            }
        }
        if rng.chance(4) {
            let n = rng.below(q.len());
            if let Some(e) = q.values_mut().nth(n) {
                *e += 1;
            }
        }
        let list: Vec<String> = q
            .iter()
            .flat_map(|(&i, &e)| std::iter::repeat_n(k.facets()[i].id().to_string(), e as usize))
            .collect();
        queries.push((k.encode(&list).unwrap(), Some(q)));
    }

    let (mut some_not_all, mut exponents_matter, mut by_remainder) = (0, 0, 0);
    for (query, q) in &queries {
        let index = index_answer(query);
        assert_eq!(index, scan_answer(query), "{query}");
        let Some(q) = q else {
            assert!(index.is_err(), "{query}");
            continue;
        };
        let want = reference(q);
        assert_eq!(index, Ok(want.clone()), "{query}");
        if !want.is_empty() && want.len() < objects.len() {
            some_not_all += 1;
        }
        // A query whose answer changes when its exponents are ignored.
        let once: Exponents = q.keys().map(|&i| (i, 1)).collect();
        if reference(&once) != want {
            exponents_matter += 1;
        }
        // Exact integer divisibility, where both numbers fit u128.
        if let Some(qn) = product_u128(&primes_of(&k, q)) {
            assert_eq!(*query, qn.to_string());
            for (object, (_, c)) in objects.iter().enumerate() {
                if let Some(cn) = product_u128(&primes_of(&k, c)) {
                    assert_eq!(cn % qn == 0, want.contains(&object), "{query} | {cn}");
                    by_remainder += 1;
                }
            }
        }
    }
    assert!(
        some_not_all > 200 && exponents_matter > 20 && by_remainder > 10_000,
        "of {} queries, {some_not_all} match some objects but not all and \
         {exponents_matter} depend on their exponents; {by_remainder} remainders",
        queries.len()
    );
    assert_eq!(index_answer("353"), Err(Error::UnknownFactor));
    assert_eq!(index_answer("01"), Err(Error::Syntax));
}

/// Random strings over digits, signs, spaces and other characters are
/// `P_COORDINATE_SYNTAX` exactly when they do not match `^[1-9][0-9]*$`.
#[test]
fn syntax_is_the_pattern() {
    let k = load(&kernel_2_value());
    let alphabet = [
        '0', '1', '2', '5', '9', '+', '-', ' ', '\n', '.', 'e', '\u{FF15}', '\u{0665}',
    ];
    let mut rng = Rng(12);
    for _ in 0..20_000 {
        let len = rng.below(8);
        let s: String = (0..len)
            .map(|_| alphabet[rng.below(alphabet.len())])
            .collect();
        let pattern = s
            .as_bytes()
            .first()
            .is_some_and(|b| (b'1'..=b'9').contains(b))
            && s.bytes().all(|b| b.is_ascii_digit());
        assert_eq!(evidence_coordinate::is_canonical(&s), pattern, "{s:?}");
        assert_eq!(k.decode(&s) == Err(Error::Syntax), !pattern, "{s:?}");
    }
}
