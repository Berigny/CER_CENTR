"""Runs every store vector (test-vectors/store) through the Python package.

    pip install ./crates/evidence-py
    python -m unittest discover -s crates/evidence-py/tests

Export reports must equal the golden reports in canonical form, byte for
byte (spec/store-0.md, "Conformance").
"""

from __future__ import annotations

import hashlib
import json
import re
import unittest
from pathlib import Path
from typing import Any, List

import evidence_layer as ev

STORE = Path(__file__).resolve().parents[3] / "test-vectors" / "store"
EXPORTS = STORE / "exports"
SEGMENT_FILE = re.compile(r"^([0-9]{6}|[1-9][0-9]{6,})\.jsonl$")


def canonical(value: Any) -> str:
    return ev.canonicalize(json.dumps(value))


def leaf_hash(leaf: bytes) -> str:
    return "sha256:" + hashlib.sha256(b"\x00" + leaf).hexdigest()


def segments(case: Path) -> List[bytes]:
    """The case's segment files in file-name order, as raw bytes."""
    names = sorted(p.name for p in case.iterdir() if SEGMENT_FILE.match(p.name))
    return [(case / name).read_bytes() for name in names]


class EntryVectors(unittest.TestCase):
    vectors = json.loads((STORE / "entries.json").read_text(encoding="utf-8"))

    def test_valid(self) -> None:
        self.assertGreater(len(self.vectors["valid"]), 0)
        for case in self.vectors["valid"]:
            with self.subTest(case["name"]):
                self.assertEqual(canonical(case["entry"]), case["canonical"])
                self.assertEqual(ev.entry_hash(case["canonical"]), case["hash"])
                self.assertEqual(ev.entry_hash(case["canonical"].encode()), case["hash"])
                self.assertEqual(ev.entry_hash(json.dumps(case["entry"], indent=2)), case["hash"])

    def test_invalid(self) -> None:
        self.assertGreater(len(self.vectors["invalid"]), 0)
        for case in self.vectors["invalid"]:
            with self.subTest(case["name"]):
                with self.assertRaises(ev.EvidenceError) as raised:
                    ev.entry_hash(case["input"])
                self.assertEqual(raised.exception.code, "E_USAGE")
                self.assertTrue(raised.exception.detail.startswith(case["error"]))


class MerkleVectors(unittest.TestCase):
    def test_merkle(self) -> None:
        trees = json.loads((STORE / "merkle.json").read_text(encoding="utf-8"))
        self.assertGreater(len(trees), 0)
        for tree in trees:
            with self.subTest(tree["name"]):
                leaves = tree["leaves"]
                n = len(leaves)
                self.assertEqual(ev.merkle_root(leaves), tree["root"])
                self.assertEqual(ev.merkle_root([leaf.encode() for leaf in leaves]), tree["root"])
                for p in tree["inclusion"]:
                    leaf = leaves[p["index"]].encode() if p["index"] < n else b""
                    got = ev.verify_inclusion(leaf_hash(leaf), p["index"], n, p["proof"], tree["root"])
                    self.assertIs(got, p["valid"], f"inclusion of {p['index']}")
                for p in tree["consistency"]:
                    got = ev.verify_consistency(p["old_size"], n, p["old_root"], tree["root"], p["proof"])
                    self.assertIs(got, p["valid"], f"consistency from {p['old_size']}")

    def test_empty_tree(self) -> None:
        self.assertEqual(ev.merkle_root([]), "sha256:" + hashlib.sha256(b"").hexdigest())


class ExportVectors(unittest.TestCase):
    def test_exports(self) -> None:
        cases = sorted(EXPORTS.iterdir())
        self.assertGreater(len(cases), 0)
        for case in cases:
            with self.subTest(case.name):
                spec = json.loads((case / "case.json").read_text(encoding="utf-8"))
                report = ev.verify_export(
                    (case / "manifest.json").read_bytes(),
                    segments(case),
                    spec.get("max_findings", 1000),
                )
                self.assertEqual(canonical(report), canonical(spec["expect"]))

    def test_proofs(self) -> None:
        """Case 003's proofs verify, and their leaf hashes and roots are the
        ones the export's lines give."""
        case = EXPORTS / "003-attestations-two-checkpoints"
        proofs = json.loads((case / "case.json").read_text(encoding="utf-8"))["proofs"]
        entries = [
            canonical(json.loads(line)["entry"]).encode()
            for segment in segments(case)
            for line in segment.split(b"\n")[:-1]
        ]
        self.assertGreater(len(proofs["inclusion"]), 0)
        for p in proofs["inclusion"]:
            with self.subTest(seq=p["seq"]):
                self.assertTrue(
                    ev.verify_inclusion(p["leaf_hash"], p["seq"] - 1, p["size"], p["proof"], p["root"])
                )
                self.assertEqual(p["leaf_hash"], leaf_hash(entries[p["seq"] - 1]))
                self.assertEqual(p["root"], ev.merkle_root(entries[: p["size"]]))
        self.assertEqual(len(proofs["consistency"]), 1)
        for p in proofs["consistency"]:
            self.assertTrue(
                ev.verify_consistency(
                    old_size=p["old_size"],
                    new_size=p["new_size"],
                    old_root=p["old_root"],
                    new_root=p["new_root"],
                    proof=p["proof"],
                )
            )
            self.assertEqual(p["old_root"], ev.merkle_root(entries[: p["old_size"]]))
            self.assertEqual(p["new_root"], ev.merkle_root(entries[: p["new_size"]]))


class StoreArguments(unittest.TestCase):
    case = EXPORTS / "002-records-and-checkpoint"
    one = leaf_hash(b"leaf-0")

    def test_forms_agree(self) -> None:
        manifest = (self.case / "manifest.json").read_bytes()
        files = segments(self.case)
        from_bytes = ev.verify_export(manifest, files)
        from_text = ev.verify_export(manifest.decode(), [f.decode() for f in files])
        by_keyword = ev.verify_export(
            manifest=bytearray(manifest), segments=tuple(files), max_findings=1000
        )
        self.assertEqual(from_bytes, from_text)
        self.assertEqual(from_bytes, by_keyword)
        self.assertTrue(from_bytes["ok"])

    def test_damage_is_a_report(self) -> None:
        report = ev.verify_export(b"", [b"\xff\n"], max_findings=1)
        self.assertFalse(report["ok"])
        self.assertTrue(report["truncated"])
        self.assertEqual(report["findings"][0]["code"], "S_MANIFEST_INVALID")
        self.assertTrue(ev.verify_inclusion(self.one, 0, 1, [], self.one))
        self.assertFalse(ev.verify_inclusion(self.one, 2**53 - 2, 2**53 - 1, [], self.one))
        self.assertFalse(ev.verify_consistency(1, 1, self.one, self.one, []))

    def test_usage_errors(self) -> None:
        one = self.one
        h = "sha256:" + "b" * 64
        manifest = (self.case / "manifest.json").read_bytes()
        bad = {
            "entry_hash of an int": lambda: ev.entry_hash(5),  # type: ignore[arg-type]
            "entry_hash of a lone surrogate": lambda: ev.entry_hash("\ud800"),
            "leaves as one bytes": lambda: ev.merkle_root(b"leaf-0"),
            "leaves of ints": lambda: ev.merkle_root([1, 2]),  # type: ignore[list-item]
            "leaves not iterable": lambda: ev.merkle_root(None),  # type: ignore[arg-type]
            "index -1": lambda: ev.verify_inclusion(one, -1, 1, [], one),
            "index True": lambda: ev.verify_inclusion(one, True, 1, [], one),
            "index 0.0": lambda: ev.verify_inclusion(one, 0.0, 1, [], one),  # type: ignore[arg-type]
            "size 2**53": lambda: ev.verify_inclusion(one, 0, 2**53, [], one),
            "size 2**64": lambda: ev.verify_inclusion(one, 0, 2**64, [], one),
            "uppercase hash": lambda: ev.verify_inclusion(one.upper().replace("SHA256", "sha256"), 0, 1, [], one),
            "hash as bytes": lambda: ev.verify_inclusion(one.encode(), 0, 1, [], one),  # type: ignore[arg-type]
            "proof as text": lambda: ev.verify_consistency(1, 2, one, h, h),
            "short proof element": lambda: ev.verify_consistency(1, 2, one, h, ["sha256:00"]),
            "proof of None": lambda: ev.verify_consistency(1, 2, one, h, [None]),  # type: ignore[list-item]
            "max_findings 0": lambda: ev.verify_export(manifest, [], 0),
            "max_findings False": lambda: ev.verify_export(manifest, [], False),
            "max_findings 2**53": lambda: ev.verify_export(manifest, [], 2**53),
            "segments as one bytes": lambda: ev.verify_export(manifest, b"x"),
            "manifest None": lambda: ev.verify_export(None, []),  # type: ignore[arg-type]
        }
        for what, fn in bad.items():
            with self.subTest(what):
                with self.assertRaises(ev.EvidenceError) as raised:
                    fn()
                self.assertEqual(raised.exception.code, "E_USAGE")


if __name__ == "__main__":
    unittest.main()
