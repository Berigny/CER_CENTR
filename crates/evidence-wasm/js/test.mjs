// Runs every test vector through the WebAssembly module, with no skips:
// unlike the JS reference, the module is the Rust core itself.
//
//   sh tools/build-wasm.sh && node crates/evidence-wasm/js/test.mjs

import { existsSync, readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { EvidenceError, Evidence, load } from "./evidence.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const vectors = join(here, "..", "..", "..", "test-vectors");
const wasm = process.argv[2] ?? join(here, "..", "pkg", "evidence_wasm.wasm");
const ev = await load(wasm);
const failures = [];
let count = 0;

const fail = (name, msg) => failures.push(`${name}: ${msg}`);
const codeOf = (fn) => {
  try {
    fn();
    return null;
  } catch (e) {
    if (!(e instanceof EvidenceError)) throw e;
    return e.code;
  }
};

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

if (failures.length) {
  console.error(`${failures.length} of ${count} WebAssembly checks failed:\n  ${failures.join("\n  ")}`);
  process.exit(1);
}
console.log(`WebAssembly module passes all ${count} checks.`);
