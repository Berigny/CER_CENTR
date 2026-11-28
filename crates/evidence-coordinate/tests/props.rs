//! Property tests with a fixed seed, so every run draws the same cases and
//! no dependency is needed. Each property is checked against an oracle that
//! does not use the crate's arithmetic: the multiset that built a
//! coordinate, u128 arithmetic where it fits, trial division, and a table of
//! factor rows queried the way an index would.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::*;
use evidence_coordinate::{Dictionary, Error, Values};

/// Facet index → exponent, the oracle for a coordinate.
type Exponents = BTreeMap<usize, u32>;

/// A random list of facets, as ids, that meets every family rule, and its
/// exponents. `max` is the most facets drawn.
fn valid_list(rng: &mut Rng, d: &Dictionary, max: usize) -> (Vec<String>, Exponents) {
    let mut exponents = Exponents::new();
    let mut used_one = BTreeSet::new();
    let count = rng.below(max + 1);
    for _ in 0..count {
        let i = rng.below(d.facets().len());
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
    let (mut below, mut above) = (0, 0);
    for _ in 0..300 {
        let (list, want) = valid_list(&mut rng, &wide, 160);
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
    assert!(below > 20 && above > 20, "below {below}, above {above}");
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
/// returns the same objects as testing `divides` on every stored coordinate,
/// and the same error for a query that does not decode.
#[test]
fn factor_rows_agree_with_divides() {
    let k = load(&kernel_2_value());
    let mut rng = Rng(16);
    let objects: Vec<String> = (0..400)
        .map(|_| {
            let max = if rng.chance(10) { 0 } else { 40 };
            let (list, _) = valid_list(&mut rng, &k, max);
            k.encode(&list).unwrap()
        })
        .collect();
    assert!(objects.iter().any(|c| c == "1"));
    assert!(
        objects.iter().any(|c| c.len() > 19),
        "no coordinate past 2^63"
    );

    // The index: one row per (object, prime), and one per object.
    let mut rows: Vec<(usize, u32, u32)> = Vec::new();
    for (object, coordinate) in objects.iter().enumerate() {
        for f in k.decode(coordinate).unwrap() {
            rows.push((object, f.prime, f.exponent));
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
        for (object, coordinate) in objects.iter().enumerate() {
            if k.divides(query, coordinate)? {
                out.insert(object);
            }
        }
        Ok(out)
    };

    let mut queries: Vec<String> = vec![
        "1".to_string(),
        "353".to_string(),
        "01".to_string(),
        "2".to_string(),
    ];
    // Part of a stored coordinate, which that object matches; and, one time
    // in four, with one exponent raised by one, which it may not.
    while queries.len() < 304 {
        let base = &objects[rng.below(objects.len())];
        let facets = k.decode(base).unwrap();
        if facets.is_empty() {
            continue;
        }
        let mut list = Vec::new();
        for (n, f) in facets.iter().enumerate() {
            if n == 0 || rng.chance(3) {
                for _ in 0..1 + rng.below(f.exponent as usize) {
                    list.push(f.id.clone());
                }
            }
        }
        if rng.chance(4) {
            list.push(list[0].clone());
        }
        queries.push(k.encode(&list).unwrap());
    }
    let mut nonempty = 0;
    for query in &queries {
        let index = index_answer(query);
        let scan = scan_answer(query);
        assert_eq!(index, scan, "{query}");
        if index
            .as_ref()
            .is_ok_and(|s| !s.is_empty() && s.len() < objects.len())
        {
            nonempty += 1;
        }
    }
    assert!(
        nonempty > 200,
        "{nonempty} of {} queries match some objects but not all",
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
