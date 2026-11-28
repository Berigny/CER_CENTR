//! A small unsigned big integer: exactly what coordinate/0 needs, and no
//! more (spec section Performance). Every operation is between a big number
//! and a 32-bit one: multiplication, division with remainder, comparison and
//! bit length, and decimal conversion.
//!
//! Limbs are 32 bits, least significant first, with no high zero limb, so
//! zero has no limbs and each value has one representation.

use std::cmp::Ordering;

/// The largest power of ten that fits a limb, and its exponent: decimal
/// conversion works in chunks of nine digits.
const CHUNK: u32 = 1_000_000_000;
const CHUNK_DIGITS: usize = 9;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Big {
    limbs: Vec<u32>,
}

impl Big {
    pub(crate) fn one() -> Big {
        Big { limbs: vec![1] }
    }

    /// The value of a string of ASCII digits. The caller has checked that
    /// every byte is a digit; leading zeros are allowed here.
    pub(crate) fn from_digits(digits: &[u8]) -> Big {
        let mut n = Big {
            limbs: Vec::with_capacity(digits.len() / CHUNK_DIGITS + 1),
        };
        let head = digits.len() % CHUNK_DIGITS;
        let (first, rest) = digits.split_at(head);
        if !first.is_empty() {
            n.mul_add(10u32.pow(head as u32), chunk_value(first));
        }
        for chunk in rest.chunks(CHUNK_DIGITS) {
            n.mul_add(CHUNK, chunk_value(chunk));
        }
        n
    }

    /// The canonical decimal string: no sign and no leading zero.
    pub(crate) fn to_decimal(&self) -> String {
        if self.is_zero() {
            return "0".to_string();
        }
        let mut n = self.clone();
        let mut chunks = Vec::with_capacity(self.limbs.len() * 32 / 29 + 1);
        while !n.is_zero() {
            chunks.push(n.div_rem(CHUNK));
        }
        let mut out = String::with_capacity(chunks.len() * CHUNK_DIGITS);
        let mut iter = chunks.iter().rev();
        if let Some(top) = iter.next() {
            out.push_str(&top.to_string());
        }
        for chunk in iter {
            out.push_str(&format!("{chunk:09}"));
        }
        out
    }

    pub(crate) fn is_zero(&self) -> bool {
        self.limbs.is_empty()
    }

    pub(crate) fn is_one(&self) -> bool {
        self.limbs == [1]
    }

    /// The number of bits needed to write the value: 0 for zero, 1 for one,
    /// k + 1 for 2^k.
    pub(crate) fn bits(&self) -> u64 {
        match self.limbs.last() {
            None => 0,
            Some(&top) => {
                (self.limbs.len() as u64 - 1) * 32 + u64::from(u32::BITS - top.leading_zeros())
            }
        }
    }

    /// The value, when it fits 64 bits.
    pub(crate) fn to_u64(&self) -> Option<u64> {
        match self.limbs.as_slice() {
            [] => Some(0),
            [lo] => Some(u64::from(*lo)),
            [lo, hi] => Some(u64::from(*hi) << 32 | u64::from(*lo)),
            _ => None,
        }
    }

    /// `self = self * m`.
    pub(crate) fn mul(&mut self, m: u32) {
        self.mul_add(m, 0);
    }

    /// `self = self * m + a`.
    fn mul_add(&mut self, m: u32, a: u32) {
        let mut carry = u64::from(a);
        for limb in &mut self.limbs {
            let t = u64::from(*limb) * u64::from(m) + carry;
            *limb = t as u32;
            carry = t >> 32;
        }
        if carry != 0 {
            self.limbs.push(carry as u32);
        }
        self.normalise();
    }

    /// `self mod d`, leaving `self` unchanged. `d` must not be zero.
    pub(crate) fn rem(&self, d: u32) -> u32 {
        let d = u64::from(d);
        let mut r = 0u64;
        for &limb in self.limbs.iter().rev() {
            r = (r << 32 | u64::from(limb)) % d;
        }
        r as u32
    }

    /// `self = self / d`, rounding down, and returns the remainder. `d` must
    /// not be zero.
    pub(crate) fn div_rem(&mut self, d: u32) -> u32 {
        let d = u64::from(d);
        let mut r = 0u64;
        for limb in self.limbs.iter_mut().rev() {
            let cur = r << 32 | u64::from(*limb);
            *limb = (cur / d) as u32;
            r = cur % d;
        }
        self.normalise();
        r as u32
    }

    fn normalise(&mut self) {
        while self.limbs.last() == Some(&0) {
            self.limbs.pop();
        }
    }
}

impl Ord for Big {
    fn cmp(&self, other: &Big) -> Ordering {
        self.limbs
            .len()
            .cmp(&other.limbs.len())
            .then_with(|| self.limbs.iter().rev().cmp(other.limbs.iter().rev()))
    }
}

impl PartialOrd for Big {
    fn partial_cmp(&self, other: &Big) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// The value of at most nine ASCII digits.
fn chunk_value(digits: &[u8]) -> u32 {
    digits
        .iter()
        .fold(0u32, |acc, &d| acc * 10 + u32::from(d - b'0'))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// splitmix64: a small deterministic generator, so the tests need no
    /// dependency and every run draws the same numbers.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }

        fn u128(&mut self) -> u128 {
            // Vary the size: shift right by 0 to 127 bits.
            let n = u128::from(self.next()) << 64 | u128::from(self.next());
            n >> (self.next() % 128)
        }

        fn divisor(&mut self) -> u32 {
            match self.next() % 4 {
                0 => (self.next() % 64) as u32 + 1,
                1 => (self.next() % 65_536) as u32 + 1,
                2 => u32::MAX - (self.next() % 16) as u32,
                _ => (self.next() as u32).max(1),
            }
        }
    }

    fn big(n: u128) -> Big {
        Big::from_digits(n.to_string().as_bytes())
    }

    fn value(b: &Big) -> u128 {
        b.to_decimal().parse().unwrap()
    }

    #[test]
    fn small_values() {
        assert_eq!(Big::from_digits(b"0").to_decimal(), "0");
        assert!(Big::from_digits(b"000").is_zero());
        assert_eq!(Big::from_digits(b"0001").to_decimal(), "1");
        assert!(Big::one().is_one());
        assert_eq!(Big::one().bits(), 1);
        assert_eq!(Big::from_digits(b"0").bits(), 0);
        assert_eq!(big(4294967296).bits(), 33);
        assert_eq!(big(4294967295).bits(), 32);
        assert_eq!(big(u128::from(u64::MAX)).to_u64(), Some(u64::MAX));
        assert_eq!(big(u128::from(u64::MAX) + 1).to_u64(), None);
        assert_eq!(Big::from_digits(b"1000000000").to_decimal(), "1000000000");
        assert_eq!(Big::from_digits(b"999999999").to_decimal(), "999999999");
    }

    #[test]
    fn agrees_with_u128() {
        let mut rng = Rng(1);
        for _ in 0..20_000 {
            let n = rng.u128();
            let b = big(n);
            assert_eq!(value(&b), n);
            assert_eq!(b.bits(), u64::from(128 - n.leading_zeros()));
            assert_eq!(b.to_u64(), u64::try_from(n).ok());

            let d = rng.divisor();
            assert_eq!(u128::from(b.rem(d)), n % u128::from(d));
            let mut q = b.clone();
            let r = q.div_rem(d);
            assert_eq!(u128::from(r), n % u128::from(d));
            assert_eq!(value(&q), n / u128::from(d));

            let m = rng.divisor();
            if let Some(p) = n.checked_mul(u128::from(m)) {
                let mut x = b.clone();
                x.mul(m);
                assert_eq!(value(&x), p);
            }

            let other = rng.u128();
            assert_eq!(b.cmp(&big(other)), n.cmp(&other));
        }
    }

    #[test]
    fn large_values_round_trip() {
        // 2^1024 and its neighbours, written out by repeated doubling.
        let mut n = Big::one();
        for _ in 0..1024 {
            n.mul(2);
        }
        assert_eq!(n.bits(), 1025);
        let text = n.to_decimal();
        assert_eq!(text.len(), 309);
        assert!(text.starts_with("17976931348623159077"));
        assert_eq!(Big::from_digits(text.as_bytes()), n);
        for _ in 0..1024 {
            assert_eq!(n.div_rem(2), 0);
        }
        assert!(n.is_one());

        // Multiplying and dividing by the same numbers gives the start back.
        let mut rng = Rng(7);
        let mut x = Big::one();
        let mut factors = Vec::new();
        while x.bits() < 2000 {
            let f = rng.divisor();
            factors.push(f);
            x.mul(f);
        }
        let again = Big::from_digits(x.to_decimal().as_bytes());
        assert_eq!(again, x);
        for f in factors.iter().rev() {
            assert_eq!(x.rem(*f), 0);
            assert_eq!(x.div_rem(*f), 0);
        }
        assert!(x.is_one());
    }
}
