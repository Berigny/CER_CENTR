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
- store/exports: every manifest.json must pass the segments schema (case 127's
  manifest is invalid only in its bytes, which no schema sees), and every
  golden report (case.json `expect`) the store-report schema. In the segment
  lines, every entry, object and attestation must pass its schema, and every
  genesis and checkpoint object's content.inline the store-genesis or
  checkpoint schema, unless the golden report has the finding that rejects it
  at that line's seq.
- coordinate/dictionaries.json: every valid dictionary must pass the
  coordinate-dictionary schema, and every invalid one whose `detail` comes from
  a step the schema covers (2, 3, 4, 5a, 5b, 6, 7, 8a, 8d and 8e of
  spec/coordinate-0.md) must fail it. So must catalogue-demo with a newline
  appended to one of its strings, or with an unknown member at the top level,
  and wide-demo with a 4,097th facet. Step 1 cases (`core` set) are skipped,
  as above.

Missing store and coordinate vectors are a failure, not a skip.

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
    exports_checked = check_exports(failures)
    dictionaries_checked = check_dictionaries(failures)

    if failures:
        print(f"{len(failures)} disagreement(s):\n  " + "\n  ".join(failures))
        return 1
    print(f"JSON Schemas agree with all {checked} schema-level object vectors.")
    print(f"The log-entry schema agrees with all {entries_checked} store entry vectors.")
    summary = ", ".join(f"{n} {what}" for what, n in exports_checked.items())
    print(f"The store schemas agree with the export vectors: {summary}.")
    print(f"The coordinate-dictionary schema agrees with all {dictionaries_checked} dictionary cases (the vectors and variants built from them).")
    return 0


def check_entries(failures: list[str]) -> int:
    """Runs the log-entry schema over store/entries.json; returns the cases checked."""
    vectors = ROOT / "test-vectors" / "store" / "entries.json"
    if not vectors.exists():
        failures.append("store/entries.json: missing")
        return 0
    entry_schema = validator("log-entry-0.schema.json")
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


def segment_lines(data: bytes) -> list[bytes]:
    """The lines of a segment file: split at every LF, with the bytes after the
    last LF as one more line when the file does not end in LF."""
    lines = data.split(b"\n")
    if lines and lines[-1] == b"":
        lines.pop()
    return lines


def check_exports(failures: list[str]) -> dict[str, int]:
    """Runs the store schemas over store/exports; returns what was checked."""
    counts = dict.fromkeys(
        ["manifests", "reports", "entries", "objects", "attestations", "genesis contents", "checkpoint contents"], 0
    )
    exports = ROOT / "test-vectors" / "store" / "exports"
    if not exports.is_dir() or not any(exports.iterdir()):
        failures.append("store/exports: missing")
        return counts
    if not (ROOT / "test-vectors" / "store" / "merkle.json").exists():
        failures.append("store/merkle.json: missing")
    names = ["segments", "store-report", "log-entry", "evidence-object", "attestation", "store-genesis", "checkpoint"]
    schemas = {name: validator(f"{name}-0.schema.json") for name in names}
    contents = {"evidence-store/genesis@0": "store-genesis", "evidence-store/checkpoint@0": "checkpoint"}

    def errors(name: str, value: object) -> list[str]:
        return [e.message for e in schemas[name].iter_errors(value)]

    for case_dir in sorted(exports.iterdir()):
        where = f"store/exports/{case_dir.name}"
        case_file, manifest_file = case_dir / "case.json", case_dir / "manifest.json"
        if not (case_file.exists() and manifest_file.exists()):
            failures.append(f"{where}: case.json or manifest.json missing")
            continue
        expect = json.loads(case_file.read_text(encoding="utf-8"))["expect"]
        if errs := errors("store-report", expect):
            failures.append(f"{where}: store-report schema rejects the golden report: {errs[0]}")
        counts["reports"] += 1
        if errs := errors("segments", json.loads(manifest_file.read_bytes())):
            failures.append(f"{where}: segments schema rejects manifest.json: {errs[0]}")
        counts["manifests"] += 1

        # (seq, code, core) of every finding at a seq: a line may fail a schema
        # only where the report has the finding that rejects it.
        found = {(f.get("seq"), f["code"], f.get("core")) for f in expect["findings"]}
        entry_rejected = any(f["code"] == "S_ENTRY_INVALID" for f in expect["findings"])
        for segment in sorted(case_dir.glob("*.jsonl")):
            for raw in segment_lines(segment.read_bytes()):
                try:
                    line = json.loads(raw.decode("utf-8"))
                except (UnicodeDecodeError, ValueError):
                    continue  # not JSON: S_LINE_INVALID, which no schema sees
                if not isinstance(line, dict) or not isinstance(line.get("entry"), dict):
                    continue
                entry = line["entry"]
                seq = entry.get("seq")
                if (errs := errors("log-entry", entry)) and not entry_rejected:
                    failures.append(f"{where}: log-entry schema rejects the entry at seq {seq}: {errs[0]}")
                counts["entries"] += 1
                if isinstance(line.get("object"), dict):
                    obj = line["object"]
                    shape_finding = (seq, "S_OBJECT_INVALID", "E_SHAPE") in found
                    errs = errors("evidence-object", obj)
                    if errs and not shape_finding:
                        failures.append(f"{where}: object schema rejects the object at seq {seq}: {errs[0]}")
                    if shape_finding and not errs:
                        failures.append(f"{where}: object schema accepts the object at seq {seq}, which has E_SHAPE")
                    counts["objects"] += 1
                    kind = contents.get(obj.get("schema", {}).get("ref"))
                    if kind and obj.get("subject", {}).get("system") == "evidence-store":
                        rejecting = ["S_GENESIS_INVALID", "S_CHECKPOINT_INVALID", "S_RESERVED_SUBJECT"]
                        rejected = any((seq, code, None) in found for code in rejecting)
                        if (errs := errors(kind, obj.get("content", {}).get("inline"))) and not rejected:
                            failures.append(f"{where}: {kind} schema rejects the content at seq {seq}: {errs[0]}")
                        counts["genesis contents" if kind == "store-genesis" else "checkpoint contents"] += 1
                if isinstance(line.get("attestation"), dict):
                    # Only one direction: E_ATTESTATION_SHAPE also covers an `at`
                    # that is not a real instant, which the schema cannot see.
                    shape_finding = (seq, "S_ATTESTATION_INVALID", "E_ATTESTATION_SHAPE") in found
                    if (errs := errors("attestation", line["attestation"])) and not shape_finding:
                        failures.append(f"{where}: attestation schema rejects the attestation at seq {seq}: {errs[0]}")
                    counts["attestations"] += 1
    if not (counts["genesis contents"] and counts["checkpoint contents"]):
        failures.append("store/exports: no genesis or no checkpoint content checked")
    return counts


# The `detail` values of the dictionary checks a schema can express.
SCHEMA_DETAILS = {
    "shape",
    "version",
    "profile",
    "family-name",
    "values",
    "empty",
    "too-many",
    "id-form",
    "not-integer",
    "prime-range",
}


def check_dictionaries(failures: list[str]) -> int:
    """Runs the coordinate-dictionary schema over coordinate/dictionaries.json;
    returns the cases checked."""
    root = ROOT / "test-vectors" / "coordinate"
    vectors = root / "dictionaries.json"
    if not vectors.exists():
        failures.append("coordinate/dictionaries.json: missing")
        return 0
    schema = validator("coordinate-dictionary-0.schema.json")
    cases = json.loads(vectors.read_text(encoding="utf-8"))

    def load(kind: str, name: str) -> object:
        return json.loads((root / "dictionaries" / kind / f"{name}.json").read_bytes())

    checked = 0
    for case in cases["valid"]:
        if errors := [e.message for e in schema.iter_errors(load("valid", case["name"]))]:
            failures.append(f"coordinate/dictionaries/valid/{case['name']}: schema rejects it: {errors[0]}")
        checked += 1
    for case in cases["invalid"]:
        error = case["error"]
        if "core" in error or error["detail"] not in SCHEMA_DETAILS:
            continue
        if not list(schema.iter_errors(load("invalid", case["name"]))):
            failures.append(f"coordinate/dictionaries/invalid/{case['name']}: schema accepts it ({error['detail']})")
        checked += 1
    # A newline after any string of catalogue-demo: the code rejects every one.
    base = load("valid", "catalogue-demo")
    variants = [{**base, "v": base["v"] + "\n"}, {**base, "profile": base["profile"] + "\n"}]
    for key, items in (("families", ("name", "values")), ("facets", ("id",))):
        for i, item in enumerate(base[key]):
            for member in items:
                changed = [dict(x) for x in base[key]]
                changed[i][member] += "\n"
                variants.append({**base, key: changed})
    for variant in variants:
        if not list(schema.iter_errors(variant)):
            failures.append("coordinate/dictionaries/valid/catalogue-demo: schema accepts a trailing newline")
    # Rules that the vectors test only together with another: an unknown member
    # at the top level, and 4,097 facets that are each valid. (An unknown member
    # in a family has a vector of its own, family-member-unknown.)
    wide = load("valid", "wide-demo")
    others = [
        ("an unknown member", {**base, "note": "a member the format does not have"}),
        ("4,097 facets", {**wide, "facets": wide["facets"] + [{"id": "flag.p38891", "prime": 38891}]}),
    ]
    for what, variant in others:
        if not list(schema.iter_errors(variant)):
            failures.append(f"coordinate/dictionaries: schema accepts a dictionary with {what}")
    return checked + len(variants) + len(others)


if __name__ == "__main__":
    sys.exit(main())
