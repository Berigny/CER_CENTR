// Generates test-vectors/ from the JavaScript reference implementation.
// Deterministic: running it twice produces identical files, and CI checks that
// the committed vectors match a fresh run.
//
//   node tools/make-vectors.mjs

import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { createHash, createPublicKey, verify as nodeVerify } from "node:crypto";
import * as ev from "./reference/evidence.mjs";
import * as log from "./reference/log.mjs";
import * as cs from "./reference/coordinate.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..", "test-vectors");
const write = (rel, text) => {
  const path = join(ROOT, rel);
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, text);
};
const pretty = (v) => `${JSON.stringify(v, null, 2)}\n`;
// "\uXXXX" as six characters of JSON text (not the character itself).
const esc = (hex) => `\\u${hex}`;

// --- Canonicalisation vectors ------------------------------------------------

const canonInputs = [
  ["literals-and-whitespace", ` [ true , false , null ] `],
  ["nesting-and-member-order", `{ "b" : [ 1 , { "d" : 2 , "c" : 3 } ] , "a" : {} , "": [] }`],
  [
    "rfc8785-member-order-is-utf16",
    `{"${esc("20ac")}":"Euro","${esc("000d")}":"CR","${esc("fb33")}":"Hebrew","1":"One",` +
      `"${esc("d83d")}${esc("de00")}":"Smiley","${esc("0080")}":"Control","${esc("00f6")}":"Latin"}`,
  ],
  [
    "string-escapes",
    `"${["0000", "0008", "0009", "000a", "000c", "000d", "001f", "0022", "005c"].map(esc).join("")}` +
      `\\/${["007f", "2028", "00e9", "d83d", "de00"].map(esc).join("")}"`,
  ],
  ["no-unicode-normalisation", `{"${esc("00e9")}":1,"e${esc("0301")}":2}`],
  ["numbers", `[1.0, -0, 0.1, 1e-7, 1E2, 123456789.125, 9007199254740991, -9007199254740991, 5e-324, 1e-400]`],
  ["top-level-string", `"evidence"`],
];
const canonErrors = [
  ["duplicate-member", `{"a":1,"a":2}`, "E_DUPLICATE_KEY"],
  ["duplicate-member-after-unescaping", `{"a":1,"${esc("0061")}":2}`, "E_DUPLICATE_KEY"],
  ["integer-above-2^53-1", `9007199254740992`, "E_NUMBER_RANGE"],
  ["float-above-2^53-1", `1e16`, "E_NUMBER_RANGE"],
  ["overflow-to-infinity", `-1e400`, "E_NUMBER_RANGE"],
  ["lone-high-surrogate", `"${esc("d800")}"`, "E_PARSE"],
  ["lone-low-surrogate", `"${esc("dc00")}x"`, "E_PARSE"],
  ["unescaped-control-character", `"a\tb"`, "E_PARSE"],
  ["leading-zero", `01`, "E_PARSE"],
  ["trailing-comma", `[1,]`, "E_PARSE"],
  ["trailing-content", `[1] 2`, "E_PARSE"],
  ["byte-order-mark", `${String.fromCharCode(0xfeff)}{}`, "E_PARSE"],
  ["nan", `NaN`, "E_PARSE"],
];
const canon = [
  ...canonInputs.map(([name, input]) => ({ name, input, expected: ev.canonicalize(ev.parseStrict(input)) })),
  ...canonErrors.map(([name, input, error]) => ({ name, input, error })),
];
write("canon.json", pretty(canon));

// --- Number formatting vectors (ECMAScript Number::toString) ------------------

let state = 0x9e3779b97f4a7c15n; // splitmix64, fixed seed
const MASK = (1n << 64n) - 1n;
function next64() {
  state = (state + 0x9e3779b97f4a7c15n) & MASK;
  let z = state;
  z = ((z ^ (z >> 30n)) * 0xbf58476d1ce4e5b9n) & MASK;
  z = ((z ^ (z >> 27n)) * 0x94d049bb133111ebn) & MASK;
  return z ^ (z >> 31n);
}
const view = new DataView(new ArrayBuffer(8));
const fromBits = (bits) => (view.setBigUint64(0, bits), view.getFloat64(0));
const toBits = (x) => (view.setFloat64(0, x), view.getBigUint64(0));
const hex = (bits) => bits.toString(16).padStart(16, "0");

const numberBits = [
  // RFC 8785 Appendix B
  0x0000000000000000n, 0x8000000000000000n, 0x0000000000000001n, 0x8000000000000001n,
  0x7fefffffffffffffn, 0xffefffffffffffffn, 0x4340000000000000n, 0xc340000000000000n,
  0x4430000000000000n, 0x44b52d02c7e14af5n, 0x44b52d02c7e14af6n, 0x44b52d02c7e14af7n,
  0x444b1ae4d6e2ef4en, 0x444b1ae4d6e2ef4fn, 0x444b1ae4d6e2ef50n, 0x3eb0c6f7a0b5ed8cn,
  0x3eb0c6f7a0b5ed8dn, 0x41b3de4355555553n, 0x41b3de4355555554n, 0x41b3de4355555555n,
  0x41b3de4355555556n, 0x41b3de4355555557n, 0xbecbf647612f3696n, 0x43143ff3c1cb0959n,
];
for (let i = 0; i < 3000; i++) numberBits.push(next64()); // across all exponents
for (let i = 0; i < 500; i++) {
  // Exact halfway cases, where ECMAScript picks the even digit: n + 0.25 and n + 0.75 in [2^50, 2^51).
  const n = 2 ** 50 + Number(next64() >> 14n);
  numberBits.push(toBits(n + (i % 2 ? 0.25 : 0.75)));
}
for (let i = 0; i < 500; i++) {
  // Large integers whose shortest form ends in zeros.
  numberBits.push(toBits(2 ** (54 + (i % 15)) * (1 + Number(next64() >> 12n) / 2 ** 52)));
}
const numbers = numberBits
  .filter((b) => ((b >> 52n) & 0x7ffn) !== 0x7ffn) // no NaN or infinity
  .map((b) => ({ bits: hex(b), expected: String(fromBits(b)) }));
write("numbers.json", pretty(numbers));

// --- Fixtures, keys ----------------------------------------------------------

const fixtures = {
  "installation-v12.ddl.sql": `-- Synthetic stand-in for an existing system-of-record schema.
-- Not a real CER schema: the point is that evidence/0 hashes whatever
-- definition the system of record already has.
CREATE TABLE installation (
  installation_id          VARCHAR(32) PRIMARY KEY,
  system_type              VARCHAR(32) NOT NULL,
  inverter_model           VARCHAR(64),
  battery_model            VARCHAR(64),
  installer_accreditation  VARCHAR(32) NOT NULL,
  commissioned_on          DATE,
  capacity_kw              NUMERIC(7,3),
  status                   VARCHAR(16) NOT NULL
);
`,
  "photo-check-result-v1.schema.json": pretty({
    $schema: "https://json-schema.org/draft/2020-12/schema",
    title: "Synthetic AI photo-check result",
    type: "object",
    required: ["check", "model", "inputs", "result"],
    properties: {
      check: { type: "string" },
      model: { type: "string" },
      inputs: { type: "array", items: { type: "string" } },
      result: { enum: ["pass", "fail", "refer"] },
      score: { type: "number" },
    },
  }),
  "lab-report-v1.md": "# Synthetic lab report format v1\n\nPlain text, one finding per line.\n",
  "test-report.txt": "Synthetic test report for evidence/0 vectors.\nFinding 1: conforms.\n",
};
for (const [name, text] of Object.entries(fixtures)) write(`fixtures/${name}`, text);
const fixtureHash = (name) => ev.sha256(readFileSync(join(ROOT, "fixtures", name)));

// store-demo is the store key of the export vectors (test-vectors/store/exports).
const keyNames = ["rec-registry-cdc", "photo-check-agent", "assessor-01", "store-demo"];
const seeds = {};
for (const name of keyNames) {
  seeds[name] = createHash("sha256").update(`evidence/0 test-vector key: ${name}`).digest("hex");
  write(`keys/${name}.seed`, `${seeds[name]}\n`);
}
write(
  "keys/public.json",
  pretty(Object.fromEntries(keyNames.map((n) => [n, ev.publicKey(seeds[n])]))),
);

// --- Object vectors ------------------------------------------------------------

rmSync(join(ROOT, "objects"), { recursive: true, force: true });
const clone = (v) => structuredClone(v);
const seal = (o) => ev.seal({ v: "evidence/0", id: "", ...o });
const line = (att) => `${ev.canonicalize(att)}\n`;

function vector(name, { description, object, text, attestations, store, require, expect }) {
  const dir = `objects/${name}`;
  write(`${dir}/object.json`, text ?? pretty(object));
  if (attestations) write(`${dir}/attestations.jsonl`, attestations.map(line).join(""));
  for (const [i, s] of (store ?? []).entries()) write(`${dir}/store/${i}.json`, pretty(s));
  const caseFile = { description };
  if (store) caseFile.store = true;
  if (require) caseFile.require = require;
  caseFile.expect = expect;
  write(`${dir}/case.json`, pretty(caseFile));
}

const record = {
  installation_id: "INS-TEST-000001",
  system_type: "solar-pv+battery",
  inverter_model: "TEST-INV-5K",
  battery_model: "TEST-BAT-10",
  installer_accreditation: "SAA-TEST-0001",
  commissioned_on: "2026-11-20",
  capacity_kw: 6.6,
  status: "commissioned",
};
const installationSchema = { ref: "rec-registry/installation@12", hash: fixtureHash("installation-v12.ddl.sql") };
const installation = { system: "rec-registry", type: "installation", key: "INS-TEST-000001" };
const cdc = { id: "service:rec-registry-cdc", kind: "service" };

const createBody = {
  op: "create",
  schema: installationSchema,
  subject: installation,
  content: { media_type: "application/json", hash: ev.digest(record), inline: record },
  actor: cdc,
  at: "2026-11-20T05:12:44Z",
  prev: [],
};
const obj001 = seal(createBody);

const record2 = { ...record, status: "audited" };
const updateBody = {
  op: "update",
  schema: installationSchema,
  subject: installation,
  content: { media_type: "application/json", hash: ev.digest(record2), inline: record2 },
  actor: cdc,
  at: "2026-11-21T01:00:00Z",
  prev: [obj001.id],
};
const obj002 = seal(updateBody);

const photo = (n) => ev.sha256(Buffer.from(`synthetic photo ${n}`));
const result = {
  check: "installer-attendance-photos",
  model: "test-photo-model-1",
  inputs: [photo(1), photo(2), photo(3)],
  result: "pass",
  score: 0.93,
};
const assertBody = {
  op: "assert",
  schema: { ref: "photo-check/result@1", hash: fixtureHash("photo-check-result-v1.schema.json") },
  subject: installation,
  content: { media_type: "application/json", hash: ev.digest(result), inline: result },
  actor: { id: "agent:photo-check", kind: "agent", on_behalf_of: "person:assessor-01" },
  at: "2026-11-21T02:30:00Z",
  prev: [],
};
const obj003 = seal(assertBody);
const authorAtt = ev.signAttestation(
  { object: obj003.id, purpose: "author", signer: "agent:photo-check", at: "2026-11-21T02:30:01Z" },
  seeds["photo-check-agent"],
);
const approveAtt = ev.signAttestation(
  { object: obj003.id, purpose: "approve", signer: "person:assessor-01", at: "2026-11-21T03:05:00Z" },
  seeds["assessor-01"],
);
const otherAtt = ev.signAttestation(
  { object: obj001.id, purpose: "author", signer: "service:rec-registry-cdc", at: "2026-11-20T05:12:45Z" },
  seeds["rec-registry-cdc"],
);

vector("001-create-inline", {
  description: "A system-of-record row captured as evidence. The file is pretty-printed; the id is over canonical bytes.",
  object: obj001,
  expect: { ok: true, id: obj001.id, content_checked: true },
});
vector("002-update-with-prev", {
  description: "An update that points at the version it replaces. The store holds that version.",
  object: obj002,
  store: [obj001],
  expect: { ok: true, id: obj002.id, content_checked: true },
});
vector("003-agent-assertion-approved", {
  description: "An AI check recorded as an assertion by an agent, signed by the agent and approved by a person. The line for another object is ignored.",
  object: obj003,
  attestations: [authorAtt, approveAtt, otherAtt],
  require: ["approve"],
  expect: { ok: true, id: obj003.id, content_checked: true, attestations: { approve: 1, author: 1 } },
});
const obj004 = seal({
  op: "create",
  schema: { ref: "lab-report@1", hash: fixtureHash("lab-report-v1.md") },
  subject: { system: "product-certification", type: "test-report", key: "TR-TEST-0001" },
  content: { media_type: "text/plain", hash: fixtureHash("test-report.txt"), uri: "fixtures/test-report.txt" },
  actor: { id: "person:lab-officer-01", kind: "human" },
  at: "2026-11-18T23:00:00Z",
  prev: [],
});
vector("004-external-content", {
  description: "Content held elsewhere: the object carries only its hash and location, so content_checked is false.",
  object: obj004,
  expect: { ok: true, id: obj004.id, content_checked: false },
});
const record5 = { ...record, installation_id: "INS-TEST-000002" };
const obj005 = seal({
  ...clone(createBody),
  subject: { ...installation, key: "INS-TEST-000002" },
  content: { media_type: "application/json", hash: ev.digest(record5), inline: record5 },
  ext: { "centr/0": { regime: "national-code", jurisdiction: "VIC" } },
});
vector("005-profile-extension", {
  description: "Profile data under ext, keyed by profile id. Covered by the id like everything else.",
  object: obj005,
  expect: { ok: true, id: obj005.id, content_checked: true },
});

const tampered = clone(obj001);
tampered.subject.key = "INS-TEST-000009";
vector("101-id-mismatch", {
  description: "A field changed after sealing.",
  object: tampered,
  expect: { ok: false, code: "E_ID_MISMATCH" },
});
const wrongContent = clone(createBody);
wrongContent.content.inline = { ...record, capacity_kw: 9.9 };
vector("102-content-hash-mismatch", {
  description: "Inline content changed and the object resealed: the id is consistent, the content hash is not.",
  object: seal(wrongContent),
  expect: { ok: false, code: "E_CONTENT_HASH_MISMATCH" },
});
const noSponsor = clone(assertBody);
delete noSponsor.actor.on_behalf_of;
vector("103-agent-without-sponsor", {
  description: "An agent must name the person or body it acts for.",
  object: seal(noSponsor),
  expect: { ok: false, code: "E_SHAPE" },
});
vector("104-unknown-member", {
  description: "Members outside the schema are rejected; profile data belongs under ext.",
  object: seal({ ...clone(createBody), note: "free text" }),
  expect: { ok: false, code: "E_SHAPE" },
});
const textInline = clone(createBody);
textInline.content.media_type = "text/plain";
vector("105-inline-requires-json", {
  description: "Inline content is JSON, so the media type must say so.",
  object: seal(textInline),
  expect: { ok: false, code: "E_SHAPE" },
});
vector("106-duplicate-member", {
  description: "Two op members. A last-one-wins parser reads op=create and accepts the id; a strict parser refuses.",
  text: `{"op":"delete",${ev.canonicalize(obj001).slice(1)}\n`,
  expect: { ok: false, code: "E_DUPLICATE_KEY" },
});
vector("107-number-out-of-range", {
  description: "Integers above 2^53 - 1 lose precision in many languages; they must be strings.",
  text: `${ev.canonicalize(obj001).replace('"capacity_kw":6.6', '"capacity_kw":9007199254740993')}\n`,
  expect: { ok: false, code: "E_NUMBER_RANGE" },
});
vector("108-timestamp-not-a-date", {
  description: "Matches the timestamp pattern but 30 February does not exist.",
  object: seal({ ...clone(createBody), at: "2026-02-30T00:00:00Z" }),
  expect: { ok: false, code: "E_TIMESTAMP" },
});
vector("109-timestamp-with-offset", {
  description: "Timestamps are UTC with Z so they order without time-zone rules.",
  object: seal({ ...clone(createBody), at: "2026-11-20T16:12:44+11:00" }),
  expect: { ok: false, code: "E_SHAPE" },
});
vector("110-prev-missing", {
  description: "prev points at an object the store does not hold.",
  object: seal({ ...clone(updateBody), prev: [ev.sha256(Buffer.from("not in the store"))] }),
  store: [obj001],
  expect: { ok: false, code: "E_PREV_MISSING" },
});
vector("111-prev-after-object", {
  description: "An object cannot replace a version dated after itself.",
  object: seal({ ...clone(updateBody), at: "2026-11-19T00:00:00Z" }),
  store: [obj001],
  expect: { ok: false, code: "E_PREV_ORDER" },
});
vector("112-signature-invalid", {
  description: "The approval's time was edited after signing.",
  object: obj003,
  attestations: [authorAtt, { ...approveAtt, at: "2026-11-21T03:06:00Z" }],
  require: ["approve"],
  expect: { ok: false, code: "E_SIGNATURE_INVALID" },
});
vector("113-approval-missing", {
  description: "Only the agent signed; an approval is required.",
  object: obj003,
  attestations: [authorAtt],
  require: ["approve"],
  expect: { ok: false, code: "E_ATTESTATION_MISSING" },
});
vector("114-lone-surrogate", {
  description: "A string that is not valid Unicode text.",
  text: `${ev.canonicalize(obj001).replace('"key":"INS-TEST-000001"', `"key":"INS-${esc("d800")}"`)}\n`,
  expect: { ok: false, code: "E_PARSE" },
});
vector("115-attestation-bad-key", {
  description: "An attestation whose public key is not 32 bytes.",
  object: obj003,
  attestations: [{ ...approveAtt, key: approveAtt.key.slice(0, -4) }],
  expect: { ok: false, code: "E_ATTESTATION_SHAPE" },
});

console.log(`wrote ${canon.length} canon, ${numbers.length} number and 20 object vectors to ${ROOT}`);

// --- Store vectors (store/0) ------------------------------------------------------
// Built with tools/reference/log.mjs: entries.json, merkle.json and the export
// cases under store/exports/.

const assert = (cond, what) => {
  if (!cond) throw new Error(`generator check failed: ${what}`);
};
const entryError = (fn) => {
  try {
    fn();
    return null;
  } catch (e) {
    if (!(e instanceof ev.EvidenceError)) throw e;
    return e.code;
  }
};

// The demo store key, derived like the other test keys (keys/store-demo.seed).
const storeSeed = seeds["store-demo"];
const demoGenesis = log.makeGenesis("cer-demo", ev.publicKey(storeSeed));
const genesisObj = log.genesisObject(demoGenesis, "2026-11-26T00:00:00.000Z");
assert(log.genesisFromObject(genesisObj).key === demoGenesis.key, "genesis reads back");
const logId = genesisObj.id;
const genesisAuthor = ev.signAttestation(
  { object: logId, purpose: "author", signer: demoGenesis.signer, at: genesisObj.at },
  storeSeed,
);

// One short, coherent chain: the genesis, its author attestation, then three
// objects from the object vectors. Each entry's prev is the hash of the one before.
const chain = [];
const chainEntry = (name, members) => {
  const prev = chain.length ? log.entryHash(chain.at(-1).entry) : logId;
  const entry = log.makeEntry({ log: logId, seq: chain.length + 1, prev, ...members });
  chain.push({ name, entry });
  return entry;
};
chainEntry("genesis", { kind: "object", ref: logId, received_at: "2026-11-26T00:00:00.000Z", epoch: 1 });
chainEntry("genesis-author-attestation", {
  kind: "attestation",
  ref: log.attestationRef(genesisAuthor),
  received_at: "2026-11-26T00:00:00.000Z",
  epoch: 1,
});
const objectEntry = chainEntry("object", {
  kind: "object",
  ref: obj001.id,
  received_at: "2026-11-26T00:00:01.000Z",
  epoch: 1,
});
chainEntry("received-at-nine-fraction-digits", {
  kind: "object",
  ref: obj002.id,
  received_at: "2026-11-26T00:00:02.123456789Z",
  epoch: 1,
});
chainEntry("epoch-7", { kind: "object", ref: obj003.id, received_at: "2026-11-26T00:00:03.000Z", epoch: 7 });
const genesisEntry = chain[0].entry;
assert(genesisEntry.log === logId && genesisEntry.ref === logId && genesisEntry.prev === logId, "seq 1 equalities");

const validEntries = chain.map(({ name, entry }) => {
  const canonical = log.entryCanonical(entry);
  return { name, entry, canonical, hash: ev.sha256(Buffer.from(canonical, "utf8")) };
});

// Each invalid case changes one thing in the object entry.
const withEntry = (change) => {
  const e = { ...objectEntry };
  change(e);
  return ev.canonicalize(e);
};
const invalidEntries = [
  ["seq-zero", (e) => (e.seq = 0)],
  ["seq-not-integer", (e) => (e.seq = 1.5)],
  ["unknown-member", (e) => (e.note = "free text")],
  ["prev-missing", (e) => delete e.prev],
  ["kind-blob", (e) => (e.kind = "blob")],
  ["received-at-with-offset", (e) => (e.received_at = "2026-11-26T11:00:01.000+11:00")],
  ["prev-not-a-hash", (e) => (e.prev = e.prev.toUpperCase())],
].map(([name, change]) => ({ name, input: withEntry(change), error: "S_ENTRY_INVALID" }));

for (const c of validEntries) {
  assert(entryError(() => log.parseEntry(c.canonical)) === null, `entries/${c.name} is valid`);
}
for (const c of invalidEntries) {
  assert(entryError(() => log.parseEntry(c.input)) === c.error, `entries/${c.name} is invalid`);
}
write("store/entries.json", pretty({ valid: validEntries, invalid: invalidEntries }));

// Merkle trees over the UTF-8 leaves "leaf-0" ... "leaf-(n-1)".
const leafText = (i) => `leaf-${i}`;
const flipByte = (proof, element) =>
  proof.map((d, i) => {
    const copy = Buffer.from(d);
    if (i === element) copy[0] ^= 0xff;
    return copy;
  });
const texts = (proof) => proof.map(log.digestToText);
const merkle = [1, 2, 3, 4, 5, 7, 8, 9, 16, 17, 100].map((n) => {
  const leaves = Array.from({ length: n }, (_, i) => leafText(i));
  const hashes = leaves.map((s) => log.leafHash(Buffer.from(s, "utf8")));
  const root = log.merkleRoot(hashes);
  const leafAt = (i) => log.leafHash(Buffer.from(leafText(i), "utf8"));

  const inclusion = [...new Set([0, Math.floor(n / 2), n - 1])]
    .sort((a, b) => a - b)
    .map((index) => ({ index, proof: log.inclusionProof(hashes, index), valid: true }));
  if (n >= 2) {
    // The middle index's proof with the first byte of its last element inverted.
    const middle = inclusion.find((c) => c.index === Math.floor(n / 2));
    inclusion.push({ index: middle.index, proof: flipByte(middle.proof, middle.proof.length - 1), valid: false });
    // An index equal to the tree size, with an empty proof.
    inclusion.push({ index: n, proof: [], valid: false });
  }
  for (const c of inclusion) {
    assert(log.verifyInclusion(leafAt(c.index), c.index, n, c.proof, root) === c.valid, `inclusion ${n}/${c.index}`);
  }

  const consistency = [...new Set([1, 2, 3, n - 1])]
    .filter((m) => m >= 1 && m < n)
    .sort((a, b) => a - b)
    .map((m) => ({
      old_size: m,
      old_root: log.merkleRoot(hashes.slice(0, m)),
      proof: log.consistencyProof(hashes, m),
      valid: true,
    }));
  if (n >= 3) {
    // The proof from n - 1 with the first byte of its last element inverted.
    const last = consistency.find((c) => c.old_size === n - 1);
    consistency.push({ ...last, proof: flipByte(last.proof, last.proof.length - 1), valid: false });
  }
  for (const c of consistency) {
    const ok = log.verifyConsistency(c.old_size, n, c.old_root, root, c.proof);
    assert(ok === c.valid, `consistency ${c.old_size}->${n}`);
  }

  return {
    name: `size-${n}`,
    leaves,
    root: log.digestToText(root),
    inclusion: inclusion.map(({ index, proof, valid }) => ({ index, proof: texts(proof), valid })),
    consistency: consistency.map(({ old_size, old_root, proof, valid }) => ({
      old_size,
      old_root: log.digestToText(old_root),
      proof: texts(proof),
      valid,
    })),
  };
});
write("store/merkle.json", pretty(merkle));

const entryCount = validEntries.length + invalidEntries.length;
console.log(`wrote ${entryCount} entry and ${merkle.length} Merkle tree vectors to ${join(ROOT, "store")}`);

// --- Store export vectors ----------------------------------------------------------
// Valid cases (0xx) are inputs.json operation scripts replayed through the
// reference writer. Tampered cases (1xx) are edited (the files of a valid case
// changed afterwards) or built (written by the reference writer with one rule
// broken on purpose). Every golden report is the reference verifier's, and the
// generator checks that it holds the findings spec/store-0.md lists for the case.

rmSync(join(ROOT, "store", "exports"), { recursive: true, force: true });
const readVector = (rel) => readFileSync(join(ROOT, rel));
const STORE = { name: "cer-demo", seed: "store-demo" };
const SEGMENT_SIZE = 10000;
const T0 = log.parseTimestamp("2026-11-26T00:00:00.000Z");
const time = (secs, ms = 0) => log.formatMillis({ secs: T0.secs + secs, nanos: ms * 1_000_000 });
const ticks = (n) => Array.from({ length: n }, (_, i) => time(i));
const ticker = () => {
  let i = 0;
  return () => time(i++);
};
const plusOneMilli = (at) => {
  const t = log.parseTimestamp(at);
  return log.formatMillis({ secs: t.secs, nanos: t.nanos + 1_000_000 });
};

const OBJ = (name) => `objects/${name}/object.json`;
const ATT003 = "objects/003-agent-assertion-approved/attestations.jsonl";
const lineOf = (file, n) => readVector(file).toString("utf8").split("\n")[n - 1];
const op = {
  init: { op: "init" },
  append: (file) => ({ op: "append", file }),
  appendObject: (object) => ({ op: "append", object }),
  attest: (file, line) => ({ op: "attest", file, line }),
  reopen: { op: "reopen" },
  checkpoint: { op: "checkpoint" },
};
const clockReads = (ops) => ops.filter((o) => o.op !== "reopen").length;
const inputsFor = (ops, { segment_size = SEGMENT_SIZE, clock } = {}) => ({
  ...STORE,
  segment_size,
  clock: clock ?? ticks(clockReads(ops)),
  ops,
});

// A finding as the checks below spell it: code, then (core), @seq and [file].
const show = (f) => `${f.code}${f.core ? `(${f.core})` : ""}${f.seq ? `@${f.seq}` : ""}${f.file ? `[${f.file}]` : ""}`;
// `only`: exactly these findings, in order. `has`: each of these, spelled in
// full. `codes`: some finding with each of these codes. `lacks`: no finding
// with any of these codes.
function expectFindings(name, report, { only, has = [], codes = [], lacks = [] }) {
  const got = report.findings.map(show);
  const list = got.join(", ") || "none";
  if (only) {
    assert(JSON.stringify(got) === JSON.stringify(only), `${name}: findings ${list}, want only ${only.join(", ")}`);
  }
  for (const f of has) assert(got.includes(f), `${name}: findings ${list} lack ${f}`);
  for (const c of codes) assert(report.findings.some((f) => f.code === c), `${name}: findings ${list} lack ${c}`);
  for (const c of lacks) assert(!report.findings.some((f) => f.code === c), `${name}: findings ${list} hold ${c}`);
  assert(report.ok === (report.findings.length === 0), `${name}: ok agrees with findings`);
}

const segmentNames = (files) => [...files.keys()].filter((f) => f !== "manifest.json").sort();
const exportLines = (files) =>
  segmentNames(files).flatMap((f) => log.splitSegment(files.get(f)).lines.map((l) => log.parseLine(l)));
const verifyFiles = (files, maxFindings) =>
  log.verifyExport(
    files.get("manifest.json"),
    segmentNames(files).map((f) => files.get(f)),
    maxFindings ? { maxFindings } : {},
  );

let exportCount = 0;
// Writes one case: case.json ({description, expect, max_findings?, proofs?}),
// inputs.json for a valid case, and the export's files.
function exportCase(name, { description, files, inputs, maxFindings, proofs, check }) {
  const report = verifyFiles(files, maxFindings);
  check(report);
  assert(JSON.stringify(verifyFiles(files, maxFindings)) === JSON.stringify(report), `${name}: verification repeats`);
  const dir = `store/exports/${name}`;
  const caseFile = { description, expect: report };
  if (maxFindings) caseFile.max_findings = maxFindings;
  if (proofs) caseFile.proofs = proofs;
  write(`${dir}/case.json`, pretty(caseFile));
  if (inputs) write(`${dir}/inputs.json`, pretty(inputs));
  for (const [file, bytes] of files) write(`${dir}/${file}`, bytes);
  exportCount += 1;
}

// A valid case: replays `inputs` and checks the report is ok. Returns the files.
function validCase(name, description, inputs, { check = () => {}, proofs } = {}) {
  const writer = log.replay(inputs, readVector);
  const files = writer.exportFiles(inputs.segment_size);
  exportCase(name, {
    description,
    files,
    inputs,
    proofs: proofs?.(files),
    check: (r) => {
      expectFindings(name, r, { only: [] });
      check(r, writer);
    },
  });
  return files;
}

// A built case: `build(writer)` runs on a writer with the demo store key and a
// clock that ticks one second per transaction from T0.
function builtCase(name, description, build, expect) {
  const writer = new log.Writer({ name: STORE.name, seed: storeSeed, clock: ticker() });
  build(writer);
  const files = writer.exportFiles(SEGMENT_SIZE);
  exportCase(name, { description, files, check: (r) => expect(r, writer) });
  return files;
}

// A copy of an export's files with `change` applied. The manifest's segment
// hashes are then recomputed, and nothing else in it changes, unless `rehash`
// is false.
function edited(base, change, { rehash = true } = {}) {
  const files = new Map([...base].map(([f, bytes]) => [f, Buffer.from(bytes)]));
  change(files);
  if (rehash) {
    const manifest = JSON.parse(files.get("manifest.json").toString("utf8"));
    for (const s of manifest.segments) s.hash = ev.sha256(files.get(s.file));
    files.set("manifest.json", Buffer.from(ev.canonicalize(manifest), "utf8"));
  }
  return files;
}
// Rewrites the lines of one segment file, which must end in LF: `fn` maps the
// list of lines (strings, without LF) to a new list.
function editLines(files, file, fn) {
  const text = files.get(file).toString("utf8");
  assert(text.endsWith("\n"), `${file} ends in LF`);
  files.set(file, Buffer.from(fn(text.slice(0, -1).split("\n")).map((l) => `${l}\n`).join(""), "utf8"));
}
// Replaces `from` by `to` in line n (from 1) of a segment file; `from` must
// occur exactly once in that line.
function replaceInLine(files, file, n, from, to) {
  editLines(files, file, (lines) => {
    assert(lines[n - 1].split(from).length === 2, `${file} line ${n} holds ${from} once`);
    return lines.map((l, i) => (i === n - 1 ? l.replace(from, to) : l));
  });
}
// The line of a segment file that holds the object with this id.
function lineHolding(files, file, id) {
  const lines = log.splitSegment(files.get(file)).lines.map((l) => log.parseLine(l));
  const n = lines.findIndex((l) => l.kind === "object" && l.body.id === id) + 1;
  assert(n > 0, `${file} holds ${id}`);
  return n;
}

// One byte of a string in an object's inline content, changed with the line
// kept canonical: the object's id no longer matches.
const FLIP_FROM = '"inverter_model":"TEST-INV-5K"';
const FLIP_TO = '"inverter_model":"TEST-INV-6K"';
const flip = (id) => (files) =>
  replaceInLine(files, "000001.jsonl", lineHolding(files, "000001.jsonl", id), FLIP_FROM, FLIP_TO);

// The log id of another store, for entries that name the wrong log.
const otherLogId = log.genesisObject(log.makeGenesis("other-demo", ev.publicKey(storeSeed)), time(0)).id;

// --- Valid cases (0xx)

const ops002 = [
  op.init,
  op.append(OBJ("001-create-inline")),
  op.append(OBJ("002-update-with-prev")),
  op.attest(ATT003, 3),
  op.checkpoint,
];
const ops003 = [
  ...ops002,
  op.append(OBJ("003-agent-assertion-approved")),
  op.attest(ATT003, 1),
  op.attest(ATT003, 2),
  op.append(OBJ("004-external-content")),
  op.reopen,
  op.checkpoint,
];
// The same operations as case 002, on a writer the caller holds.
function build002(w) {
  w.init();
  w.append(readVector(OBJ("001-create-inline")));
  w.append(readVector(OBJ("002-update-with-prev")));
  w.attest(lineOf(ATT003, 3));
  w.checkpoint();
}

const files001 = validCase(
  "001-genesis",
  "A new store: the genesis and its author attestation.",
  inputsFor([op.init]),
  { check: (r) => assert(r.head.seq === 2 && !r.covered, "001: head is seq 2, nothing covered") },
);

const files002 = validCase(
  "002-records-and-checkpoint",
  "The genesis, object vectors 001 and 002, the author line for 001, and a witnessed checkpoint.",
  inputsFor(ops002),
  {
    check: (r) => {
      const c = r.checked;
      assert(c.entries === 7 && c.objects === 3 && c.attestations === 3 && c.checkpoints === 1, "002: checked");
      assert(r.covered?.size === 5, "002: covered size 5");
    },
  },
);

// Inclusion proofs for every seq up to the latest checkpoint, and the
// consistency proof from the first checkpoint to it, made from the export's lines.
function proofsFor(files) {
  const manifest = JSON.parse(files.get("manifest.json").toString("utf8"));
  const lines = exportLines(files);
  const leaves = lines.map((l) => log.leafHash(Buffer.from(l.entryCanonical, "utf8")));
  const checkpoints = lines.filter((l) => l.kind === "object" && l.body.schema.ref === log.CHECKPOINT_SCHEMA_REF);
  assert(checkpoints.length === 2, "two checkpoints");
  const [first, latest] = checkpoints.map((l) => l.body.content.inline);
  assert(checkpoints[1].body.id === manifest.checkpoint.id, "the latest checkpoint is the manifest's");
  const tree = leaves.slice(0, latest.size);
  const inclusion = tree.map((leaf, i) => ({
    seq: i + 1,
    size: latest.size,
    leaf_hash: log.digestToText(leaf),
    proof: log.inclusionProof(tree, i).map(log.digestToText),
    root: latest.root,
  }));
  const consistency = [
    {
      old_size: first.size,
      new_size: latest.size,
      old_root: first.root,
      new_root: latest.root,
      proof: log.consistencyProof(tree, first.size).map(log.digestToText),
    },
  ];
  const d = log.digestFromText;
  for (const p of inclusion) {
    assert(log.verifyInclusion(d(p.leaf_hash), p.seq - 1, p.size, p.proof.map(d), d(p.root)), `inclusion ${p.seq}`);
  }
  for (const p of consistency) {
    const ok = log.verifyConsistency(p.old_size, p.new_size, d(p.old_root), d(p.new_root), p.proof.map(d));
    assert(ok, `consistency ${p.old_size}->${p.new_size}`);
  }
  return { inclusion, consistency };
}

validCase(
  "003-attestations-two-checkpoints",
  "Case 002, then object vector 003 with its author and approve lines, object vector 004, a reopen and a second checkpoint.",
  inputsFor(ops003),
  {
    proofs: proofsFor,
    check: (r) => {
      assert(r.head.seq === 13 && r.covered?.size === 11, "003: head 13, covered 11");
      assert(r.checked.checkpoints === 2 && r.checked.attestations === 6, "003: checked");
    },
  },
);

const files004 = validCase(
  "004-multi-segment",
  "The operations of case 003 exported with segment_size 4: four segment files.",
  inputsFor(ops003, { segment_size: 4 }),
  { check: (r) => assert(r.head.seq === 13, "004: head 13") },
);

const forkSubject = { ...installation, key: "INS-TEST-000003" };
const forkCreate = (status, at) => {
  const r = { ...record, installation_id: "INS-TEST-000003", status };
  return seal({
    op: "create",
    schema: installationSchema,
    subject: forkSubject,
    content: { media_type: "application/json", hash: ev.digest(r), inline: r },
    actor: cdc,
    at,
    prev: [],
  });
};
const files005 = validCase(
  "005-fork",
  "Two creates for one subject, neither listing the other in prev: its record lane has two heads, a fork, which is not a finding.",
  inputsFor([
    op.init,
    op.appendObject(forkCreate("commissioned", "2026-11-25T09:00:00Z")),
    op.appendObject(forkCreate("pending", "2026-11-25T09:00:05Z")),
  ]),
  { check: (r) => assert(r.counts.forks === 1, "005: one fork") },
);

validCase(
  "006-clock-backwards",
  "A clock that steps backwards after a reading finer than a millisecond: received_at repeats rather than decreasing, and the second checkpoint's at equals the first's.",
  inputsFor(
    [op.init, op.append(OBJ("001-create-inline")), op.checkpoint, op.append(OBJ("004-external-content")), op.checkpoint],
    { clock: [time(0), time(2), "2026-11-26T00:00:03.123456789Z", time(1), time(1)] },
  ),
  {
    check: (r, w) => {
      const lines = w.lines().map((l) => log.parseLine(l));
      const received = lines.map((l) => l.entry.received_at);
      const repeated = new Set(received.slice(3)).size === 1 && received[3] === "2026-11-26T00:00:03.123Z";
      assert(repeated, "006: received_at repeats");
      const cps = lines.filter((l) => l.kind === "object" && l.body.schema.ref === log.CHECKPOINT_SCHEMA_REF);
      assert(cps.length === 2 && cps[0].body.at === cps[1].body.at, "006: second checkpoint at equals the first's");
      assert(r.covered?.size === 6, "006: covered by the second checkpoint");
    },
  },
);

// Three versions of one record, each dated after the store received it. With
// the clock at T0 + 1, 2 and 3 s: 300 s and 1 ns after counts, exactly 300 s
// (written without a fraction) does not, and a day after counts.
const lateSubject = { ...installation, key: "INS-TEST-000004" };
const lateVersion = (status, at, prev) => {
  const r = { ...record, installation_id: "INS-TEST-000004", status };
  return seal({
    op: prev.length ? "update" : "create",
    schema: installationSchema,
    subject: lateSubject,
    content: { media_type: "application/json", hash: ev.digest(r), inline: r },
    actor: cdc,
    at,
    prev,
  });
};
const late1 = lateVersion("commissioned", "2026-11-26T00:05:01.000000001Z", []);
const late2 = lateVersion("audited", "2026-11-26T00:05:02Z", [late1.id]);
const late3 = lateVersion("decommissioned", "2026-11-27T00:00:03.000Z", [late2.id]);
validCase(
  "007-late-claim",
  "Three versions of one record dated after the store received them: 300 s and 1 ns after and a day after are counted in counts.at_after_received, exactly 300 s after is not.",
  inputsFor([op.init, op.appendObject(late1), op.appendObject(late2), op.appendObject(late3)]),
  {
    check: (r, w) => {
      const received = w.lines().map((l) => log.parseLine(l).entry.received_at);
      assert(received.slice(2).join() === [time(1), time(2), time(3)].join(), "007: received_at T0 + 1, 2 and 3 s");
      assert(r.counts.at_after_received === 2 && r.counts.forks === 0, "007: two late claims, no fork");
    },
  },
);

// --- Tampered cases (1xx)

// `only(list, more)`: exactly these findings, then any further checks.
const only = (list, more = () => {}) => (name) => (r, w) => {
  expectFindings(name, r, { only: list });
  more(r, w);
};
// A tampered case. `make` is a build function (a built case) or { files,
// maxFindings } (an edited case). `expect` is only(...) or the options of
// expectFindings.
function tamperedCase(name, description, make, expect) {
  const check = typeof expect === "function" ? expect(name) : (r) => expectFindings(name, r, expect);
  if (typeof make === "function") {
    builtCase(name, description, make, check);
  } else {
    exportCase(name, { description, files: make.files, maxFindings: make.maxFindings, check });
  }
}

const files101 = edited(files002, flip(obj002.id));
tamperedCase(
  "101-object-byte-flipped",
  "Edited 002: one byte of a string in object 002's content changed, the line kept canonical. Leaves cover entry bytes, not object bytes, so only the object is invalid.",
  { files: files101 },
  only(["S_OBJECT_INVALID(E_ID_MISMATCH)@4"]),
);

tamperedCase(
  "102-entry-removed",
  "Edited 004: the line of object 002 removed from the first segment file. first and last in the manifest are not compared with the lines.",
  {
    files: edited(files004, (f) => {
      const n = lineHolding(f, "000001.jsonl", obj002.id);
      editLines(f, "000001.jsonl", (lines) => lines.filter((_, i) => i !== n - 1));
    }),
  },
  { codes: ["S_SEQ_GAP", "S_CHAIN_BREAK", "S_CHECKPOINT_MISMATCH"], lacks: ["S_MANIFEST_MISMATCH"] },
);

tamperedCase(
  "103-entries-swapped",
  "Edited 002: the lines of objects 001 and 002 swapped. Checkpoint leaves follow reading order.",
  {
    files: edited(files002, (f) => {
      const a = lineHolding(f, "000001.jsonl", obj001.id);
      const b = lineHolding(f, "000001.jsonl", obj002.id);
      assert(b === a + 1, "103: 002 follows 001");
      editLines(f, "000001.jsonl", (lines) => {
        const swapped = [...lines];
        [swapped[a - 1], swapped[b - 1]] = [lines[b - 1], lines[a - 1]];
        return swapped;
      });
    }),
  },
  { codes: ["S_SEQ_GAP", "S_CHAIN_BREAK", "S_PREV_POSITION", "S_CHECKPOINT_MISMATCH"] },
);

tamperedCase(
  "104-prev-after-child",
  "Built: object vector 002 written before object vector 001, which its prev names.",
  (w) => {
    w.init();
    w.append(readVector(OBJ("002-update-with-prev")), { unchecked: true });
    w.append(readVector(OBJ("001-create-inline")));
  },
  only(["S_PREV_POSITION@3"]),
);

tamperedCase(
  "105-checkpoint-root-wrong",
  "Built: a witnessed checkpoint whose root is the tree hash of one entry fewer than its size.",
  (w) => {
    w.init();
    w.append(readVector(OBJ("001-create-inline")));
    w.checkpoint({ root: w.root(w.size - 1) });
  },
  only(["S_CHECKPOINT_MISMATCH@4"], (r) => assert(!r.covered, "105: nothing covered")),
);

tamperedCase(
  "106-received-regression",
  "Built: the entry of object vector 004 has a received_at half a second below the entry before it.",
  (w) => {
    w.init();
    w.append(readVector(OBJ("001-create-inline")));
    w.append(readVector(OBJ("004-external-content")), { entry: { received_at: time(0, 500) } });
  },
  only(["S_RECEIVED_REGRESSION@4"]),
);

tamperedCase(
  "107-epoch-regression",
  "Built: after a reopen (epoch 2), the entry of object vector 005 has epoch 1.",
  (w) => {
    w.init();
    w.append(readVector(OBJ("001-create-inline")));
    w.reopen();
    w.append(readVector(OBJ("004-external-content")));
    w.append(readVector(OBJ("005-profile-extension")), { entry: { epoch: 1 } });
  },
  only(["S_EPOCH_REGRESSION@5"]),
);

tamperedCase(
  "108-attestation-signature-invalid",
  "Built: object vector 003, then its approve line with the time edited after signing (line 2 of object vector 112's attestations). The entry's ref is the line's real hash.",
  (w) => {
    w.init();
    w.append(readVector(OBJ("003-agent-assertion-approved")));
    w.attest(lineOf("objects/112-signature-invalid/attestations.jsonl", 2), { unchecked: true });
  },
  only(["S_ATTESTATION_INVALID(E_SIGNATURE_INVALID)@4"]),
);

tamperedCase(
  "109-attestation-before-object",
  "Built: the author line for object vector 001 written before the object.",
  (w) => {
    w.init();
    w.attest(lineOf(ATT003, 3), { unchecked: true });
    w.append(readVector(OBJ("001-create-inline")));
  },
  only(["S_ATTESTATION_TARGET@3"]),
);

tamperedCase(
  "110-checkpoint-unsigned",
  "Built: a checkpoint written without its witness attestation.",
  (w) => {
    w.init();
    w.append(readVector(OBJ("001-create-inline")));
    w.checkpoint({ witness: false });
  },
  only(["S_CHECKPOINT_UNSIGNED@4"], (r) => assert(!r.covered, "110: nothing covered")),
);

tamperedCase(
  "111-wrong-log",
  "Built: the entry of object vector 001 names another log's id. The line is unusable, so the next line's seq is a gap.",
  (w) => {
    w.init();
    w.append(readVector(OBJ("001-create-inline")), { entry: { log: otherLogId } });
    w.append(readVector(OBJ("004-external-content")));
  },
  only(["S_WRONG_LOG@3", "S_SEQ_GAP@4"]),
);

tamperedCase(
  "112-line-not-canonical",
  "Edited 002: in the line of object 002, the number 6.6 written as 6.60, which is the same value but not its canonical form.",
  {
    files: edited(files002, (f) =>
      replaceInLine(
        f,
        "000001.jsonl",
        lineHolding(f, "000001.jsonl", obj002.id),
        '"capacity_kw":6.6,',
        '"capacity_kw":6.60,',
      ),
    ),
  },
  { has: ["S_LINE_INVALID"] },
);

tamperedCase(
  "113-duplicate-ref",
  "Built: object vector 001 appended a second time.",
  (w) => {
    w.init();
    w.append(readVector(OBJ("001-create-inline")));
    w.append(readVector(OBJ("001-create-inline")), { unchecked: true });
  },
  only(["S_DUPLICATE_REF@4"]),
);

tamperedCase(
  "114-genesis-missing",
  "Edited 002: the genesis line removed. The first line read has no previous line, so there is no chain break.",
  { files: edited(files002, (f) => editLines(f, "000001.jsonl", (lines) => lines.slice(1))) },
  {
    has: ["S_MANIFEST_MISMATCH", "S_GENESIS_INVALID", "S_SEQ_GAP@2", "S_ATTESTATION_TARGET@2", "S_RESERVED_SUBJECT@6"],
    lacks: ["S_CHAIN_BREAK"],
  },
);

tamperedCase(
  "115-prev-dated-after",
  "Built: object vector 001, then object vector 111, whose prev (001) is dated after it.",
  (w) => {
    w.init();
    w.append(readVector(OBJ("001-create-inline")));
    w.append(readVector(OBJ("111-prev-after-object")), { unchecked: true });
  },
  only(["S_OBJECT_INVALID(E_PREV_ORDER)@4"]),
);

tamperedCase(
  "116-segment-hash-mismatch",
  "Edited 002: object 002 changed as in case 101, and the manifest left as it was. The lines are still verified.",
  { files: edited(files002, flip(obj002.id), { rehash: false }) },
  only(["S_SEGMENT_HASH[000001.jsonl]", "S_OBJECT_INVALID(E_ID_MISMATCH)@4"]),
);

const synthetic = { note: "A synthetic object that claims a store subject." };
tamperedCase(
  "117-reserved-subject",
  "Built: an ordinary object whose subject.system is evidence-store, which only the store itself may write.",
  (w) => {
    w.init();
    w.append(
      seal({
        op: "assert",
        schema: { ref: "synthetic/note@1", hash: ev.sha256(Buffer.from("synthetic note schema")) },
        subject: { system: log.STORE_SYSTEM, type: "note", key: STORE.name },
        content: { media_type: "application/json", hash: ev.digest(synthetic), inline: synthetic },
        actor: { id: "service:synthetic-writer", kind: "service" },
        at: "2026-11-25T12:00:00Z",
        prev: [],
      }),
      { unchecked: true },
    );
  },
  only(["S_RESERVED_SUBJECT@3"]),
);

tamperedCase(
  "118-ref-mismatch",
  "Built: the entry of object vector 004 has a ref that is not the object's id; the chain is otherwise consistent.",
  (w) => {
    w.init();
    w.append(readVector(OBJ("004-external-content")), { entry: { ref: ev.sha256(Buffer.from("not this item")) } });
  },
  only(["S_REF_MISMATCH@3"]),
);

tamperedCase(
  "119-final-lf-missing",
  "Edited 002: the segment file's final LF removed, so its last line, the witness, is unusable and the checkpoint is unsigned.",
  { files: edited(files002, (f) => f.set("000001.jsonl", f.get("000001.jsonl").subarray(0, -1))) },
  only(["S_MANIFEST_MISMATCH", "S_LINE_INVALID", "S_CHECKPOINT_UNSIGNED@6"], (r) => {
    const c = r.checked;
    assert(r.head.seq === 6 && !r.covered, "119: head is the checkpoint, nothing covered");
    assert(c.entries === 6 && c.objects === 3 && c.attestations === 2 && c.checkpoints === 1, "119: checked");
  }),
);

tamperedCase(
  "120-blank-line",
  "Edited 001: one more LF at the end of the segment file, an empty last line.",
  { files: edited(files001, (f) => f.set("000001.jsonl", Buffer.concat([f.get("000001.jsonl"), Buffer.from("\n")]))) },
  only(["S_LINE_INVALID"], (r) => assert(r.head.seq === 2, "120: head is seq 2")),
);

tamperedCase(
  "121-crlf",
  "Edited 001: every line ends in CR LF, so no line is usable and the lines have no log id.",
  {
    files: edited(files001, (f) =>
      f.set("000001.jsonl", Buffer.from(f.get("000001.jsonl").toString("utf8").replaceAll("\n", "\r\n"), "utf8")),
    ),
  },
  only(["S_MANIFEST_MISMATCH", "S_LINE_INVALID", "S_LINE_INVALID", "S_GENESIS_INVALID"], (r) => {
    assert(!("log" in r) && !("head" in r), "121: no log or head");
    assert(Object.values(r.checked).every((n) => n === 0), "121: nothing checked");
  }),
);

tamperedCase(
  "122-first-line-unusable",
  "Edited 002: a space inserted after the first { of the genesis line. The log id is read from line 2.",
  {
    files: edited(files002, (f) =>
      editLines(f, "000001.jsonl", (lines) => [`{ ${lines[0].slice(1)}`, ...lines.slice(1)]),
    ),
  },
  only(
    [
      "S_MANIFEST_MISMATCH",
      "S_LINE_INVALID",
      "S_GENESIS_INVALID",
      "S_SEQ_GAP@2",
      "S_ATTESTATION_TARGET@2",
      "S_RESERVED_SUBJECT@6",
    ],
    (r) => assert(r.log === JSON.parse(files002.get("manifest.json")).log, "122: the log id of case 002"),
  ),
);

tamperedCase(
  "123-genesis-author-signature-invalid",
  "Built 001: the author attestation was signed one millisecond after the genesis and then dated at the genesis, so its signature does not verify.",
  (w) =>
    w.init({
      author: ({ logId, genesis, at, seed }) => ({
        ...ev.signAttestation({ object: logId, purpose: "author", signer: genesis.signer, at: plusOneMilli(at) }, seed),
        at,
      }),
    }),
  only(["S_ATTESTATION_INVALID(E_SIGNATURE_INVALID)@2"]),
);

tamperedCase(
  "124-genesis-author-wrong-at",
  "Built 001: the author attestation is validly signed by the store key but dated one millisecond after the genesis.",
  (w) =>
    w.init({
      author: ({ logId, genesis, at, seed }) =>
        ev.signAttestation({ object: logId, purpose: "author", signer: genesis.signer, at: plusOneMilli(at) }, seed),
    }),
  only(["S_GENESIS_INVALID@2"]),
);

const files125 = edited(files002, flip(obj001.id));
tamperedCase(
  "125-invalid-object-not-held",
  "Edited 002: object 001 changed as case 101 changes object 002. An object that fails evidence/0 steps 1-5 is not held, so neither object 002's prev nor 001's author line finds it.",
  { files: files125 },
  only(["S_OBJECT_INVALID(E_ID_MISMATCH)@3", "S_OBJECT_INVALID(E_PREV_MISSING)@4", "S_ATTESTATION_TARGET@5"]),
);

// Case 126: one synthetic subject S, with ids X, Y and Z that no line holds,
// and T before every received_at.
const subjectS = { ...installation, key: "INS-TEST-000004" };
const T = "2026-11-25T12:00:00Z";
const absentId = (name) => ev.sha256(Buffer.from(`synthetic id that no line holds: ${name}`));
const [X, Y, Z] = ["X", "Y", "Z"].map(absentId);
const recordS = (op, status, at, prev) => {
  const r = { ...record, installation_id: "INS-TEST-000004", status };
  return seal({
    op,
    schema: installationSchema,
    subject: subjectS,
    content: { media_type: "application/json", hash: ev.digest(r), inline: r },
    actor: cdc,
    at,
    prev,
  });
};
const checkS = (outcome, prev) => {
  const r = { ...result, result: outcome };
  return seal({
    op: "assert",
    schema: { ref: "photo-check/result@1", hash: fixtureHash("photo-check-result-v1.schema.json") },
    subject: subjectS,
    content: { media_type: "application/json", hash: ev.digest(r), inline: r },
    actor: { id: "service:photo-check", kind: "service" },
    at: T,
    prev,
  });
};
const A1 = recordS("create", "commissioned", T, []);
const A2 = recordS("create", "pending", T, []);
const M = recordS("update", "audited", "2026-11-25T11:59:55Z", [X, A1.id, A2.id]);
const N = recordS("update", "suspended", "2026-11-25T11:59:54Z", [M.id, Z]);
const L = checkS("pass", []);
const P = checkS("refer", [L.id, Y]);
tamperedCase(
  "126-prev-list-order",
  "Built: step 6 takes prev in list order and stops at the first finding. A1 and A2 create S; M (prev X, A1, A2) names X first, held nowhere; N (prev M, Z) is dated before M; P (prev L, Y) names L, held only on a later line.",
  (w) => {
    w.init();
    w.append(A1);
    w.append(A2);
    w.append(M, { unchecked: true });
    w.append(N, { unchecked: true });
    w.append(P, { unchecked: true });
    w.append(L);
  },
  only(["S_OBJECT_INVALID(E_PREV_MISSING)@5", "S_OBJECT_INVALID(E_PREV_ORDER)@6", "S_PREV_POSITION@7"], (r) =>
    assert(r.counts.forks === 0, "126: M merges A1 and A2"),
  ),
);

tamperedCase(
  "127-manifest-invalid",
  "Edited 002: an LF appended to manifest.json, and object 002 changed as in case 101 with the segment hash left as it was. The manifest is not used further, and the lines are still verified.",
  {
    files: edited(
      files002,
      (f) => {
        flip(obj002.id)(f);
        f.set("manifest.json", Buffer.concat([f.get("manifest.json"), Buffer.from("\n")]));
      },
      { rehash: false },
    ),
  },
  only(["S_MANIFEST_INVALID", "S_OBJECT_INVALID(E_ID_MISMATCH)@4"]),
);

tamperedCase(
  "128-checkpoint-recognition",
  "Built 002, then a checkpoint-shaped object whose actor is not the genesis signer, then a checkpoint by the genesis signer whose content log is another log's id, each witnessed by the store key. The manifest's checkpoint is still the checkpoint of 002.",
  (w) => {
    build002(w);
    const first = w.latestCheckpoint;
    const witness = (obj) =>
      ev.canonicalize(
        ev.signAttestation({ object: obj.id, purpose: "witness", signer: w.genesis.signer, at: obj.at }, storeSeed),
      );
    const content = () => ({ size: w.size, head: w.entryHash(w.size), root: w.root() });
    const otherActor = log.checkpointObject(
      { log: w.logId, ...content() },
      { signer: log.signerFor("other-demo") },
      time(5),
      first.id,
    );
    w.append(otherActor, { unchecked: true });
    w.attest(witness(otherActor));
    const wrongLog = log.checkpointObject({ log: otherLogId, ...content() }, w.genesis, time(7), first.id);
    w.append(wrongLog, { unchecked: true });
    w.attest(witness(wrongLog));
    assert(w.latestCheckpoint.id === first.id, "128: the writer's latest checkpoint is 002's");
  },
  only(["S_RESERVED_SUBJECT@8", "S_CHECKPOINT_INVALID@10"], (r) => {
    assert(r.checked.checkpoints === 2, "128: two checkpoints recognised");
    assert(r.covered?.size === 5, "128: covered by the checkpoint of 002");
  }),
);

tamperedCase(
  "129-findings-truncated",
  "The export of case 125 verified with max_findings 2. The E_PREV_MISSING decided at the end of the log displaces the S_ATTESTATION_TARGET found before it.",
  { files: files125, maxFindings: 2 },
  only(["S_OBJECT_INVALID(E_ID_MISMATCH)@3", "S_OBJECT_INVALID(E_PREV_MISSING)@4"], (r) =>
    assert(r.truncated === true, "129: truncated"),
  ),
);

// Cases 130 and 131: signatures that Node's Ed25519 check accepts and evidence/0
// rejects, because a point of small order is involved: the signature's R in
// 130, the key in 131. Each case asserts that Node accepts what it holds, so
// it shows the small-order rule, and no other, at work.
const SPKI_ED25519 = Buffer.from("302a300506032b6570032100", "hex");
function nodeAccepts(att) {
  const { sig, ...unsigned } = att;
  const raw = Buffer.from(att.key.slice("ed25519:".length), "base64url");
  const key = createPublicKey({ key: Buffer.concat([SPKI_ED25519, raw]), format: "der", type: "spki" });
  return nodeVerify(null, Buffer.from(ev.canonicalize(unsigned), "utf8"), key, Buffer.from(sig, "base64url"));
}
const ED25519_L = 2n ** 252n + 27742317777372353535851937790883648493n;
const ED25519_P = 2n ** 255n - 19n;
const sha512 = (...parts) => createHash("sha512").update(Buffer.concat(parts)).digest();
const fromLE = (bytes) => BigInt(`0x${Buffer.from(bytes).reverse().toString("hex")}`);
const toLE = (n) => Buffer.from(n.toString(16).padStart(64, "0"), "hex").reverse();
const b64url = (bytes) => Buffer.from(bytes).toString("base64url");
const IDENTITY = toLE(1n); // the point (0, 1), of order 1
const BASE_POINT = Buffer.from(`58${"66".repeat(31)}`, "hex"); // B, y = 4/5

// An attestation signed with the store key at nonce 0: R = [0]B is the
// identity, and S = k * a mod L, so R equals [S]B - [k]A as Ed25519 requires.
function signNonceZero({ object, purpose, signer, at }, seedHex) {
  const unsigned = { v: "attestation/0", object, purpose, signer, key: ev.publicKey(seedHex), at };
  const h = sha512(Buffer.from(seedHex, "hex"));
  h[0] &= 248;
  h[31] = (h[31] & 127) | 64;
  const a = fromLE(h.subarray(0, 32));
  const A = Buffer.from(unsigned.key.slice("ed25519:".length), "base64url");
  const k = fromLE(sha512(IDENTITY, A, Buffer.from(ev.canonicalize(unsigned), "utf8"))) % ED25519_L;
  return { ...unsigned, sig: b64url(Buffer.concat([IDENTITY, toLE((k * a) % ED25519_L)])) };
}

// An attestation by the identity point as key, written as `key`, with R = B
// and S = 1: [S]B - [k]A = B for every message, so it verifies under a check
// that lets small-order keys through.
const forgedBy = (key, { object, purpose, signer, at }) => ({
  v: "attestation/0",
  object,
  purpose,
  signer,
  key: `ed25519:${b64url(key)}`,
  at,
  sig: b64url(Buffer.concat([BASE_POINT, toLE(1n)])),
});
const smallOrderOnly = (name, att) =>
  assert(nodeAccepts(att) && !ev.verifyAttestation(att), `${name}: only the small-order rule rejects it`);

tamperedCase(
  "130-witness-nonce-zero",
  "Built 002, with the checkpoint's witness signed by the store key at nonce 0, so its R is the identity, a point of small order. Node's Ed25519 check accepts it; evidence/0 does not.",
  (w) => {
    w.init();
    w.append(readVector(OBJ("001-create-inline")));
    w.append(readVector(OBJ("002-update-with-prev")));
    w.attest(lineOf(ATT003, 3));
    w.checkpoint({ witness: false });
    const cp = w.latestCheckpoint;
    const fields = { object: cp.id, purpose: "witness", signer: w.genesis.signer, at: cp.at };
    const witness = signNonceZero(fields, storeSeed);
    smallOrderOnly("130", witness);
    w.attest(ev.canonicalize(witness), { unchecked: true });
  },
  only(["S_CHECKPOINT_UNSIGNED@6", "S_ATTESTATION_INVALID(E_SIGNATURE_INVALID)@7"], (r) =>
    assert(!r.covered, "130: nothing covered"),
  ),
);

tamperedCase(
  "131-attestation-small-order-key",
  "Built: object vector 001, then two approve lines whose key is the identity point, written canonically (y = 1) and as y = p + 1, each with R = B and S = 1, which Node's Ed25519 check accepts for any message.",
  (w) => {
    w.init();
    w.append(readVector(OBJ("001-create-inline")));
    for (const key of [IDENTITY, toLE(ED25519_P + 1n)]) {
      const att = forgedBy(key, { object: obj001.id, purpose: "approve", signer: "person:assessor-01", at: time(5) });
      smallOrderOnly("131", att);
      w.attest(ev.canonicalize(att), { unchecked: true });
    }
  },
  only(["S_ATTESTATION_INVALID(E_SIGNATURE_INVALID)@4", "S_ATTESTATION_INVALID(E_SIGNATURE_INVALID)@5"]),
);

tamperedCase(
  "132-entry-invalid",
  "Edited 005: the entry of the first create gets one more member, the line kept canonical. The line is unusable and has no seq, and the next line's seq is a gap.",
  {
    files: edited(files005, (f) =>
      replaceInLine(f, "000001.jsonl", 3, '"epoch":1,"kind":"object"', '"epoch":1,"extra":true,"kind":"object"'),
    ),
  },
  only(["S_ENTRY_INVALID", "S_SEQ_GAP@4"], (r) => {
    assert(r.head.seq === 4 && r.checked.entries === 3, "132: head 4, three usable lines");
    assert(r.counts.forks === 0, "132: one create held, no fork");
  }),
);

tamperedCase(
  "133-genesis-not-exact",
  "Built 001: the genesis object has an ext member and is sealed again, so it is valid evidence/0 with valid content but not exactly the genesis built from its own name, key and at. Its author line is signed by the store key over its id.",
  (w) =>
    w.init({
      edit: (obj) => ev.seal({ ...obj, ext: { "synthetic/0": { note: "not part of a genesis" } } }),
    }),
  only(["S_GENESIS_INVALID@1", "S_GENESIS_INVALID@2"]),
);

tamperedCase(
  "134-attestation-key-padded",
  "Built: object vector 001, then its author line (line 3 of object vector 003's attestations) with = padding added to the key, which is not its one canonical spelling.",
  (w) => {
    w.init();
    w.append(readVector(OBJ("001-create-inline")));
    const att = JSON.parse(lineOf(ATT003, 3));
    w.attest(ev.canonicalize({ ...att, key: `${att.key}=` }), { unchecked: true });
  },
  only(["S_ATTESTATION_INVALID(E_ATTESTATION_SHAPE)@4"]),
);

// Case 135: objects that fail evidence/0 steps 2, 3 and 5, each sealed again
// so that its id matches.
const unknownMember = ev.seal({ ...clone(obj004), note: "a member evidence/0 does not have" });
const notADate = ev.seal({ ...clone(obj004), at: "2026-02-30T00:00:00Z" });
const wrongContentHash = clone(obj001);
wrongContentHash.content.hash = ev.sha256(Buffer.from("not the inline content"));
tamperedCase(
  "135-object-steps-2-3-5",
  "Built: object vector 004 with an unknown member, object vector 004 dated 30 February, and object vector 001 with a content.hash that its inline content does not hash to, each sealed again.",
  (w) => {
    w.init();
    for (const obj of [unknownMember, notADate, ev.seal(wrongContentHash)]) w.append(obj, { unchecked: true });
  },
  only([
    "S_OBJECT_INVALID(E_SHAPE)@3",
    "S_OBJECT_INVALID(E_TIMESTAMP)@4",
    "S_OBJECT_INVALID(E_CONTENT_HASH_MISMATCH)@5",
  ]),
);

console.log(`wrote ${exportCount} export cases to ${join(ROOT, "store", "exports")}`);

// --- Coordinate vectors (coordinate/0) ----------------------------------------------
// Built with tools/reference/coordinate.mjs: the cases of spec/coordinate-0.md,
// section Conformance, in its order. Every `expect` and `error` is what the
// reference gives. The generator also asserts the results the spec states, and
// builds the kernel-2 dictionary from Kernel/2's allocation rule and checks it
// against the tables of docs/kernel-2.md.

const CO = "coordinate";
rmSync(join(ROOT, CO), { recursive: true, force: true });
const canonOf = (v) => ev.canonicalize(v);
const outcomeOf = (fn) => {
  try {
    return { expect: fn() };
  } catch (e) {
    if (!(e instanceof cs.CoordinateError)) throw e;
    return { error: e.error };
  }
};
const E = (code, members = {}) => ({ error: { code: `P_COORDINATE_${code}`, ...members } });
const V = (value) => ({ expect: value });
const sameResult = (got, want) => canonOf(got) === canonOf(want);
const times = (n, id) => Array(n).fill(id);
const pow2 = (k) => (1n << BigInt(k)).toString();

// Primes by a sieve, independent of the reference's trial division.
const PRIMES = (() => {
  const n = 46341; // above the square root of 2^31 + 11
  const composite = new Uint8Array(n);
  const out = [];
  for (let i = 2; i < n; i++) {
    if (composite[i]) continue;
    out.push(i);
    for (let j = i * i; j < n; j += i) composite[j] = 1;
  }
  return out;
})();
const primeByDivision = (n) => n >= 2 && PRIMES.every((p) => p * p > n || n % p !== 0);
const productOf = (primes) => primes.reduce((a, p) => a * BigInt(p), 1n);
assert(PRIMES[4095] === 38873 && PRIMES[4096] === 38891, "the 4,096th prime is 38,873 and the 4,097th 38,891");
assert(primeByDivision(2147483647) && primeByDivision(2147483659), "2^31 - 1 and 2147483659 are prime");
const P16 = "32589158477190044730";
const P20 = "557940830126698960967415390";
assert(productOf(PRIMES.slice(0, 16)).toString() === P16, "p16 is the product of the first 16 primes");
assert(productOf(PRIMES.slice(0, 20)).toString() === P20, "p20 is the product of the first 20 primes");
assert(BigInt(P16).toString(2).length === 65 && BigInt(P20).toString(2).length === 89, "p16 has 65 bits, p20 89");

// --- Dictionaries

const DICT_V = cs.DICTIONARY_V;
const sortFamilies = (families) => [...families].sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0));
const sortFacets = (facets) => [...facets].sort((a, b) => a.prime - b.prime);
const dictValue = (profile, families, facets) => ({
  v: DICT_V,
  profile,
  families: sortFamilies(families),
  facets: sortFacets(facets),
});

// The dictionary of spec/coordinate-0.md, section Dictionary, as printed there.
const catalogue = {
  v: DICT_V,
  profile: "catalogue-demo/0",
  families: [
    { name: "jurisdiction", values: "one" },
    { name: "product", values: "one" },
    { name: "record", values: "one" },
    { name: "standard", values: "set" },
  ],
  facets: [
    ["record.product-listing", 2],
    ["record.installation", 3],
    ["product.inverter", 5],
    ["standard.as-nzs-4777-2", 7],
    ["jurisdiction.vic", 11],
    ["product.battery", 13],
    ["product.pv-module", 17],
    ["standard.as-nzs-5033", 19],
    ["standard.as-nzs-3000", 23],
    ["standard.as-nzs-5139", 29],
    ["jurisdiction.nsw", 31],
    ["jurisdiction.qld", 37],
    ["jurisdiction.sa", 41],
    ["jurisdiction.wa", 43],
    ["jurisdiction.tas", 47],
    ["jurisdiction.act", 53],
    ["jurisdiction.nt", 59],
    ["record.inspection", 61],
    ["standard.as-nzs-4777-1", 67],
  ].map(([id, prime]) => ({ id, prime })),
};
const CATALOGUE_ID = "sha256:005aad78a1aaae4ef493f0f8b20facec76f5dadddefc8de05903754998ca6278";
const KERNEL_ID = "sha256:28ec5d10ee5e6e5fd8effc7515e9adec7f049f75beeccce5064d738d922cac71";
const cat = () => structuredClone(catalogue);
const withFacets = (value, ...facets) => ({ ...value, facets: sortFacets([...value.facets, ...facets]) });
const withoutFacet = (value, id) => ({ ...value, facets: value.facets.filter((f) => f.id !== id) });
const setFacet = (value, id, change) => ({
  ...value,
  facets: value.facets.map((f) => (f.id === id ? { ...f, ...change } : f)),
});
const setFamily = (value, name, change) => ({
  ...value,
  families: value.families.map((f) => (f.name === name ? { ...f, ...change } : f)),
});

// Kernel/2's dictionary, from its allocation rule (docs/kernel-2.md section
// 2.1): block b, position i gets the prime P[10b + i]. The identifiers are
// those of its section 2.2, read from the document, and the result must equal
// the table of its section 2.3 (REQ-KRN-003 to 005).
const kernelDoc = readFileSync(join(ROOT, "..", "docs", "kernel-2.md"), "utf8");
const docSection = (start, end) => {
  const a = kernelDoc.indexOf(start);
  const b = kernelDoc.indexOf(end, a + 1);
  assert(a >= 0 && b > a, `docs/kernel-2.md has a section from "${start}" to "${end}"`);
  return kernelDoc.slice(a, b);
};
// The rows of the Markdown table whose first line starts with `header`, as
// arrays of trimmed cells; rows 0 and 1 are the header and the delimiter.
const docTable = (text, header) => {
  const lines = text.split("\n");
  const start = lines.findIndex((l) => l.startsWith(header));
  assert(start >= 0, `docs/kernel-2.md has a table starting "${header}"`);
  const rows = [];
  for (let i = start; i < lines.length && lines[i].startsWith("|"); i++) rows.push(lines[i]);
  return rows.map((l) => l.slice(1, -1).split("|").map((c) => c.trim()));
};
const unquote = (cell) => {
  const m = /^`([^`]+)`$/.exec(cell);
  assert(m, `a cell in backticks: ${cell}`);
  return m[1];
};
const section22 = docSection("### 2.2 ", "### 2.3 ");
const blockRows = docTable(section22, "| Block (`b`) |").slice(2);
const positionRows = docTable(section22, "| Position |").slice(2);
assert(blockRows.every((r, b) => r[0] === String(b)) && blockRows.length === 7, "Kernel/2 has blocks 0 to 6");
assert(positionRows.every((r, i) => r[0] === String(i)) && positionRows.length === 10, "Kernel/2 has positions 0 to 9");
const BLOCKS = blockRows.map((r) => unquote(r[1]));
const POSITIONS = positionRows.map((r) => unquote(r[1]));
const kernelFacets = BLOCKS.flatMap((block, b) =>
  POSITIONS.map((position, i) => ({ id: `${block}.${position}`, prime: PRIMES[10 * b + i] })),
);
const kernel = dictValue(
  "kernel/2",
  BLOCKS.map((name) => ({ name, values: "multiset" })),
  kernelFacets,
);
{
  const table = docTable(docSection("### 2.3 ", "## 3. "), "| Block |");
  assert(JSON.stringify(table[0].slice(1)) === JSON.stringify(POSITIONS), "Kernel/2 2.3: columns in position order");
  const rows = table.slice(2);
  assert(rows.length === 7, "Kernel/2 2.3: seven blocks");
  for (const [b, row] of rows.entries()) {
    assert(row[0] === BLOCKS[b], `Kernel/2 2.3: row ${b} is ${BLOCKS[b]}`);
    const want = PRIMES.slice(10 * b, 10 * b + 10);
    assert(JSON.stringify(row.slice(1).map(Number)) === JSON.stringify(want), `Kernel/2 2.3: the primes of ${row[0]}`);
  }
  const primes = kernel.facets.map((f) => f.prime);
  assert(JSON.stringify(primes) === JSON.stringify(PRIMES.slice(0, 70)) && primes.at(-1) === 349, "REQ-KRN-003");
  assert(JSON.stringify(kernel.facets) === JSON.stringify(kernelFacets), "the allocation rule gives ascending primes");
  for (const block of BLOCKS) {
    const p = (position) => kernel.facets.find((f) => f.id === `${block}.${position}`).prime;
    const states = POSITIONS.slice(2).map(p);
    assert(POSITIONS[0] === "entry" && POSITIONS[1] === "exit", "REQ-KRN-004: entry, exit, then the states");
    assert(p("entry") < p("exit") && states.every((s) => p("exit") < s), `REQ-KRN-005 in ${block}`);
  }
}
const KERNEL_IDS = kernel.facets.map((f) => f.id);

// The first 4,096 primes: multi.p<prime> up to 53, flag.p<prime> after.
const wide = dictValue(
  "wide-demo/0",
  [
    { name: "flag", values: "set" },
    { name: "multi", values: "multiset" },
  ],
  PRIMES.slice(0, 4096).map((p) => ({ id: `${p <= 53 ? "multi" : "flag"}.p${p}`, prime: p })),
);
assert(wide.facets.filter((f) => f.id.startsWith("multi.")).length === 16, "wide-demo: 16 multi facets");
const maxPrime = dictValue(
  "max-prime-demo/0",
  [
    { name: "big", values: "multiset" },
    { name: "unused", values: "one" },
  ],
  [{ id: "big.p2147483647", prime: 2147483647 }],
);

// The same dictionary as catalogue-demo, written differently: members in
// another order, tabs and CRLF, \u escapes in some strings (one with upper-case
// hex), 7 written 7.0 and 59 written 58.99999999999999999.
const escapeFirst = (s) => `"\\u${s.charCodeAt(0).toString(16).padStart(4, "0")}${JSON.stringify(s).slice(2)}`;
const reorderedText = (() => {
  const str = (s, i) => (i % 3 === 0 ? escapeFirst(s) : JSON.stringify(s));
  const num = (p) => ({ 7: "7.0", 59: "58.99999999999999999" })[p] ?? String(p);
  const facets = catalogue.facets.map((f, i) => `{\t"prime" :${num(f.prime)} ,"id":${str(f.id, i)}}`);
  const families = catalogue.families.map((f, i) => `{ "values":${JSON.stringify(f.values)},  "name" : ${str(f.name, i)} }`);
  return [
    "{",
    `\t"facets" : [\t${facets.join(",\r\n\t\t")}`,
    "\t],",
    '\t"profile":"catalogue\\u002Ddemo/0",',
    `\t"families":[ ${families.join(",")} ],`,
    '\t"v" : "coordinate-dictionary\\u002f0"',
    "}",
    "",
  ].join("\r\n");
})();
assert(reorderedText.includes(":7.0 ") && reorderedText.includes(":58.99999999999999999 "), "reordered: 7.0 and 58.99999999999999999");
assert(JSON.parse(reorderedText).facets[16].prime === 59, "58.99999999999999999 parses to 59");

const v2 = withFacets(
  { ...cat(), families: sortFamilies([...catalogue.families, { name: "phase", values: "one" }]) },
  { id: "product.heat-pump", prime: 71 },
  { id: "phase.single", prime: 73 },
  { id: "phase.three", prime: 79 },
);
const validDictionaries = [
  ["catalogue-demo", pretty(catalogue)],
  ["catalogue-demo-reordered", reorderedText],
  ["catalogue-demo-v2", pretty(v2)],
  ["catalogue-demo-values-changed", pretty(setFamily(cat(), "standard", { values: "multiset" }))],
  ["catalogue-demo-prime-changed", pretty({ ...cat(), facets: sortFacets(setFacet(cat(), "jurisdiction.nt", { prime: 71 }).facets) })],
  ["catalogue-demo-facet-removed", pretty(withoutFacet(cat(), "record.inspection"))],
  ["kernel-2", pretty(kernel)],
  ["wide-demo", pretty(wide)],
  ["max-prime-demo", pretty(maxPrime)],
];
const D = {}; // name -> loaded dictionary
const dictionaryCases = { valid: [], invalid: [] };
for (const [name, text] of validDictionaries) {
  const rel = `${CO}/dictionaries/valid/${name}.json`;
  write(rel, text);
  const got = outcomeOf(() => cs.parseDictionary(readFileSync(join(ROOT, rel))));
  assert(got.expect, `${rel} loads: ${JSON.stringify(got.error)}`);
  D[name] = got.expect;
  dictionaryCases.valid.push({ name, id: got.expect.id });
}
assert(D["catalogue-demo"].id === CATALOGUE_ID, "catalogue-demo has the id the spec states");
assert(D["catalogue-demo-reordered"].id === CATALOGUE_ID, "catalogue-demo-reordered has catalogue-demo's id");
assert(D["kernel-2"].id === KERNEL_ID, "kernel-2 has the id the spec states");
assert(D["wide-demo"].facets.length === cs.MAX_FACETS && D["wide-demo"].facets.at(-1).prime === 38873, "wide-demo");

// Invalid dictionaries: catalogue-demo with one change, unless the name says
// otherwise. A case is [name, bytes or text or value, the detail the spec
// states (and the core code for step 1)].
const pc = pretty(catalogue);
const replaceOnce = (text, from, to) => {
  assert(text.split(from).length === 2, `"${from}" occurs once`);
  return text.replace(from, to);
};
const vicAt = pc.indexOf('"jurisdiction.vic"') + 1; // not-utf-8: a byte 0xff starts that id
const extra = (prime) => withFacets(cat(), { id: "record.extra", prime });
const familiesSwapped = () => ({ ...cat(), families: [catalogue.families[1], catalogue.families[0], ...catalogue.families.slice(2)] });
const invalidDictionaries = [
  ["not-json", replaceOnce(pc, '    }\n  ]\n}\n', '    },\n  ]\n}\n'), "shape", "E_PARSE"],
  [
    "duplicate-member",
    replaceOnce(pc, '  "profile": "catalogue-demo/0",\n', '  "profile": "catalogue-demo/0",\n  "profile": "catalogue-demo/0",\n'),
    "shape",
    "E_DUPLICATE_KEY",
  ],
  ["number-range", pretty({ ...cat(), facets: [...catalogue.facets, { id: "record.extra", prime: 2 ** 53 }] }), "shape", "E_NUMBER_RANGE"],
  // More step 1 cases, beyond those the spec lists: the bytes are not UTF-8,
  // start with a byte order mark, hold a lone surrogate, or nest too deep.
  [
    "not-utf-8",
    Buffer.concat([Buffer.from(pc.slice(0, vicAt)), Buffer.from([0xff]), Buffer.from(pc.slice(vicAt))]),
    "shape",
    "E_PARSE",
  ],
  ["byte-order-mark", Buffer.concat([Buffer.from([0xef, 0xbb, 0xbf]), Buffer.from(pc)]), "shape", "E_PARSE"],
  ["lone-surrogate", replaceOnce(pc, '"jurisdiction.vic"', '"jurisdiction.vic\\ud800"'), "shape", "E_PARSE"],
  ["nesting-too-deep", replaceOnce(pc, '  "v": ', `  "note": ${"[".repeat(1000)}${"]".repeat(1000)},\n  "v": `), "shape", "E_PARSE"],
  ["not-an-object", pretty(catalogue.facets), "shape"],
  ["member-missing", pretty({ v: DICT_V, profile: catalogue.profile, families: catalogue.families }), "shape"],
  ["member-unknown", pretty(setFacet(cat(), "record.installation", { label: "Installation" })), "shape"],
  ["prime-not-a-number", pretty(setFacet(cat(), "standard.as-nzs-4777-2", { prime: "7" })), "shape"],
  ["shape-before-version", pretty({ ...cat(), v: "coordinate-dictionary/1", note: "a member the format does not have" }), "shape"],
  ["version", pretty({ ...cat(), v: "coordinate-dictionary/1" }), "version"],
  ["version-before-profile", pretty({ ...cat(), v: "coordinate-dictionary/1", profile: "catalogue-demo" }), "version"],
  ["profile", pretty({ ...cat(), profile: "catalogue-demo" }), "profile"],
  ["profile-leading-zero", pretty({ ...cat(), profile: "catalogue-demo/00" }), "profile"],
  ["profile-trailing-newline", pretty({ ...cat(), profile: "catalogue-demo/0\n" }), "profile"],
  ["family-name", pretty(setFamily(cat(), "jurisdiction", { name: "Jurisdiction" })), "family-name"],
  ["family-name-trailing-newline", pretty(setFamily(cat(), "jurisdiction", { name: "jurisdiction\n" })), "family-name"],
  ["name-before-values", pretty(setFamily(cat(), "standard", { name: "Standard", values: "many" })), "family-name"],
  ["values", pretty(setFamily(cat(), "standard", { values: "many" })), "values"],
  [
    "values-before-duplicate",
    pretty({ ...cat(), families: [...catalogue.families, { name: "product", values: "many" }] }),
    "values",
  ],
  ["family-duplicate", pretty({ ...cat(), families: [...catalogue.families, { name: "product", values: "one" }] }), "family-duplicate"],
  ["family-order", pretty(familiesSwapped()), "family-order"],
  ["families-before-facets", pretty(setFacet(familiesSwapped(), "jurisdiction.nt", { prime: 57 })), "family-order"],
  ["families-before-empty", pretty({ ...familiesSwapped(), facets: [] }), "family-order"],
  ["empty", pretty({ ...cat(), facets: [] }), "empty"],
  [
    "too-many",
    pretty({
      ...wide,
      facets: [{ id: "multi.P2", prime: 2 }, ...wide.facets.slice(1), { id: `flag.p${PRIMES[4096]}`, prime: PRIMES[4096] }],
    }),
    "too-many",
  ],
  ["id-form", pretty(setFacet(cat(), "product.inverter", { id: "product.Inverter" })), "id-form"],
  ["id-form-trailing-newline", pretty(setFacet(cat(), "jurisdiction.vic", { id: "jurisdiction.vic\n" })), "id-form"],
  ["checks-within-a-facet", pretty(setFacet(cat(), "product.inverter", { id: "product.Inverter", prime: 5.5 })), "id-form"],
  ["unknown-family", pretty({ ...cat(), facets: [...catalogue.facets, { id: "phase.single", prime: 71 }] }), "unknown-family"],
  ["id-duplicate", pretty({ ...cat(), facets: [...catalogue.facets, { id: "jurisdiction.vic", prime: 71 }] }), "id-duplicate"],
  ["not-integer", pretty(setFacet(cat(), "jurisdiction.nt", { prime: 59.5 })), "not-integer"],
  ["integer-before-range", pretty({ ...cat(), facets: [...catalogue.facets, { id: "record.extra", prime: 2147483648.5 }] }), "not-integer"],
  ["prime-range-low", pretty(setFacet(cat(), "record.product-listing", { prime: 1 })), "prime-range"],
  ["prime-range-high", pretty(extra(2147483659)), "prime-range"],
  ["range-before-primality", pretty(extra(2 ** 31)), "prime-range"],
  ["range-before-order", pretty({ ...cat(), facets: [...catalogue.facets, { id: "record.extra", prime: 1 }] }), "prime-range"],
  ["not-prime", pretty(setFacet(cat(), "jurisdiction.nt", { prime: 57 })), "not-prime"],
  ["primality-before-order", pretty({ ...cat(), facets: [...catalogue.facets, { id: "record.extra", prime: 9 }] }), "not-prime"],
  [
    "document-order",
    pretty(setFacet(setFacet(cat(), "jurisdiction.nt", { prime: 57 }), "record.inspection", { id: "record.Inspection" })),
    "not-prime",
  ],
  ["prime-duplicate", pretty(setFacet(cat(), "record.inspection", { prime: 59 })), "prime-duplicate"],
  [
    "prime-order",
    pretty({ ...cat(), facets: catalogue.facets.map((f) => ({ 11: catalogue.facets[5], 13: catalogue.facets[4] })[f.prime] ?? f) }),
    "prime-order",
  ],
];
// Appended facets go last, as the spec says; extra() sorts, which these primes
// above 67 keep last as well.
assert(extra(2147483659).facets.at(-1).prime === 2147483659 && extra(2 ** 31).facets.at(-1).prime === 2 ** 31, "appended last");
const seenDetails = new Set();
for (const [name, content, detail, core] of invalidDictionaries) {
  const rel = `${CO}/dictionaries/invalid/${name}.json`;
  write(rel, content);
  const got = outcomeOf(() => cs.parseDictionary(readFileSync(join(ROOT, rel))));
  const want = { code: cs.CODES.DICTIONARY, detail, ...(core ? { core } : {}) };
  assert(got.error && sameResult(got.error, want), `${rel}: want ${canonOf(want)}, got ${JSON.stringify(got.error ?? "ok")}`);
  seenDetails.add(core ? `${detail}/${core}` : detail);
  dictionaryCases.invalid.push({ name, error: got.error });
}
{
  const details = ["shape/E_PARSE", "shape/E_DUPLICATE_KEY", "shape/E_NUMBER_RANGE", "shape", "version", "profile"];
  details.push("family-name", "values", "family-duplicate", "family-order", "empty", "too-many", "id-form");
  details.push("unknown-family", "id-duplicate", "not-integer", "prime-range", "not-prime", "prime-duplicate", "prime-order");
  assert(details.every((d) => seenDetails.has(d)) && seenDetails.size === details.length, "an invalid case for every detail");
}
write(`${CO}/dictionaries.json`, pretty(dictionaryCases));

// --- Operation cases

// Records a case: `fields` in file order, then the reference's `expect` or
// `error`. `want`, where given, is the result the spec states.
const opFile = (file) => {
  const cases = [];
  const names = new Set();
  return {
    cases,
    add(name, fields, run, want) {
      assert(!names.has(name), `${file}: one case named ${name}`);
      names.add(name);
      const got = outcomeOf(run);
      if (want !== undefined) {
        assert(sameResult(got, want), `${file} ${name}: want ${canonOf(want)}, got ${canonOf(got)}`);
      }
      cases.push({ name, ...fields, ...got });
      return got;
    },
  };
};
const facetsOf = (decoded) => cs.expand(decoded);
const multiset = (ids) => canonOf([...ids].sort());
const facetList = (dictionary, entries) => entries.map(([id, exponent]) => ({ id, exponent }));

const encodeCases = opFile("encode.json");
const encodeCase = (name, dictionary, facets, want) =>
  encodeCases.add(name, { dictionary, facets }, () => cs.encode(D[dictionary], facets), want);
const decodeCases = opFile("decode.json");
const decodeCase = (name, dictionary, coordinate, want) =>
  decodeCases.add(name, { dictionary, coordinate }, () => cs.decode(D[dictionary], coordinate), want);
const dividesCases = opFile("divides.json");
const dividesCase = (name, dictionary, query, coordinate, want) =>
  dividesCases.add(name, { dictionary, query, coordinate }, () => cs.divides(D[dictionary], query, coordinate), want);
const checkCases = opFile("check.json");
const checkCase = (name, dictionary, facets, coordinate, want) =>
  checkCases.add(name, { dictionary, facets, coordinate }, () => cs.check(D[dictionary], facets, coordinate), want);
const extendsCases = opFile("extends.json");
const extendsCase = (name, old, next, want) =>
  extendsCases.add(name, { old, new: next }, () => cs.extends(D[old], D[next]), want);

const FOUR = ["record.installation", "product.inverter", "standard.as-nzs-4777-2", "jurisdiction.vic"];
const FOUR_DECODED = FOUR.map((id) => ({ id, exponent: 1 }));
const K16 = [...KERNEL_IDS.slice(0, 10), ...["entry", "exit", "unclassified", "quantified", "transfer", "locked"].map((s) => `measurement.${s}`)];
const K20 = [...K16, ...["committed", "unit", "linked", "executed"].map((s) => `measurement.${s}`)];
assert(JSON.stringify(K20) === JSON.stringify(KERNEL_IDS.slice(0, 20)), "the first 20 kernel facets");
const ones = (ids) => ids.map((id) => ({ id, exponent: 1 }));

// Kernel/2 section 5: the events, and S and H after each, from the document.
// S applies the events oldest first: an exit removes its block's entry and
// states, anything else is added. No event there both exits a block and adds
// to it, so the order of the facets within an event does not matter here.
const section5 = docTable(docSection("## 5. ", "## 6. "), "| Event |").slice(2);
assert(section5.length === 5, "Kernel/2 section 5 has five events");
const trace = [];
{
  const S = new Map();
  const H = [];
  for (const [i, row] of section5.entries()) {
    const add = row[1].split(",").map((c) => unquote(c.trim()));
    const blockOf = (id) => id.slice(0, id.indexOf("."));
    for (const id of add) {
      assert(KERNEL_IDS.includes(id), `Kernel/2 section 5: ${id} is a facet`);
      if (id.endsWith(".exit")) {
        assert(add.filter((x) => blockOf(x) === blockOf(id)).length === 1, "an exit is its block's only facet in an event");
      }
    }
    for (const id of add) {
      H.push(id);
      if (id.endsWith(".exit")) {
        for (const k of [...S.keys()]) if (blockOf(k) === blockOf(id)) S.delete(k);
      } else {
        S.set(id, true);
      }
    }
    trace.push({ event: `e${i + 1}`, S: [...S.keys()], H: [...H], wantS: unquote(row[2]), wantH: unquote(row[3]) });
  }
}
const H_E5 = "1144592878288235028923541329666";
assert(trace[4].wantH === H_E5, "the spec's H after E5 is Kernel/2's");

// encode.json
encodeCase("empty", "catalogue-demo", [], V("1"));
encodeCase("catalogue-example", "catalogue-demo", FOUR, V("1155"));
encodeCase("list-order", "catalogue-demo", [...FOUR].reverse(), V("1155"));
encodeCase("set-family", "catalogue-demo", ["standard.as-nzs-4777-2", "standard.as-nzs-5033", "standard.as-nzs-3000"], V("3059"));
encodeCase("unknown-facet", "catalogue-demo", ["product.heat-pump"], E("UNKNOWN_FACET", { facet: "product.heat-pump" }));
encodeCase(
  "unknown-facet-trailing-newline",
  "catalogue-demo",
  ["jurisdiction.vic\n"],
  E("UNKNOWN_FACET", { facet: "jurisdiction.vic\n" }),
);
encodeCase(
  "first-unknown-in-list-order",
  "catalogue-demo",
  ["jurisdiction.vic", "product.heat-pump", "phase.single"],
  E("UNKNOWN_FACET", { facet: "product.heat-pump" }),
);
encodeCase(
  "unknown-before-cardinality",
  "catalogue-demo",
  ["jurisdiction.vic", "jurisdiction.nsw", "product.heat-pump"],
  E("UNKNOWN_FACET", { facet: "product.heat-pump" }),
);
encodeCase("one-two-facets", "catalogue-demo", ["jurisdiction.vic", "jurisdiction.nsw"], E("CARDINALITY", { family: "jurisdiction" }));
encodeCase("one-repeated", "catalogue-demo", times(2, "jurisdiction.vic"), E("CARDINALITY", { family: "jurisdiction" }));
encodeCase("set-repeated", "catalogue-demo", times(2, "standard.as-nzs-5033"), E("CARDINALITY", { family: "standard" }));
encodeCase(
  "first-family-in-families-order",
  "catalogue-demo",
  ["record.product-listing", "record.installation", "jurisdiction.vic", "jurisdiction.nsw"],
  E("CARDINALITY", { family: "jurisdiction" }),
);
encodeCase(
  "two-kinds-broken",
  "catalogue-demo",
  [...times(2, "standard.as-nzs-4777-2"), "product.inverter", "product.battery"],
  E("CARDINALITY", { family: "product" }),
);
encodeCase("new-facets", "catalogue-demo-v2", ["product.heat-pump", "phase.single"], V("5183"));
encodeCase("multiset-repeated", "kernel-2", times(2, "universal.entry"), V("4"));
for (const t of trace) encodeCase(`section-5-s-${t.event}`, "kernel-2", t.S, V(t.wantS));
for (const t of trace) encodeCase(`section-5-h-${t.event}`, "kernel-2", t.H, V(t.wantH));
encodeCase("first-16-primes", "kernel-2", K16, V(P16));
encodeCase("first-20-primes", "kernel-2", K20, V(P20));
const all70 = productOf(PRIMES.slice(0, 70));
assert(all70.toString(2).length === 467, "the product of the first 70 primes has 467 bits");
encodeCase("all-70", "kernel-2", KERNEL_IDS, V(all70.toString()));
// Round trips: 1 to 40 kernel facets, repeats allowed, from splitmix64 with a
// fixed seed of its own.
{
  let seed = createHash("sha256").update("coordinate/0 round-trip cases").digest().readBigUInt64BE(0);
  const draw = () => {
    seed = (seed + 0x9e3779b97f4a7c15n) & MASK;
    let z = seed;
    z = ((z ^ (z >> 30n)) * 0xbf58476d1ce4e5b9n) & MASK;
    z = ((z ^ (z >> 27n)) * 0x94d049bb133111ebn) & MASK;
    return z ^ (z >> 31n);
  };
  for (let i = 1; i <= 100; i++) {
    const n = 1 + Number(draw() % 40n);
    const facets = Array.from({ length: n }, () => KERNEL_IDS[Number(draw() % 70n)]);
    const product = productOf(facets.map((id) => kernel.facets.find((f) => f.id === id).prime));
    encodeCase(`round-trip-${String(i).padStart(3, "0")}`, "kernel-2", facets, V(product.toString()));
  }
}
encodeCase("2^1023", "wide-demo", times(1023, "multi.p2"), V(pow2(1023)));
encodeCase("2^1024", "wide-demo", times(1024, "multi.p2"), E("RANGE"));
const flagsTo = (last) => wide.facets.filter((f) => f.prime >= 59 && f.prime <= last).map((f) => f.id);
assert(flagsTo(787).length === 122 && flagsTo(797).length === 123, "122 flag facets to 787, 123 to 797");
const below = productOf(flagsTo(787).map((id) => Number(id.slice(6))));
assert(below.toString(2).length === 1021 && below * 797n >= cs.LIMIT, "59 to 787 is 1,021 bits, and 797 more passes 2^1024");
encodeCase("set-family-below-range", "wide-demo", flagsTo(787), V(below.toString()));
encodeCase("set-family-range", "wide-demo", flagsTo(797), E("RANGE"));
encodeCase(
  "cardinality-before-range",
  "wide-demo",
  [...times(1024, "multi.p2"), ...times(2, "flag.p59")],
  E("CARDINALITY", { family: "flag" }),
);
// The nearest coordinates below and above 2^53, 2^63 and 2^64 that decode
// without error under wide-demo, found by testing 2^k - 1, 2^k - 2, ... and
// 2^k + 1, 2^k + 2, ... in turn (the spec lists them).
const nearest = {};
for (const k of [53, 63, 64]) {
  for (const [side, step] of [
    ["below", -1n],
    ["above", 1n],
  ]) {
    let n = (1n << BigInt(k)) + step;
    while (outcomeOf(() => cs.decode(D["wide-demo"], n.toString())).error) n += step;
    nearest[`${side}-2^${k}`] = n.toString();
  }
}
assert(
  canonOf(Object.values(nearest)) ===
    canonOf([
      "9007199254740990",
      "9007199254741096",
      "9223372036854775800",
      "9223372036854776017",
      "18446744073709551600",
      "18446744073709552034",
    ]),
  `the nearest coordinates are those the spec lists: ${JSON.stringify(nearest)}`,
);
for (const [name, coordinate] of Object.entries(nearest)) {
  encodeCase(name, "wide-demo", facetsOf(cs.decode(D["wide-demo"], coordinate)), V(coordinate));
}
encodeCase("max-prime", "max-prime-demo", ["big.p2147483647"], V("2147483647"));

// decode.json
decodeCase("one", "catalogue-demo", "1", V([]));
decodeCase("catalogue-example", "catalogue-demo", "1155", V(FOUR_DECODED));
for (const [name, coordinate] of [
  ["syntax-zero", "0"],
  ["syntax-leading-zero", "01"],
  ["syntax-plus", "+5"],
  ["syntax-minus", "-5"],
  ["syntax-leading-space", " 5"],
  ["syntax-trailing-space", "5 "],
  ["syntax-trailing-newline", "5\n"],
  ["syntax-fraction", "5.0"],
  ["syntax-exponent", "1e3"],
  ["syntax-empty", ""],
  ["syntax-fullwidth-digit", String.fromCharCode(0xff15)], // FULLWIDTH DIGIT FIVE
  ["syntax-number", 5],
  ["syntax-before-range", `0${"1".repeat(399)}`],
  ["syntax-late-in-long-string", `1${"0".repeat(400)}x`],
]) {
  decodeCase(name, "catalogue-demo", coordinate, E("SYNTAX"));
}
decodeCase("unknown-factor", "catalogue-demo", "97", E("UNKNOWN_FACTOR"));
decodeCase("new-facet-under-old-version", "catalogue-demo", "71", E("UNKNOWN_FACTOR"));
assert(11 * 31 * 97 === 33077 && 2 * 3 * 11 * 31 === 2046 && 5 * 7 * 7 * 13 === 3185, "the products in the case names");
decodeCase("unknown-factor-before-cardinality", "catalogue-demo", "33077", E("UNKNOWN_FACTOR"));
decodeCase("one-two-facets", "catalogue-demo", "341", E("CARDINALITY", { family: "jurisdiction" }));
decodeCase("one-squared", "catalogue-demo", "121", E("CARDINALITY", { family: "jurisdiction" }));
decodeCase("set-squared", "catalogue-demo", "49", E("CARDINALITY", { family: "standard" }));
decodeCase("first-family-in-families-order", "catalogue-demo", "2046", E("CARDINALITY", { family: "jurisdiction" }));
decodeCase("two-kinds-broken", "catalogue-demo", "3185", E("CARDINALITY", { family: "product" }));
decodeCase("new-facet", "catalogue-demo-v2", "71", V(ones(["product.heat-pump"])));
decodeCase("old-coordinate-under-new-version", "catalogue-demo-v2", "1155", V(FOUR_DECODED));
decodeCase("unclassified-as-a-whole", "kernel-2", "1", V([]));
decodeCase("universal-unclassified", "kernel-2", "5", V(ones(["universal.unclassified"])));
decodeCase("multiset-squared", "kernel-2", "4", V(facetList(kernel, [["universal.entry", 2]])));
decodeCase("sqlite-65-bit", "kernel-2", P16, V(ones(K16)));
decodeCase("sqlite-89-bit", "kernel-2", P20, V(ones(K20)));
decodeCase(
  "section-5-h-e5",
  "kernel-2",
  H_E5,
  V(cs.decode(D["kernel-2"], cs.encode(D["kernel-2"], trace[4].H))),
);
assert(multiset(facetsOf(cs.decode(D["kernel-2"], H_E5))) === multiset(trace[4].H), "H after E5 decodes to its facets");
decodeCase("prime-after-dictionary", "kernel-2", "353", E("UNKNOWN_FACTOR"));
for (const c of encodeCases.cases.filter((c) => c.name.startsWith("round-trip-"))) {
  decodeCase(c.name, "kernel-2", c.expect);
}
for (const k of [53, 63, 64]) decodeCase(`2^${k}`, "wide-demo", pow2(k), V(facetList(wide, [["multi.p2", k]])));
for (const k of [53, 63, 64]) {
  decodeCase(`2^${k}-minus-1`, "wide-demo", ((1n << BigInt(k)) - 1n).toString(), E("UNKNOWN_FACTOR"));
  decodeCase(`2^${k}-plus-1`, "wide-demo", ((1n << BigInt(k)) + 1n).toString(), E("UNKNOWN_FACTOR"));
}
for (const [name, coordinate] of Object.entries(nearest)) decodeCase(name, "wide-demo", coordinate);
decodeCase("2^1023", "wide-demo", pow2(1023), V(facetList(wide, [["multi.p2", 1023]])));
const limitMinus1 = (cs.LIMIT - 1n).toString();
assert(limitMinus1.length === 309 && cs.LIMIT_DIGITS === 309, "2^1024 - 1 and 2^1024 have 309 digits");
decodeCase("2^1024-minus-1", "wide-demo", limitMinus1, E("UNKNOWN_FACTOR"));
decodeCase("2^1024", "wide-demo", cs.LIMIT.toString(), E("RANGE"));
decodeCase("2^1024-plus-1", "wide-demo", (cs.LIMIT + 1n).toString(), E("RANGE"));
decodeCase(
  "10^308",
  "wide-demo",
  `1${"0".repeat(308)}`,
  V(facetList(wide, [
    ["multi.p2", 308],
    ["multi.p5", 308],
  ])),
);
decodeCase("2x10^308", "wide-demo", `2${"0".repeat(308)}`, E("RANGE"));
decodeCase("309-nines", "wide-demo", "9".repeat(309), E("RANGE"));
decodeCase("10^309", "wide-demo", `1${"0".repeat(309)}`, E("RANGE"));
decodeCase("set-squared-wide", "wide-demo", "3481", E("CARDINALITY", { family: "flag" }));
decodeCase("max-prime", "max-prime-demo", "2147483647", V(ones(["big.p2147483647"])));
assert((2147483647n ** 2n).toString() === "4611686014132420609", "(2^31 - 1)^2");
decodeCase("max-prime-squared", "max-prime-demo", "4611686014132420609", V(facetList(maxPrime, [["big.p2147483647", 2]])));
decodeCase("unknown-small-factor", "max-prime-demo", "2", E("UNKNOWN_FACTOR"));
{
  const one = decodeCases.cases.find((c) => c.name === "unclassified-as-a-whole").expect;
  const five = decodeCases.cases.find((c) => c.name === "universal-unclassified").expect;
  assert(canonOf(one) !== canonOf(five), "REQ-KRN-011: \"1\" and \"5\" decode differently");
}

// divides.json
const big400 = `1${"0".repeat(400)}`;
dividesCase("one-divides-one", "catalogue-demo", "1", "1", V(true));
dividesCase("one-divides-everything", "catalogue-demo", "1", "1155", V(true));
dividesCase("query-matches", "catalogue-demo", "231", "1155", V(true));
dividesCase("query-does-not-match", "catalogue-demo", "231", "3255", V(false));
dividesCase("query-equals-coordinate", "catalogue-demo", "1155", "1155", V(true));
dividesCase("query-unknown-factor", "catalogue-demo", "97", "1155", E("UNKNOWN_FACTOR"));
dividesCase("query-cardinality", "catalogue-demo", "341", "1155", E("CARDINALITY", { family: "jurisdiction" }));
dividesCase("query-syntax", "catalogue-demo", "01", "1155", E("SYNTAX"));
dividesCase("query-trailing-newline", "catalogue-demo", "231\n", "1155", E("SYNTAX"));
dividesCase("coordinate-unknown-factor", "catalogue-demo", "1", "97", E("UNKNOWN_FACTOR"));
dividesCase("coordinate-syntax", "catalogue-demo", "231", 1155, E("SYNTAX"));
dividesCase("coordinate-syntax-late", "catalogue-demo", "1", `1${"0".repeat(400)}x`, E("SYNTAX"));
dividesCase("query-error-first", "catalogue-demo", "-5", big400, E("SYNTAX"));
// The SQLite cases must divide: 53 is the 16th prime, a factor of p16 and p20.
dividesCase("sqlite-65-bit", "kernel-2", "53", P16, V(true));
dividesCase("sqlite-89-bit", "kernel-2", "53", P20, V(true));
assert(BigInt(P16) % 53n === 0n && BigInt(P20) % 53n === 0n && BigInt(P16) % 59n !== 0n, "53 divides p16 and p20");
dividesCase("sqlite-65-bit-absent", "kernel-2", "59", P16, V(false));
dividesCase("exponent-short", "kernel-2", "4", "2", V(false));
dividesCase("exponent-enough", "kernel-2", "4", "8", V(true));
dividesCase("past-2^63", "wide-demo", pow2(63), pow2(64), V(true));
dividesCase("past-2^63-reversed", "wide-demo", pow2(64), pow2(63), V(false));
dividesCase("largest", "wide-demo", "2", pow2(1023), V(true));

// check.json
checkCase("ok", "catalogue-demo", FOUR, "1155", V(true));
checkCase("ok-list-order", "catalogue-demo", [...FOUR].reverse(), "1155", V(true));
checkCase("ok-empty", "catalogue-demo", [], "1", V(true));
checkCase("drift-value", "catalogue-demo", FOUR, "1156", E("DRIFT"));
checkCase("drift-not-canonical", "catalogue-demo", FOUR, "01155", E("DRIFT"));
checkCase("drift-number", "catalogue-demo", FOUR, 1155, E("DRIFT"));
checkCase("drift-unclassified", "catalogue-demo", ["jurisdiction.vic"], "1", E("DRIFT"));
checkCase("unknown-facet-first", "catalogue-demo", ["product.heat-pump"], "71", E("UNKNOWN_FACET", { facet: "product.heat-pump" }));
checkCase(
  "cardinality-first",
  "catalogue-demo",
  ["jurisdiction.vic", "jurisdiction.nsw"],
  "341",
  E("CARDINALITY", { family: "jurisdiction" }),
);
checkCase("range-first", "wide-demo", times(1024, "multi.p2"), "1", E("RANGE"));
checkCase("section-5-h-e5", "kernel-2", trace[4].H, H_E5, V(true));

// extends.json
extendsCase("same", "catalogue-demo", "catalogue-demo", V(true));
extendsCase("same-reordered", "catalogue-demo", "catalogue-demo-reordered", V(true));
extendsCase("added", "catalogue-demo", "catalogue-demo-v2", V(true));
extendsCase("family-removed", "catalogue-demo-v2", "catalogue-demo", E("NOT_EXTENSION", { detail: "family" }));
extendsCase("profile", "catalogue-demo", "kernel-2", E("NOT_EXTENSION", { detail: "profile" }));
extendsCase("values-changed", "catalogue-demo", "catalogue-demo-values-changed", E("NOT_EXTENSION", { detail: "family" }));
extendsCase("prime-changed", "catalogue-demo", "catalogue-demo-prime-changed", E("NOT_EXTENSION", { detail: "facet" }));
extendsCase("facet-removed", "catalogue-demo", "catalogue-demo-facet-removed", E("NOT_EXTENSION", { detail: "facet" }));

// Round trips: every decode case with `expect` encodes back to its coordinate,
// and every encode case with `expect` decodes back to its facets.
for (const c of decodeCases.cases.filter((c) => "expect" in c)) {
  assert(cs.encode(D[c.dictionary], facetsOf(c.expect)) === c.coordinate, `decode ${c.name} encodes back`);
}
for (const c of encodeCases.cases.filter((c) => "expect" in c)) {
  assert(multiset(facetsOf(cs.decode(D[c.dictionary], c.expect))) === multiset(c.facets), `encode ${c.name} decodes back`);
}
// Every case: a name, the members of its file, and exactly one of expect and error.
const valid = new Set(dictionaryCases.valid.map((c) => c.name));
for (const { cases } of [encodeCases, decodeCases, dividesCases, checkCases, extendsCases]) {
  for (const c of cases) {
    assert(("expect" in c) !== ("error" in c), `${c.name}: exactly one of expect and error`);
    for (const k of ["dictionary", "old", "new"]) if (k in c) assert(valid.has(c[k]), `${c.name}: ${k} is a valid dictionary`);
  }
}

write(`${CO}/encode.json`, pretty(encodeCases.cases));
write(`${CO}/decode.json`, pretty(decodeCases.cases));
write(`${CO}/divides.json`, pretty(dividesCases.cases));
write(`${CO}/check.json`, pretty(checkCases.cases));
write(`${CO}/extends.json`, pretty(extendsCases.cases));

const coordinateCounts = {
  dictionaries: `${dictionaryCases.valid.length} valid and ${dictionaryCases.invalid.length} invalid dictionaries`,
  encode: encodeCases.cases.length,
  decode: decodeCases.cases.length,
  divides: dividesCases.cases.length,
  check: checkCases.cases.length,
  extends: extendsCases.cases.length,
};
console.log(
  `wrote ${coordinateCounts.dictionaries}, ${coordinateCounts.encode} encode, ${coordinateCounts.decode} decode, ` +
    `${coordinateCounts.divides} divides, ${coordinateCounts.check} check and ${coordinateCounts.extends} extends cases ` +
    `to ${join(ROOT, CO)}`,
);
