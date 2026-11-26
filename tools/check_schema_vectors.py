"""Checks that the JSON Schemas and the test vectors agree.

The Rust core hand-codes the schema rules. This script makes sure the published
JSON Schema accepts and rejects the same vectors, so the two cannot drift:

- vectors expecting E_SHAPE must fail the object schema;
- vectors expecting E_ATTESTATION_SHAPE must have an attestation line that fails
  the attestation schema;
- every other vector must pass both schemas (parse-level vectors are skipped,
  because Python's json module is lenient about exactly those inputs).

    pip install "jsonschema>=4.23,<5"
    python tools/check_schema_vectors.py
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parent.parent
PARSE_LEVEL = {"E_PARSE", "E_DUPLICATE_KEY", "E_NUMBER_RANGE"}


def validator(name: str) -> Draft202012Validator:
    schema = json.loads((ROOT / "spec" / "schema" / name).read_text(encoding="utf-8"))
    Draft202012Validator.check_schema(schema)
    return Draft202012Validator(schema)


def main() -> int:
    obj_schema = validator("evidence-object-0.schema.json")
    att_schema = validator("attestation-0.schema.json")
    failures: list[str] = []
    checked = 0

    for case_dir in sorted((ROOT / "test-vectors" / "objects").iterdir()):
        expect = json.loads((case_dir / "case.json").read_text(encoding="utf-8"))["expect"]
        code = expect.get("code")
        if code in PARSE_LEVEL:
            continue
        checked += 1
        obj = json.loads((case_dir / "object.json").read_text(encoding="utf-8"))
        obj_errors = [e.message for e in obj_schema.iter_errors(obj)]
        if code == "E_SHAPE" and not obj_errors:
            failures.append(f"{case_dir.name}: schema accepts an object the spec rejects")
        if code != "E_SHAPE" and obj_errors:
            failures.append(f"{case_dir.name}: schema rejects a shape-valid object: {obj_errors[0]}")

        att_file = case_dir / "attestations.jsonl"
        lines = att_file.read_text(encoding="utf-8").splitlines() if att_file.exists() else []
        bad_lines = [ln for ln in lines if ln.strip() and list(att_schema.iter_errors(json.loads(ln)))]
        if code == "E_ATTESTATION_SHAPE" and not bad_lines:
            failures.append(f"{case_dir.name}: attestation schema accepts every line")
        if code != "E_ATTESTATION_SHAPE" and bad_lines:
            failures.append(f"{case_dir.name}: attestation schema rejects a line")

    if failures:
        print(f"{len(failures)} disagreement(s):\n  " + "\n  ".join(failures))
        return 1
    print(f"JSON Schemas agree with all {checked} schema-level object vectors.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
