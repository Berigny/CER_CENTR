// Types for evidence.mjs.

export type Bytes = string | Uint8Array | ArrayBuffer | ArrayBufferView;
export type Purpose = "author" | "approve" | "witness";

export declare const PURPOSES: readonly Purpose[];

export declare class EvidenceError extends Error {
  readonly code: string;
  readonly detail: string;
}

export interface Report {
  ok: boolean;
  id?: string;
  code?: string;
  detail?: string;
  content_checked: boolean;
  attestations: Partial<Record<Purpose, number>>;
}

export interface VerifyOptions {
  attestations?: Bytes;
  /** Objects that `prev` may point at. Omit to skip the prev check. */
  store?: Bytes[];
  require?: Purpose[];
}

export interface SignOptions {
  seedHex: string;
  signer: string;
  purpose: Purpose;
  /** UTC timestamp; defaults to now. */
  at?: string;
}

export declare class Evidence {
  version(): { spec: string; crate: string };
  canonicalize(input: Bytes): string;
  contentHash(input: Bytes): string;
  fileHash(input: Bytes): string;
  computeId(input: Bytes): string;
  seal(input: Bytes): string;
  verify(input: Bytes, options?: VerifyOptions): Report;
  sign(input: Bytes, options: SignOptions): string;
  publicKey(seedHex: string): string;
  static generateSeed(): string;
}

export declare function load(
  source?: WebAssembly.Module | Bytes | Response | URL | string | Promise<Response>,
): Promise<Evidence>;
