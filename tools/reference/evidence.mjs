// Independent reference implementation of evidence/0, in JavaScript with no
// dependencies (Node >= 20). It exists so the Rust core is checked against a
// second implementation, not just against itself.
//
// Scope: canonicalisation, ids, inline content hashes, Ed25519 attestations and
// required purposes. Not covered here (the Rust core and the JSON Schema cover
// them): duplicate member names (JSON.parse keeps the last one silently), shape
// rules, calendar checks on timestamps, and prev links.

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

// Parse JSON text with the evidence/0 number and string rules.
export function parseStrict(text) {
  let value;
  try {
    value = JSON.parse(text);
  } catch (e) {
    throw new EvidenceError("E_PARSE", e.message);
  }
  check(value);
  return value;
}

function check(v) {
  if (typeof v === "number") {
    if (!(Math.abs(v) <= Number.MAX_SAFE_INTEGER)) {
      throw new EvidenceError("E_NUMBER_RANGE", `number ${v} has magnitude above 2^53 - 1`);
    }
  } else if (typeof v === "string") {
    if (!v.isWellFormed()) throw new EvidenceError("E_PARSE", "lone surrogate in string");
  } else if (Array.isArray(v)) {
    v.forEach(check);
  } else if (v !== null && typeof v === "object") {
    for (const [k, item] of Object.entries(v)) {
      if (!k.isWellFormed()) throw new EvidenceError("E_PARSE", "lone surrogate in member name");
      check(item);
    }
  }
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

export function verifyAttestation(att) {
  const { sig, ...unsigned } = att;
  const raw = decodeB64(String(att.key).replace(/^ed25519:/, ""), 32);
  const sigBytes = decodeB64(String(sig), 64);
  if (!raw || !sigBytes) return false;
  const key = createPublicKey({ key: Buffer.concat([SPKI_PREFIX, raw]), format: "der", type: "spki" });
  return edVerify(null, Buffer.from(canonicalize(unsigned), "utf8"), key, sigBytes);
}

// --- Verification (integrity subset) -----------------------------------------

export function verifyIntegrity(objectText, { attestations = "", require = [] } = {}) {
  const obj = parseStrict(objectText);
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
    const att = parseStrict(line);
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
  "E_NUMBER_RANGE",
  "E_ID_MISMATCH",
  "E_CONTENT_HASH_MISMATCH",
  "E_SIGNATURE_INVALID",
  "E_ATTESTATION_MISSING",
]);
