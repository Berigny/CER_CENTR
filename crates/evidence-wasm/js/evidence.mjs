// JavaScript wrapper for the evidence/0 WebAssembly module.
// Works in browsers and Node 18+. No dependencies.
//
//   import { load } from "./evidence.mjs";
//   const ev = await load();                      // evidence_wasm.wasm next to this file
//   const report = ev.verify(objectBytes, { attestations, require: ["approve"] });
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
    if (store != null) {
      if (store.every((s) => typeof s === "string")) args.store = [...store];
      else args.store_base64 = store.map((s) => toBase64(toBytes(s)));
    }
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

function toBytes(value) {
  if (typeof value === "string") return encoder.encode(value);
  if (value instanceof Uint8Array) return value;
  if (value instanceof ArrayBuffer) return new Uint8Array(value);
  if (ArrayBuffer.isView(value)) return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
  throw new TypeError("expected a string, Uint8Array or ArrayBuffer");
}

function toBase64(bytes) {
  if (typeof Buffer !== "undefined") return Buffer.from(bytes.buffer, bytes.byteOffset, bytes.byteLength).toString("base64");
  let binary = "";
  for (let i = 0; i < bytes.length; i += 0x8000) {
    binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  }
  return btoa(binary);
}
