"""Checks that the JSON Schemas and the test vectors agree.

The Rust core hand-codes the schema rules. This script makes sure the published
JSON Schema accepts and rejects the same vectors, so the two cannot drift:

- vectors expecting E_SHAPE must fail the object schema;
- vectors expecting E_ATTESTATION_SHAPE must have an attestation line that fails
  the attestation schema;
- every other vector must pass both schemas (parse-level vectors are skipped,
  because Python's json module is lenient about exactly those inputs);
- store/entries.json: every valid entry must pass the log-entry schema, and
  every invalid input must fail it. So must each valid entry with a newline
  appended to one of its strings, which the code rejects.

JSON Schema patterns are ECMA-262 regular expressions. The jsonschema package
runs them with Python's `re.search`, whose `$` also matches just before a final
newline, so `^sha256:[0-9a-f]{64}$` would accept "sha256:…\n". This script
runs patterns with ECMA-262's `$` (end of input only) instead.

    pip install "jsonschema>=4.23,<5"
    python tools/check_schema_vectors.py
"""

from __future__ import annotations

import functools
import json
import re
import sys
from pathlib import Path

from jsonschema import Draft202012Validator, ValidationError, validators

ROOT = Path(__file__).resolve().parent.parent
PARSE_LEVEL = {"E_PARSE", "E_DUPLICATE_KEY", "E_NUMBER_RANGE"}


@functools.lru_cache(maxsize=None)
def ecma_regex(pattern: str) -> re.Pattern[str]:
    """Compiles a schema pattern so that `$` means what it does in ECMA-262:
    the end of the input. Each `$` anchor outside a character class becomes
    Python's `\\Z`; escapes and character classes are kept as written."""
    out: list[str] = []
    in_class = False
    i = 0
    while i < len(pattern):
        c = pattern[i]
        if c == "\\":
            out.append(pattern[i : i + 2])
            i += 2
            continue
        if in_class:
            in_class = c != "]"
        elif c == "[":
            in_class = True
        elif c == "$":
            c = r"\Z"
        out.append(c)
        i += 1
    return re.compile("".join(out))


def ecma_pattern(validator, pattern, instance, schema):
    """The `pattern` keyword, with ECMA-262's `$`."""
    if validator.is_type(instance, "string") and not ecma_regex(pattern).search(instance):
        yield ValidationError(f"{instance!r} does not match {pattern!r}")


SchemaValidator = validators.extend(Draft202012Validator, {"pattern": ecma_pattern})

assert ecma_regex("^a$").search("a") and not ecma_regex("^a$").search("a\n")
assert ecma_regex("^[a$]+$").search("a$") and not ecma_regex("^[a$]+$").search("a$\n")


def validator(name: str) -> Draft202012Validator:
    schema = json.loads((ROOT / "spec" / "schema" / name).read_text(encoding="utf-8"))
    Draft202012Validator.check_schema(schema)
    return SchemaValidator(schema)


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

    entries_checked = check_entries(failures)

    if failures:
        print(f"{len(failures)} disagreement(s):\n  " + "\n  ".join(failures))
        return 1
    print(f"JSON Schemas agree with all {checked} schema-level object vectors.")
    if entries_checked:
        print(f"The log-entry schema agrees with all {entries_checked} store entry vectors.")
    return 0


def check_entries(failures: list[str]) -> int:
    """Runs the log-entry schema over store/entries.json; returns the cases checked."""
    schema_file = ROOT / "spec" / "schema" / "log-entry-0.schema.json"
    vectors = ROOT / "test-vectors" / "store" / "entries.json"
    if not (schema_file.exists() and vectors.exists()):
        return 0
    entry_schema = validator(schema_file.name)
    cases = json.loads(vectors.read_text(encoding="utf-8"))
    for case in cases["valid"]:
        errors = [e.message for e in entry_schema.iter_errors(case["entry"])]
        if errors:
            failures.append(f"store/entries/{case['name']}: schema rejects a valid entry: {errors[0]}")
    for case in cases["invalid"]:
        if not list(entry_schema.iter_errors(json.loads(case["input"]))):
            failures.append(f"store/entries/{case['name']}: schema accepts an entry the spec rejects")
    # A newline after any string member: the code rejects every one.
    derived = 0
    for case in cases["valid"]:
        for key, value in case["entry"].items():
            if isinstance(value, str):
                derived += 1
                if not list(entry_schema.iter_errors({**case["entry"], key: value + "\n"})):
                    failures.append(f"store/entries/{case['name']}: schema accepts {key} with a trailing newline")
    return len(cases["valid"]) + len(cases["invalid"]) + derived


if __name__ == "__main__":
    sys.exit(main())
