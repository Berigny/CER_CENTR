"""Runs every test vector through the Python package.

    pip install ./crates/evidence-py
    python -m unittest discover -s crates/evidence-py/tests
"""

from __future__ import annotations

import json
import unittest
from pathlib import Path

import evidence_layer as ev

VECTORS = Path(__file__).resolve().parents[3] / "test-vectors"


class CanonVectors(unittest.TestCase):
    def test_canon(self) -> None:
        for case in json.loads((VECTORS / "canon.json").read_text(encoding="utf-8")):
            with self.subTest(case["name"]):
                if "error" in case:
                    with self.assertRaises(ev.EvidenceError) as raised:
                        ev.canonicalize(case["input"])
                    self.assertEqual(raised.exception.code, case["error"])
                else:
                    self.assertEqual(ev.canonicalize(case["input"]), case["expected"])


class ObjectVectors(unittest.TestCase):
    def test_objects(self) -> None:
        dirs = sorted((VECTORS / "objects").iterdir())
        self.assertGreater(len(dirs), 0)
        for d in dirs:
            with self.subTest(d.name):
                case = json.loads((d / "case.json").read_text(encoding="utf-8"))
                att = d / "attestations.jsonl"
                store = [p.read_bytes() for p in sorted((d / "store").iterdir())] if case.get("store") else None
                report = ev.verify(
                    (d / "object.json").read_bytes(),
                    attestations=att.read_bytes() if att.exists() else None,
                    store=store,
                    require=case.get("require", []),
                )
                want = case["expect"]
                self.assertEqual(report["ok"], want["ok"], report)
                for key in ("code", "id", "content_checked"):
                    if key in want:
                        self.assertEqual(report.get(key), want[key], report)
                for purpose, n in want.get("attestations", {}).items():
                    self.assertEqual(report["attestations"].get(purpose, 0), n)


class Signing(unittest.TestCase):
    case = VECTORS / "objects" / "003-agent-assertion-approved"

    def test_matches_reference_signature(self) -> None:
        lines = (self.case / "attestations.jsonl").read_text(encoding="utf-8").splitlines()
        approve = next(line for line in lines if '"purpose":"approve"' in line)
        line = ev.sign(
            (self.case / "object.json").read_bytes(),
            seed_hex=(VECTORS / "keys" / "assessor-01.seed").read_text(encoding="utf-8"),
            signer="person:assessor-01",
            purpose="approve",
            at=json.loads(approve)["at"],
        )
        self.assertEqual(line, approve)

    def test_round_trip_and_errors(self) -> None:
        obj = (VECTORS / "objects" / "001-create-inline" / "object.json").read_bytes()
        seed = ev.generate_seed()
        line = ev.sign(obj, seed_hex=seed, signer="person:test", purpose="witness")
        self.assertEqual(json.loads(line)["key"], ev.public_key(seed))
        report = ev.verify(obj, attestations=line + "\n", require=["witness"])
        self.assertTrue(report["ok"], report)

        with self.assertRaises(ev.EvidenceError) as raised:
            ev.verify(obj, require=["bless"])
        self.assertEqual(raised.exception.code, "E_USAGE")
        with self.assertRaises(ev.EvidenceError) as raised:
            ev.sign(b"{}", seed_hex=seed, signer="x", purpose="approve")
        self.assertEqual(raised.exception.code, "E_SHAPE")
        self.assertEqual(ev.SPEC_VERSION, "evidence/0")


if __name__ == "__main__":
    unittest.main()
