// Independent reference implementation of coordinate/0 (spec/coordinate-0.md),
// in JavaScript with no dependencies (Node >= 20). Like evidence.mjs and
// log.mjs, it exists so the Rust implementation is checked against a second
// implementation written from the spec, not against itself.
//
// Scope: loading a coordinate-dictionary/0 (the checks of "Loading a
// dictionary", in their order) and its dictionary id; the five operations
// encode, decode, divides, check and extends, with their error codes and the
// order in which errors are reported. Coordinates are BigInts inside and
// canonical decimal strings outside.
//
// Errors: a coordinate/0 result that is an error is thrown as CoordinateError,
// whose `error` member is the error object of the spec ({code, ...members},
// no free text). An invalid argument to these functions, such as a facet list
// that is not an array of strings, is EvidenceError E_USAGE, never a
// coordinate/0 result.

import { canonicalize, digest, EvidenceError } from "./evidence.mjs";
import { parseStrictJson } from "./log.mjs";

export const DICTIONARY_V = "coordinate-dictionary/0";
export const MAX_FACETS = 4096;
export const PRIME_LIMIT = 2 ** 31; // primes are below 2^31
export const LIMIT = 1n << 1024n; // coordinates are below 2^1024
export const LIMIT_DIGITS = LIMIT.toString().length; // 309

export const CODES = Object.freeze({
  DICTIONARY: "P_COORDINATE_DICTIONARY",
  SYNTAX: "P_COORDINATE_SYNTAX",
  RANGE: "P_COORDINATE_RANGE",
  UNKNOWN_FACET: "P_COORDINATE_UNKNOWN_FACET",
  UNKNOWN_FACTOR: "P_COORDINATE_UNKNOWN_FACTOR",
  CARDINALITY: "P_COORDINATE_CARDINALITY",
  DRIFT: "P_COORDINATE_DRIFT",
  NOT_EXTENSION: "P_COORDINATE_NOT_EXTENSION",
});

export const VALUES = Object.freeze(["one", "set", "multiset"]);

// Every pattern must match the whole string. Without the m flag, ECMAScript's
// `$` matches only at the end of the input, never before a final newline, and
// `[a-z]` and `[0-9]` are ASCII.
const PROFILE_RE = /^[a-z][a-z0-9-]*\/(0|[1-9][0-9]*)$/;
const FAMILY_RE = /^[a-z][a-z0-9-]*$/;
const FACET_ID_RE = /^[a-z][a-z0-9-]*\.[a-z0-9][a-z0-9-]*$/;
const COORDINATE_RE = /^[1-9][0-9]*$/;

export class CoordinateError extends Error {
  constructor(error) {
    super(canonicalize(error));
    this.error = error;
    this.code = error.code;
  }
}

const fail = (error) => {
  throw new CoordinateError(error);
};
const usage = (why) => {
  throw new EvidenceError("E_USAGE", why);
};
const isPlainObject = (v) => v !== null && typeof v === "object" && !Array.isArray(v);
const hasExactly = (obj, names) => {
  const keys = Object.keys(obj);
  return keys.length === names.length && names.every((n) => Object.hasOwn(obj, n));
};

// Trial division. Only called for integers 2 <= n < 2^31, where divisors up to
// 46,340 are enough.
export function isPrime(n) {
  if (n < 2) return false;
  if (n % 2 === 0) return n === 2;
  for (let d = 3; d * d <= n; d += 2) if (n % d === 0) return false;
  return true;
}

// --- Loading a dictionary ---------------------------------------------------------

const loaded = new WeakSet();

const dictionaryError = (detail, core) =>
  fail(core ? { code: CODES.DICTIONARY, detail, core } : { code: CODES.DICTIONARY, detail });

const utf8 = new TextDecoder("utf-8", { fatal: true, ignoreBOM: true });

// Loads a dictionary from its bytes (a Buffer or Uint8Array) or its JSON text
// (a string). Step 1 is evidence/0's strict parse; the rest is
// validateDictionary. Returns the loaded dictionary, whose `id` is the
// dictionary id.
export function parseDictionary(input) {
  let text;
  if (typeof input === "string") {
    text = input;
  } else if (input instanceof Uint8Array) {
    try {
      text = utf8.decode(input); // a BOM is kept, so the strict parse rejects it
    } catch {
      dictionaryError("shape", "E_PARSE");
    }
  } else {
    usage("a dictionary is loaded from bytes or a string");
  }
  let value;
  try {
    value = parseStrictJson(text);
  } catch (e) {
    if (!(e instanceof EvidenceError)) throw e;
    dictionaryError("shape", e.code);
  }
  return validateDictionary(value);
}

// Steps 2 to 8 on an already parsed value (a value as JSON.parse gives it).
export function validateDictionary(value) {
  // Step 2: the whole shape, before any later step.
  const familyShape = (f) =>
    isPlainObject(f) && hasExactly(f, ["name", "values"]) && typeof f.name === "string" && typeof f.values === "string";
  const facetShape = (f) =>
    isPlainObject(f) && hasExactly(f, ["id", "prime"]) && typeof f.id === "string" && typeof f.prime === "number";
  const shapeOk =
    isPlainObject(value) &&
    hasExactly(value, ["v", "profile", "families", "facets"]) &&
    typeof value.v === "string" &&
    typeof value.profile === "string" &&
    Array.isArray(value.families) &&
    value.families.every(familyShape) &&
    Array.isArray(value.facets) &&
    value.facets.every(facetShape);
  if (!shapeOk) dictionaryError("shape");

  if (value.v !== DICTIONARY_V) dictionaryError("version"); // step 3
  if (!PROFILE_RE.test(value.profile)) dictionaryError("profile"); // step 4

  // Step 5: every check on one family before the next.
  const names = new Set();
  let previousName = null;
  for (const f of value.families) {
    if (!FAMILY_RE.test(f.name)) dictionaryError("family-name");
    if (!VALUES.includes(f.values)) dictionaryError("values");
    if (names.has(f.name)) dictionaryError("family-duplicate");
    // Names are ASCII by now, so comparing UTF-16 code units is code point order.
    if (previousName !== null && !(f.name > previousName)) dictionaryError("family-order");
    names.add(f.name);
    previousName = f.name;
  }

  if (value.facets.length < 1) dictionaryError("empty"); // step 6
  if (value.facets.length > MAX_FACETS) dictionaryError("too-many"); // step 7

  // Step 8: every check on one facet before the next.
  const ids = new Set();
  const primes = new Set();
  let previousPrime = null;
  for (const f of value.facets) {
    if (!FACET_ID_RE.test(f.id)) dictionaryError("id-form");
    if (!names.has(f.id.slice(0, f.id.indexOf(".")))) dictionaryError("unknown-family");
    if (ids.has(f.id)) dictionaryError("id-duplicate");
    if (!Number.isInteger(f.prime)) dictionaryError("not-integer"); // the parsed double
    if (!(f.prime >= 2 && f.prime < PRIME_LIMIT)) dictionaryError("prime-range");
    if (!isPrime(f.prime)) dictionaryError("not-prime");
    if (primes.has(f.prime)) dictionaryError("prime-duplicate");
    if (previousPrime !== null && !(f.prime > previousPrime)) dictionaryError("prime-order");
    ids.add(f.id);
    primes.add(f.prime);
    previousPrime = f.prime;
  }

  return makeDictionary(value);
}

function makeDictionary(input) {
  // A fresh copy with the members in spec order; -0 cannot occur (step 8e).
  const value = {
    v: input.v,
    profile: input.profile,
    families: input.families.map((f) => ({ name: f.name, values: f.values })),
    facets: input.facets.map((f) => ({ id: f.id, prime: f.prime })),
  };
  const families = value.families.map((f) => Object.freeze({ ...f }));
  const facets = value.facets.map((f) =>
    Object.freeze({ id: f.id, prime: f.prime, big: BigInt(f.prime), family: f.id.slice(0, f.id.indexOf(".")) }),
  );
  const dictionary = Object.freeze({
    value: deepFreeze(value),
    id: digest(value),
    profile: value.profile,
    families: Object.freeze(families), // sorted by name
    facets: Object.freeze(facets), // sorted by ascending prime
    familyByName: new Map(families.map((f) => [f.name, f])),
    facetById: new Map(facets.map((f) => [f.id, f])),
  });
  loaded.add(dictionary);
  return dictionary;
}

function deepFreeze(v) {
  if (v !== null && typeof v === "object") {
    for (const item of Object.values(v)) deepFreeze(item);
    Object.freeze(v);
  }
  return v;
}

const requireDictionary = (d, what = "dictionary") => {
  if (!loaded.has(d)) usage(`${what} is not a loaded dictionary (use parseDictionary or validateDictionary)`);
};

// The dictionary id: "sha256:" + lowercase hex SHA-256 of the canonical form.
// Takes a loaded dictionary, or a value that is first validated (steps 2 to 8).
export function dictionaryId(dictionary) {
  return loaded.has(dictionary) ? dictionary.id : validateDictionary(dictionary).id;
}

// The canonical form of a loaded dictionary, as UTF-8 text.
export function dictionaryCanonical(dictionary) {
  requireDictionary(dictionary);
  return canonicalize(dictionary.value);
}

// --- Family rules ---------------------------------------------------------------------

// The first family, in the order of `families`, whose rule the exponents break,
// or null. `exponents` maps facet ids of the dictionary to exponents >= 1.
function brokenFamily(d, exponents) {
  const total = new Map(); // family name -> sum of exponents
  const largest = new Map(); // family name -> largest exponent
  for (const [id, e] of exponents) {
    const { family } = d.facetById.get(id);
    total.set(family, (total.get(family) ?? 0) + e);
    largest.set(family, Math.max(largest.get(family) ?? 0, e));
  }
  for (const f of d.families) {
    if (f.values === "one" && (total.get(f.name) ?? 0) > 1) return f.name;
    if (f.values === "set" && (largest.get(f.name) ?? 0) > 1) return f.name;
  }
  return null;
}

// --- Operations -------------------------------------------------------------------------

// encode(dictionary, facets): a list of facet ids (repeats count, order does
// not matter) to its coordinate, a canonical decimal string.
export function encode(d, facets) {
  requireDictionary(d);
  if (!Array.isArray(facets) || !facets.every((f) => typeof f === "string")) usage("facets is an array of strings");
  // 1. Every id in the dictionary; the first unknown one in list order.
  for (const id of facets) if (!d.facetById.has(id)) fail({ code: CODES.UNKNOWN_FACET, facet: id });
  // 2. Family rules, in the order of `families`.
  const exponents = new Map();
  for (const id of facets) exponents.set(id, (exponents.get(id) ?? 0) + 1);
  const family = brokenFamily(d, exponents);
  if (family !== null) fail({ code: CODES.CARDINALITY, family });
  // 3. The product, below 2^1024. Every prime is at least 2, so an exponent of
  //    1,024 or more reaches 2^1024 on its own; otherwise stop once it does.
  let product = 1n;
  for (const [id, e] of exponents) {
    if (e >= 1024) fail({ code: CODES.RANGE });
    product *= d.facetById.get(id).big ** BigInt(e);
    if (product >= LIMIT) fail({ code: CODES.RANGE });
  }
  return product.toString(10);
}

// Steps 1 and 2 of decode: a canonical decimal string below 2^1024, as a BigInt.
function coordinateValue(coordinate) {
  // The pattern reads the whole string before any length shortcut.
  if (typeof coordinate !== "string" || !COORDINATE_RE.test(coordinate)) fail({ code: CODES.SYNTAX });
  if (coordinate.length > LIMIT_DIGITS) fail({ code: CODES.RANGE });
  const n = BigInt(coordinate);
  if (n >= LIMIT) fail({ code: CODES.RANGE });
  return n;
}

// decode(dictionary, coordinate): a list of {id, exponent}, one for each facet
// with exponent >= 1, in ascending prime order. `coordinate` is any JSON value.
export function decode(d, coordinate) {
  requireDictionary(d);
  let residue = coordinateValue(coordinate);
  // 3. Divide out each prime, in ascending order, as often as it divides.
  const out = [];
  for (const f of d.facets) {
    if (residue === 1n) break;
    let exponent = 0;
    while (residue % f.big === 0n) {
      residue /= f.big;
      exponent += 1;
    }
    if (exponent) out.push({ id: f.id, exponent });
  }
  if (residue !== 1n) fail({ code: CODES.UNKNOWN_FACTOR });
  // 4. Family rules.
  const family = brokenFamily(d, new Map(out.map((x) => [x.id, x.exponent])));
  if (family !== null) fail({ code: CODES.CARDINALITY, family });
  return out;
}

// divides(dictionary, query, coordinate): true exactly when the query divides
// the coordinate. Both are always decoded, the query first.
export function divides(d, query, coordinate) {
  requireDictionary(d);
  const q = decode(d, query);
  const c = new Map(decode(d, coordinate).map((x) => [x.id, x.exponent]));
  return q.every((x) => (c.get(x.id) ?? 0) >= x.exponent);
}

// check(dictionary, facets, claimed): true (ok) when `claimed` is exactly the
// string that the facets encode to. `claimed` is any JSON value.
export function check(d, facets, claimed) {
  const encoded = encode(d, facets);
  if (claimed !== encoded) fail({ code: CODES.DRIFT });
  return true;
}

// extends(old, new): true (ok) when `next` only adds families and facets to `old`.
function extends_(old, next) {
  requireDictionary(old, "old");
  requireDictionary(next, "new");
  const notExtension = (detail) => fail({ code: CODES.NOT_EXTENSION, detail });
  if (old.profile !== next.profile) notExtension("profile");
  for (const f of old.families) {
    const g = next.familyByName.get(f.name);
    if (!g || g.values !== f.values) notExtension("family");
  }
  for (const f of old.facets) {
    const g = next.facetById.get(f.id);
    if (!g || g.prime !== f.prime) notExtension("facet");
  }
  return true;
}
// `extends` is a reserved word, so it cannot name a function; it can name an
// export.
export { extends_ as extends };

// The facets of a decode result as a list of ids, each repeated as often as
// its exponent says: encode of this list gives the coordinate back.
export const expand = (decoded) => decoded.flatMap((x) => Array(x.exponent).fill(x.id));
