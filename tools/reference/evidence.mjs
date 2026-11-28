// Independent reference implementation of evidence/0, in JavaScript with no
// dependencies (Node >= 20). It exists so the Rust core is checked against a
// second implementation, not just against itself.
//
// Scope: the strict parse of step 1, canonicalisation, ids, inline content
// hashes, Ed25519 attestations and required purposes. Not covered here (the
// Rust core and the JSON Schema cover them): shape rules, calendar checks on
// timestamps, and prev links. tools/reference/log.mjs adds the shape and
// calendar checks that a log needs.

import {
  createHash,
  createPrivateKey,
  createPublicKey,
  sign as edSign,
  verify as edVerify,
} from "node:crypto";

export class EvidenceError extends Error {
  constructor(code, detail) {
    super(`${code}: ${detail}`);
    this.code = code;
    this.detail = detail;
  }
}

// How deep arrays and objects may nest: the limit of the Rust evidence/0
// parser (evidence_core::json::MAX_DEPTH). `{}` and `[]` are depth 1.
export const MAX_DEPTH = 128;

const utf8 = new TextDecoder("utf-8", { fatal: true, ignoreBOM: true });
const NUMBER = /-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?/y;

// evidence/0 step 1: strict JSON, given as text or as bytes (decoded as strict
// UTF-8, keeping a byte order mark so that JSON.parse rejects it). Returns the
// value. When the input has several faults, the code is fixed whatever their
// positions: E_PARSE if it has any fault other than a number out of range or
// a repeated name (not UTF-8, a byte order mark, a syntax error, a lone
// surrogate, nesting deeper than `maxDepth`), otherwise E_NUMBER_RANGE if any
// number has magnitude above 2^53 - 1, otherwise E_DUPLICATE_KEY if an object
// names a member twice.
//
// The nesting check comes first, because JSON.parse and canonicalize recurse.
// JSON.parse then decides the syntax. It keeps only the last of a repeated
// member, so the other faults are found by reading the text's tokens, which
// also sees the values JSON.parse drops.
export function parseStrict(input, maxDepth = MAX_DEPTH) {
  let text = input;
  if (typeof input !== "string") {
    try {
      text = utf8.decode(input);
    } catch {
      throw new EvidenceError("E_PARSE", "input is not valid UTF-8");
    }
  }
  if (nestsDeeperThan(text, maxDepth)) throw new EvidenceError("E_PARSE", `nesting deeper than ${maxDepth}`);
  let value;
  try {
    value = JSON.parse(text);
  } catch (e) {
    throw new EvidenceError("E_PARSE", e.message);
  }
  const fault = tokenFault(text);
  if (fault) throw new EvidenceError(fault.code, fault.detail);
  return value;
}

// True if arrays and objects in `text` nest deeper than `limit`. It counts
// brackets outside strings in one pass, without recursion, so hostile nesting
// cannot exhaust the stack. Exact for valid JSON; for anything else JSON.parse
// decides, and the code is E_PARSE either way.
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

// The step-1 fault of text that JSON.parse has accepted, or null, in one pass
// over its tokens: a string or member name that is not well-formed Unicode (a
// lone surrogate) is E_PARSE and ends the pass; otherwise the first number
// out of range, and otherwise the first member name repeated in one object.
// Names are compared after decoding, so "v" and "\u0076" collide.
function tokenFault(text) {
  let range = null;
  let duplicate = null;
  const open = []; // one entry per open container: a Set of names, or null for an array
  let expectName = false;
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (c === '"') {
      let j = i + 1;
      while (text[j] !== '"') j += text[j] === "\\" ? 2 : 1;
      const s = JSON.parse(text.slice(i, j + 1));
      if (!s.isWellFormed()) return { code: "E_PARSE", detail: "lone surrogate in a string" };
      if (expectName) {
        const names = open[open.length - 1];
        if (names.has(s)) duplicate ??= `member name ${JSON.stringify(s)} appears more than once`;
        names.add(s);
        expectName = false;
      }
      i = j;
    } else if (c === "-" || (c >= "0" && c <= "9")) {
      NUMBER.lastIndex = i;
      const token = NUMBER.exec(text)[0];
      if (!(Math.abs(Number(token)) <= Number.MAX_SAFE_INTEGER)) {
        range ??= `number ${token} has magnitude above 2^53 - 1`;
      }
      i += token.length - 1;
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
  if (range) return { code: "E_NUMBER_RANGE", detail: range };
  if (duplicate) return { code: "E_DUPLICATE_KEY", detail: duplicate };
  return null;
}

// RFC 8785: JSON.stringify for primitives; members sorted by UTF-16 code
// units, which is what Array.prototype.sort does by default.
export function canonicalize(v) {
  if (v === null || typeof v === "boolean" || typeof v === "string") return JSON.stringify(v);
  if (typeof v === "number") {
    if (!Number.isFinite(v)) throw new EvidenceError("E_NUMBER_RANGE", String(v));
    return JSON.stringify(v);
  }
  if (Array.isArray(v)) return `[${v.map(canonicalize).join(",")}]`;
  const keys = Object.keys(v).sort();
  return `{${keys.map((k) => `${JSON.stringify(k)}:${canonicalize(v[k])}`).join(",")}}`;
}

export const sha256 = (bytes) => `sha256:${createHash("sha256").update(bytes).digest("hex")}`;
export const digest = (v) => sha256(Buffer.from(canonicalize(v), "utf8"));

export function objectId(obj) {
  const { id: _ignored, ...rest } = obj;
  return digest(rest);
}

export function seal(obj) {
  return { ...obj, id: objectId(obj) };
}

// --- Ed25519 -----------------------------------------------------------------

const PKCS8_PREFIX = Buffer.from("302e020100300506032b657004220420", "hex");
const SPKI_PREFIX = Buffer.from("302a300506032b6570032100", "hex");

function privateKey(seedHex) {
  const seed = Buffer.from(seedHex.trim(), "hex");
  if (seed.length !== 32) throw new Error("seed must be 32 bytes of hex");
  return createPrivateKey({ key: Buffer.concat([PKCS8_PREFIX, seed]), format: "der", type: "pkcs8" });
}

export function publicKey(seedHex) {
  const der = createPublicKey(privateKey(seedHex)).export({ format: "der", type: "spki" });
  return `ed25519:${der.subarray(-32).toString("base64url")}`;
}

export function signAttestation({ object, purpose, signer, at }, seedHex) {
  const unsigned = { v: "attestation/0", object, purpose, signer, key: publicKey(seedHex), at };
  const sig = edSign(null, Buffer.from(canonicalize(unsigned), "utf8"), privateKey(seedHex));
  return { ...unsigned, sig: sig.toString("base64url") };
}

function decodeB64(s, length) {
  const bytes = Buffer.from(s, "base64url");
  return bytes.length === length && bytes.toString("base64url") === s ? bytes : null;
}

// The y coordinates (little-endian, sign bit cleared) of the points of small
// order, 1, 2, 4 or 8: y = 1 (the identity), p - 1 (order 2), 0 (order 4), the
// two values of the order-8 points, and p and p + 1, which are 0 and 1 written
// non-canonically (y >= p). A point is decoded with y reduced mod p, so these
// are every 32-byte encoding of a small-order point, with either sign bit.
const nearP = (low) => `${low}${"ff".repeat(30)}7f`; // p - 1, p and p + 1 by their lowest byte
const SMALL_ORDER_Y = new Set([
  "00".repeat(32),
  `01${"00".repeat(31)}`,
  "26e8958fc2b227b045c3f489f2ef98f0d5dfac05d3c63339b13802886d53fc05",
  "c7176a703d4dd84fba3c0b760d10670f2a2053fa2c39ccc64ec7fd7792ac037a",
  nearP("ec"),
  nearP("ed"),
  nearP("ee"),
]);

function isSmallOrder(encoding) {
  const y = Buffer.from(encoding);
  y[31] &= 0x7f;
  return SMALL_ORDER_Y.has(y.toString("hex"));
}

// Ed25519 verification as evidence/0 requires it (strict, like ed25519-dalek's
// verify_strict): Node's check (S below the group order, and R equal to the
// encoding of [S]B - [k]A), and also neither the key A nor the signature's R
// may be a point of small order. Node alone accepts those, and with them a
// signature that verifies for any message.
export function verifyAttestation(att) {
  const { sig, ...unsigned } = att;
  const raw = decodeB64(String(att.key).replace(/^ed25519:/, ""), 32);
  const sigBytes = decodeB64(String(sig), 64);
  if (!raw || !sigBytes) return false;
  if (isSmallOrder(raw) || isSmallOrder(sigBytes.subarray(0, 32))) return false;
  const key = createPublicKey({ key: Buffer.concat([SPKI_PREFIX, raw]), format: "der", type: "spki" });
  return edVerify(null, Buffer.from(canonicalize(unsigned), "utf8"), key, sigBytes);
}

// --- Verification (integrity subset) -----------------------------------------

// `input` is the object as JSON text or bytes.
export function verifyIntegrity(input, { attestations = "", require = [] } = {}) {
  const obj = parseStrict(input);
  const id = objectId(obj);
  if (obj.id !== id) {
    throw new EvidenceError("E_ID_MISMATCH", `id is ${obj.id} but the object hashes to ${id}`);
  }
  if (obj.content && "inline" in obj.content) {
    const h = digest(obj.content.inline);
    if (h !== obj.content.hash) {
      throw new EvidenceError("E_CONTENT_HASH_MISMATCH", `content.inline hashes to ${h}`);
    }
  }
  const counts = {};
  for (const line of attestations.split("\n")) {
    if (!line.trim()) continue;
    // A line that is not strict JSON is not attestation/0 (evidence/0 step 7).
    let att;
    try {
      att = parseStrict(line);
    } catch (e) {
      if (e instanceof EvidenceError) throw new EvidenceError("E_ATTESTATION_SHAPE", `not strict JSON (${e.code})`);
      throw e;
    }
    if (att.object !== id) continue;
    if (!verifyAttestation(att)) {
      throw new EvidenceError("E_SIGNATURE_INVALID", `${att.purpose} signature by ${att.signer}`);
    }
    counts[att.purpose] = (counts[att.purpose] ?? 0) + 1;
  }
  for (const purpose of require) {
    if (!counts[purpose]) throw new EvidenceError("E_ATTESTATION_MISSING", purpose);
  }
  return { id, attestations: counts };
}

// Error codes this reference can detect; vectors expecting other codes are skipped.
export const SUPPORTED_CODES = new Set([
  "E_PARSE",
  "E_DUPLICATE_KEY",
  "E_NUMBER_RANGE",
  "E_ID_MISMATCH",
  "E_CONTENT_HASH_MISMATCH",
  "E_SIGNATURE_INVALID",
  "E_ATTESTATION_MISSING",
]);
