// Re-checks test-vectors/ with the JavaScript reference implementation.
// Vectors that expect a code the reference does not implement are skipped
// and listed. Exit status 1 on any disagreement.
//
//   node tools/check-vectors.mjs

import { existsSync, readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import * as ev from "./reference/evidence.mjs";
import * as log from "./reference/log.mjs";

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

// --- Store vectors (store/0), checked with log.mjs ------------------------------

if (existsSync(join(ROOT, "store", "entries.json"))) {
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

if (existsSync(join(ROOT, "store", "merkle.json"))) {
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

console.log(`skipped (not implemented in the JS reference): ${skipped.join(", ") || "none"}`);
if (failures.length) {
  console.error(`${failures.length} disagreement(s):\n  ${failures.join("\n  ")}`);
  process.exit(1);
}
console.log("JS reference agrees with every vector it implements.");
