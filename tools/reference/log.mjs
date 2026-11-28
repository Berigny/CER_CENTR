// Independent reference implementation of store/0 (spec/store-0.md), in
// JavaScript with no dependencies (Node >= 20). Like evidence.mjs, it exists so
// the Rust crates are checked against a second implementation written from the
// spec, not against themselves.
//
// Scope: log-entry/0 (build, canonical form, entry hash, strict validation),
// segment lines, the RFC 9162 Merkle tree (tree hash, inclusion and consistency
// proofs), the genesis and checkpoint objects, lanes and the heads fold, the
// two-clock rules, the evidence/0 checks a log needs (object steps 1-5 and the
// attestation/0 shape), the segments/0 manifest, the streaming verifier and its
// store-report/0, verifyExport, and a writer that replays an inputs.json
// operation script into an export.
//
// Errors are EvidenceError (from evidence.mjs) with an S_ or E_ code. Merkle
// proof verification never throws: it answers true or false. Verification never
// throws on untrusted input either: whatever it is given, it returns a report.

import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import {
  canonicalize,
  digest,
  EvidenceError,
  MAX_DEPTH,
  objectId,
  parseStrict,
  publicKey,
  sha256,
  signAttestation,
  verifyAttestation,
} from "./evidence.mjs";

export const STORE_SPEC = "store/0";
const MAX_SAFE = Number.MAX_SAFE_INTEGER; // 2^53 - 1

const HASH_RE = /^sha256:[0-9a-f]{64}$/;
const isHash = (v) => typeof v === "string" && HASH_RE.test(v);
const isPosition = (v) => Number.isInteger(v) && v >= 1 && v <= MAX_SAFE;
const isPlainObject = (v) => v !== null && typeof v === "object" && !Array.isArray(v);
const fail = (code, detail) => {
  throw new EvidenceError(code, detail);
};

// Members of an object in insertion order, compared as a set against `names`.
function hasExactly(obj, names) {
  const keys = Object.keys(obj);
  return keys.length === names.length && names.every((n) => Object.hasOwn(obj, n));
}

// How deep arrays and objects may nest: evidence/0's limit (evidence.mjs's
// MAX_DEPTH). A segment line wraps its entry and item in one more object, so a
// line may nest one level deeper, and every object that evidence/0 accepts
// still fits in a line.
export { MAX_DEPTH };
export const MAX_LINE_DEPTH = MAX_DEPTH + 1;

// evidence/0's strict parse of JSON text: evidence.mjs's parseStrict, with its
// fixed order of step-1 codes. Exported for coordinate.mjs, which loads
// dictionaries with the same rules.
export function parseStrictJson(text, maxDepth = MAX_DEPTH) {
  return parseStrict(text, maxDepth);
}

// Byte order of the UTF-8 encodings, which is how Rust orders `String`s.
// JavaScript's default sort uses UTF-16 code units, which differs above U+FFFF.
export function compareUtf8(a, b) {
  return Buffer.compare(Buffer.from(a, "utf8"), Buffer.from(b, "utf8"));
}

// --- Schemas -------------------------------------------------------------------
// The hashes are computed from the bytes of the published schema files, which
// are authoritative.

const schemaHash = (file) => sha256(readFileSync(new URL(`../../spec/schema/${file}`, import.meta.url)));

export const GENESIS_SCHEMA_REF = "evidence-store/genesis@0";
export const GENESIS_SCHEMA_HASH = schemaHash("store-genesis-0.schema.json");
export const CHECKPOINT_SCHEMA_REF = "evidence-store/checkpoint@0";
export const CHECKPOINT_SCHEMA_HASH = schemaHash("checkpoint-0.schema.json");

// --- Timestamps and the two clocks ---------------------------------------------
// A timestamp is { secs, nanos }: whole seconds since 1970-01-01T00:00:00Z and
// nanoseconds within that second. Years 0000-9999 keep secs well inside the
// range where a JavaScript number is exact.

const TS_RE = /^([0-9]{4})-([0-9]{2})-([0-9]{2})T([0-9]{2}):([0-9]{2}):([0-9]{2})(?:\.([0-9]{1,9}))?Z$/;

const isLeapYear = (y) => (y % 4 === 0 && y % 100 !== 0) || y % 400 === 0;
function daysInMonth(y, m) {
  if (m === 2) return isLeapYear(y) ? 29 : 28;
  return [4, 6, 9, 11].includes(m) ? 30 : 31;
}

// Days since 1970-01-01 of a proleptic Gregorian date, counted directly
// (no Date object: Date.UTC maps years 0-99 to 1900-1999).
function daysSinceEpoch(y, m, d) {
  const before = y - 1; // whole years before year y, counted from year 1
  const leapDaysBefore = (n) => Math.floor(n / 4) - Math.floor(n / 100) + Math.floor(n / 400);
  // Days from 0001-01-01 to y-01-01, then to the date, then shift to 1970.
  let days = before * 365 + leapDaysBefore(before);
  for (let month = 1; month < m; month++) days += daysInMonth(y, month);
  days += d - 1;
  const EPOCH_FROM_YEAR_1 = 719162; // days from 0001-01-01 to 1970-01-01
  return days - EPOCH_FROM_YEAR_1;
}

function dateOfDays(days) {
  // Inverse of daysSinceEpoch, by stepping years then months. At most 10,000
  // years, so a simple walk is fast enough for a reference implementation.
  let y = 1970;
  let rem = days;
  if (rem >= 0) {
    for (;;) {
      const len = isLeapYear(y) ? 366 : 365;
      if (rem < len) break;
      rem -= len;
      y += 1;
    }
  } else {
    while (rem < 0) {
      y -= 1;
      rem += isLeapYear(y) ? 366 : 365;
    }
  }
  let m = 1;
  while (rem >= daysInMonth(y, m)) {
    rem -= daysInMonth(y, m);
    m += 1;
  }
  return [y, m, rem + 1];
}

// Parses an evidence/0 timestamp, or returns null if it does not match the
// pattern or is not a real calendar instant (30 February, hour 24, second 60).
export function parseTimestamp(text) {
  if (typeof text !== "string") return null;
  const m = TS_RE.exec(text);
  if (!m) return null;
  const [year, month, day, hour, minute, second] = m.slice(1, 7).map(Number);
  if (month < 1 || month > 12 || day < 1 || day > daysInMonth(year, month)) return null;
  if (hour > 23 || minute > 59 || second > 59) return null;
  const nanos = m[7] === undefined ? 0 : Number(m[7].padEnd(9, "0"));
  const secs = daysSinceEpoch(year, month, day) * 86400 + hour * 3600 + minute * 60 + second;
  return { secs, nanos };
}

export const compareTimestamps = (a, b) => (a.secs - b.secs) || (a.nanos - b.nanos);

const later = (a, b) => (compareTimestamps(a, b) >= 0 ? a : b);

// The same instant with nanos below one second. parseTimestamp never returns
// anything else, but a hand-built { secs, nanos } might; whole seconds in nanos
// are carried into secs, as the Rust clock module does.
const normalize = (t) => ({ secs: t.secs + Math.floor(t.nanos / 1_000_000_000), nanos: t.nanos % 1_000_000_000 });

// Drops the digits after the milliseconds; never rounds up.
export function truncateMillis(t) {
  const n = normalize(t);
  return { secs: n.secs, nanos: n.nanos - (n.nanos % 1_000_000) };
}

// The first whole millisecond at or after t. A time the store wrote itself is
// already a whole millisecond, so this leaves it unchanged. Only an earlier time
// with more than 3 fraction digits (from an imported log, or an object a writer
// dated) moves, so the result is never below it and is still exact in 3 digits.
function ceilMillis(timestamp) {
  const t = normalize(timestamp);
  const down = truncateMillis(t);
  if (down.nanos === t.nanos) return t;
  const nanos = down.nanos + 1_000_000;
  return nanos >= 1_000_000_000 ? { secs: t.secs + 1, nanos: nanos - 1_000_000_000 } : { secs: t.secs, nanos };
}

// received_at for a new entry: the clock truncated to milliseconds, or the last
// entry's received_at (rounded up to a whole millisecond) if that is later, so
// received_at never decreases, even once formatted with 3 digits.
export function nextReceived(now, last) {
  const t = truncateMillis(now);
  return last ? later(t, ceilMillis(last)) : t;
}

// `at` for an object the store authors in a chain: the clock truncated to
// milliseconds, or the latest `at` among its prev objects (each rounded up to a
// whole millisecond) if that is later.
export function serviceAt(now, prevAts = []) {
  return prevAts.map(ceilMillis).reduce(later, truncateMillis(now));
}

// YYYY-MM-DDTHH:MM:SS.mmmZ, exactly 3 fraction digits (truncated).
export function formatMillis(timestamp) {
  const t = normalize(timestamp);
  const days = Math.floor(t.secs / 86400);
  const rem = t.secs - days * 86400;
  const [y, mo, d] = dateOfDays(days);
  const p = (n, w = 2) => String(n).padStart(w, "0");
  const ms = Math.floor(t.nanos / 1_000_000);
  const time = `${p(Math.floor(rem / 3600))}:${p(Math.floor(rem / 60) % 60)}:${p(rem % 60)}`;
  return `${p(y, 4)}-${p(mo)}-${p(d)}T${time}.${p(ms, 3)}Z`;
}

// --- Log entry (log-entry/0) ----------------------------------------------------

const utf8 = new TextDecoder("utf-8", { fatal: true, ignoreBOM: true });

// A string as it is, or bytes decoded as strict UTF-8 (a BOM is kept, so the
// strict parse rejects it); `bad` is called if they are not UTF-8.
function toText(input, bad) {
  if (typeof input === "string") return input;
  try {
    return utf8.decode(input);
  } catch {
    return bad("not UTF-8");
  }
}

export const ENTRY_V = "log-entry/0";
export const KINDS = ["object", "attestation"];
const ENTRY_MEMBERS = ["v", "log", "seq", "kind", "ref", "received_at", "epoch", "prev"];

// Checks that `value` is a valid log-entry/0 on its own terms: exactly the eight
// members, each of its type and pattern, and a real calendar received_at.
// Returns the entry with its members in spec order; throws S_ENTRY_INVALID.
// Rules relating an entry to its neighbours (dense seq, chain, regressions,
// seq-1 equalities) are not checked here.
export function validateEntry(value) {
  const bad = (why) => fail("S_ENTRY_INVALID", why);
  if (!isPlainObject(value)) bad("an entry is a JSON object");
  for (const k of Object.keys(value)) if (!ENTRY_MEMBERS.includes(k)) bad(`unknown member ${JSON.stringify(k)}`);
  for (const k of ENTRY_MEMBERS) if (!Object.hasOwn(value, k)) bad(`missing member ${k}`);
  const { v, log, seq, kind, ref, received_at, epoch, prev } = value;
  if (v !== ENTRY_V) bad(`v is not ${ENTRY_V}`);
  if (!isHash(log)) bad("log is not a hash");
  if (!isPosition(seq)) bad("seq is not an integer from 1 to 2^53 - 1");
  if (!KINDS.includes(kind)) bad(`kind is not one of ${KINDS.join(", ")}`);
  if (!isHash(ref)) bad("ref is not a hash");
  if (!parseTimestamp(received_at)) bad("received_at is not a UTC timestamp at a real instant");
  if (!isPosition(epoch)) bad("epoch is not an integer from 1 to 2^53 - 1");
  if (!isHash(prev)) bad("prev is not a hash");
  return { v, log, seq, kind, ref, received_at, epoch, prev };
}

// Parses entry JSON text (a string or UTF-8 bytes). Anything that is not a
// valid entry, including text that is not strict JSON (duplicate members and
// nesting deeper than MAX_DEPTH included), is S_ENTRY_INVALID.
export function parseEntry(input) {
  const text = toText(input, (why) => fail("S_ENTRY_INVALID", why));
  let value;
  try {
    value = parseStrictJson(text);
  } catch (e) {
    if (e instanceof EvidenceError) fail("S_ENTRY_INVALID", `not strict JSON (${e.code})`);
    throw e;
  }
  return validateEntry(value);
}

// Builds a valid entry from its members; throws S_ENTRY_INVALID on bad input.
export function makeEntry({ log, seq, kind, ref, received_at, epoch, prev }) {
  return validateEntry({ v: ENTRY_V, log, seq, kind, ref, received_at, epoch, prev });
}

export const entryCanonical = (entry) => canonicalize(entry);
export const entryHash = (entry) => sha256(Buffer.from(entryCanonical(entry), "utf8"));

// --- Segment lines ---------------------------------------------------------------

// The ref of an attestation: sha256 of its canonical form, sig included.
export const attestationRef = (attestation) => digest(attestation);

// One segment line without its LF. Members sort as written: "attestation" <
// "entry" < "object", so the result is already canonical.
export function renderLine(entryCanonicalText, kind, bodyCanonicalText) {
  if (kind === "object") return `{"entry":${entryCanonicalText},"object":${bodyCanonicalText}}`;
  if (kind === "attestation") return `{"attestation":${bodyCanonicalText},"entry":${entryCanonicalText}}`;
  throw new Error(`unknown kind ${kind}`);
}

// Line checks a, b and b2 of spec/store-0.md: strict JSON nesting at most
// MAX_LINE_DEPTH deep, with exactly `entry` and one body member, byte-identical
// to its canonical form (S_LINE_INVALID); a valid entry (S_ENTRY_INVALID); and
// the body member that entry.kind names (S_LINE_INVALID). Accepts a string or
// UTF-8 bytes (no trailing LF). Returns { entry, entryCanonical, kind, body }.
export function parseLine(input) {
  const bad = (why) => fail("S_LINE_INVALID", why);
  const text = toText(input, bad);
  let value;
  try {
    value = parseStrictJson(text, MAX_LINE_DEPTH);
  } catch (e) {
    if (e instanceof EvidenceError) bad(`not strict JSON (${e.code})`);
    throw e;
  }
  if (!isPlainObject(value)) bad("a line is a JSON object");
  const bodyKind = hasExactly(value, ["entry", "object"])
    ? "object"
    : hasExactly(value, ["entry", "attestation"])
      ? "attestation"
      : bad("members are not entry and one of object or attestation");
  if (canonicalize(value) !== text) bad("not in canonical form");
  const entry = validateEntry(value.entry);
  if (entry.kind !== bodyKind) bad(`entry.kind is ${entry.kind} but the line holds ${bodyKind}`);
  return { entry, entryCanonical: entryCanonical(entry), kind: bodyKind, body: value[bodyKind] };
}

// --- Merkle tree (RFC 9162 section 2.1) ----------------------------------------
// Digests are 32-byte Buffers. Indices and sizes are non-negative integers no
// larger than 2^53 - 1 (numbers; bigints are accepted by the verifiers).

const H = (...parts) => createHash("sha256").update(Buffer.concat(parts)).digest();
const LEAF = Buffer.from([0x00]);
const NODE = Buffer.from([0x01]);

export const leafHash = (leafBytes) => H(LEAF, Buffer.from(leafBytes));
export const nodeHash = (left, right) => H(NODE, left, right);

export const digestToText = (d) => `sha256:${Buffer.from(d).toString("hex")}`;
export function digestFromText(text) {
  return isHash(text) ? Buffer.from(text.slice(7), "hex") : null;
}

// The largest power of two strictly less than n, for n >= 2.
function splitPoint(n) {
  let k = 1;
  while (k * 2 < n) k *= 2;
  return k;
}

// MTH (2.1.1) over leaf hashes. store/0 never uses the empty tree, but RFC 9162
// defines it as SHA-256 of the empty string.
export function merkleRoot(leafHashes) {
  const n = leafHashes.length;
  if (n === 0) return H();
  if (n === 1) return leafHashes[0];
  const k = splitPoint(n);
  return nodeHash(merkleRoot(leafHashes.slice(0, k)), merkleRoot(leafHashes.slice(k)));
}

// The tree over leaf hashes pushed one at a time, kept so that the tree hash
// of any prefix takes O(log n): row k holds the hash of every complete subtree
// of 2^k leaves, in order. The verifier checks each checkpoint against it, so
// a log of n lines with many checkpoints takes O(n log n), not O(n^2 log n).
export class MerkleTree {
  #levels = []; // #levels[k][i]: the subtree over leaves i * 2^k to (i + 1) * 2^k - 1

  get size() {
    return this.#levels.length ? this.#levels[0].length : 0;
  }

  push(leaf) {
    let node = leaf;
    for (let level = 0; ; level++) {
      if (this.#levels.length === level) this.#levels.push([]);
      const row = this.#levels[level];
      row.push(node);
      if (row.length % 2 === 1) return;
      node = nodeHash(row[row.length - 2], row[row.length - 1]);
    }
  }

  // MTH of the first `size` leaves, or null if size is not 1 to this.size. The
  // tree of `size` leaves splits at the largest power of two below `size`, so it
  // is the complete subtrees given by the binary digits of `size`, largest
  // first, hashed together from the right.
  root(size) {
    if (!Number.isSafeInteger(size) || size < 1 || size > this.size) return null;
    const subtrees = [];
    let offset = 0;
    for (let level = this.#levels.length - 1; level >= 0; level--) {
      const width = 2 ** level;
      if (offset + width <= size) {
        subtrees.push(this.#levels[level][offset / width]);
        offset += width;
      }
    }
    return subtrees.reduceRight((right, left) => nodeHash(left, right));
  }
}

// PATH(m, D[n]) (2.1.3.1), ordered from the leaf upwards. Throws on an index
// outside the tree: producing a proof is a writer operation on trusted input.
export function inclusionProof(leafHashes, index) {
  const n = leafHashes.length;
  if (!Number.isSafeInteger(index) || index < 0 || index >= n) {
    throw new RangeError(`index ${index} not in tree of size ${n}`);
  }
  const path = (m, d) => {
    if (d.length <= 1) return [];
    const k = splitPoint(d.length);
    return m < k
      ? [...path(m, d.slice(0, k)), merkleRoot(d.slice(k))]
      : [...path(m - k, d.slice(k)), merkleRoot(d.slice(0, k))];
  };
  return path(index, leafHashes);
}

// PROOF(m, D[n]) = SUBPROOF(m, D[n], true) (2.1.4.1), for 0 < m < n.
export function consistencyProof(leafHashes, oldSize) {
  const n = leafHashes.length;
  if (!Number.isSafeInteger(oldSize) || oldSize <= 0 || oldSize >= n) {
    throw new RangeError(`old size ${oldSize} not between 0 and ${n} (exclusive)`);
  }
  const subproof = (m, d, b) => {
    if (m === d.length) return b ? [] : [merkleRoot(d)];
    const k = splitPoint(d.length);
    return m <= k
      ? [...subproof(m, d.slice(0, k), b), merkleRoot(d.slice(k))]
      : [...subproof(m - k, d.slice(k), false), merkleRoot(d.slice(0, k))];
  };
  return subproof(oldSize, leafHashes, true);
}

function toCount(v) {
  if (typeof v === "bigint") return v >= 0n && v <= BigInt(MAX_SAFE) ? v : null;
  return Number.isSafeInteger(v) && v >= 0 ? BigInt(v) : null;
}
const isDigest = (d) => Buffer.isBuffer(d) && d.length === 32;
const lsb = (x) => (x & 1n) === 1n;

// The verification algorithm of 2.1.3.2. Bit operations use bigints, because
// JavaScript's bitwise operators truncate numbers to 32 bits.
export function verifyInclusion(leaf, index, size, proof, root) {
  const m = toCount(index);
  const n = toCount(size);
  if (m === null || n === null || !isDigest(leaf) || !isDigest(root) || !Array.isArray(proof)) return false;
  if (!proof.every(isDigest)) return false;
  if (m >= n) return false; // 1
  let fn = m; // 2
  let sn = n - 1n;
  let r = leaf; // 3
  for (const p of proof) { // 4
    if (sn === 0n) return false; // a
    if (lsb(fn) || fn === sn) { // b
      r = nodeHash(p, r);
      if (!lsb(fn)) {
        while (!lsb(fn) && fn !== 0n) {
          fn >>= 1n;
          sn >>= 1n;
        }
      }
    } else {
      r = nodeHash(r, p);
    }
    fn >>= 1n; // c
    sn >>= 1n;
  }
  return sn === 0n && r.equals(root); // 5
}

// The verification algorithm of 2.1.4.2, for 0 < oldSize < newSize.
export function verifyConsistency(oldSize, newSize, oldRoot, newRoot, proof) {
  const m = toCount(oldSize);
  const n = toCount(newSize);
  if (m === null || n === null || !isDigest(oldRoot) || !isDigest(newRoot) || !Array.isArray(proof)) return false;
  if (!proof.every(isDigest)) return false;
  if (!(m > 0n && m < n)) return false;
  if (proof.length === 0) return false; // 1
  const path = (m & (m - 1n)) === 0n ? [oldRoot, ...proof] : [...proof]; // 2
  let fn = m - 1n; // 3
  let sn = n - 1n;
  while (lsb(fn)) { // 4
    fn >>= 1n;
    sn >>= 1n;
  }
  let fr = path[0]; // 5
  let sr = path[0];
  for (const c of path.slice(1)) { // 6
    if (sn === 0n) return false; // a
    if (lsb(fn) || fn === sn) { // b
      fr = nodeHash(c, fr);
      sr = nodeHash(c, sr);
      if (!lsb(fn)) {
        while (!lsb(fn) && fn !== 0n) {
          fn >>= 1n;
          sn >>= 1n;
        }
      }
    } else {
      sr = nodeHash(sr, c);
    }
    fn >>= 1n; // c
    sn >>= 1n;
  }
  return fr.equals(oldRoot) && sr.equals(newRoot) && sn === 0n; // 7
}

// --- Store objects ------------------------------------------------------------------

const NAME_RE = /^[a-z0-9][a-z0-9.-]{0,62}$/;
export const STORE_SYSTEM = "evidence-store";
export const signerFor = (name) => `service:${STORE_SYSTEM}/${name}`;

// ed25519: + exactly 32 bytes in canonical unpadded base64url.
function isStoreKey(key) {
  if (typeof key !== "string" || !key.startsWith("ed25519:")) return false;
  const b64 = key.slice("ed25519:".length);
  const raw = Buffer.from(b64, "base64url");
  return raw.length === 32 && raw.toString("base64url") === b64;
}

// Seals an evidence/0 body: id = sha256 of the canonical form without `id`.
function sealObject(body) {
  return { v: "evidence/0", id: digest({ v: "evidence/0", ...body }), ...body };
}

function storeObject({ schema, subject, inline, actorId, at, prev }) {
  return sealObject({
    op: "assert",
    schema,
    subject,
    content: { media_type: "application/json", hash: digest(inline), inline },
    actor: { id: actorId, kind: "service" },
    at,
    prev,
  });
}

// The genesis declaration: { name, signer, key }. Throws S_GENESIS_INVALID if
// the name or key is not valid store/0.
export function makeGenesis(name, key) {
  if (typeof name !== "string" || !NAME_RE.test(name)) {
    fail("S_GENESIS_INVALID", "name does not match the store name pattern");
  }
  if (!isStoreKey(key)) fail("S_GENESIS_INVALID", "key is not an Ed25519 public key");
  return { name, signer: signerFor(name), key };
}

// The sealed genesis object (seq 1). Its id is the log id.
export function genesisObject(genesis, at) {
  return storeObject({
    schema: { ref: GENESIS_SCHEMA_REF, hash: GENESIS_SCHEMA_HASH },
    subject: { system: STORE_SYSTEM, type: "log", key: genesis.name },
    inline: { v: STORE_SPEC, name: genesis.name, signer: genesis.signer, key: genesis.key },
    actorId: genesis.signer,
    at,
    prev: [],
  });
}

// Reads the genesis declaration back from an object, which must be exactly the
// genesis built from its own name, key and at. Throws S_GENESIS_INVALID.
export function genesisFromObject(obj) {
  const bad = (why) => fail("S_GENESIS_INVALID", why);
  const inline = obj?.content?.inline;
  if (!isPlainObject(inline) || !hasExactly(inline, ["v", "name", "signer", "key"])) bad("content is not store/0");
  if (inline.v !== STORE_SPEC) bad(`content v is not ${STORE_SPEC}`);
  const genesis = makeGenesis(inline.name, inline.key);
  if (inline.signer !== genesis.signer) bad("signer is not service:evidence-store/ + name");
  if (!parseTimestamp(obj.at)) bad("at is not a timestamp");
  if (canonicalize(genesisObject(genesis, obj.at)) !== canonicalize(obj)) {
    bad("not the genesis built from its own name, key and at");
  }
  return genesis;
}

// The sealed checkpoint object. `checkpoint` is { log, size, head, root };
// `prev` is the previous checkpoint's id, or null for the first.
export function checkpointObject(checkpoint, genesis, at, prev = null) {
  const { log, size, head, root } = checkpoint;
  return storeObject({
    schema: { ref: CHECKPOINT_SCHEMA_REF, hash: CHECKPOINT_SCHEMA_HASH },
    subject: { system: STORE_SYSTEM, type: "checkpoint", key: log },
    inline: { v: "checkpoint/0", log, size, head, root },
    actorId: genesis.signer,
    at,
    prev: prev === null ? [] : [prev],
  });
}

// Reads a checkpoint back from an object, which must have valid checkpoint/0
// content for log `logId` and be exactly the object built from that content,
// the genesis signer, its at and its prev. Throws S_CHECKPOINT_INVALID.
// Returns { log, size, head, root, at, prev }.
//
// prev may be [] or any one id: that it names the previous checkpoint is a
// writer rule, which verification does not check (spec/store-0.md, step j).
export function checkpointFromObject(obj, genesis, logId) {
  const bad = (why) => fail("S_CHECKPOINT_INVALID", why);
  const inline = obj?.content?.inline;
  if (!isPlainObject(inline) || !hasExactly(inline, ["v", "log", "size", "head", "root"])) {
    bad("content is not checkpoint/0");
  }
  const { v, log, size, head, root } = inline;
  if (v !== "checkpoint/0") bad("content v is not checkpoint/0");
  if (!isHash(log) || !isHash(head) || !isHash(root)) bad("log, head and root are hashes");
  if (!isPosition(size)) bad("size is not an integer from 1 to 2^53 - 1");
  if (log !== logId) bad("log is not this log's id");
  if (!parseTimestamp(obj.at)) bad("at is not a timestamp");
  const prevList = obj.prev;
  if (!Array.isArray(prevList) || prevList.length > 1 || !prevList.every(isHash)) {
    bad("prev is not [] or one checkpoint id");
  }
  const prev = prevList.length ? prevList[0] : null;
  const rebuilt = checkpointObject({ log, size, head, root }, genesis, obj.at, prev);
  if (canonicalize(rebuilt) !== canonicalize(obj)) {
    bad("not the checkpoint built from its content, signer, at and prev");
  }
  return { log, size, head, root, at: obj.at, prev };
}

// --- Lanes and heads -------------------------------------------------------------------

export const RECORD = "record";

export function subjectOf(obj) {
  const s = isPlainObject(obj?.subject) ? obj.subject : {};
  const str = (v) => (typeof v === "string" ? v : "");
  return { system: str(s.system), type: str(s.type), key: str(s.key) };
}

export function laneOf(obj) {
  if (["create", "update", "delete"].includes(obj?.op)) return RECORD;
  if (obj?.op === "assert" && isPlainObject(obj.ext) && Object.hasOwn(obj.ext, "capture/0")) return RECORD;
  const ref = typeof obj?.schema?.ref === "string" ? obj.schema.ref : "";
  const at = ref.indexOf("@");
  return `assert:${at === -1 ? ref : ref.slice(0, at)}`;
}

const compareSubjects = (a, b) =>
  compareUtf8(a.system, b.system) || compareUtf8(a.type, b.type) || compareUtf8(a.key, b.key);

// heads(S, L) kept by a fold in seq order: for each object, remove its prev ids
// from its own subject and lane, then add its id. A prev in another subject or
// lane is never in this set, so it changes nothing.
export class Heads {
  #sets = new Map(); // JSON [system, type, key, lane] -> Set of ids

  static #key(subject, lane) {
    return JSON.stringify([subject.system, subject.type, subject.key, lane]);
  }

  apply(id, subject, lane, prev) {
    const key = Heads.#key(subject, lane);
    const set = this.#sets.get(key) ?? new Set();
    for (const p of prev) set.delete(p);
    set.add(id);
    this.#sets.set(key, set);
  }

  // Sorted ids; [] when the subject and lane have none.
  get(subject, lane) {
    return [...(this.#sets.get(Heads.#key(subject, lane)) ?? [])].sort(compareUtf8);
  }

  // Record-lane subjects with more than one head.
  recordForks() {
    let forks = 0;
    for (const [key, set] of this.#sets) if (JSON.parse(key)[3] === RECORD && set.size > 1) forks += 1;
    return forks;
  }

  // Every (subject, lane, id), sorted by subject, lane, then id.
  rows() {
    const rows = [];
    for (const [key, set] of this.#sets) {
      const [system, type, k, lane] = JSON.parse(key);
      for (const id of set) rows.push([{ system, type, key: k }, lane, id]);
    }
    return rows.sort((a, b) => compareSubjects(a[0], b[0]) || compareUtf8(a[1], b[1]) || compareUtf8(a[2], b[2]));
  }
}

// --- evidence/0 checks for log items ----------------------------------------------
// evidence.mjs covers parsing, ids, content hashes and signatures. A log verifier
// also needs evidence/0 steps 2 and 3 (the object shape and a calendar-valid
// `at`) and the attestation/0 shape, so they are written here from
// spec/evidence-0.md and its JSON Schemas.

const OPS = ["create", "update", "delete", "assert"];
const ACTOR_KINDS = ["human", "service", "agent"];
export const PURPOSES = ["author", "approve", "witness"];
const MEDIA_TYPE_RE = /^[a-z0-9][a-z0-9!#$&^_.+-]*\/[a-z0-9][a-z0-9!#$&^_.+-]*$/;
const EXT_KEY_RE = /^[a-z0-9][a-z0-9.-]*\/[0-9]+$/;
const isText = (v) => typeof v === "string" && v.length > 0;

// A plain object whose members are all of `required` and some of `optional`.
function hasMembers(obj, required, optional = []) {
  if (!isPlainObject(obj)) return false;
  for (const k of Object.keys(obj)) if (!required.includes(k) && !optional.includes(k)) return false;
  return required.every((k) => Object.hasOwn(obj, k));
}

// evidence/0 step 2: the rules of schema/evidence-object-0.schema.json.
function objectShapeOk(o) {
  const required = ["v", "id", "op", "schema", "subject", "content", "actor", "at", "prev"];
  if (!hasMembers(o, required, ["ext"])) return false;
  if (o.v !== "evidence/0" || !isHash(o.id) || !OPS.includes(o.op)) return false;
  const { schema, subject, content, actor, prev } = o;
  if (!hasMembers(schema, ["ref", "hash"]) || !isText(schema.ref) || !isHash(schema.hash)) return false;
  if (!hasMembers(subject, ["system", "type", "key"])) return false;
  if (![subject.system, subject.type, subject.key].every(isText)) return false;
  if (!hasMembers(content, ["media_type", "hash"], ["uri", "inline"])) return false;
  if (typeof content.media_type !== "string" || !MEDIA_TYPE_RE.test(content.media_type)) return false;
  if (!isHash(content.hash)) return false;
  if (Object.hasOwn(content, "uri") && !isText(content.uri)) return false;
  if (Object.hasOwn(content, "inline") && content.media_type !== "application/json") return false;
  if (!hasMembers(actor, ["id", "kind"], ["on_behalf_of"])) return false;
  if (!isText(actor.id) || !ACTOR_KINDS.includes(actor.kind)) return false;
  if (Object.hasOwn(actor, "on_behalf_of") ? !isText(actor.on_behalf_of) : actor.kind === "agent") return false;
  if (typeof o.at !== "string" || !TS_RE.test(o.at)) return false;
  if (!Array.isArray(prev) || !prev.every(isHash) || new Set(prev).size !== prev.length) return false;
  if (Object.hasOwn(o, "ext")) {
    if (!isPlainObject(o.ext) || !Object.keys(o.ext).every((k) => EXT_KEY_RE.test(k))) return false;
  }
  return true;
}

// evidence/0 steps 2 to 5 on a parsed object (step 1 is the strict parse that
// produced it): the E_ code of the first step that fails, or null.
export function objectCode(obj) {
  if (!objectShapeOk(obj)) return "E_SHAPE";
  if (!parseTimestamp(obj.at)) return "E_TIMESTAMP";
  if (obj.id !== objectId(obj)) return "E_ID_MISMATCH";
  if (Object.hasOwn(obj.content, "inline") && digest(obj.content.inline) !== obj.content.hash) {
    return "E_CONTENT_HASH_MISMATCH";
  }
  return null;
}

// evidence/0 steps 1 to 5 on object JSON text or UTF-8 bytes. Returns the
// parsed object; throws EvidenceError with the E_ code of the first failure.
export function checkObject(input) {
  const text = toText(input, (why) => fail("E_PARSE", why));
  const value = parseStrictJson(text);
  const code = objectCode(value);
  if (code) fail(code, "the object fails evidence/0 steps 2-5");
  return value;
}

// Unpadded base64url of exactly `length` bytes, in its one canonical spelling.
function isBase64url(text, length) {
  if (typeof text !== "string") return false;
  const raw = Buffer.from(text, "base64url");
  return raw.length === length && raw.toString("base64url") === text;
}

// The attestation/0 shape (E_ATTESTATION_SHAPE otherwise): exactly its seven
// members, each of its type and pattern, and an `at` that is a real instant.
export function attestationShapeOk(a) {
  if (!hasMembers(a, ["v", "object", "purpose", "signer", "key", "at", "sig"])) return false;
  return (
    a.v === "attestation/0" &&
    isHash(a.object) &&
    PURPOSES.includes(a.purpose) &&
    isText(a.signer) &&
    isStoreKey(a.key) &&
    parseTimestamp(a.at) !== null &&
    isBase64url(a.sig, 64)
  );
}

// The Ed25519 signature check of evidence/0, for an attestation whose shape is
// valid. False rather than an exception for a key Node cannot load.
export function attestationSignatureOk(a) {
  try {
    return verifyAttestation(a);
  } catch {
    return false;
  }
}

// --- Report (store-report/0) --------------------------------------------------------

export const REPORT_V = "store-report/0";
export const DEFAULT_MAX_FINDINGS = 1000;
// Report codes in sort order (spec/store-0.md, Report codes).
export const FINDING_CODES = [
  "S_MANIFEST_INVALID",
  "S_SEGMENT_HASH",
  "S_MANIFEST_MISMATCH",
  "S_LINE_INVALID",
  "S_ENTRY_INVALID",
  "S_WRONG_LOG",
  "S_SEQ_GAP",
  "S_CHAIN_BREAK",
  "S_RECEIVED_REGRESSION",
  "S_EPOCH_REGRESSION",
  "S_REF_MISMATCH",
  "S_DUPLICATE_REF",
  "S_GENESIS_INVALID",
  "S_RESERVED_SUBJECT",
  "S_OBJECT_INVALID",
  "S_PREV_POSITION",
  "S_ATTESTATION_INVALID",
  "S_ATTESTATION_TARGET",
  "S_CHECKPOINT_INVALID",
  "S_CHECKPOINT_MISMATCH",
  "S_CHECKPOINT_UNSIGNED",
  "S_PROJECTION_DRIFT",
  "S_TRIGGERS_MISSING",
  "S_META_MISMATCH",
];
const CODE_RANK = new Map(FINDING_CODES.map((code, i) => [code, i]));

// --- Manifest (segments/0) ------------------------------------------------------------

export const SEGMENTS_V = "segments/0";

// Segment i, counting from 1: i in decimal, zero-padded to six digits, + ".jsonl".
export const segmentFileName = (i) => `${String(i).padStart(6, "0")}.jsonl`;

// Manifest check 1 of spec/store-0.md: the bytes are exactly the canonical form
// of strict JSON that matches segments/0, and the segment list is the one that
// size and segment_size give. Returns the manifest; throws S_MANIFEST_INVALID.
// Integer arithmetic is in bigints, so sizes up to 2^53 - 1 are exact.
export function parseManifest(input) {
  const bad = (why) => fail("S_MANIFEST_INVALID", why);
  const text = toText(input, bad);
  let m;
  try {
    m = parseStrictJson(text);
  } catch (e) {
    if (e instanceof EvidenceError) bad(`not strict JSON (${e.code})`);
    throw e;
  }
  if (canonicalize(m) !== text) bad("not in canonical form");
  if (!hasMembers(m, ["v", "log", "size", "head", "segment_size", "segments"], ["checkpoint"])) {
    bad("members are not those of segments/0");
  }
  if (m.v !== SEGMENTS_V) bad(`v is not ${SEGMENTS_V}`);
  if (!isHash(m.log) || !isHash(m.head)) bad("log and head are hashes");
  if (!isPosition(m.size) || !isPosition(m.segment_size)) bad("size and segment_size are positions");
  if (Object.hasOwn(m, "checkpoint")) {
    const cp = m.checkpoint;
    if (!hasMembers(cp, ["id", "size"]) || !isHash(cp.id) || !isPosition(cp.size)) bad("checkpoint is not {id, size}");
  }
  const size = BigInt(m.size);
  const per = BigInt(m.segment_size);
  const count = (size + per - 1n) / per;
  if (!Array.isArray(m.segments) || BigInt(m.segments.length) !== count) {
    bad("the segment list is not the one size and segment_size give");
  }
  m.segments.forEach((s, i) => {
    const first = BigInt(i) * per + 1n;
    const last = BigInt(i + 1) * per < size ? BigInt(i + 1) * per : size;
    const ok =
      hasMembers(s, ["file", "first", "last", "hash"]) &&
      s.file === segmentFileName(i + 1) &&
      Number.isSafeInteger(s.first) &&
      BigInt(s.first) === first &&
      Number.isSafeInteger(s.last) &&
      BigInt(s.last) === last &&
      isHash(s.hash);
    if (!ok) bad(`segment ${i + 1} is not the one size and segment_size give`);
  });
  return m;
}

// Splits a segment file into lines at every LF byte, before any decoding.
// Returns { lines, tail }: `lines` are the bytes before each LF, without it;
// `tail` is the bytes after the last LF when the file does not end in LF (one
// more line, always S_LINE_INVALID), else null. An empty file has no lines.
export function splitSegment(input) {
  const bytes = typeof input === "string" ? Buffer.from(input, "utf8") : Buffer.from(input);
  const lines = [];
  let start = 0;
  for (let lf = bytes.indexOf(0x0a, start); lf !== -1; lf = bytes.indexOf(0x0a, start)) {
    lines.push(bytes.subarray(start, lf));
    start = lf + 1;
  }
  return { lines, tail: start < bytes.length ? bytes.subarray(start) : null };
}

// --- Verifier -----------------------------------------------------------------------
// The algorithm of spec/store-0.md, Verification: line checks a, b, b2 and c;
// sequence and clock checks d to g; body checks h to k; end of log; and the
// sort and truncation of findings.
//
// Only the maxFindings smallest findings in sort order can be reported, and a
// finding found later never makes one already kept any larger. So, as in the
// Rust verifier, whenever 2 x maxFindings (at least 64) are held they are
// sorted and cut to maxFindings, and memory stays small whatever the input.
// finish() sorts what is left and keeps the first maxFindings; `truncated`
// counts every finding found.

const LATE_CLAIM_SECS = 300;

export class Verifier {
  #source;
  #maxFindings;
  #findings = []; // { code, seq?, ref?, core?, file?, pos }, pruned as they come
  #found = 0; // findings recorded so far, kept or not
  #lineCount = 0; // lines read so far; a line's position in reading order
  #logId = null;
  #expectedSeq = 1;
  #previous = null; // the previous line: null before the first, else { usable, hash }
  #lastUsable = null; // { receivedAt, epoch } of the previous usable line
  #head = null; // { seq, entry_hash } of the last usable line
  #tree = new MerkleTree(); // over the leaf hashes of the usable lines, in reading order
  #entryHashes = []; // entry hashes of the usable lines, in reading order
  #refs = new Set(); // entry.ref of every usable line read
  #seqs = new Set(); // which of seq 1 and 2 a usable line had
  #held = new Map(); // id -> at, for objects held (step i)
  #heads = new Heads();
  #genesis = null; // { name, signer, key, at } once found
  #checkpoints = []; // recognised checkpoints, in reading order
  #witnessedIds = new Set(); // objects named by a valid witness (no body finding, the genesis key)
  #deferred = []; // step 6 findings decided at the end of the log
  #checked = { entries: 0, objects: 0, attestations: 0, checkpoints: 0 };
  #atAfterReceived = 0;

  constructor({ source = "export", maxFindings = DEFAULT_MAX_FINDINGS } = {}) {
    if (source !== "export" && source !== "store") throw new RangeError(`source ${source}`);
    if (!isPosition(maxFindings)) throw new RangeError(`max_findings ${maxFindings} is not 1 to 2^53 - 1`);
    this.#source = source;
    this.#maxFindings = maxFindings;
  }

  // A finding that is not about a line: a manifest or live-store finding. `pos`
  // orders findings of one code (manifest file order for S_SEGMENT_HASH).
  addFinding({ code, file, pos = 0 }) {
    if (!CODE_RANK.has(code)) throw new RangeError(`unknown code ${code}`);
    this.#record(file === undefined ? { code, pos } : { code, file, pos });
  }

  // The sort order of findings: seq (none first), then code, then pos.
  static #compare(a, b) {
    return (a.seq ?? 0) - (b.seq ?? 0) || CODE_RANK.get(a.code) - CODE_RANK.get(b.code) || a.pos - b.pos;
  }

  #record(finding) {
    this.#found += 1;
    this.#findings.push(finding);
    if (this.#findings.length >= Math.max(2 * this.#maxFindings, 64)) {
      this.#findings.sort(Verifier.#compare);
      this.#findings.length = this.#maxFindings;
    }
  }

  // One line, without its LF, as bytes (or a string, taken as its UTF-8 bytes).
  pushLine(bytes) {
    const pos = this.#lineCount++;
    // A line is the canonical form of a JSON object, so it starts with "{";
    // any other line fails step a. This spares an exception for each one.
    if ((typeof bytes === "string" ? bytes.charCodeAt(0) : bytes[0]) !== 0x7b) {
      this.#unusable({ code: "S_LINE_INVALID", pos });
      return;
    }
    let line;
    try {
      line = parseLine(bytes);
    } catch (e) {
      if (!(e instanceof EvidenceError)) throw e;
      this.#unusable({ code: e.code, pos }); // steps a, b, b2
      return;
    }
    const { entry, kind, body } = line;
    if (this.#logId === null) this.#logId = entry.log;
    // A finding on this line: seq and ref always, core for the two codes that carry one.
    const flag = (code, core) => {
      const finding = { code, seq: entry.seq, ref: entry.ref, pos };
      this.#record(core ? { ...finding, core } : finding);
    };
    if (entry.log !== this.#logId) {
      this.#unusable({ code: "S_WRONG_LOG", seq: entry.seq, ref: entry.ref, pos }); // step c
      return;
    }

    // Sequence and clock checks: each can add a finding; the line goes on.
    const entryCanonicalBytes = Buffer.from(line.entryCanonical, "utf8");
    const hash = sha256(entryCanonicalBytes);
    const receivedAt = parseTimestamp(entry.received_at);
    if (entry.seq !== this.#expectedSeq) flag("S_SEQ_GAP"); // d
    this.#expectedSeq = entry.seq + 1;
    if (entry.seq === 1) {
      if (entry.prev !== this.#logId) flag("S_CHAIN_BREAK"); // e
    } else if (this.#previous?.usable && entry.prev !== this.#previous.hash) {
      flag("S_CHAIN_BREAK");
    }
    if (this.#lastUsable) {
      if (compareTimestamps(receivedAt, this.#lastUsable.receivedAt) < 0) flag("S_RECEIVED_REGRESSION"); // f
      if (entry.epoch < this.#lastUsable.epoch) flag("S_EPOCH_REGRESSION"); // g
    }

    // Body checks: at most one finding.
    if (kind === "object") {
      const isCheckpoint = this.#objectBody(entry, body, receivedAt, pos, flag);
      this.#checked[isCheckpoint ? "checkpoints" : "objects"] += 1;
    } else {
      this.#attestationBody(entry, body, flag);
      this.#checked.attestations += 1;
    }

    this.#checked.entries += 1;
    this.#refs.add(entry.ref);
    if (entry.seq === 1 || entry.seq === 2) this.#seqs.add(entry.seq);
    this.#tree.push(leafHash(entryCanonicalBytes));
    this.#entryHashes.push(hash);
    this.#head = { seq: entry.seq, entry_hash: hash };
    this.#previous = { usable: true, hash };
    this.#lastUsable = { receivedAt, epoch: entry.epoch };
  }

  // The bytes after the last LF of a segment file that does not end in LF.
  pushUnterminatedLine() {
    this.#unusable({ code: "S_LINE_INVALID", pos: this.#lineCount++ });
  }

  // The lines of one segment file, as splitSegment splits them, one at a time.
  pushSegment(input) {
    let bytes = input;
    if (typeof input === "string") bytes = Buffer.from(input, "utf8");
    else if (!Buffer.isBuffer(input)) bytes = Buffer.from(input);
    let start = 0;
    for (let lf = bytes.indexOf(0x0a, start); lf !== -1; lf = bytes.indexOf(0x0a, start)) {
      this.pushLine(bytes.subarray(start, lf));
      start = lf + 1;
    }
    if (start < bytes.length) this.pushUnterminatedLine();
  }

  #unusable(finding) {
    this.#record(finding);
    this.#previous = { usable: false };
  }

  // Steps h, i and j for an object line. `flag(code, core)` records a finding on
  // this line. Returns true when the object is recognised as a checkpoint
  // (step j item 2), whatever items 3 and 4 find.
  #objectBody(entry, obj, receivedAt, pos, flag) {
    // h. Reference.
    if (typeof obj?.id !== "string" || obj.id !== entry.ref) {
      flag("S_REF_MISMATCH");
      return false;
    }
    if (this.#refs.has(entry.ref)) {
      flag("S_DUPLICATE_REF");
      return false;
    }

    // i. evidence/0 steps 1-5 (the line's strict parse was step 1).
    const core = objectCode(obj);
    if (core) {
      flag("S_OBJECT_INVALID", core);
      return false;
    }
    const objAt = parseTimestamp(obj.at);
    // Step 6, in list order, stopping at the first prev that is not held on an
    // earlier line (decided at the end of the log) or is dated after this object.
    let prevFinding = false;
    for (const p of obj.prev) {
      const prevAt = this.#held.get(p);
      if (prevAt === undefined) {
        this.#deferred.push({ seq: entry.seq, ref: entry.ref, prev: p, pos });
        prevFinding = true;
        break;
      }
      if (compareTimestamps(prevAt, objAt) > 0) {
        flag("S_OBJECT_INVALID", "E_PREV_ORDER");
        prevFinding = true;
        break;
      }
    }
    // Held from this line on, whatever step 6 or step j finds.
    if (!this.#held.has(obj.id)) this.#held.set(obj.id, objAt);
    this.#heads.apply(obj.id, subjectOf(obj), laneOf(obj), obj.prev);
    const lateBound = { secs: receivedAt.secs + LATE_CLAIM_SECS, nanos: receivedAt.nanos };
    if (compareTimestamps(objAt, lateBound) > 0) this.#atAfterReceived += 1;
    if (prevFinding) return false;

    // j. Store subjects.
    if (entry.seq === 1) {
      let genesis = null;
      try {
        genesis = genesisFromObject(obj);
      } catch (e) {
        if (!(e instanceof EvidenceError)) throw e;
      }
      if (genesis === null || obj.id !== this.#logId) flag("S_GENESIS_INVALID");
      else if (this.#genesis === null) this.#genesis = { ...genesis, at: objAt };
      return false;
    }
    if (entry.seq === 2) {
      flag("S_GENESIS_INVALID");
      return false;
    }
    if (obj.subject.system !== STORE_SYSTEM) return false;
    const genesis = this.#genesis;
    if (genesis === null || obj.subject.type !== "checkpoint" || obj.actor.id !== genesis.signer) {
      flag("S_RESERVED_SUBJECT");
      return false;
    }
    const cp = { seq: entry.seq, ref: entry.ref, id: obj.id, pos, size: null, invalid: false, bodyFinding: false };
    this.#checkpoints.push(cp);
    let content;
    try {
      content = checkpointFromObject(obj, genesis, this.#logId);
    } catch (e) {
      if (!(e instanceof EvidenceError)) throw e;
      cp.invalid = true;
      cp.bodyFinding = true;
      flag("S_CHECKPOINT_INVALID");
      return true;
    }
    cp.size = content.size;
    const n = content.size;
    const root = this.#tree.root(n); // null when fewer than n usable lines come before
    const matches =
      n < entry.seq &&
      root !== null &&
      this.#entryHashes[n - 1] === content.head &&
      digestToText(root) === content.root;
    if (!matches) {
      cp.bodyFinding = true;
      flag("S_CHECKPOINT_MISMATCH");
    }
    return true;
  }

  // Steps h and k for an attestation line: the reference, then the shape, the
  // signature, the target and the genesis rule, stopping at the first finding.
  #attestationBody(entry, att, flag) {
    if (attestationRef(att) !== entry.ref) {
      flag("S_REF_MISMATCH");
    } else if (this.#refs.has(entry.ref)) {
      flag("S_DUPLICATE_REF");
    } else if (!attestationShapeOk(att)) {
      flag("S_ATTESTATION_INVALID", "E_ATTESTATION_SHAPE");
    } else if (!attestationSignatureOk(att)) {
      flag("S_ATTESTATION_INVALID", "E_SIGNATURE_INVALID");
    } else if (!this.#held.has(att.object)) {
      flag("S_ATTESTATION_TARGET");
    } else if (entry.seq === 1 || (entry.seq === 2 && !this.#isAuthorAttestation(att))) {
      flag("S_GENESIS_INVALID");
    } else if (att.purpose === "witness" && this.#genesis !== null && att.key === this.#genesis.key) {
      this.#witnessedIds.add(att.object);
    }
  }

  // The genesis author attestation: the genesis was found on an earlier line,
  // and the line attests the log id, as author, by the genesis signer and key,
  // at the genesis `at` (compared as instants).
  #isAuthorAttestation(att) {
    const g = this.#genesis;
    return (
      g !== null &&
      att.object === this.#logId &&
      att.purpose === "author" &&
      att.signer === g.signer &&
      att.key === g.key &&
      compareTimestamps(parseTimestamp(att.at), g.at) === 0
    );
  }

  #witnessed(cp) {
    return this.#witnessedIds.has(cp.id);
  }

  // What the manifest is compared with after the lines: the log id of the
  // lines, the head, and the latest checkpoint (the last recognised checkpoint
  // without S_CHECKPOINT_INVALID) as { id, size }. Each is null when absent.
  summary() {
    const latest = this.#checkpoints.filter((cp) => !cp.invalid).at(-1);
    return {
      log: this.#logId,
      head: this.#head,
      checkpoint: latest ? { id: latest.id, size: latest.size } : null,
    };
  }

  // Ends the log and returns the store-report/0 object. It changes nothing, so
  // it gives the same report if called again.
  finish() {
    const findings = [...this.#findings]; // those found at the end of the log join these
    for (const d of this.#deferred) {
      findings.push(
        this.#held.has(d.prev)
          ? { code: "S_PREV_POSITION", seq: d.seq, ref: d.ref, pos: d.pos }
          : { code: "S_OBJECT_INVALID", seq: d.seq, ref: d.ref, core: "E_PREV_MISSING", pos: d.pos },
      );
    }
    for (const cp of this.#checkpoints) {
      if (!this.#witnessed(cp)) findings.push({ code: "S_CHECKPOINT_UNSIGNED", seq: cp.seq, ref: cp.ref, pos: cp.pos });
    }
    if (!this.#seqs.has(1) || !this.#seqs.has(2)) findings.push({ code: "S_GENESIS_INVALID", pos: 0 });

    let covered = null;
    for (const cp of this.#checkpoints) {
      if (cp.bodyFinding || !this.#witnessed(cp)) continue;
      if (covered === null || cp.size > covered.size) covered = cp;
    }

    // pos restores reading order for findings decided at the end of the log.
    const found = this.#found + findings.length - this.#findings.length;
    findings.sort(Verifier.#compare);
    const kept = findings.slice(0, this.#maxFindings).map(({ pos: _pos, ...f }) => f);
    const report = { v: REPORT_V, ok: kept.length === 0, source: this.#source };
    if (this.#logId !== null) {
      report.log = this.#logId;
      report.head = { ...this.#head };
    }
    if (covered) report.covered = { size: covered.size, checkpoint: covered.id };
    report.checked = { ...this.#checked };
    report.counts = { forks: this.#heads.recordForks(), at_after_received: this.#atAfterReceived };
    report.findings = kept;
    report.truncated = found > kept.length;
    return report;
  }
}

// Verifies an export: the bytes of manifest.json and of the segment files in
// manifest order (Buffers, Uint8Arrays or strings). Returns store-report/0.
export function verifyExport(manifest, segments, { maxFindings = DEFAULT_MAX_FINDINGS } = {}) {
  const verifier = new Verifier({ source: "export", maxFindings });
  let m = null;
  try {
    m = parseManifest(manifest);
  } catch (e) {
    if (!(e instanceof EvidenceError)) throw e;
    verifier.addFinding({ code: "S_MANIFEST_INVALID" });
  }
  const files = segments.map((s) => (typeof s === "string" ? Buffer.from(s, "utf8") : Buffer.from(s)));
  if (m) {
    m.segments.forEach((s, i) => {
      if (i < files.length && sha256(files[i]) !== s.hash) {
        verifier.addFinding({ code: "S_SEGMENT_HASH", file: s.file, pos: i });
      }
    });
  }
  for (const file of files) verifier.pushSegment(file);
  if (m) {
    const { log, head, checkpoint } = verifier.summary();
    const mismatch =
      files.length !== m.segments.length ||
      log === null ||
      m.log !== log ||
      head === null ||
      m.size !== head.seq ||
      m.head !== head.entry_hash ||
      (checkpoint === null) !== !Object.hasOwn(m, "checkpoint") ||
      (checkpoint !== null && (m.checkpoint.id !== checkpoint.id || m.checkpoint.size !== checkpoint.size));
    if (mismatch) verifier.addFinding({ code: "S_MANIFEST_MISMATCH" });
  }
  return verifier.finish();
}

// --- Writer -----------------------------------------------------------------------------
// A store in memory that follows the intake rules and the two-clock rules of
// spec/store-0.md, and exports itself as segments/0. It exists to generate the
// export vectors: replay() runs an inputs.json operation script through it.
//
// For tampered vectors that are built rather than edited, append and attest
// take { unchecked: true } to skip every intake check but the strict parse, and
// { entry: {...} } to override members of the item's entry. checkpoint takes
// { root } to write a wrong root and { witness: false } to leave the witness
// out. init takes { author } and { edit } to change the author attestation or
// the genesis object. Everything else stays consistent: each entry's prev is
// the hash of the entry actually written before it.

const isBytes = (v) => v instanceof Uint8Array;

export class Writer {
  #name;
  #seed;
  #clock;
  #epoch = 1;
  #lines = []; // canonical segment lines, without LF
  #leaves = []; // leaf hashes, by seq - 1
  #hashes = []; // entry hashes, by seq - 1
  #lastReceived = null; // timestamp of the last entry's received_at
  #held = new Map(); // object id -> at (timestamp)
  #receipts = new Map(); // item ref -> receipt
  #genesis = null;
  #logId = null;
  #checkpoint = null; // { id, size, at } of the latest checkpoint written by checkpoint()

  // `seed` is the store key's seed in hex. `clock` is a list of timestamps,
  // one per transaction, consumed in order, or a function returning the next.
  constructor({ name, seed, clock }) {
    this.#name = name;
    this.#seed = seed;
    this.#clock = typeof clock === "function" ? clock : [...clock];
  }

  get logId() {
    return this.#logId;
  }
  get genesis() {
    return this.#genesis;
  }
  get size() {
    return this.#lines.length;
  }
  // The clock readings not yet used (an array clock only).
  get clockLeft() {
    return typeof this.#clock === "function" ? 0 : this.#clock.length;
  }
  get latestCheckpoint() {
    return this.#checkpoint ? { ...this.#checkpoint } : null;
  }
  lines() {
    return [...this.#lines];
  }
  entryHash(seq) {
    return this.#hashes[seq - 1];
  }
  root(size = this.size) {
    return digestToText(merkleRoot(this.#leaves.slice(0, size)));
  }

  // One clock reading: one transaction.
  #now() {
    let text;
    if (typeof this.#clock === "function") text = this.#clock();
    else if (this.#clock.length) text = this.#clock.shift();
    else throw new Error("the scripted clock has no reading left");
    const t = parseTimestamp(text);
    if (!t) throw new Error(`clock reading ${JSON.stringify(text)} is not a timestamp`);
    return t;
  }

  #received(now) {
    return formatMillis(nextReceived(now, this.#lastReceived));
  }

  #append(kind, itemRef, bodyCanonical, receivedAt, overrides = {}) {
    const seq = this.#lines.length + 1;
    const prev = seq === 1 ? this.#logId : this.#hashes[seq - 2];
    const entry = makeEntry({
      log: this.#logId,
      seq,
      kind,
      ref: itemRef,
      received_at: receivedAt,
      epoch: this.#epoch,
      prev,
      ...overrides,
    });
    const text = entryCanonical(entry);
    const canonical = Buffer.from(text, "utf8");
    this.#lines.push(renderLine(text, kind, bodyCanonical));
    this.#leaves.push(leafHash(canonical));
    this.#hashes.push(sha256(canonical));
    this.#lastReceived = parseTimestamp(entry.received_at);
    const receipt = { kind, ref: entry.ref, seq, entry_hash: this.#hashes[seq - 1], received_at: entry.received_at };
    if (!this.#receipts.has(itemRef)) this.#receipts.set(itemRef, receipt);
    return { ...receipt, duplicate: false };
  }

  #hold(obj) {
    const at = parseTimestamp(obj?.at);
    if (typeof obj?.id === "string" && at && !this.#held.has(obj.id)) this.#held.set(obj.id, at);
  }

  #signAs(object, purpose, at) {
    return signAttestation({ object, purpose, signer: this.#genesis.signer, at }, this.#seed);
  }

  // Creates the store at epoch 1: the genesis and its author attestation, in
  // one transaction. `author`, if given, builds the author attestation instead:
  // ({ logId, genesis, at, seed }) => attestation. `edit`, if given, changes the
  // genesis object before it is written: (object) => object, whose id is then
  // the log id.
  init({ author, edit } = {}) {
    if (this.#lines.length) throw new Error("init is the first operation, and only there");
    const now = this.#now();
    const genesis = makeGenesis(this.#name, publicKey(this.#seed));
    const at = formatMillis(serviceAt(now));
    const obj = edit ? edit(genesisObject(genesis, at)) : genesisObject(genesis, at);
    this.#genesis = genesis;
    this.#logId = obj.id;
    const received = this.#received(now);
    this.#append("object", obj.id, canonicalize(obj), received);
    this.#hold(obj);
    const att = author ? author({ logId: obj.id, genesis, at, seed: this.#seed }) : this.#signAs(obj.id, "author", at);
    this.#append("attestation", attestationRef(att), canonicalize(att), received);
    return obj;
  }

  // Appends one object, given as JSON text or bytes (stored in canonical form)
  // or as a value. Returns its receipt.
  append(input, { unchecked = false, entry = {} } = {}) {
    const now = this.#now();
    const given = typeof input === "string" || isBytes(input);
    const obj = given ? parseStrictJson(toText(input, (why) => fail("E_PARSE", why))) : input;
    if (!unchecked) {
      const code = objectCode(obj);
      if (code) fail(code, "the object fails evidence/0 steps 2-5");
      if (obj.subject.system === STORE_SYSTEM) fail("S_RESERVED_SUBJECT", "store subjects are the store's own");
      const held = this.#receipts.get(obj.id);
      if (held) return { ...held, duplicate: true };
      const at = parseTimestamp(obj.at);
      for (const p of obj.prev) {
        const prevAt = this.#held.get(p);
        if (prevAt === undefined) fail("E_PREV_MISSING", `prev ${p} is not held`);
        if (compareTimestamps(prevAt, at) > 0) fail("E_PREV_ORDER", `prev ${p} is dated after the object`);
      }
    }
    const receipt = this.#append("object", obj.id, canonicalize(obj), this.#received(now), entry);
    this.#hold(obj);
    return receipt;
  }

  // Appends one attestation line (text or bytes, stored in canonical form).
  attest(input, { unchecked = false, entry = {} } = {}) {
    const now = this.#now();
    let att;
    try {
      att = parseStrictJson(toText(input, (why) => fail("E_ATTESTATION_SHAPE", why)));
    } catch (e) {
      if (e instanceof EvidenceError) fail("E_ATTESTATION_SHAPE", `not strict JSON (${e.code})`);
      throw e;
    }
    const itemRef = attestationRef(att);
    if (!unchecked) {
      if (!attestationShapeOk(att)) fail("E_ATTESTATION_SHAPE", "not attestation/0");
      if (!this.#held.has(att.object)) fail("S_ATTESTATION_TARGET", "the target object is not held");
      if (!attestationSignatureOk(att)) fail("E_SIGNATURE_INVALID", "the signature does not verify");
      const held = this.#receipts.get(itemRef);
      if (held) return { ...held, duplicate: true };
    }
    return this.#append("attestation", itemRef, canonicalize(att), this.#received(now), entry);
  }

  // Closes the writer and opens the store again: the epoch increases by one.
  reopen() {
    this.#epoch += 1;
  }

  // Writes a checkpoint over every entry so far and its witness, in one
  // transaction. Returns { id, seq, size, root, head }.
  checkpoint({ root, witness = true } = {}) {
    const now = this.#now();
    const prev = this.#checkpoint;
    const at = formatMillis(serviceAt(now, prev ? [parseTimestamp(prev.at)] : []));
    const size = this.#lines.length;
    const content = { log: this.#logId, size, head: this.#hashes[size - 1], root: root ?? this.root(size) };
    const obj = checkpointObject(content, this.#genesis, at, prev ? prev.id : null);
    const received = this.#received(now);
    const { seq } = this.#append("object", obj.id, canonicalize(obj), received);
    this.#hold(obj);
    this.#checkpoint = { id: obj.id, size, at };
    if (witness) {
      const att = this.#signAs(obj.id, "witness", at);
      this.#append("attestation", attestationRef(att), canonicalize(att), received);
    }
    return { id: obj.id, seq, size, root: content.root, head: content.head };
  }

  // The export with `segmentSize` lines per segment file: a Map from file name
  // to bytes, the segment files in order and then manifest.json.
  exportFiles(segmentSize) {
    if (!isPosition(segmentSize)) throw new RangeError(`segment_size ${segmentSize}`);
    const size = this.#lines.length;
    const files = new Map();
    const segments = [];
    for (let first = 1, i = 1; first <= size; first += segmentSize, i += 1) {
      const last = Math.min(first + segmentSize - 1, size);
      const bytes = Buffer.from(this.#lines.slice(first - 1, last).map((l) => `${l}\n`).join(""), "utf8");
      const file = segmentFileName(i);
      files.set(file, bytes);
      segments.push({ file, first, last, hash: sha256(bytes) });
    }
    const manifest = {
      v: SEGMENTS_V,
      log: this.#logId,
      size,
      head: this.#hashes[size - 1],
      segment_size: segmentSize,
      segments,
    };
    if (this.#checkpoint) manifest.checkpoint = { id: this.#checkpoint.id, size: this.#checkpoint.size };
    files.set("manifest.json", Buffer.from(canonicalize(manifest), "utf8"));
    return files;
  }
}

// --- Replaying inputs.json ----------------------------------------------------------

const INPUT_MEMBERS = ["name", "seed", "segment_size", "clock", "ops"];

// Line n (counting from 1, without its LF) of a JSON Lines file's bytes.
function nthLine(bytes, n) {
  const { lines } = splitSegment(bytes);
  if (!Number.isSafeInteger(n) || n < 1 || n > lines.length) throw new RangeError(`no line ${n}`);
  return lines[n - 1];
}

// Runs an inputs.json operation script (spec/store-0.md, Export case files)
// through a Writer. `readFile(path)` returns the bytes of a path under
// test-vectors/: the seed file and the files the operations name. Every
// operation must succeed and none may be a duplicate; the clock must hold
// exactly one reading per operation that reads it. Returns the Writer.
export function replay(inputs, readFile) {
  if (!hasMembers(inputs, INPUT_MEMBERS)) throw new Error(`inputs members are ${INPUT_MEMBERS.join(", ")}`);
  const { name, seed, segment_size, clock, ops } = inputs;
  if (!isPosition(segment_size)) throw new Error("segment_size is a position");
  if (!Array.isArray(clock) || !clock.every((t) => parseTimestamp(t))) throw new Error("clock is a list of timestamps");
  if (!Array.isArray(ops) || ops[0]?.op !== "init") throw new Error("ops start with init");
  const seedHex = Buffer.from(readFile(`keys/${seed}.seed`)).toString("utf8").trim();
  const writer = new Writer({ name, seed: seedHex, clock });
  ops.forEach((op, i) => {
    const where = `ops[${i}]`;
    const fresh = (receipt) => {
      if (receipt.duplicate) throw new Error(`${where} is a duplicate`);
    };
    switch (op?.op) {
      case "init":
        if (i !== 0 || !hasMembers(op, ["op"])) throw new Error(`${where}: init is first and only first`);
        writer.init();
        break;
      case "append":
        if (hasMembers(op, ["op", "file"])) fresh(writer.append(readFile(op.file)));
        else if (hasMembers(op, ["op", "object"])) fresh(writer.append(op.object));
        else throw new Error(`${where}: append takes file or object`);
        break;
      case "attest":
        if (!hasMembers(op, ["op", "file", "line"])) throw new Error(`${where}: attest takes file and line`);
        fresh(writer.attest(nthLine(readFile(op.file), op.line)));
        break;
      case "reopen":
        if (!hasMembers(op, ["op"])) throw new Error(`${where}: reopen takes nothing`);
        writer.reopen();
        break;
      case "checkpoint":
        if (!hasMembers(op, ["op"])) throw new Error(`${where}: checkpoint takes nothing`);
        writer.checkpoint();
        break;
      default:
        throw new Error(`${where}: unknown op ${JSON.stringify(op?.op)}`);
    }
  });
  if (writer.clockLeft !== 0) throw new Error(`${writer.clockLeft} clock reading(s) left unused`);
  return writer;
}
