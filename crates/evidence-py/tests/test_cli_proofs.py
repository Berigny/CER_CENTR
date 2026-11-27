"""T11 through the Python package (docs/m1-store-spec.md section 11).

Builds a store with the ``evidence`` command-line tool, asks it for
inclusion and consistency proofs, and checks each one with evidence_layer::

    cargo build -p evidence-cli
    EVIDENCE_BIN=$PWD/target/debug/evidence python -m unittest discover -s crates/evidence-py/tests

The store: ``store init`` (name ``proofs``, the test key store-demo.seed),
object vectors 001-005 appended one file at a time, the attestations of 003,
and three checkpoints (sizes 10, 12 and 14; head 16). Each proof verifies,
its leaf hash and roots are the ones the store's lines give, and the last hex
digit changed in the first proof element, the leaf hash or a root makes
verification false.

``EVIDENCE_BIN`` names the CLI binary. When it is unset the tests are
skipped, unless ``CI`` is ``true``: then they fail, so CI cannot skip them.
"""

from __future__ import annotations

import hashlib
import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path
from typing import Any, ClassVar, Dict, List

import evidence_layer as ev

VECTORS = Path(__file__).resolve().parents[3] / "test-vectors"
OBJECTS = VECTORS / "objects"
SEED = VECTORS / "keys" / "store-demo.seed"


def canonical(value: Any) -> str:
    return ev.canonicalize(json.dumps(value))


def leaf_hash(entry: str) -> str:
    return "sha256:" + hashlib.sha256(b"\x00" + entry.encode()).hexdigest()


def changed(digest: str) -> str:
    """The last hex digit replaced: 0 by 1, any other by 0."""
    return digest[:-1] + ("1" if digest.endswith("0") else "0")


def case_dir(number: str) -> Path:
    return next(p for p in OBJECTS.iterdir() if p.name.startswith(number + "-"))


class CliProofs(unittest.TestCase):
    bin: ClassVar[str]
    scratch: ClassVar[tempfile.TemporaryDirectory]
    store_dir: ClassVar[Path]
    checkpoints: ClassVar[List[Dict[str, Any]]]
    entries: ClassVar[List[str]]

    @classmethod
    def setUpClass(cls) -> None:
        binary = os.environ.get("EVIDENCE_BIN", "")
        if not binary:
            message = "EVIDENCE_BIN is not set: build the CLI (cargo build -p evidence-cli) and set it"
            if os.environ.get("CI") == "true":
                raise AssertionError(message)
            raise unittest.SkipTest(message)
        cls.bin = binary
        cls.scratch = tempfile.TemporaryDirectory()
        try:
            cls.build_store()
        except BaseException:
            cls.scratch.cleanup()
            raise

    @classmethod
    def tearDownClass(cls) -> None:
        cls.scratch.cleanup()

    @classmethod
    def store(cls, *args: str) -> List[Dict[str, Any]]:
        """Run ``evidence store <args>``, which must exit 0, and return its
        lines, each checked to be canonical JSON."""
        run = subprocess.run([cls.bin, "store", *args], capture_output=True, check=False)
        command = "evidence store " + " ".join(args)
        if run.returncode != 0:
            raise AssertionError(f"{command}: exit {run.returncode}\n{run.stdout!r}\n{run.stderr!r}")
        text = run.stdout.decode()
        if not text.endswith("\n"):
            raise AssertionError(f"{command}: output does not end in LF")
        lines = []
        for line in text[:-1].split("\n"):
            value = json.loads(line)
            if canonical(value) != line:
                raise AssertionError(f"{command}: line is not canonical: {line}")
            lines.append(value)
        return lines

    @classmethod
    def one(cls, *args: str) -> Dict[str, Any]:
        lines = cls.store(*args)
        if len(lines) != 1:
            raise AssertionError(f"evidence store {' '.join(args)}: {len(lines)} lines, want 1")
        return lines[0]

    @classmethod
    def build_store(cls) -> None:
        cls.store_dir = Path(cls.scratch.name) / "s"
        s = str(cls.store_dir)
        cls.one("init", s, "--name", "proofs", "--seed", str(SEED))
        for number in ("001", "002", "003", "004", "005"):
            cls.store("append", s, str(case_dir(number) / "object.json"))
        cls.store("attest", s, str(case_dir("003") / "attestations.jsonl"))
        cls.checkpoints = [cls.one("checkpoint", s, "--seed", str(SEED)) for _ in range(3)]
        sizes = [c["size"] for c in cls.checkpoints]
        if sizes != [10, 12, 14]:
            raise AssertionError(f"checkpoint sizes {sizes}, want [10, 12, 14]")
        log = cls.store("log", s, "--limit", "100")
        if len(log) != 16:
            raise AssertionError(f"store log printed {len(log)} lines, want 16")
        cls.entries = [canonical(line["entry"]) for line in log]

    def root(self, size: int) -> str:
        return ev.merkle_root(self.entries[:size])

    def test_checkpoint_roots(self) -> None:
        for c in self.checkpoints:
            self.assertEqual(c["root"], self.root(c["size"]))

    def test_inclusion(self) -> None:
        """``--seq n`` at the latest checkpoint's size (the default), and
        ``--seq n --size 10``."""
        s = str(self.store_dir)
        checked = 0
        for checkpoint, extra in ((self.checkpoints[2], []), (self.checkpoints[0], ["--size", "10"])):
            size = checkpoint["size"]
            for seq in range(1, size + 1):
                with self.subTest(seq=seq, size=size):
                    p = self.one("prove", s, "--seq", str(seq), *extra)

                    def verify(leaf: str, proof: List[str], root: str) -> bool:
                        return ev.verify_inclusion(leaf, seq - 1, size, proof, root)

                    self.assertEqual((p["seq"], p["size"]), (seq, size))
                    self.assertEqual(p["root"], checkpoint["root"])
                    self.assertEqual(p["root"], self.root(size))
                    self.assertEqual(p["checkpoint"], checkpoint["checkpoint"])
                    self.assertEqual(canonical(p["entry"]), self.entries[seq - 1])
                    self.assertEqual(p["leaf_hash"], leaf_hash(self.entries[seq - 1]))
                    self.assertIs(verify(p["leaf_hash"], p["proof"], p["root"]), True)
                    self.assertGreater(len(p["proof"]), 0)
                    bent = [changed(p["proof"][0])] + p["proof"][1:]
                    self.assertIs(verify(p["leaf_hash"], bent, p["root"]), False)
                    self.assertIs(verify(changed(p["leaf_hash"]), p["proof"], p["root"]), False)
                    self.assertIs(verify(p["leaf_hash"], p["proof"], changed(p["root"])), False)
                    checked += 1
        self.assertEqual(checked, 24)

    def test_consistency(self) -> None:
        """``--from a --to b`` for every 1 <= a < b <= 16."""
        s = str(self.store_dir)
        checked = 0
        for a in range(1, 17):
            for b in range(a + 1, 17):
                with self.subTest(old_size=a, new_size=b):
                    p = self.one("prove", s, "--from", str(a), "--to", str(b))

                    def verify(old: str, new: str, proof: List[str]) -> bool:
                        return ev.verify_consistency(a, b, old, new, proof)

                    self.assertEqual((p["old_size"], p["new_size"]), (a, b))
                    self.assertEqual(p["old_root"], self.root(a))
                    self.assertEqual(p["new_root"], self.root(b))
                    self.assertIs(verify(p["old_root"], p["new_root"], p["proof"]), True)
                    self.assertGreater(len(p["proof"]), 0)
                    bent = [changed(p["proof"][0])] + p["proof"][1:]
                    self.assertIs(verify(p["old_root"], p["new_root"], bent), False)
                    self.assertIs(verify(changed(p["old_root"]), p["new_root"], p["proof"]), False)
                    self.assertIs(verify(p["old_root"], changed(p["new_root"]), p["proof"]), False)
                    checked += 1
        self.assertEqual(checked, 120)


if __name__ == "__main__":
    unittest.main()
