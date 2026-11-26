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
  store?: readonly Bytes[];
  require?: Purpose[];
}

export interface SignOptions {
  seedHex: string;
  signer: string;
  purpose: Purpose;
  /** UTC timestamp; defaults to now. */
  at?: string;
}

/** `"sha256:"` followed by 64 lowercase hex digits. */
export type Hash = string;

/** The store report codes, in sort order. See spec/store-0.md. */
export type FindingCode =
  | "S_MANIFEST_INVALID"
  | "S_SEGMENT_HASH"
  | "S_MANIFEST_MISMATCH"
  | "S_LINE_INVALID"
  | "S_ENTRY_INVALID"
  | "S_WRONG_LOG"
  | "S_SEQ_GAP"
  | "S_CHAIN_BREAK"
  | "S_RECEIVED_REGRESSION"
  | "S_EPOCH_REGRESSION"
  | "S_REF_MISMATCH"
  | "S_DUPLICATE_REF"
  | "S_GENESIS_INVALID"
  | "S_RESERVED_SUBJECT"
  | "S_OBJECT_INVALID"
  | "S_PREV_POSITION"
  | "S_ATTESTATION_INVALID"
  | "S_ATTESTATION_TARGET"
  | "S_CHECKPOINT_INVALID"
  | "S_CHECKPOINT_MISMATCH"
  | "S_CHECKPOINT_UNSIGNED"
  | "S_PROJECTION_DRIFT"
  | "S_TRIGGERS_MISSING"
  | "S_META_MISMATCH";

export interface Finding {
  code: FindingCode;
  /** `entry.seq` of the line the finding is about; always with `ref`. */
  seq?: number;
  /** `entry.ref` of the line the finding is about; always with `seq`. */
  ref?: Hash;
  /** The evidence/0 code behind S_OBJECT_INVALID or S_ATTESTATION_INVALID. */
  core?: string;
  /** The segment file behind S_SEGMENT_HASH. */
  file?: string;
}

/** A store-report/0 object (spec/schema/store-report-0.schema.json). */
export interface StoreReport {
  v: "store-report/0";
  ok: boolean;
  source: "store" | "export";
  /** The log id of the lines; present together with `head`. */
  log?: Hash;
  /** The last usable line in reading order. */
  head?: { seq: number; entry_hash: Hash };
  /** The largest checkpoint with a valid witness and no finding. */
  covered?: { size: number; checkpoint: Hash };
  checked: { entries: number; objects: number; attestations: number; checkpoints: number };
  counts: { forks: number; at_after_received: number };
  findings: Finding[];
  truncated: boolean;
}

export interface VerifyExportOptions {
  /** Keep at most this many findings (1 to 2^53 - 1); defaults to 1000. */
  maxFindings?: number;
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
  /** The entry hash of a log-entry/0 entry. Throws E_USAGE (detail `S_ENTRY_INVALID…`) if invalid. */
  entryHash(input: Bytes): Hash;
  /** The RFC 9162 tree hash of these leaf bytes; each is hashed as a leaf first. Not an array: E_USAGE. */
  merkleRoot(leaves: readonly Bytes[]): Hash;
  /** `index` is 0-based (seq - 1); `leafHash` is sha256 of 0x00 and the canonical entry. */
  verifyInclusion(leafHash: Hash, index: number, size: number, proof: Hash[], root: Hash): boolean;
  verifyConsistency(oldSize: number, newSize: number, oldRoot: Hash, newRoot: Hash, proof: Hash[]): boolean;
  /** `segments` are the segment files' bytes in manifest order, in an array (anything else is E_USAGE). */
  verifyExport(manifest: Bytes, segments: readonly Bytes[], options?: VerifyExportOptions): StoreReport;
  static generateSeed(): string;
}

export declare function load(
  source?: WebAssembly.Module | Bytes | Response | URL | string | Promise<Response>,
): Promise<Evidence>;
