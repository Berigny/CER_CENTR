// Independent reference implementation of store/0 (spec/store-0.md), in
// JavaScript with no dependencies (Node >= 20). Like evidence.mjs, it exists so
// the Rust crates are checked against a second implementation written from the
// spec, not against themselves.
//
// Scope so far: log-entry/0 (build, canonical form, entry hash, strict
// validation), segment lines, the RFC 9162 Merkle tree (tree hash, inclusion and
// consistency proofs), the genesis and checkpoint objects, lanes and the heads
// fold, and the two-clock rules. The streaming verifier, the report and the
// export manifest come later.
//
// Errors are EvidenceError (from evidence.mjs) with an S_ code. Merkle proof
// verification never throws: it answers true or false.

import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { canonicalize, digest, EvidenceError, parseStrict, sha256 } from "./evidence.mjs";

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

// True if some object in `text` names a member twice. JSON.parse keeps the last
// one silently, while evidence/0's strict rules reject the text
// (E_DUPLICATE_KEY). Names are compared after decoding, so "v" and "v"
// collide. Only for text that JSON.parse has already accepted.
function hasDuplicateMembers(text) {
  const open = []; // one entry per open container: a Set of names, or null for an array
  let expectName = false;
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (c === '"') {
      let j = i + 1;
      while (text[j] !== '"') j += text[j] === "\\" ? 2 : 1;
      if (expectName) {
        const names = open[open.length - 1];
        const name = JSON.parse(text.slice(i, j + 1));
        if (names.has(name)) return true;
        names.add(name);
        expectName = false;
      }
      i = j;
    } else if (c === "{") {
      open.push(new Set());
      expectName = true;
    } else if (c === "[") {
      open.push(null);
      expectName = false;
    } else if (c === "}" || c === "]") {
      open.pop();
      expectName = false;
    } else if (c === ",") {
      expectName = open[open.length - 1] instanceof Set;
    }
  }
  return false;
}

// How deep arrays and objects may nest: the limit of the Rust evidence/0
// parser (evidence_core::json::MAX_DEPTH). A segment line wraps its entry and
// item in one more object, so a line may nest one level deeper, and every
// object that evidence/0 accepts still fits in a line.
export const MAX_DEPTH = 128;
export const MAX_LINE_DEPTH = MAX_DEPTH + 1;

// True if arrays and objects in `text` nest deeper than `limit`. It counts
// brackets outside strings in one pass, without recursion, so hostile nesting
// cannot exhaust the stack. Exact for valid JSON; for anything else JSON.parse
// decides.
function nestsDeeperThan(text, limit) {
  let depth = 0;
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (c === '"') {
      i += 1;
      while (i < text.length && text[i] !== '"') i += text[i] === "\\" ? 2 : 1;
    } else if (c === "{" || c === "[") {
      depth += 1;
      if (depth > limit) return true;
    } else if (c === "}" || c === "]") {
      depth -= 1;
    }
  }
  return false;
}

// evidence/0's strict parse: the nesting limit, evidence.mjs's parseStrict
// (number range, lone surrogates), then duplicate member names. The nesting
// check comes first, because parseStrict and canonicalize recurse.
function parseStrictJson(text, maxDepth = MAX_DEPTH) {
  if (nestsDeeperThan(text, maxDepth)) fail("E_PARSE", `nesting deeper than ${maxDepth}`);
  const value = parseStrict(text);
  if (hasDuplicateMembers(text)) fail("E_DUPLICATE_KEY", "a member name appears twice in one object");
  return value;
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
