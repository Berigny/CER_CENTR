//! The RFC 9162 Merkle tree over the log: tree hash (section 2.1.1), inclusion
//! proofs (2.1.3) and consistency proofs (2.1.4). See `spec/store-0.md`,
//! "Merkle tree" and "Proofs".
//!
//! - Leaf n of the log is the canonical bytes of entry n, at 0-based index n − 1.
//! - Leaf hash = SHA-256(0x00 ‖ leaf); node hash = SHA-256(0x01 ‖ left ‖ right).
//! - A tree of n > 1 leaves splits at k, the largest power of two less than n.
//!
//! Verification answers true or false and never panics: a bad index, a bad
//! size, a proof of the wrong length or one that does not reach the root are
//! all simply false.

use sha2::{Digest as _, Sha256};

/// A SHA-256 digest: a leaf hash, a node hash or a tree hash.
pub type Digest = [u8; 32];

const LEAF_PREFIX: u8 = 0x00;
const NODE_PREFIX: u8 = 0x01;

/// The largest tree size: a size is a seq, and a seq is at most 2^53 − 1.
/// Verification answers false for anything larger, as the JavaScript
/// reference does.
pub const MAX_SIZE: u64 = (1 << 53) - 1;

/// SHA-256(0x00 ‖ leaf).
pub fn leaf_hash(leaf: &[u8]) -> Digest {
    let mut h = Sha256::new();
    h.update([LEAF_PREFIX]);
    h.update(leaf);
    h.finalize().into()
}

/// SHA-256(0x01 ‖ left ‖ right).
fn node_hash(left: &Digest, right: &Digest) -> Digest {
    let mut h = Sha256::new();
    h.update([NODE_PREFIX]);
    h.update(left);
    h.update(right);
    h.finalize().into()
}

/// The largest power of two less than `n`, for n ≥ 2.
fn split(n: usize) -> usize {
    debug_assert!(n >= 2);
    let k = n.next_power_of_two();
    if k == n {
        n / 2
    } else {
        k / 2
    }
}

/// The Merkle Tree Hash of the leaves whose leaf hashes are given.
///
/// For no leaves this is SHA-256 of the empty string, as RFC 9162 defines;
/// store/0 never uses that tree.
pub fn root(leaf_hashes: &[Digest]) -> Digest {
    match leaf_hashes {
        [] => Sha256::digest([]).into(),
        [one] => *one,
        _ => {
            let k = split(leaf_hashes.len());
            node_hash(&root(&leaf_hashes[..k]), &root(&leaf_hashes[k..]))
        }
    }
}

/// The inclusion proof `PATH(index, D[n])` for the leaf at 0-based `index` in
/// the tree of these leaves, from the leaf's sibling up to the root's child.
///
/// An index that is not below the tree size has no proof; the result is empty.
pub fn inclusion_proof(leaf_hashes: &[Digest], index: u64) -> Vec<Digest> {
    let mut proof = Vec::new();
    if let Ok(index) = usize::try_from(index) {
        if index < leaf_hashes.len() {
            path(index, leaf_hashes, &mut proof);
        }
    }
    proof
}

fn path(m: usize, leaves: &[Digest], out: &mut Vec<Digest>) {
    if leaves.len() <= 1 {
        return;
    }
    let k = split(leaves.len());
    if m < k {
        path(m, &leaves[..k], out);
        out.push(root(&leaves[k..]));
    } else {
        path(m - k, &leaves[k..], out);
        out.push(root(&leaves[..k]));
    }
}

/// Verify an inclusion proof (RFC 9162 section 2.1.3.2): that the leaf with
/// this leaf hash is at 0-based `index` in the tree of `size` leaves whose
/// root is `root`.
pub fn verify_inclusion(
    leaf_hash: &Digest,
    index: u64,
    size: u64,
    proof: &[Digest],
    root: &Digest,
) -> bool {
    if index >= size || size > MAX_SIZE {
        return false;
    }
    let (mut f, mut s) = (index, size - 1);
    let mut r = *leaf_hash;
    for p in proof {
        if s == 0 {
            return false;
        }
        if f & 1 == 1 || f == s {
            r = node_hash(p, &r);
            while f & 1 == 0 && f != 0 {
                f >>= 1;
                s >>= 1;
            }
        } else {
            r = node_hash(&r, p);
        }
        f >>= 1;
        s >>= 1;
    }
    s == 0 && r == *root
}

/// The consistency proof `PROOF(old_size, D[n])` that the tree of the first
/// `old_size` leaves is a prefix of the tree of all of them.
///
/// RFC 9162 defines a proof for 0 < old_size < n only. For any other old size
/// the result is empty, which never verifies.
pub fn consistency_proof(leaf_hashes: &[Digest], old_size: u64) -> Vec<Digest> {
    let mut proof = Vec::new();
    if let Ok(m) = usize::try_from(old_size) {
        if 0 < m && m < leaf_hashes.len() {
            subproof(m, leaf_hashes, true, &mut proof);
        }
    }
    proof
}

fn subproof(m: usize, leaves: &[Digest], whole: bool, out: &mut Vec<Digest>) {
    let n = leaves.len();
    if m == n {
        if !whole {
            out.push(root(leaves));
        }
        return;
    }
    let k = split(n);
    if m <= k {
        subproof(m, &leaves[..k], whole, out);
        out.push(root(&leaves[k..]));
    } else {
        subproof(m - k, &leaves[k..], false, out);
        out.push(root(&leaves[..k]));
    }
}

/// Verify a consistency proof (RFC 9162 section 2.1.4.2) from the tree of
/// `old_size` leaves with root `old_root` to the tree of `new_size` leaves
/// with root `new_root`. False unless 0 < old_size < new_size ≤ [`MAX_SIZE`].
pub fn verify_consistency(
    old_size: u64,
    new_size: u64,
    old_root: &Digest,
    new_root: &Digest,
    proof: &[Digest],
) -> bool {
    if old_size == 0 || old_size >= new_size || new_size > MAX_SIZE || proof.is_empty() {
        return false;
    }
    // When the old tree is a complete subtree of the new one, the proof omits
    // its root, which the verifier already holds.
    let path: Vec<&Digest> = if old_size.is_power_of_two() {
        std::iter::once(old_root).chain(proof).collect()
    } else {
        proof.iter().collect()
    };
    let (mut f, mut s) = (old_size - 1, new_size - 1);
    while f & 1 == 1 {
        f >>= 1;
        s >>= 1;
    }
    let (mut fr, mut sr) = (*path[0], *path[0]);
    for c in &path[1..] {
        if s == 0 {
            return false;
        }
        if f & 1 == 1 || f == s {
            fr = node_hash(c, &fr);
            sr = node_hash(c, &sr);
            while f & 1 == 0 && f != 0 {
                f >>= 1;
                s >>= 1;
            }
        } else {
            sr = node_hash(&sr, c);
        }
        f >>= 1;
        s >>= 1;
    }
    fr == *old_root && sr == *new_root && s == 0
}

/// `"sha256:"` + 64 lowercase hex digits.
pub fn to_text(d: &Digest) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(71);
    out.push_str("sha256:");
    for b in d {
        out.push(HEX[usize::from(b >> 4)] as char);
        out.push(HEX[usize::from(b & 0x0f)] as char);
    }
    out
}

/// The digest written as `"sha256:"` + 64 lowercase hex digits, or None for
/// any other text (uppercase hex included, as for every evidence/0 hash).
pub fn from_text(s: &str) -> Option<Digest> {
    let hex = s.strip_prefix("sha256:")?.as_bytes();
    if hex.len() != 64 {
        return None;
    }
    let nibble = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    };
    let mut d = [0u8; 32];
    for (i, byte) in d.iter_mut().enumerate() {
        *byte = (nibble(hex[2 * i])? << 4) | nibble(hex[2 * i + 1])?;
    }
    Some(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sha(parts: &[&[u8]]) -> Digest {
        let mut h = Sha256::new();
        for p in parts {
            h.update(p);
        }
        h.finalize().into()
    }

    fn leaves(n: usize) -> Vec<Digest> {
        (0..n)
            .map(|i| leaf_hash(format!("leaf-{i}").as_bytes()))
            .collect()
    }

    /// The tree hash straight from the RFC's recursive definition, written
    /// out by hand for the small trees below.
    fn n(l: Digest, r: Digest) -> Digest {
        sha(&[&[1], &l, &r])
    }

    #[test]
    fn hashes_use_the_domain_prefixes() {
        assert_eq!(leaf_hash(b"abc"), sha(&[&[0], b"abc"]));
        let (a, b) = (leaf_hash(b"a"), leaf_hash(b"b"));
        assert_eq!(node_hash(&a, &b), sha(&[&[1], &a, &b]));
        assert_ne!(leaf_hash(b""), sha(&[b""]));
    }

    #[test]
    fn known_answers() {
        // RFC 6962 / 9162 test data: the empty tree, and the tree of one
        // empty leaf.
        assert_eq!(
            to_text(&root(&[])),
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            to_text(&leaf_hash(b"")),
            "sha256:6e340b9cffb37a989ca544e6bb780a2c78901d3fb33738768511a30617afa01d"
        );
    }

    #[test]
    fn sizes_above_the_largest_seq_never_verify() {
        // Leaf 0 of a tree of 2^k leaves: the proof is k right siblings and
        // the root folds them in on the right. 2^52 is a valid size; 2^53
        // is one more than the largest seq, so the same arithmetic is refused.
        let leaf = leaf_hash(b"leaf");
        let fold = |k: usize| {
            let proof: Vec<Digest> = (0..k).map(|i| leaf_hash(&[i as u8])).collect();
            let root = proof.iter().fold(leaf, |r, p| node_hash(&r, p));
            (proof, root)
        };
        let (proof, root52) = fold(52);
        assert!(verify_inclusion(&leaf, 0, 1 << 52, &proof, &root52));
        let (proof, root53) = fold(53);
        assert!(!verify_inclusion(&leaf, 0, 1 << 53, &proof, &root53));
        assert_eq!(MAX_SIZE, (1 << 53) - 1);

        // The old tree as the complete left half of the new one: the proof is
        // the right half's root.
        let right = leaf_hash(b"right");
        let new_root = node_hash(&root52, &right);
        assert!(verify_consistency(
            1 << 51,
            1 << 52,
            &fold(51).1,
            &node_hash(&fold(51).1, &right),
            &[right]
        ));
        assert!(!verify_consistency(
            1 << 52,
            1 << 53,
            &root52,
            &new_root,
            &[right]
        ));
    }

    #[test]
    fn split_is_the_largest_power_of_two_below_n() {
        let want = [
            (2, 1),
            (3, 2),
            (4, 2),
            (5, 4),
            (7, 4),
            (8, 4),
            (9, 8),
            (16, 8),
            (17, 16),
        ];
        for (n, k) in want {
            assert_eq!(split(n), k, "n = {n}");
        }
    }

    #[test]
    fn roots_of_small_trees_by_hand() {
        let d = leaves(7);
        assert_eq!(root(&d[..1]), d[0]);
        assert_eq!(root(&d[..2]), n(d[0], d[1]));
        assert_eq!(root(&d[..3]), n(n(d[0], d[1]), d[2]));
        assert_eq!(root(&d[..4]), n(n(d[0], d[1]), n(d[2], d[3])));
        assert_eq!(root(&d[..5]), n(n(n(d[0], d[1]), n(d[2], d[3])), d[4]));
        assert_eq!(
            root(&d[..7]),
            n(n(n(d[0], d[1]), n(d[2], d[3])), n(n(d[4], d[5]), d[6]))
        );
    }

    #[test]
    fn inclusion_proofs_by_hand() {
        let d = leaves(7);
        let (ab, cd, ef) = (n(d[0], d[1]), n(d[2], d[3]), n(d[4], d[5]));
        // RFC 9162 section 2.1.5's example tree of seven leaves.
        assert_eq!(inclusion_proof(&d, 0), vec![d[1], cd, n(ef, d[6])]);
        assert_eq!(inclusion_proof(&d, 3), vec![d[2], ab, n(ef, d[6])]);
        assert_eq!(inclusion_proof(&d, 4), vec![d[5], d[6], n(ab, cd)]);
        assert_eq!(inclusion_proof(&d, 6), vec![ef, n(ab, cd)]);
        assert_eq!(inclusion_proof(&d[..1], 0), Vec::<Digest>::new());
        // Out of range: no proof, no panic.
        assert!(inclusion_proof(&d, 7).is_empty());
        assert!(inclusion_proof(&d, u64::MAX).is_empty());
        assert!(inclusion_proof(&[], 0).is_empty());
    }

    #[test]
    fn consistency_proofs_by_hand() {
        let d = leaves(7);
        let (ab, cd, ef) = (n(d[0], d[1]), n(d[2], d[3]), n(d[4], d[5]));
        // RFC 9162 section 2.1.5's examples.
        assert_eq!(consistency_proof(&d, 3), vec![d[2], d[3], ab, n(ef, d[6])]);
        assert_eq!(consistency_proof(&d, 4), vec![n(ef, d[6])]);
        assert_eq!(consistency_proof(&d, 6), vec![ef, d[6], n(ab, cd)]);
        assert_eq!(consistency_proof(&d[..2], 1), vec![d[1]]);
        // Outside 0 < old_size < n: no proof.
        assert!(consistency_proof(&d, 0).is_empty());
        assert!(consistency_proof(&d, 7).is_empty());
        assert!(consistency_proof(&d, 8).is_empty());
        assert!(consistency_proof(&d, u64::MAX).is_empty());
    }

    #[test]
    fn every_inclusion_proof_verifies_and_every_change_fails() {
        for size in 1..=33usize {
            let d = leaves(size);
            let r = root(&d);
            for i in 0..size {
                let idx = i as u64;
                let sz = size as u64;
                let proof = inclusion_proof(&d, idx);
                assert!(
                    verify_inclusion(&d[i], idx, sz, &proof, &r),
                    "{i} of {size}"
                );

                // Wrong leaf, index, size or root.
                assert!(!verify_inclusion(&leaf_hash(b"other"), idx, sz, &proof, &r));
                assert!(!verify_inclusion(&d[i], idx, sz, &proof, &leaf_hash(b"x")));
                if size > 1 {
                    let other = (i + 1) % size;
                    assert!(!verify_inclusion(&d[i], other as u64, sz, &proof, &r));
                }
                // Any flipped byte in any element.
                for j in 0..proof.len() {
                    let mut bad = proof.clone();
                    bad[j][j % 32] ^= 0x01;
                    assert!(!verify_inclusion(&d[i], idx, sz, &bad, &r));
                }
                // Too short and too long.
                if !proof.is_empty() {
                    assert!(!verify_inclusion(&d[i], idx, sz, &proof[1..], &r));
                    assert!(!verify_inclusion(
                        &d[i],
                        idx,
                        sz,
                        &proof[..proof.len() - 1],
                        &r
                    ));
                }
                let mut long = proof.clone();
                long.push(r);
                assert!(!verify_inclusion(&d[i], idx, sz, &long, &r));
            }
            // Out of range.
            assert!(!verify_inclusion(&d[0], size as u64, size as u64, &[], &r));
            assert!(!verify_inclusion(&d[0], u64::MAX, size as u64, &[], &r));
        }
        // A tree of no leaves includes nothing.
        assert!(!verify_inclusion(&leaf_hash(b""), 0, 0, &[], &root(&[])));
        assert!(!verify_inclusion(
            &leaf_hash(b""),
            0,
            u64::MAX,
            &[],
            &root(&[])
        ));
    }

    #[test]
    fn every_consistency_proof_verifies_and_every_change_fails() {
        for size in 2..=33usize {
            let d = leaves(size);
            let new_root = root(&d);
            let ns = size as u64;
            for m in 1..size {
                let old_root = root(&d[..m]);
                let ms = m as u64;
                let proof = consistency_proof(&d, ms);
                assert!(!proof.is_empty());
                assert!(
                    verify_consistency(ms, ns, &old_root, &new_root, &proof),
                    "{m} -> {size}"
                );

                assert!(!verify_consistency(ms, ns, &new_root, &new_root, &proof));
                assert!(!verify_consistency(ms, ns, &old_root, &old_root, &proof));
                for j in 0..proof.len() {
                    let mut bad = proof.clone();
                    bad[j][31 - j % 32] ^= 0x80;
                    assert!(!verify_consistency(ms, ns, &old_root, &new_root, &bad));
                }
                assert!(!verify_consistency(
                    ms,
                    ns,
                    &old_root,
                    &new_root,
                    &proof[1..]
                ));
                assert!(!verify_consistency(
                    ms,
                    ns,
                    &old_root,
                    &new_root,
                    &proof[..proof.len() - 1]
                ));
                let mut long = proof.clone();
                long.push(new_root);
                assert!(!verify_consistency(ms, ns, &old_root, &new_root, &long));
                assert!(!verify_consistency(ms, ns, &old_root, &new_root, &[]));
            }
            // Sizes outside 0 < old < new.
            let any = [new_root];
            assert!(!verify_consistency(0, ns, &root(&[]), &new_root, &any));
            assert!(!verify_consistency(ns, ns, &new_root, &new_root, &any));
            assert!(!verify_consistency(ns + 1, ns, &new_root, &new_root, &any));
            assert!(!verify_consistency(
                u64::MAX,
                ns,
                &new_root,
                &new_root,
                &any
            ));
        }
        assert!(!verify_consistency(
            1,
            u64::MAX,
            &[0; 32],
            &[0; 32],
            &[[0; 32]]
        ));
    }

    #[test]
    fn text_round_trip() {
        let d = leaf_hash(b"leaf-0");
        let t = to_text(&d);
        assert_eq!(t.len(), 71);
        assert_eq!(from_text(&t), Some(d));
        assert_eq!(
            t,
            evidence_core::canon::sha256_hex(&[&[0u8][..], b"leaf-0"].concat())
        );
        assert_eq!(from_text(&t.to_uppercase()), None);
        assert_eq!(from_text(&t.replace("sha256:", "sha512:")), None);
        assert_eq!(from_text(&t[..70]), None);
        assert_eq!(from_text(&format!("{t}0")), None);
        assert_eq!(from_text(&format!("{}g", &t[..70])), None);
        assert_eq!(from_text(""), None);
        // Multi-byte UTF-8 of the right byte length is not hex.
        assert_eq!(from_text(&format!("sha256:{}", "é".repeat(32))), None);
    }
}
