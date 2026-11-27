// Runs every test vector through the WebAssembly module, with no skips:
// unlike the JS reference, the module is the Rust core itself. That covers
// the evidence/0 vectors and the store vectors (entries, Merkle trees, every
// export case and the proofs of case 003).
//
//   sh tools/build-wasm.sh && node crates/evidence-wasm/js/test.mjs

import { createHash } from "node:crypto";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { EvidenceError, Evidence, load } from "./evidence.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const vectors = join(here, "..", "..", "..", "test-vectors");
const wasm = process.argv[2] ?? join(here, "..", "pkg", "evidence_wasm.wasm");
const module = await WebAssembly.compile(readFileSync(wasm));
const ev = await load(module);
const failures = [];
let count = 0;

const fail = (name, msg) => failures.push(`${name}: ${msg}`);
const errorOf = (fn) => {
  try {
    fn();
    return null;
  } catch (e) {
    if (!(e instanceof EvidenceError)) throw e;
    return e;
  }
};
const codeOf = (fn) => errorOf(fn)?.code ?? null;

// The module runs in any host: it imports nothing.
{
  count++;
  const imports = WebAssembly.Module.imports(module);
  if (imports.length) fail("module/imports", JSON.stringify(imports));
}

// Canonicalisation, passing inputs as strings.
for (const c of JSON.parse(readFileSync(join(vectors, "canon.json"), "utf8"))) {
  count++;
  if (c.error) {
    const got = codeOf(() => ev.canonicalize(c.input));
    if (got !== c.error) fail(`canon/${c.name}`, `got ${got}, want ${c.error}`);
  } else {
    const got = ev.canonicalize(c.input);
    if (got !== c.expected) fail(`canon/${c.name}`, `got ${got}`);
  }
}

// Objects, passing files as exact bytes.
for (const name of readdirSync(join(vectors, "objects")).sort()) {
  count++;
  const dir = join(vectors, "objects", name);
  const c = JSON.parse(readFileSync(join(dir, "case.json"), "utf8"));
  const attPath = join(dir, "attestations.jsonl");
  const store = c.store
    ? readdirSync(join(dir, "store")).sort().map((f) => readFileSync(join(dir, "store", f)))
    : undefined;
  const report = ev.verify(readFileSync(join(dir, "object.json")), {
    attestations: existsSync(attPath) ? readFileSync(attPath) : undefined,
    store,
    require: c.require ?? [],
  });
  const want = c.expect;
  const problems = [];
  if (report.ok !== want.ok) problems.push(`ok=${report.ok} (${report.code})`);
  if (want.code && report.code !== want.code) problems.push(`code ${report.code}, want ${want.code}`);
  if (want.id && report.id !== want.id) problems.push(`id ${report.id}`);
  if (want.content_checked !== undefined && report.content_checked !== want.content_checked) {
    problems.push(`content_checked ${report.content_checked}`);
  }
  for (const [purpose, n] of Object.entries(want.attestations ?? {})) {
    if ((report.attestations[purpose] ?? 0) !== n) problems.push(`${purpose} attestations`);
  }
  if (problems.length) fail(`objects/${name}`, problems.join("; "));
}

// Signatures are deterministic: the module must reproduce the JS reference's
// approval line in vector 003 byte for byte.
{
  count++;
  const dir = join(vectors, "objects", "003-agent-assertion-approved");
  const approve = readFileSync(join(dir, "attestations.jsonl"), "utf8")
    .split("\n")
    .find((l) => l.includes('"purpose":"approve"'));
  const seedHex = readFileSync(join(vectors, "keys", "assessor-01.seed"), "utf8").trim();
  const line = ev.sign(readFileSync(join(dir, "object.json")), {
    seedHex,
    signer: "person:assessor-01",
    purpose: "approve",
    at: JSON.parse(approve).at,
  });
  if (line !== approve) fail("sign/deterministic", `got ${line}`);
}

// Round trip with a fresh key, and usage errors.
{
  count++;
  const object = readFileSync(join(vectors, "objects", "001-create-inline", "object.json"));
  const seedHex = Evidence.generateSeed();
  const line = ev.sign(object, { seedHex, signer: "person:test", purpose: "witness" });
  const report = ev.verify(object, { attestations: `${line}\n`, require: ["witness"] });
  if (!report.ok || JSON.parse(line).key !== ev.publicKey(seedHex)) fail("sign/round-trip", JSON.stringify(report));
  if (codeOf(() => ev.verify(object, { require: ["bless"] })) !== "E_USAGE") fail("usage/require", "no E_USAGE");
  if (codeOf(() => ev.publicKey("zz")) !== "E_USAGE") fail("usage/seed", "no E_USAGE");
  if (ev.version().spec !== "evidence/0") fail("version", JSON.stringify(ev.version()));
}

// Store vectors (test-vectors/store; spec/store-0.md, "Conformance").
const store = join(vectors, "store");
const sha256 = (...parts) => `sha256:${parts.reduce((h, p) => h.update(p), createHash("sha256")).digest("hex")}`;
const leafHash = (bytes) => sha256(Buffer.from([0]), bytes);
const canonical = (value) => ev.canonicalize(JSON.stringify(value));

// Entries: the hash of every valid entry, from its canonical form, from other
// JSON text and from bytes; every invalid one is E_USAGE naming S_ENTRY_INVALID.
{
  const entries = JSON.parse(readFileSync(join(store, "entries.json"), "utf8"));
  if (!entries.valid.length || !entries.invalid.length) fail("store/entries", "no cases");
  for (const c of entries.valid) {
    count++;
    const got = [ev.entryHash(c.canonical), ev.entryHash(JSON.stringify(c.entry, null, 2)), ev.entryHash(Buffer.from(c.canonical))];
    if (got.some((h) => h !== c.hash)) fail(`store/entries/${c.name}`, `got ${got.join(", ")}, want ${c.hash}`);
    if (canonical(c.entry) !== c.canonical) fail(`store/entries/${c.name}`, "canonical form differs");
  }
  for (const c of entries.invalid) {
    count++;
    const e = errorOf(() => ev.entryHash(c.input));
    if (e?.code !== "E_USAGE" || !e.detail.startsWith(c.error)) {
      fail(`store/entries/${c.name}`, `got ${e ? `${e.code} ${e.detail}` : "a hash"}, want E_USAGE ${c.error}`);
    }
  }
}

// Merkle trees: each root from strings and from bytes, and every proof to its
// `valid` value. Leaf bytes are the UTF-8 of each string.
{
  const trees = JSON.parse(readFileSync(join(store, "merkle.json"), "utf8"));
  if (!trees.length) fail("store/merkle", "no cases");
  for (const t of trees) {
    count++;
    const where = `store/merkle/${t.name}`;
    const n = t.leaves.length;
    const roots = [ev.merkleRoot(t.leaves), ev.merkleRoot(t.leaves.map((l) => Buffer.from(l)))];
    if (roots.some((r) => r !== t.root)) fail(where, `root ${roots.join(", ")}`);
    for (const p of t.inclusion) {
      const leaf = p.index < n ? leafHash(Buffer.from(t.leaves[p.index])) : leafHash(Buffer.alloc(0));
      const got = ev.verifyInclusion(leaf, p.index, n, p.proof, t.root);
      if (got !== p.valid) fail(where, `inclusion of ${p.index} verified ${got}, want ${p.valid}`);
    }
    for (const p of t.consistency) {
      const got = ev.verifyConsistency(p.old_size, n, p.old_root, t.root, p.proof);
      if (got !== p.valid) fail(where, `consistency from ${p.old_size} verified ${got}, want ${p.valid}`);
    }
  }
  count++;
  const empty = sha256(Buffer.alloc(0));
  if (ev.merkleRoot([]) !== empty) fail("store/merkle/empty", ev.merkleRoot([]));
}

// Export cases: the report's canonical form must equal the canonical form of
// `expect`, byte for byte. Segment files go in file-name order, as raw bytes.
const SEGMENT_FILE = /^([0-9]{6}|[1-9][0-9]{6,})\.jsonl$/;
const exportsDir = join(store, "exports");
const cases = readdirSync(exportsDir).sort();
if (!cases.length) fail("store/exports", "no cases");
for (const name of cases) {
  count++;
  const dir = join(exportsDir, name);
  const c = JSON.parse(readFileSync(join(dir, "case.json"), "utf8"));
  const manifest = readFileSync(join(dir, "manifest.json"));
  const segments = readdirSync(dir)
    .filter((f) => SEGMENT_FILE.test(f))
    .sort()
    .map((f) => readFileSync(join(dir, f)));
  const report = ev.verifyExport(manifest, segments, { maxFindings: c.max_findings });
  if (canonical(report) !== canonical(c.expect)) {
    fail(`store/exports/${name}`, `report ${JSON.stringify(report)}`);
  }
  if (c.proofs) checkProofs(`store/exports/${name}`, c.proofs, segments);
}

// The proofs of a case: each verifies, and its leaf hashes and roots are the
// ones the export's lines give.
function checkProofs(where, proofs, segments) {
  const entries = segments
    .flatMap((s) => s.toString("utf8").split("\n").slice(0, -1))
    .map((line) => Buffer.from(canonical(JSON.parse(line).entry)));
  const root = (size) => ev.merkleRoot(entries.slice(0, size));
  for (const p of proofs.inclusion) {
    count++;
    const ok = ev.verifyInclusion(p.leaf_hash, p.seq - 1, p.size, p.proof, p.root);
    const same = p.leaf_hash === leafHash(entries[p.seq - 1]) && p.root === root(p.size);
    if (!ok || !same) fail(where, `inclusion proof for seq ${p.seq} (verified ${ok}, matches the lines ${same})`);
  }
  for (const p of proofs.consistency) {
    count++;
    const ok = ev.verifyConsistency(p.old_size, p.new_size, p.old_root, p.new_root, p.proof);
    const same = p.old_root === root(p.old_size) && p.new_root === root(p.new_size);
    if (!ok || !same) fail(where, `consistency proof ${p.old_size}->${p.new_size} (verified ${ok}, matches the lines ${same})`);
  }
  if (!proofs.inclusion.length || proofs.consistency.length !== 1) fail(where, "proofs missing");
}

// Store functions: the string and byte forms agree, and bad arguments are
// E_USAGE while verification outcomes never are.
{
  count++;
  const dir = join(exportsDir, "002-records-and-checkpoint");
  const manifest = readFileSync(join(dir, "manifest.json"));
  const segment = readFileSync(join(dir, "000001.jsonl"));
  const fromBytes = canonical(ev.verifyExport(manifest, [segment]));
  const fromText = canonical(ev.verifyExport(manifest.toString("utf8"), [segment.toString("utf8")]));
  if (fromBytes !== fromText) fail("store/forms", "string and byte inputs disagree");
  const damaged = ev.verifyExport("", [new Uint8Array([0xff, 0x0a])], { maxFindings: 1 });
  if (damaged.ok || !damaged.truncated || damaged.findings[0].code !== "S_MANIFEST_INVALID") {
    fail("store/damaged", JSON.stringify(damaged));
  }

  const h = (c) => `sha256:${c.repeat(64)}`;
  const one = leafHash(Buffer.from("leaf-0"));
  if (!ev.verifyInclusion(one, 0, 1, [], one)) fail("store/inclusion-one", "false");
  if (ev.verifyInclusion(one, 2 ** 53 - 2, 2 ** 53 - 1, [], one)) fail("store/inclusion-max", "true");
  const usage = {
    "entry_hash without input": () => ev.entryHash(undefined),
    "merkle_root without leaves": () => ev.merkleRoot(undefined),
    "index -1": () => ev.verifyInclusion(one, -1, 1, [], one),
    "index 0.5": () => ev.verifyInclusion(one, 0.5, 1, [], one),
    "size 2^53": () => ev.verifyInclusion(one, 0, 2 ** 53, [], one),
    "size as text": () => ev.verifyInclusion(one, 0, "1", [], one),
    "size true": () => ev.verifyInclusion(one, 0, true, [], one),
    "uppercase hash": () => ev.verifyInclusion(one.toUpperCase().replace("SHA256", "sha256"), 0, 1, [], one),
    "base64 hash": () => ev.verifyInclusion(Buffer.alloc(32).toString("base64"), 0, 1, [], one),
    "proof not a list": () => ev.verifyConsistency(1, 2, one, h("b"), h("c")),
    "short proof element": () => ev.verifyConsistency(1, 2, one, h("b"), ["sha256:00"]),
    "missing new_root": () => ev.verifyConsistency(1, 2, one, undefined, [h("c")]),
    "maxFindings 0": () => ev.verifyExport(manifest, [segment], { maxFindings: 0 }),
    "maxFindings 2^53": () => ev.verifyExport(manifest, [segment], { maxFindings: 2 ** 53 }),
    "segments missing": () => ev.verifyExport(manifest, undefined),
    "manifest missing": () => ev.verifyExport(undefined, [segment]),
    // A list argument is an array of strings or bytes: never split a string or
    // a byte array into items, and never turn a lone surrogate into U+FFFD.
    "segments as one string": () => ev.verifyExport(manifest, segment.toString("utf8")),
    "segments as one byte array": () => ev.verifyExport(manifest, segment),
    "segments as a Set": () => ev.verifyExport(manifest, new Set([segment])),
    "segment a number": () => ev.verifyExport(manifest, [segment, 42]),
    "leaves as one string": () => ev.merkleRoot("ab"),
    "leaf a lone surrogate": () => ev.merkleRoot(["\ud800"]),
    "leaf a lone surrogate among bytes": () => ev.merkleRoot([new Uint8Array(0), "\ud800"]),
    "store as one string": () => ev.verify(segment, { store: "{}" }),
  };
  for (const [what, fn] of Object.entries(usage)) {
    if (codeOf(fn) !== "E_USAGE") fail(`store/usage/${what}`, "no E_USAGE");
  }
  // Mixed strings and bytes: a string goes as its UTF-8 bytes, pairs included.
  const mixed = ev.merkleRoot([Buffer.from("leaf-0"), "\u{1F600}"]);
  if (mixed !== ev.merkleRoot(["leaf-0", "\u{1F600}"])) fail("store/mixed-leaves", mixed);
}

// The browser path of the wrapper, run here with Buffer hidden, so bytes go to
// the module as base64 made by btoa. Chromium's btoa returns "" instead of
// throwing once its result would pass the longest string. The wrapper must
// give the same results over several chunks, never pass on a short encoding,
// and let an encoding error keep its own type and message.
{
  const dir = join(exportsDir, "004-multi-segment");
  const c = JSON.parse(readFileSync(join(dir, "case.json"), "utf8"));
  const manifest = readFileSync(join(dir, "manifest.json"));
  const manifestText = manifest.toString("utf8");
  const segments = ["000001.jsonl", "000002.jsonl", "000003.jsonl", "000004.jsonl"].map((f) =>
    readFileSync(join(dir, f)),
  );
  const want = canonical(c.expect);
  const pattern = (n) => Uint8Array.from({ length: n }, (_, i) => (i * 7 + (i >> 8)) & 0xff);
  // Lengths around the 3 * 8192 bytes of one btoa call.
  const lengths = [0, 1, 2, 3, 24575, 24576, 24577, 49153, 100000];
  const hashes = lengths.map((n) => ev.fileHash(pattern(n)));
  const bigRoot = ev.merkleRoot([pattern(100000)]);
  const thrown = (fn) => {
    try {
      fn();
      return null;
    } catch (e) {
      return e;
    }
  };
  const saved = { Buffer: globalThis.Buffer, btoa: globalThis.btoa };
  try {
    globalThis.Buffer = undefined;
    lengths.forEach((n, i) => {
      count++;
      const got = ev.fileHash(pattern(n));
      if (got !== hashes[i]) fail(`base64/btoa/${n} bytes`, `${got}, want ${hashes[i]}`);
    });
    count++;
    const report = canonical(ev.verifyExport(manifest, segments, { maxFindings: c.max_findings }));
    if (report !== want) fail("base64/btoa/004-multi-segment", report);

    // As Chromium does at its limit, here for results over 30,000 characters.
    globalThis.btoa = (s) => (s.length > 30000 ? "" : saved.btoa(s));
    count++;
    const hash = ev.fileHash(pattern(100000));
    if (hash !== hashes.at(-1)) fail("base64/long-btoa-empty/fileHash", `${hash}, want ${hashes.at(-1)}`);
    count++;
    const root = ev.merkleRoot([pattern(100000)]);
    if (root !== bigRoot) fail("base64/long-btoa-empty/merkleRoot", `${root}, want ${bigRoot}`);

    // A btoa that gives "" for everything: an error, never a result.
    globalThis.btoa = () => "";
    const short = {
      fileHash: () => ev.fileHash(pattern(10)),
      "verifyExport manifest": () => ev.verifyExport(manifest, [segments[0]]),
      "verifyExport segments": () => ev.verifyExport(manifestText, segments),
    };
    for (const [what, fn] of Object.entries(short)) {
      count++;
      const e = thrown(fn);
      if (!(e instanceof RangeError)) fail(`base64/btoa-empty/${what}`, e ? `${e.name}: ${e.message}` : "a result");
    }

    // An engine error while encoding a list item stays that error, not E_USAGE.
    globalThis.btoa = () => {
      throw new RangeError("Invalid string length");
    };
    count++;
    const e = thrown(() => ev.verifyExport(manifestText, segments));
    if (!(e instanceof RangeError) || e.message !== "Invalid string length") {
      fail("base64/btoa-throws", e ? `${e.name}: ${e.message}` : "a result");
    }
  } finally {
    globalThis.Buffer = saved.Buffer;
    globalThis.btoa = saved.btoa;
  }
}

if (failures.length) {
  console.error(`${failures.length} of ${count} WebAssembly checks failed:\n  ${failures.join("\n  ")}`);
  process.exit(1);
}
console.log(`WebAssembly module passes all ${count} checks.`);
