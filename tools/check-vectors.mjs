// Re-checks test-vectors/ with the JavaScript reference implementation.
// Object vectors that expect a code evidence.mjs does not implement are skipped
// and listed. Missing store and coordinate vectors are a failure, not a skip.
// Exit status 1 on any disagreement.
//
//   node tools/check-vectors.mjs

import { existsSync, readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import * as ev from "./reference/evidence.mjs";
import * as log from "./reference/log.mjs";
import * as cs from "./reference/coordinate.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..", "test-vectors");
const read = (...p) => readFileSync(join(ROOT, ...p), "utf8");
const failures = [];
const skipped = [];

const outcome = (fn) => {
  try {
    return { ok: true, value: fn() };
  } catch (e) {
    if (!(e instanceof ev.EvidenceError)) throw e;
    return { ok: false, code: e.code };
  }
};

for (const c of JSON.parse(read("canon.json"))) {
  const got = outcome(() => ev.canonicalize(ev.parseStrict(c.input)));
  if (c.error && !ev.SUPPORTED_CODES.has(c.error)) {
    skipped.push(`canon/${c.name} (${c.error})`);
  } else if (c.error ? got.code !== c.error : got.value !== c.expected) {
    failures.push(`canon/${c.name}: got ${JSON.stringify(got)}`);
  }
}

const view = new DataView(new ArrayBuffer(8));
for (const n of JSON.parse(read("numbers.json"))) {
  view.setBigUint64(0, BigInt(`0x${n.bits}`));
  const got = String(view.getFloat64(0));
  if (got !== n.expected) failures.push(`numbers/${n.bits}: got ${got}, want ${n.expected}`);
}

for (const name of readdirSync(join(ROOT, "objects")).sort()) {
  const c = JSON.parse(read("objects", name, "case.json"));
  const attPath = join(ROOT, "objects", name, "attestations.jsonl");
  const attestations = existsSync(attPath) ? readFileSync(attPath, "utf8") : "";
  if (!c.expect.ok && !ev.SUPPORTED_CODES.has(c.expect.code)) {
    skipped.push(`objects/${name} (${c.expect.code})`);
    continue;
  }
  const got = outcome(() =>
    ev.verifyIntegrity(read("objects", name, "object.json"), { attestations, require: c.require ?? [] }),
  );
  if (c.expect.ok) {
    if (!got.ok || got.value.id !== c.expect.id) failures.push(`objects/${name}: got ${JSON.stringify(got)}`);
  } else if (got.code !== c.expect.code) {
    failures.push(`objects/${name}: want ${c.expect.code}, got ${JSON.stringify(got)}`);
  }
}

// The evidence/0 steps 1-5 and attestation/0 checks that log.mjs adds for the
// verifier, against the object vectors: steps 1-5 must give the vector's code
// when it is one of theirs and pass otherwise; attestation lines must fail the
// shape check exactly in the E_ATTESTATION_SHAPE vector, and the approve line
// edited after signing must fail the signature check.
const STEPS_1_TO_5 = new Set([
  "E_PARSE",
  "E_DUPLICATE_KEY",
  "E_NUMBER_RANGE",
  "E_SHAPE",
  "E_TIMESTAMP",
  "E_ID_MISMATCH",
  "E_CONTENT_HASH_MISMATCH",
]);
for (const name of readdirSync(join(ROOT, "objects")).sort()) {
  const c = JSON.parse(read("objects", name, "case.json"));
  const want = !c.expect.ok && STEPS_1_TO_5.has(c.expect.code) ? c.expect.code : undefined;
  const got = outcome(() => log.checkObject(readFileSync(join(ROOT, "objects", name, "object.json"))));
  if (got.code !== want) failures.push(`objects/${name} steps 1-5: want ${want ?? "pass"}, got ${got.code ?? "pass"}`);
  const attPath = join(ROOT, "objects", name, "attestations.jsonl");
  if (!existsSync(attPath)) continue;
  const atts = readFileSync(attPath, "utf8").split("\n").filter(Boolean).map((l) => JSON.parse(l));
  const shapes = atts.map(log.attestationShapeOk);
  if (shapes.every(Boolean) !== (c.expect.code !== "E_ATTESTATION_SHAPE")) {
    failures.push(`objects/${name}: attestation shapes ${JSON.stringify(shapes)}`);
  }
  const signatures = atts.filter(log.attestationShapeOk).map(log.attestationSignatureOk);
  if (signatures.every(Boolean) !== (c.expect.code !== "E_SIGNATURE_INVALID")) {
    failures.push(`objects/${name}: attestation signatures ${JSON.stringify(signatures)}`);
  }
}

// --- Store vectors (store/0), checked with log.mjs ------------------------------
// Every store file must be present: a missing one is a failure, not a skip.

const required = (...p) => {
  const ok = existsSync(join(ROOT, ...p));
  if (!ok) failures.push(`${p.join("/")}: missing`);
  return ok;
};

if (required("store", "entries.json")) {
  const entries = JSON.parse(read("store", "entries.json"));
  for (const c of entries.valid) {
    const got = outcome(() => {
      const fromObject = log.validateEntry(c.entry);
      const fromText = log.parseEntry(c.canonical);
      return { canonical: log.entryCanonical(fromObject), hash: log.entryHash(fromText) };
    });
    if (!got.ok || got.value.canonical !== c.canonical || got.value.hash !== c.hash) {
      failures.push(`store/entries/${c.name}: got ${JSON.stringify(got)}`);
    }
  }
  for (const c of entries.invalid) {
    const got = outcome(() => log.parseEntry(c.input));
    if (got.code !== c.error) failures.push(`store/entries/${c.name}: want ${c.error}, got ${JSON.stringify(got)}`);
  }
}

if (required("store", "merkle.json")) {
  const digests = (texts) => texts.map((t) => log.digestFromText(t));
  // Producing a proof for an index or size outside the tree throws; that is a
  // disagreement with a vector that marks the proof valid, not a crash.
  const attempt = (fn) => {
    try {
      return fn();
    } catch (e) {
      if (!(e instanceof RangeError)) throw e;
      return null;
    }
  };
  for (const c of JSON.parse(read("store", "merkle.json"))) {
    const hashes = c.leaves.map((s) => log.leafHash(Buffer.from(s, "utf8")));
    const root = log.merkleRoot(hashes);
    const where = `store/merkle/${c.name}`;
    if (log.digestToText(root) !== c.root) failures.push(`${where}: root ${log.digestToText(root)}`);
    for (const p of c.inclusion) {
      // For an index outside the tree there is no leaf; any leaf must give false.
      const leaf = log.leafHash(Buffer.from(c.leaves[p.index] ?? `leaf-${p.index}`, "utf8"));
      if (p.valid) {
        const made = attempt(() => log.inclusionProof(hashes, p.index).map(log.digestToText));
        if (JSON.stringify(made) !== JSON.stringify(p.proof)) {
          failures.push(`${where}: inclusion proof for ${p.index}`);
        }
      }
      const got = log.verifyInclusion(leaf, p.index, c.leaves.length, digests(p.proof), log.digestFromText(c.root));
      if (got !== p.valid) failures.push(`${where}: inclusion ${p.index} verified ${got}, want ${p.valid}`);
    }
    for (const p of c.consistency) {
      if (p.valid) {
        const made = attempt(() => log.consistencyProof(hashes, p.old_size).map(log.digestToText));
        const oldRoot = log.digestToText(log.merkleRoot(hashes.slice(0, p.old_size)));
        if (JSON.stringify(made) !== JSON.stringify(p.proof) || oldRoot !== p.old_root) {
          failures.push(`${where}: consistency proof from ${p.old_size}`);
        }
      }
      const got = log.verifyConsistency(
        p.old_size,
        c.leaves.length,
        log.digestFromText(p.old_root),
        log.digestFromText(c.root),
        digests(p.proof),
      );
      if (got !== p.valid) failures.push(`${where}: consistency from ${p.old_size} verified ${got}, want ${p.valid}`);
    }
  }
}

// Export cases: every report recomputed with the JS verifier and compared in
// canonical form; every valid case replayed from inputs.json through the JS
// writer, byte for byte; and the proofs of case 003 verified and reproduced.
const EXPORTS = join(ROOT, "store", "exports");
const CASE_MEMBERS = ["description", "expect", "max_findings", "proofs"];
const SEGMENT_FILE = /^([0-9]{6}|[1-9][0-9]{6,})\.jsonl$/;
let exportCases = 0;
if (required("store", "exports")) {
  for (const name of readdirSync(EXPORTS).sort()) {
    const where = `store/exports/${name}`;
    const dir = join(EXPORTS, name);
    const files = readdirSync(dir).sort();
    if (!files.includes("case.json") || !files.includes("manifest.json")) {
      failures.push(`${where}: case.json or manifest.json missing`);
      continue;
    }
    exportCases += 1;
    const c = JSON.parse(readFileSync(join(dir, "case.json"), "utf8"));
    const extra = Object.keys(c).filter((k) => !CASE_MEMBERS.includes(k));
    if (extra.length || typeof c.description !== "string" || typeof c.expect !== "object") {
      failures.push(`${where}: case.json members ${Object.keys(c).join(", ")}`);
      continue;
    }
    const manifest = readFileSync(join(dir, "manifest.json"));
    const segmentNames = files.filter((f) => SEGMENT_FILE.test(f));
    const segments = segmentNames.map((f) => readFileSync(join(dir, f)));
    const got = outcome(() => log.verifyExport(manifest, segments, { maxFindings: c.max_findings ?? 1000 }));
    if (!got.ok || ev.canonicalize(got.value) !== ev.canonicalize(c.expect)) {
      failures.push(`${where}: report ${JSON.stringify(got.value ?? got)}`);
    }

    const valid = name.startsWith("0");
    if (valid !== (c.expect.ok === true)) {
      failures.push(`${where}: a ${valid ? "0xx" : "1xx"} case with ok ${c.expect.ok}`);
    }
    if (valid !== files.includes("inputs.json")) failures.push(`${where}: inputs.json is for valid cases only`);
    if (valid && files.includes("inputs.json")) {
      const inputs = JSON.parse(readFileSync(join(dir, "inputs.json"), "utf8"));
      const made = outcome(() =>
        log.replay(inputs, (rel) => readFileSync(join(ROOT, rel))).exportFiles(inputs.segment_size),
      );
      const exported = files.filter((f) => f === "manifest.json" || SEGMENT_FILE.test(f));
      if (!made.ok) {
        failures.push(`${where}: replay of inputs.json failed: ${JSON.stringify(made)}`);
      } else if (
        JSON.stringify([...made.value.keys()].sort()) !== JSON.stringify(exported) ||
        exported.some((f) => !made.value.get(f).equals(readFileSync(join(dir, f))))
      ) {
        failures.push(`${where}: replay of inputs.json does not reproduce the export`);
      }
    }

    if ((c.proofs !== undefined) !== (name === "003-attestations-two-checkpoints")) {
      failures.push(`${where}: proofs are in case 003 only`);
    }
    if (c.proofs !== undefined) checkProofs(where, c.proofs, segments, JSON.parse(manifest.toString("utf8")));
  }
  if (exportCases === 0) failures.push("store/exports: no cases");
}

// The proofs of a case: each verifies, and each is what the export's lines give.
function checkProofs(where, proofs, segments, manifest) {
  const d = log.digestFromText;
  const leaves = segments
    .flatMap((s) => log.splitSegment(s).lines)
    .map((l) => log.leafHash(Buffer.from(log.parseLine(l).entryCanonical, "utf8")));
  const texts = (proof) => proof.map(log.digestToText);
  const root = (size) => log.digestToText(log.merkleRoot(leaves.slice(0, size)));
  if (!Array.isArray(proofs.inclusion) || !Array.isArray(proofs.consistency)) {
    failures.push(`${where}: proofs are {inclusion, consistency}`);
    return;
  }
  const latest = manifest.checkpoint.size;
  const seqs = proofs.inclusion.map((p) => p.seq);
  if (JSON.stringify(seqs) !== JSON.stringify(Array.from({ length: latest }, (_, i) => i + 1))) {
    failures.push(`${where}: inclusion proofs are not for seq 1 to ${latest}`);
  }
  for (const p of proofs.inclusion) {
    const ok = log.verifyInclusion(d(p.leaf_hash), p.seq - 1, p.size, p.proof.map(d), d(p.root));
    const tree = leaves.slice(0, p.size);
    const same =
      p.size === latest &&
      p.leaf_hash === log.digestToText(leaves[p.seq - 1]) &&
      p.root === root(p.size) &&
      JSON.stringify(texts(log.inclusionProof(tree, p.seq - 1))) === JSON.stringify(p.proof);
    if (!ok || !same) failures.push(`${where}: inclusion proof for seq ${p.seq} (verified ${ok}, reproduced ${same})`);
  }
  if (proofs.consistency.length !== 1) failures.push(`${where}: one consistency proof`);
  for (const p of proofs.consistency) {
    const ok = log.verifyConsistency(p.old_size, p.new_size, d(p.old_root), d(p.new_root), p.proof.map(d));
    const same =
      p.new_size === latest &&
      p.old_root === root(p.old_size) &&
      p.new_root === root(p.new_size) &&
      JSON.stringify(texts(log.consistencyProof(leaves.slice(0, p.new_size), p.old_size))) === JSON.stringify(p.proof);
    if (!ok || !same) {
      failures.push(`${where}: consistency proof ${p.old_size}->${p.new_size} (verified ${ok}, reproduced ${same})`);
    }
  }
}

// Nesting: an item at the evidence/0 limit fits in a line; one level more, or
// hostile depth, is rejected with an S_ code rather than a stack overflow. The
// Rust tests in crates/evidence-log/src/line.rs pin the same boundary.
{
  const h = (c) => `sha256:${c.repeat(64)}`;
  const entry = (kind) =>
    log.entryCanonical(
      log.makeEntry({ log: h("a"), seq: 4, kind, ref: h("b"), received_at: "2026-11-26T00:00:01.000Z", epoch: 2, prev: h("c") }),
    );
  const nested = (d) => "[".repeat(d) + "]".repeat(d);
  const expected = [
    [log.MAX_DEPTH, undefined],
    [log.MAX_DEPTH + 1, "S_LINE_INVALID"],
    [1e6, "S_LINE_INVALID"],
  ];
  for (const kind of log.KINDS) {
    for (const [depth, want] of expected) {
      const got = outcome(() => log.parseLine(Buffer.from(log.renderLine(entry(kind), kind, nested(depth)))));
      if (got.code !== want) failures.push(`store/nesting/${kind} item ${depth} deep: got ${got.code ?? "ok"}`);
    }
  }
  const got = outcome(() => log.parseEntry("[".repeat(1e6)));
  if (got.code !== "S_ENTRY_INVALID") failures.push(`store/nesting/entry 1e6 deep: got ${JSON.stringify(got.code ?? "ok")}`);
}

// The verifier keeps only the smallest findings as it reads. With many
// findings (case 125, then its lines 3 and 7 repeated 150 times, each repeat a
// gap, a chain break and a duplicate, and those at seq 3 sorting before
// findings read earlier), a small max_findings gives the start of the full
// report, the E_PREV_MISSING decided at the end of the log included. The Rust
// test a_small_limit_survives_many_findings pins the same rule.
if (exportCases) {
  const dir = join(EXPORTS, "125-invalid-object-not-held");
  const manifest = readFileSync(join(dir, "manifest.json"));
  const segment = readFileSync(join(dir, "000001.jsonl"));
  const { lines } = log.splitSegment(segment);
  const repeat = Buffer.concat([lines[2], lines.at(-1)].map((l) => Buffer.concat([l, Buffer.from("\n")])));
  const long = Buffer.concat([segment, ...Array(150).fill(repeat)]);
  const full = log.verifyExport(manifest, [long], { maxFindings: 1e6 });
  if (full.truncated || full.findings.length < 900) failures.push(`store/pruning: full report has ${full.findings.length}`);
  for (const max of [1, 2, 3, 63, 64, 65, 500]) {
    const r = log.verifyExport(manifest, [long], { maxFindings: max });
    const want = { ...full, ok: false, findings: full.findings.slice(0, max), truncated: true };
    if (ev.canonicalize(r) !== ev.canonicalize(want)) failures.push(`store/pruning: max_findings ${max} disagrees`);
  }
}

// The verifier's incremental tree gives the tree hash of every prefix.
{
  const tree = new log.MerkleTree();
  const leaves = [];
  for (let i = 0; i < 100; i++) {
    leaves.push(log.leafHash(Buffer.from(`leaf-${i}`)));
    tree.push(leaves[i]);
  }
  for (let n = 1; n <= leaves.length; n++) {
    if (!tree.root(n).equals(log.merkleRoot(leaves.slice(0, n)))) failures.push(`store/tree: prefix ${n}`);
  }
  if (tree.root(0) !== null || tree.root(101) !== null) failures.push("store/tree: a prefix outside the tree");
}

// --- Coordinate vectors (coordinate/0), checked with coordinate.mjs ----------------
// Every file must be present. A case passes when the canonical form of the
// result, or of the error, equals that of `expect` or `error`.

const COORDINATE_DIR = join(ROOT, "coordinate");
const coordinateCounts = {};
const coordinateOutcome = (fn) => {
  try {
    return { expect: fn() };
  } catch (e) {
    if (!(e instanceof cs.CoordinateError)) throw e;
    return { error: e.error };
  }
};
const sameOutcome = (got, c) =>
  "expect" in c ? "expect" in got && ev.canonicalize(got.expect) === ev.canonicalize(c.expect)
    : "error" in got && ev.canonicalize(got.error) === ev.canonicalize(c.error);
const dictionaries = {};
const COORDINATE_FILES = ["dictionaries.json", "encode.json", "decode.json", "divides.json", "check.json", "extends.json"];
if (COORDINATE_FILES.map((f) => required("coordinate", f)).every(Boolean)) {
  const readCases = (file) => JSON.parse(read("coordinate", file));
  const dicts = readCases("dictionaries.json");
  for (const [kind, member] of [
    ["valid", "id"],
    ["invalid", "error"],
  ]) {
    const files = readdirSync(join(COORDINATE_DIR, "dictionaries", kind)).sort();
    const names = dicts[kind].map((c) => `${c.name}.json`).sort();
    if (JSON.stringify(files) !== JSON.stringify(names)) failures.push(`coordinate/dictionaries/${kind}: files and cases differ`);
    for (const c of dicts[kind]) {
      const where = `coordinate/dictionaries/${kind}/${c.name}`;
      if (JSON.stringify(Object.keys(c)) !== JSON.stringify(["name", member])) failures.push(`${where}: members`);
      const path = join(COORDINATE_DIR, "dictionaries", kind, `${c.name}.json`);
      if (!existsSync(path)) continue;
      const got = coordinateOutcome(() => cs.parseDictionary(readFileSync(path)));
      if (kind === "valid") {
        if (!got.expect || got.expect.id !== c.id) failures.push(`${where}: got ${JSON.stringify(got.error ?? got.expect.id)}`);
        else dictionaries[c.name] = got.expect;
      } else if (!sameOutcome(got, { error: c.error })) {
        failures.push(`${where}: want ${JSON.stringify(c.error)}, got ${JSON.stringify(got.error ?? "a dictionary")}`);
      }
    }
  }
  coordinateCounts.dictionaries = dicts.valid.length + dicts.invalid.length;

  // The members of each file's cases, before `expect` or `error`.
  const operations = {
    "encode.json": [["dictionary", "facets"], (d, c) => cs.encode(d[c.dictionary], c.facets)],
    "decode.json": [["dictionary", "coordinate"], (d, c) => cs.decode(d[c.dictionary], c.coordinate)],
    "divides.json": [["dictionary", "query", "coordinate"], (d, c) => cs.divides(d[c.dictionary], c.query, c.coordinate)],
    "check.json": [["dictionary", "facets", "coordinate"], (d, c) => cs.check(d[c.dictionary], c.facets, c.coordinate)],
    "extends.json": [["old", "new"], (d, c) => cs.extends(d[c.old], d[c.new])],
  };
  for (const [file, [members, run]] of Object.entries(operations)) {
    const cases = readCases(file);
    const names = new Set();
    for (const c of cases) {
      const where = `coordinate/${file.replace(".json", "")}/${c.name}`;
      const outcomeMember = "expect" in c ? "expect" : "error";
      if (JSON.stringify(Object.keys(c)) !== JSON.stringify(["name", ...members, outcomeMember]) || names.has(c.name)) {
        failures.push(`${where}: members ${Object.keys(c).join(", ")}, or a name used twice`);
        continue;
      }
      names.add(c.name);
      const unknown = ["dictionary", "old", "new"].filter((k) => k in c && !(c[k] in dictionaries));
      if (unknown.length) {
        failures.push(`${where}: ${unknown.join(", ")} is not a valid dictionary`);
        continue;
      }
      const got = coordinateOutcome(() => run(dictionaries, c));
      if (!sameOutcome(got, c)) failures.push(`${where}: got ${JSON.stringify(got)}`);
    }
    coordinateCounts[file.replace(".json", "")] = cases.length;
  }

  // The checks the spec asks of the generator, made again: decode results
  // encode back, encode results decode back to their facets, and divides
  // agrees with exact divisibility of the two integers.
  const sorted = (ids) => JSON.stringify([...ids].sort());
  for (const c of readCases("decode.json").filter((c) => "expect" in c && c.dictionary in dictionaries)) {
    const back = coordinateOutcome(() => cs.encode(dictionaries[c.dictionary], cs.expand(c.expect)));
    if (back.expect !== c.coordinate) failures.push(`coordinate/decode/${c.name}: does not encode back`);
  }
  for (const c of readCases("encode.json").filter((c) => "expect" in c && c.dictionary in dictionaries)) {
    const back = coordinateOutcome(() => cs.expand(cs.decode(dictionaries[c.dictionary], c.expect)));
    if (!back.expect || sorted(back.expect) !== sorted(c.facets)) failures.push(`coordinate/encode/${c.name}: does not decode back`);
  }
  for (const c of readCases("divides.json").filter((c) => "expect" in c)) {
    if ((BigInt(c.coordinate) % BigInt(c.query) === 0n) !== c.expect) {
      failures.push(`coordinate/divides/${c.name}: disagrees with exact divisibility`);
    }
  }
}
// An invalid argument is E_USAGE, never a coordinate/0 result.
if (dictionaries["catalogue-demo"]) {
  const d = dictionaries["catalogue-demo"];
  const usage = [
    () => cs.encode(d, "jurisdiction.vic"),
    () => cs.encode(d, [11]),
    () => cs.decode(d.value, "1"), // a dictionary value, not a loaded dictionary
    () => cs.parseDictionary(null),
  ];
  for (const [i, fn] of usage.entries()) {
    const got = outcome(fn);
    if (got.code !== "E_USAGE") failures.push(`coordinate/usage ${i}: got ${JSON.stringify(got)}`);
  }

  // A value that is already parsed starts at step 2. Values built in code can
  // hold what JSON cannot: a NaN or an infinity fails step 8d, a number above
  // 2^53 - 1 step 8e, and a hole in an array step 2.
  const parsed = cs.validateDictionary(JSON.parse(readFileSync(join(COORDINATE_DIR, "dictionaries", "valid", "catalogue-demo.json"), "utf8")));
  if (parsed.id !== d.id) failures.push(`coordinate/validateDictionary: catalogue-demo has id ${parsed.id}`);
  const oneFacet = (prime) => ({
    v: cs.DICTIONARY_V,
    profile: "m-demo/0",
    families: [{ name: "m", values: "one" }],
    facets: [{ id: "m.a", prime }],
  });
  const withHole = (array, i) => {
    delete array[i];
    return array;
  };
  const built = [
    ["NaN", oneFacet(NaN), "not-integer"],
    ["+infinity", oneFacet(Infinity), "not-integer"],
    ["-infinity", oneFacet(-Infinity), "not-integer"],
    ["-0", oneFacet(-0), "prime-range"],
    ["2.5", oneFacet(2.5), "not-integer"],
    ["2^53", oneFacet(2 ** 53), "prime-range"],
    ["1e300", oneFacet(1e300), "prime-range"],
    ["a hole in facets", { ...oneFacet(2), facets: new Array(1) }, "shape"],
    ["a hole between facets", { ...oneFacet(2), facets: withHole([{ id: "m.a", prime: 2 }, null, { id: "m.b", prime: 5 }], 1) }, "shape"],
    ["a hole in families", { ...oneFacet(2), families: withHole([null, { name: "m", values: "one" }], 0) }, "shape"],
    ["5,000 holes", { ...oneFacet(2), facets: new Array(5000) }, "shape"],
  ];
  for (const [what, value, detail] of built) {
    let got;
    try {
      got = coordinateOutcome(() => cs.validateDictionary(value));
    } catch (e) {
      got = { thrown: `${e.name}: ${e.message}` }; // never a coordinate/0 result
    }
    if (!got.error || got.error.code !== cs.CODES.DICTIONARY || got.error.detail !== detail) {
      failures.push(`coordinate/validateDictionary ${what}: want ${detail}, got ${JSON.stringify(got.error ?? got.thrown ?? "a dictionary")}`);
    }
  }
  if (!coordinateOutcome(() => cs.validateDictionary(oneFacet(2))).expect) failures.push("coordinate/validateDictionary: prime 2 is rejected");
}

console.log(`skipped by evidence.mjs (not implemented there): ${skipped.join(", ") || "none"}`);
console.log("checked evidence/0 steps 1-5 and attestation/0 shapes of every object vector with log.mjs");
console.log(`checked ${exportCases} store export cases`);
console.log(
  `checked coordinate/0: ${Object.entries(coordinateCounts)
    .map(([k, n]) => `${n} ${k}`)
    .join(", ")}`,
);
if (failures.length) {
  console.error(`${failures.length} disagreement(s):\n  ${failures.join("\n  ")}`);
  process.exit(1);
}
console.log("JS reference agrees with every vector it implements.");
