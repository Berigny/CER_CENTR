// Re-checks test-vectors/ with the JavaScript reference implementation.
// Vectors that expect a code the reference does not implement are skipped
// and listed. Exit status 1 on any disagreement.
//
//   node tools/check-vectors.mjs

import { existsSync, readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import * as ev from "./reference/evidence.mjs";

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

console.log(`skipped (not implemented in the JS reference): ${skipped.join(", ") || "none"}`);
if (failures.length) {
  console.error(`${failures.length} disagreement(s):\n  ${failures.join("\n  ")}`);
  process.exit(1);
}
console.log("JS reference agrees with every vector it implements.");
