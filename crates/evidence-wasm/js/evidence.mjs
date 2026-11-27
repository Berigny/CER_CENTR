// JavaScript wrapper for the evidence/0 WebAssembly module.
// Works in browsers and Node 18+. No dependencies.
//
//   import { load } from "./evidence.mjs";
//   const ev = await load();                      // evidence_wasm.wasm next to this file
//   const report = ev.verify(objectBytes, { attestations, require: ["approve"] });
//   const storeReport = ev.verifyExport(manifestBytes, [segment1Bytes, segment2Bytes]);
//
// Pass file contents as Uint8Array (exact bytes) or string. Results are
// byte-for-byte the same as the Rust command-line tool and the Python package.

export class EvidenceError extends Error {
  constructor(code, detail) {
    super(`${code}: ${detail}`);
    this.name = "EvidenceError";
    this.code = code;
    this.detail = detail;
  }
}

export const PURPOSES = Object.freeze(["author", "approve", "witness"]);

const encoder = new TextEncoder();
const decoder = new TextDecoder("utf-8", { fatal: true });
const isNode = typeof process !== "undefined" && Boolean(process.versions?.node);

/**
 * Load the module. `source` may be a WebAssembly.Module, bytes, a Response,
 * a URL, or (in Node) a file path. Defaults to `evidence_wasm.wasm` beside this file.
 */
export async function load(source = new URL("./evidence_wasm.wasm", import.meta.url)) {
  const module = await compile(await source);
  const { exports } = await WebAssembly.instantiate(module, {});
  return new Evidence(exports);
}

async function compile(source) {
  if (source instanceof WebAssembly.Module) return source;
  if (source instanceof ArrayBuffer || ArrayBuffer.isView(source)) return WebAssembly.compile(source);
  if (typeof Response !== "undefined" && source instanceof Response) {
    return WebAssembly.compile(await source.arrayBuffer());
  }
  if (isNode && (typeof source === "string" || source.protocol === "file:")) {
    const { readFile } = await import("node:fs/promises");
    return WebAssembly.compile(await readFile(source));
  }
  const response = await fetch(source);
  if (!response.ok) throw new Error(`cannot fetch ${source}: HTTP ${response.status}`);
  return WebAssembly.compile(await response.arrayBuffer());
}

export class Evidence {
  #exports;

  constructor(exports) {
    this.#exports = exports;
  }

  /** `{spec, crate}` versions. */
  version() {
    return this.#call("version", {});
  }

  /** Canonical JSON (RFC 8785) of any JSON text, under the strict parsing rules. */
  canonicalize(input) {
    return this.#call("canonicalize", bytesArg({}, "input", input));
  }

  /** `content.hash` for a JSON record: sha256 of its canonical form. */
  contentHash(input) {
    return this.#call("content_hash", bytesArg({}, "input", input));
  }

  /** sha256 of raw bytes: `schema.hash`, and `content.hash` for non-JSON content. */
  fileHash(input) {
    return this.#call("file_hash", bytesArg({}, "input", input));
  }

  /** The id an object should have. */
  computeId(input) {
    return this.#call("compute_id", bytesArg({}, "input", input));
  }

  /** The object with its id filled in, as canonical JSON. Call verify() to check it. */
  seal(input) {
    return this.#call("seal", bytesArg({}, "input", input));
  }

  /**
   * Verify an object. Returns the report (`ok`, `id`, `code`, `detail`,
   * `content_checked`, `attestations`). Throws EvidenceError (E_USAGE) only
   * for invalid arguments.
   */
  verify(input, { attestations, store, require = [] } = {}) {
    const args = bytesArg({}, "input", input);
    bytesArg(args, "attestations", attestations);
    bytesListArg(args, "store", store);
    args.require = [...require];
    return this.#call("verify", args);
  }

  /** Sign a valid object; returns one attestation line (without the newline). */
  sign(input, { seedHex, signer, purpose, at = new Date().toISOString() }) {
    const args = bytesArg({}, "input", input);
    Object.assign(args, { seed_hex: seedHex, signer, purpose, at });
    return this.#call("sign", args);
  }

  /** The `ed25519:` public key for a hex seed. */
  publicKey(seedHex) {
    return this.#call("public_key", { seed_hex: seedHex });
  }

  /**
   * The entry hash of a log-entry/0 entry: sha256 of its canonical form.
   * Throws EvidenceError E_USAGE, detail beginning `S_ENTRY_INVALID`, for an
   * invalid entry.
   */
  entryHash(input) {
    return this.#call("entry_hash", bytesArg({}, "input", input));
  }

  /** The RFC 9162 tree hash of these leaves (each hashed as a leaf first). */
  merkleRoot(leaves) {
    return this.#call("merkle_root", bytesListArg({}, "leaves", leaves));
  }

  /**
   * Check an inclusion proof: that the leaf with this leaf hash
   * (sha256 of 0x00 and the canonical entry) is at 0-based `index`
   * (seq - 1) in the tree of `size` leaves with this root. True or false.
   */
  verifyInclusion(leafHash, index, size, proof, root) {
    return this.#call("verify_inclusion", { leaf_hash: leafHash, index, size, proof, root });
  }

  /** Check a consistency proof from the tree of `oldSize` leaves to the tree of `newSize`. True or false. */
  verifyConsistency(oldSize, newSize, oldRoot, newRoot, proof) {
    return this.#call("verify_consistency", {
      old_size: oldSize,
      new_size: newSize,
      old_root: oldRoot,
      new_root: newRoot,
      proof,
    });
  }

  /**
   * Verify an export: the bytes of `manifest.json` and of the segment files
   * in manifest order. Returns the store-report/0 object, however damaged the
   * export is; throws EvidenceError (E_USAGE) only for invalid arguments.
   */
  verifyExport(manifest, segments, { maxFindings } = {}) {
    const args = bytesArg({}, "manifest", manifest);
    bytesListArg(args, "segments", segments);
    if (maxFindings != null) args.max_findings = maxFindings;
    return this.#call("verify_export", args);
  }

  /** A new random 32-byte seed as hex. Development use; keep production keys in an HSM. */
  static generateSeed() {
    const seed = crypto.getRandomValues(new Uint8Array(32));
    return Array.from(seed, (b) => b.toString(16).padStart(2, "0")).join("");
  }

  #call(fn, args) {
    const { memory, evidence_alloc, evidence_call, evidence_free } = this.#exports;
    const request = encoder.encode(JSON.stringify({ fn, args }));
    const ptr = evidence_alloc(request.length);
    new Uint8Array(memory.buffer, ptr, request.length).set(request);
    const out = evidence_call(ptr, request.length);
    evidence_free(ptr, request.length);
    // Views are taken after each call: memory.buffer changes if memory grows.
    const length = new DataView(memory.buffer).getUint32(out, true);
    const text = decoder.decode(new Uint8Array(memory.buffer, out + 4, length));
    evidence_free(out, 4 + length);
    const response = JSON.parse(text);
    if (response.error) throw new EvidenceError(response.error.code, response.error.detail);
    return response.result;
  }
}

function bytesArg(args, name, value) {
  if (value == null) return args;
  if (typeof value === "string") args[name] = value;
  else args[`${name}_base64`] = toBase64(toBytes(value));
  return args;
}

// A code point in the surrogate range: with the u flag a well-formed pair is
// one code point outside it, so this matches only a lone surrogate.
const LONE_SURROGATE = /\p{Cs}/u;

// A list of byte inputs, which must be an array: a string or a single byte
// array is E_USAGE, not a list of its characters or bytes. Strings go as they
// are when all items are strings, otherwise every item goes as base64, so exact
// bytes survive; a string with a lone surrogate, which has no UTF-8 form, is
// then E_USAGE, as it is when sent as it is.
function bytesListArg(args, name, values) {
  if (values == null) return args;
  if (!Array.isArray(values)) throw new EvidenceError("E_USAGE", `"${name}" must be an array`);
  if (values.every((s) => typeof s === "string")) {
    args[name] = [...values];
    return args;
  }
  args[`${name}_base64`] = values.map((s, i) => {
    if (typeof s === "string" && LONE_SURROGATE.test(s)) {
      throw new EvidenceError("E_USAGE", `"${name}"[${i}] has a lone surrogate, which is not text`);
    }
    let bytes;
    try {
      bytes = toBytes(s);
    } catch (error) {
      if (!(error instanceof TypeError)) throw error;
      throw new EvidenceError("E_USAGE", `"${name}"[${i}] must be a string, Uint8Array or ArrayBuffer`);
    }
    // Outside the try: an input too large to encode keeps its own error.
    return toBase64(bytes);
  });
  return args;
}

function toBytes(value) {
  if (typeof value === "string") return encoder.encode(value);
  if (value instanceof Uint8Array) return value;
  if (value instanceof ArrayBuffer) return new Uint8Array(value);
  if (ArrayBuffer.isView(value)) return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
  throw new TypeError("expected a string, Uint8Array or ArrayBuffer");
}

// Bytes per btoa call: a multiple of 3, so the encoded chunks join into one
// standard base64 text. Small calls matter: Chromium's btoa returns "" instead
// of throwing when its result would be longer than the longest string.
const BASE64_CHUNK = 3 * 8192;

// Standard base64 of the bytes. An input whose base64 form does not fit in one
// string throws the engine's error (a RangeError in browsers). A result of any
// other length than 4 * ceil(n / 3) throws too, rather than reach the module as
// shorter bytes.
function toBase64(bytes) {
  let text;
  if (typeof Buffer !== "undefined") {
    text = Buffer.from(bytes.buffer, bytes.byteOffset, bytes.byteLength).toString("base64");
  } else {
    const chunks = [];
    for (let i = 0; i < bytes.length; i += BASE64_CHUNK) {
      chunks.push(btoa(String.fromCharCode(...bytes.subarray(i, i + BASE64_CHUNK))));
    }
    text = chunks.join("");
  }
  const want = 4 * Math.ceil(bytes.length / 3);
  if (text.length !== want) {
    throw new RangeError(`base64 of ${bytes.length} bytes came out as ${text.length} characters, not ${want}`);
  }
  return text;
}
