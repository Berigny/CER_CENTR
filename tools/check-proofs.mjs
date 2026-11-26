// T11 through the WebAssembly module (docs/m1-store-spec.md section 11):
// builds a store with the command-line tool, asks it for inclusion and
// consistency proofs, and checks each one in Node with the module.
//
//   cargo build -p evidence-cli && sh tools/build-wasm.sh
//   EVIDENCE_BIN=target/debug/evidence node tools/check-proofs.mjs
//
// The store: `store init` (name proofs, the test key store-demo.seed), object
// vectors 001-005 appended one file at a time, the attestations of 003, and
// three checkpoints (sizes 10, 12 and 14; head 16). The checks:
// - inclusion: `store prove --seq n` for every n from 1 to 14 (the latest
//   checkpoint's size), and `--seq n --size 10` for every n up to 10;
// - consistency: `store prove --from a --to b` for every 1 <= a < b <= 16.
// Each proof verifies. Its leaf hash is sha256(0x00 || canonical entry), and
// its roots and checkpoint are the ones the store's lines and checkpoints
// give. The last hex digit changed in the first proof element, the leaf hash
// or a root makes verification false.
//
// EVIDENCE_BIN names the CLI binary. Unset, the script exits 2: it never
// skips. Exit 0 when every check passes, 1 otherwise.

import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { existsSync, mkdtempSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const OBJECTS = join(ROOT, "test-vectors", "objects");
const SEED = join(ROOT, "test-vectors", "keys", "store-demo.seed");
const PKG = join(ROOT, "crates", "evidence-wasm", "pkg");

const BIN = process.env.EVIDENCE_BIN;
if (!BIN) {
  console.error(
    "check-proofs: EVIDENCE_BIN is not set. Build the CLI (cargo build -p evidence-cli) and set EVIDENCE_BIN=target/debug/evidence.",
  );
  process.exit(2);
}
if (!existsSync(join(PKG, "evidence_wasm.wasm"))) {
  console.error("check-proofs: crates/evidence-wasm/pkg/ is missing; run sh tools/build-wasm.sh");
  process.exit(1);
}
const { load } = await import(pathToFileURL(join(PKG, "evidence.mjs")).href);
const ev = await load();

const failures = [];
const check = (ok, what) => {
  if (!ok) failures.push(what);
};
const canonical = (value) => ev.canonicalize(JSON.stringify(value));
const caseDir = (number) => join(OBJECTS, readdirSync(OBJECTS).find((name) => name.startsWith(`${number}-`)));
const leafHash = (entry) =>
  "sha256:" + createHash("sha256").update(Buffer.concat([Buffer.from([0]), Buffer.from(entry, "utf8")])).digest("hex");
// The last hex digit replaced: 0 by 1, any other by 0.
const changed = (hash) => hash.slice(0, -1) + (hash.endsWith("0") ? "1" : "0");

// Run `evidence store <args>`, which must exit 0, and return its lines,
// each checked to be canonical JSON.
function store(...args) {
  const run = spawnSync(BIN, ["store", ...args], { encoding: "utf8" });
  const command = `evidence store ${args.join(" ")}`;
  if (run.error) throw new Error(`${command}: cannot run ${BIN}: ${run.error.message}`);
  if (run.status !== 0) throw new Error(`${command}: exit ${run.status}\n${run.stdout}${run.stderr}`);
  if (!run.stdout.endsWith("\n")) throw new Error(`${command}: output does not end in LF`);
  return run.stdout
    .slice(0, -1)
    .split("\n")
    .map((line) => {
      const value = JSON.parse(line);
      if (canonical(value) !== line) throw new Error(`${command}: line is not canonical: ${line}`);
      return value;
    });
}
const one = (lines) => {
  if (lines.length !== 1) throw new Error(`expected one line, got ${lines.length}`);
  return lines[0];
};

let inclusions = 0;
let consistencies = 0;
const scratch = mkdtempSync(join(tmpdir(), "check-proofs-"));
try {
  const s = join(scratch, "s");
  one(store("init", s, "--name", "proofs", "--seed", SEED));
  for (const number of ["001", "002", "003", "004", "005"]) {
    store("append", s, join(caseDir(number), "object.json"));
  }
  store("attest", s, join(caseDir("003"), "attestations.jsonl"));
  const checkpoints = [1, 2, 3].map(() => one(store("checkpoint", s, "--seed", SEED)));
  const sizes = checkpoints.map((c) => c.size);
  if (sizes.join() !== "10,12,14") throw new Error(`checkpoint sizes ${sizes}, want 10,12,14`);
  const log = store("log", s, "--limit", "100");
  if (log.length !== 16) throw new Error(`store log printed ${log.length} lines, want 16`);
  const entries = log.map((line) => canonical(line.entry));
  const root = (n) => ev.merkleRoot(entries.slice(0, n));
  for (const c of checkpoints) check(c.root === root(c.size), `checkpoint of size ${c.size}: root`);

  // Inclusion, at the latest checkpoint's size (the default) and at the first's.
  for (const [checkpoint, extra] of [
    [checkpoints[2], []],
    [checkpoints[0], ["--size", "10"]],
  ]) {
    const size = checkpoint.size;
    for (let seq = 1; seq <= size; seq++) {
      const where = `inclusion of seq ${seq} at size ${size}`;
      const p = one(store("prove", s, "--seq", String(seq), ...extra));
      const verify = (leaf, proof, rootHash) => ev.verifyInclusion(leaf, seq - 1, size, proof, rootHash);
      check(p.seq === seq && p.size === size, `${where}: seq ${p.seq}, size ${p.size}`);
      check(p.root === checkpoint.root && p.root === root(size), `${where}: root`);
      check(p.checkpoint === checkpoint.checkpoint, `${where}: checkpoint`);
      check(canonical(p.entry) === entries[seq - 1], `${where}: entry`);
      check(p.leaf_hash === leafHash(entries[seq - 1]), `${where}: leaf_hash`);
      check(verify(p.leaf_hash, p.proof, p.root) === true, `${where}: does not verify`);
      check(p.proof.length > 0, `${where}: empty proof`);
      if (p.proof.length > 0) {
        const bent = [changed(p.proof[0]), ...p.proof.slice(1)];
        check(verify(p.leaf_hash, bent, p.root) === false, `${where}: verifies with a changed proof element`);
      }
      check(verify(changed(p.leaf_hash), p.proof, p.root) === false, `${where}: verifies with a changed leaf hash`);
      check(verify(p.leaf_hash, p.proof, changed(p.root)) === false, `${where}: verifies with a changed root`);
      inclusions++;
    }
  }

  // Consistency, for every 1 <= a < b <= head.
  for (let a = 1; a <= 16; a++) {
    for (let b = a + 1; b <= 16; b++) {
      const where = `consistency from ${a} to ${b}`;
      const p = one(store("prove", s, "--from", String(a), "--to", String(b)));
      const verify = (oldRoot, newRoot, proof) => ev.verifyConsistency(a, b, oldRoot, newRoot, proof);
      check(p.old_size === a && p.new_size === b, `${where}: sizes ${p.old_size}, ${p.new_size}`);
      check(p.old_root === root(a) && p.new_root === root(b), `${where}: roots`);
      check(verify(p.old_root, p.new_root, p.proof) === true, `${where}: does not verify`);
      check(p.proof.length > 0, `${where}: empty proof`);
      if (p.proof.length > 0) {
        const bent = [changed(p.proof[0]), ...p.proof.slice(1)];
        check(verify(p.old_root, p.new_root, bent) === false, `${where}: verifies with a changed proof element`);
      }
      check(verify(changed(p.old_root), p.new_root, p.proof) === false, `${where}: verifies with a changed old root`);
      check(verify(p.old_root, changed(p.new_root), p.proof) === false, `${where}: verifies with a changed new root`);
      consistencies++;
    }
  }
  check(inclusions === 24, `${inclusions} inclusion proofs, want 24`);
  check(consistencies === 120, `${consistencies} consistency proofs, want 120`);
} catch (error) {
  failures.push(error.message);
} finally {
  rmSync(scratch, { recursive: true, force: true });
}

if (failures.length) {
  console.error(`check-proofs: ${failures.length} failure(s):\n  ${failures.join("\n  ")}`);
  process.exit(1);
}
console.log(
  `check-proofs: ${inclusions} inclusion and ${consistencies} consistency proofs from the CLI verify through the WebAssembly module, and none with a changed byte.`,
);
