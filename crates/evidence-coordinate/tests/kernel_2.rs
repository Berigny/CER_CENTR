//! Kernel/2's dictionary rules, REQ-KRN-001 to 005 (`docs/kernel-2.md`
//! section 4), on the `kernel-2` dictionary vector, and the coordinates of
//! its section 5 example.
//!
//! The test reads the vector from disk. The library may not (see
//! `clippy.toml`), so the purity lints are allowed here only.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

mod common;

use std::fs;
use std::path::Path;

use common::*;
use evidence_coordinate::{extends, Dictionary, Values};

fn vector() -> Dictionary {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test-vectors/coordinate/dictionaries/valid/kernel-2.json");
    let bytes = fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    Dictionary::parse(&bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn trial_division(n: u32) -> bool {
    n >= 2
        && (2..n)
            .take_while(|d| d * d <= n)
            .all(|d| !n.is_multiple_of(d))
}

#[test]
fn req_krn_001_to_005() {
    let k = vector();
    assert_eq!(k.profile(), "kernel/2");
    assert_eq!(k.id(), KERNEL_2_ID);
    let primes: Vec<u32> = k.facets().iter().map(|f| f.prime()).collect();

    // REQ-KRN-001: every dictionary value is prime (loading checks it; this
    // checks it again by trial division).
    assert_eq!(primes.len(), 70);
    assert_eq!(primes.iter().filter(|&&p| trial_division(p)).count(), 70);

    // REQ-KRN-002: the 70 values are distinct.
    let mut distinct = primes.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(distinct.len(), 70);

    // REQ-KRN-003: the largest is 349, and the values are the first 70
    // primes in order.
    assert_eq!(primes.iter().max(), Some(&349));
    assert_eq!(primes, first_primes(70));

    // The seven blocks are the families, each a multiset.
    let mut blocks = KERNEL_BLOCKS.to_vec();
    blocks.sort_unstable();
    let families: Vec<(&str, Values)> = k
        .families()
        .iter()
        .map(|f| (f.name(), f.values()))
        .collect();
    assert_eq!(
        families,
        blocks
            .iter()
            .map(|b| (*b, Values::Multiset))
            .collect::<Vec<_>>()
    );

    let first_70 = first_primes(70);
    for (b, block) in KERNEL_BLOCKS.iter().enumerate() {
        let prime = |position: &str| {
            k.facet(&format!("{block}.{position}"))
                .unwrap_or_else(|| panic!("{block}.{position} is missing"))
                .prime()
        };
        // REQ-KRN-004: block b, position i is P[10b + i]: entry, exit, then
        // the eight states in the order of section 2.2.
        for (i, position) in KERNEL_POSITIONS.iter().enumerate() {
            assert_eq!(prime(position), first_70[10 * b + i], "{block}.{position}");
        }
        // REQ-KRN-005: entry < exit < every state.
        assert!(prime("entry") < prime("exit"), "{block}");
        for state in &KERNEL_POSITIONS[2..] {
            assert!(prime("exit") < prime(state), "{block}.{state}");
        }
    }
    assert_eq!(
        k.facets().iter().filter(|f| f.id().contains('.')).count(),
        70
    );

    // The vector equals the dictionary built here from the allocation rule.
    let built = load(&kernel_2_value());
    assert_eq!(built, k);
    assert_eq!(extends(&built, &k), Ok(()));
}

/// Kernel/2 section 5: S and H after each event, derived by section 3.2's
/// rules and encoded, against the table.
#[test]
fn section_5_coordinates() {
    let k = vector();
    let events: [(&[&str], &str, &str); 5] = [
        (
            &["universal.entry", "registry.entry", "registry.locked"],
            "179422",
            "179422",
        ),
        (
            &[
                "infrastructure.entry",
                "infrastructure.quantified",
                "infrastructure.committed",
            ],
            "2822653806194",
            "2822653806194",
        ),
        (
            &[
                "measurement.entry",
                "measurement.quantified",
                "measurement.unit",
            ],
            "229518448943052722",
            "229518448943052722",
        ),
        (
            &[
                "measurement.exit",
                "reporting.entry",
                "reporting.quantified",
                "reporting.linked",
            ],
            "8321327376004027894",
            "25035387438225574245358414",
        ),
        (
            &["reporting.exit", "registry.executed"],
            "985106178361706",
            "1144592878288235028923541329666",
        ),
    ];
    let mut state: Vec<String> = Vec::new();
    let mut history: Vec<String> = Vec::new();
    for (n, (add, want_s, want_h)) in events.iter().enumerate() {
        for facet in *add {
            history.push(facet.to_string());
            let (block, position) = facet.split_once('.').unwrap();
            if position == "exit" {
                state.retain(|f| !f.starts_with(&format!("{block}.")));
            } else {
                state.push(facet.to_string());
            }
        }
        assert_eq!(
            k.encode(&state).as_deref(),
            Ok(*want_s),
            "S after E{}",
            n + 1
        );
        assert_eq!(
            k.encode(&history).as_deref(),
            Ok(*want_h),
            "H after E{}",
            n + 1
        );
        // REQ-KRN-013: the derived coordinate checks; another one is drift.
        assert_eq!(k.check(&history, want_h), Ok(()));
        assert!(k.check(&state, want_h).is_err() || want_s == want_h);
    }
    // H after E5 is 100 bits; it decodes back to the multiset that built it.
    let decoded = k.decode("1144592878288235028923541329666").unwrap();
    let total: u32 = decoded.iter().map(|f| f.exponent).sum();
    assert_eq!(total as usize, history.len());
}
