// Generates test-vectors/ from the JavaScript reference implementation.
// Deterministic: running it twice produces identical files, and CI checks that
// the committed vectors match a fresh run.
//
//   node tools/make-vectors.mjs

import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { createHash } from "node:crypto";
import * as ev from "./reference/evidence.mjs";
import * as log from "./reference/log.mjs";

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

const keyNames = ["rec-registry-cdc", "photo-check-agent", "assessor-01"];
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
// Built with tools/reference/log.mjs. Only entries.json and merkle.json are
// written here; nothing else under store/ is touched.

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

// The demo store key, derived like the other test keys. Its seed file is not
// written yet; the export vectors will add keys/store-demo.seed.
const storeSeed = createHash("sha256").update("evidence/0 test-vector key: store-demo").digest("hex");
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
