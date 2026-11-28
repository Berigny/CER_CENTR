# coordinate/0

Status: draft for discussion. Version tag `coordinate/0`; breaking changes bump the number.

coordinate/0, facet coordinates, turns a list of facets into one number, the **coordinate**, and back, using a published dictionary that gives each facet its own prime. It is a separate format outside the [evidence/0](evidence-0.md) core ([D6](../docs/decisions.md#d6-one-small-core-many-profiles)), with its own vectors ([D7](../docs/decisions.md#d7-the-vectors-are-the-contract)). D6 keeps prime coordinates out of the core and lets them come back in profiles and query layers; coordinate/0 is the arithmetic they share, for [Kernel/2](../docs/kernel-2.md) and for the index's coordinate column ([roadmap M4b](../docs/roadmap.md#m4b-coordinate-column-and-factor-table)). It changes nothing in evidence/0 or [store/0](store-0.md).

## Purpose and scope

A profile or a query layer that sorts records into categories (record type, product class, standard, jurisdiction, a lifecycle state) can summarise the categories of one record as one integer: the product of their primes. By unique factorisation the integer decodes back to exactly those categories. A query such as "every installation record in Victoria under AS/NZS 4777.2" then has one exact meaning: the query's coordinate divides the record's coordinate.

The arithmetic is long established; it is the idea behind Gödel numbering. coordinate/0 fixes the details that two implementations must agree on. Together with a profile's rules for which facets a record has, the same dictionary version, watermark and query then give the same rows in any implementation, so third parties can reproduce the results of faceted queries as well as the evidence.

coordinate/0 defines:

- the dictionary format, `coordinate-dictionary/0`, and the dictionary id;
- the coordinate, a canonical decimal string below 2^1024;
- five operations (encode, decode, divides, check and extends), their error codes, and the order in which errors are reported;
- a tagged form for coordinates in exports and indexes, and rules for indexing them.

It does not say which facets a record has. That is a profile's rule: for Kernel/2, the facets its events add along the chain; for another profile, values read from the record. coordinate/0 starts from the list of facets.

Coordinates are derived. A coordinate is never stored in the object it describes and never part of that object's id, and a reader recomputes it rather than trusting a stored value.

Kernel/2 uses coordinate/0's encoding and keeps its own rules: its allocation rule, its events, its two coordinates S and H, its gates and its codes.

### Terms

- **Strict JSON** and **canonical form:** as in [store/0](store-0.md#terms). UTF-8 with no byte order mark, no duplicate members, no lone surrogates, no number with magnitude above 2^53 − 1, nesting at most 128 deep; the canonical form is [RFC 8785](https://www.rfc-editor.org/rfc/rfc8785).
- **Patterns:** every pattern in this spec must match the whole string. `^` is the start of the string and `$` its end, so a string with a trailing newline does not match. Where an engine's `$` also matches before a final newline, as Python's `re` and PCRE do, an implementation uses an anchor that matches only at the end of input (Python's `\Z`, PCRE's `\z`) or a whole-string match. Character classes are ASCII: `[0-9]` is the ten ASCII digits, and `[a-z]` the 26 ASCII lowercase letters.
- **Facet:** one category, written `<family>.<name>`, such as `jurisdiction.vic`.
- **Family:** a group of facets with one rule for how many of them a coordinate may hold: `one`, `set` or `multiset`.
- **Dictionary:** a published `coordinate-dictionary/0` document that declares a profile's families and gives each facet a distinct prime.
- **Exponent:** how many times a facet occurs in a coordinate, 1 or more.
- **Coordinate:** the product of the primes of a multiset of facets, each prime raised to its facet's exponent. The empty multiset has coordinate 1.
- **Coordinate name:** what a profile calls one of the coordinates it derives for an object, such as Kernel/2's `S` and `H`.
- **Query:** a coordinate used to select records. A record matches when the query divides its coordinate.
- **Residue:** what is left of a coordinate after every dictionary prime has been divided out of it as often as it divides.
- **Watermark:** a log position, seq plus entry hash ([roadmap M4](../docs/roadmap.md#m4-memory-reads-index-query-api-bundles-read-only-service)), that fixes which entries a query result covers.

### Codes

coordinate/0 uses `P_COORDINATE_` codes, in the `P_` namespace that store/0 leaves to profiles ([store/0](store-0.md#code-namespaces)), because profiles and query layers use it. An invalid argument to a programming interface, such as a facet list that is not a list of strings, gives evidence/0's `E_USAGE`. It is never a coordinate/0 result.

## Dictionary

A dictionary is a JSON object with exactly these members. This one is the vectors' `catalogue-demo` dictionary.

```json
{"v": "coordinate-dictionary/0",
 "profile": "catalogue-demo/0",
 "families": [
   {"name": "jurisdiction", "values": "one"},
   {"name": "product", "values": "one"},
   {"name": "record", "values": "one"},
   {"name": "standard", "values": "set"}],
 "facets": [
   {"id": "record.product-listing", "prime": 2},
   {"id": "record.installation", "prime": 3},
   {"id": "product.inverter", "prime": 5},
   {"id": "standard.as-nzs-4777-2", "prime": 7},
   {"id": "jurisdiction.vic", "prime": 11},
   {"id": "product.battery", "prime": 13},
   {"id": "product.pv-module", "prime": 17},
   {"id": "standard.as-nzs-5033", "prime": 19},
   {"id": "standard.as-nzs-3000", "prime": 23},
   {"id": "standard.as-nzs-5139", "prime": 29},
   {"id": "jurisdiction.nsw", "prime": 31},
   {"id": "jurisdiction.qld", "prime": 37},
   {"id": "jurisdiction.sa", "prime": 41},
   {"id": "jurisdiction.wa", "prime": 43},
   {"id": "jurisdiction.tas", "prime": 47},
   {"id": "jurisdiction.act", "prime": 53},
   {"id": "jurisdiction.nt", "prime": 59},
   {"id": "record.inspection", "prime": 61},
   {"id": "standard.as-nzs-4777-1", "prime": 67}]}
```

Its id is `sha256:005aad78a1aaae4ef493f0f8b20facec76f5dadddefc8de05903754998ca6278`.

This dictionary is synthetic and illustrative. Its names are public categories (Australian jurisdictions and published AS/NZS standards), but it is not a published dictionary, its primes are arbitrary, and it does not decide which dictionary, if any, installation or device records would use. That is open ([turn-reader.md open decision 9](../docs/turn-reader.md#9-kernel2-rules-the-reader-depends-on), [cer-edge.md](../docs/cer-edge.md)). One product class per record, which makes `product` a `one` family, is also a simplification for the illustration: an installation record usually covers several products, such as PV modules and an inverter, often with a battery, and a dictionary for such records could make that family a `set`.

| Member | Rule |
|---|---|
| `v` | `"coordinate-dictionary/0"` |
| `profile` | The id of the profile that publishes the dictionary. Matches `^[a-z][a-z0-9-]*/(0\|[1-9][0-9]*)$`, such as `kernel/2` or `catalogue-demo/0`. |
| `families` | An array of `{"name", "values"}`. Each `name` matches `^[a-z][a-z0-9-]*$`. Names are distinct, and the array is sorted by name in code point order (names are ASCII, so this is byte order). |
| `facets` | An array of `{"id", "prime"}`, at least 1 and at most 4,096 of them. Each `id` is `<family>.<name>`: a declared family's name, a dot, and a name matching `^[a-z0-9][a-z0-9-]*$`. Ids are distinct. Each `prime` is an integer, prime, with 2 ≤ p < 2^31. Primes are distinct, and the array is sorted by ascending prime. |

The profile pattern is a subset of evidence/0's profile ids, its `ext` keys (`^[a-z0-9][a-z0-9.-]*/[0-9]+$`): a profile id here starts with a letter, holds no dot, and writes its version without leading zeros. The dot then only ever separates a family from a facet name, and each profile version has one spelling, so `extends`, tagged forms and indexes, which compare profile ids as strings, cannot take `kernel/02` and `kernel/2` for two versions.

No other members are allowed, at any level. The dictionary holds no labels or descriptions: the profile's own documentation says what each facet means, and code and documents are generated from the dictionary, not copied from it by hand. A profile that maps values from a system of record to facets, such as a standard's code or a jurisdiction, publishes that mapping once, beside its dictionary, pointing at the source values rather than declaring the domain again: a domain declared in two places drifts ([D2](../docs/decisions.md#d2-point-at-the-existing-schema-dont-restate-the-domain)). A source value with no facet is an error, never skipped.

`values` is the family's rule:

| `values` | A coordinate may hold | Example |
|---|---|---|
| `one` | At most one facet of the family, with exponent 1. | A record has one jurisdiction. |
| `set` | Any number of distinct facets of the family, each with exponent 1. | A record cites several standards. |
| `multiset` | Any facets of the family, repeated any number of times. | Kernel/2's history coordinate H, where a block can be entered twice. |

A family may have no facets yet; a later version can add some.

A prime is a JSON number, so being an integer is a property of its value, the IEEE 754 double that evidence/0 parses, not of how it is written, as for every number under evidence/0 ([D5](../docs/decisions.md#d5-stricter-than-rfc-8785-on-input)). `7.0` is the integer 7, and `58.99999999999999999` parses to 59; the canonical form writes them `7` and `59`. A prime below 2^31 fits a signed 32-bit integer in any database.

**Dictionary id.** `"sha256:"` + lowercase hex SHA-256 of the canonical form of the dictionary, as evidence/0 hashes JSON content. The sort rules fix the order of the arrays, and the canonical form fixes everything else, so equal dictionaries have equal ids however their files are laid out. The id names a dictionary version.

### Loading a dictionary

Loading takes a dictionary's bytes and gives either a valid dictionary and its id, or `P_COORDINATE_DICTIONARY` with a `detail`. The checks run in this order and stop at the first failure. Steps 5 and 8 take the families and the facets in array order and run every check on one item before moving to the next.

| Step | Check | `detail` |
|---|---|---|
| 1 | The bytes are strict JSON. | `shape`, with `core` set to evidence/0's code: `E_PARSE`, `E_DUPLICATE_KEY` or `E_NUMBER_RANGE` |
| 2 | The value has the dictionary's shape: an object with exactly the members `v`, `profile`, `families` and `facets`; `v` and `profile` are strings; `families` is an array of objects with exactly `name` and `values`, both strings; `facets` is an array of objects with exactly `id`, a string, and `prime`, a number. | `shape` |
| 3 | `v` is `"coordinate-dictionary/0"`. | `version` |
| 4 | `profile` matches its pattern. | `profile` |
| 5a | For each family: its `name` matches its pattern. | `family-name` |
| 5b | Its `values` is `"one"`, `"set"` or `"multiset"`. | `values` |
| 5c | Its `name` differs from every earlier family's. | `family-duplicate` |
| 5d | Its `name` comes after the previous family's, in code point order. | `family-order` |
| 6 | `facets` has at least one member. | `empty` |
| 7 | `facets` has at most 4,096 members. | `too-many` |
| 8a | For each facet: its `id` is a family name pattern, a dot and a facet name pattern (`^[a-z][a-z0-9-]*\.[a-z0-9][a-z0-9-]*$`). | `id-form` |
| 8b | The part of `id` before the dot is the name of a family in `families`. | `unknown-family` |
| 8c | Its `id` differs from every earlier facet's. | `id-duplicate` |
| 8d | Its `prime` is an integer: its value, the IEEE 754 double that evidence/0 parses, has no fractional part. | `not-integer` |
| 8e | 2 ≤ `prime` < 2^31. | `prime-range` |
| 8f | `prime` is prime. | `not-prime` |
| 8g | Its `prime` differs from every earlier facet's. | `prime-duplicate` |
| 8h | Its `prime` is greater than the previous facet's. | `prime-order` |

- Step 2 covers the whole value before any later step, as evidence/0's `E_SHAPE` does. So a dictionary with an unknown member and a wrong `v` gives `shape`.
- The number of facets (steps 6 and 7) is checked before any facet, so 5,000 facets give `too-many` whatever they hold.
- A repeated name, id or prime is a duplicate even where it also breaks the order: families `a`, `b`, `a` give `family-duplicate`, not `family-order`.
- The range is checked before primality, so 1 and 2^31 give `prime-range`, and primality is only ever tested below 2^31, where trial division up to 46,340 is enough.
- Step 1 gives the code of evidence/0's strict parser. Each vector has one step-1 fault. Where the bytes have more than one, evidence/0 does not yet fix which code comes first, and the two parsers in this repository differ: `{"a": 1, "a": 2, "b": 9007199254740993}` gives `E_DUPLICATE_KEY` in the Rust core and `E_NUMBER_RANGE` in the JavaScript reference. Until evidence/0 fixes it, `core` for such bytes is not part of the contract.
- An implementation that takes an already parsed value starts at step 2. A NaN or an infinity, which only a value built in code can hold, fails step 8d (`not-integer`), and a number above 2^53 − 1 fails step 8e (`prime-range`).
- The JSON Schema is [`schema/coordinate-dictionary-0.schema.json`](schema/coordinate-dictionary-0.schema.json). It holds the checks a schema can express: steps 2, 3, 4, 5a, 5b, 6, 7, 8a, 8d and 8e. A schema cannot express the rest (distinct and sorted names, ids and primes, declared families, primality) or the order of the checks. As for evidence/0 and store/0, implementations check every rule in code, and `tools/check_schema_vectors.py` runs the schema over the dictionary vectors so the two cannot drift ([D9](../docs/decisions.md#d9-hand-written-shape-checks-in-the-core)): every valid dictionary passes it, and every invalid one whose `detail` comes from one of those steps fails it.

## Coordinate

A coordinate is a JSON string that holds a canonical decimal integer:

- It matches `^[1-9][0-9]*$`: no sign, no leading zero, no whitespace (a trailing newline included), no fraction, no exponent. A regular expression whose digit class also matches other scripts' digits does not meet this.
- Its value is below 2^1024, so it has at most 1,024 bits. 2^1024 has 309 decimal digits: a string of 310 digits or more is out of range, and a string of 309 digits may be either.
- It is never a JSON number. evidence/0 rejects JSON numbers above 2^53 − 1 ([evidence/0](evidence-0.md#parsing-and-canonical-form)), and coordinates pass that quickly.
- `"1"` is the coordinate of no facets, and decodes to the empty list. A profile may show the empty list under its own label: Kernel/2 calls it UNCLASSIFIED, "unclassified as a whole", which is distinct from any facet, `universal.unclassified` (5) included (REQ-KRN-011). The vectors check that `"1"` and `"5"` decode differently.

Because the form is canonical, two coordinates are equal exactly when their strings are equal. `check` and indexes compare them that way.

## Operations

Every operation takes a valid, loaded dictionary. Each gives a result or one error. Where more than one error applies, the first in the order given is the result.

### encode

`encode(dictionary, facets)` gives a coordinate. `facets` is a list of facet ids. An id that appears more than once counts once for each time it appears, and the order of the list does not matter.

1. Every id must be in the dictionary. Otherwise `P_COORDINATE_UNKNOWN_FACET`, with `facet` set to the first unknown id in list order. An unknown facet is an error, never skipped.
2. The exponents, the number of times each facet appears, must meet every family's rule. Otherwise `P_COORDINATE_CARDINALITY`, with `family` set to the first family whose rule is broken, in the order of `families` (by name). This is not the order of the facets or of the list.
3. The product of each facet's prime raised to its exponent must be below 2^1024. Otherwise `P_COORDINATE_RANGE`. An implementation may stop multiplying once the product reaches 2^1024.

The result is that product as a canonical decimal string. The empty list gives `"1"`.

### decode

`decode(dictionary, coordinate)` gives a list of `{"id", "exponent"}`, one for each facet whose exponent is at least 1, in ascending prime order.

1. The coordinate must be a string in canonical decimal form. Otherwise `P_COORDINATE_SYNTAX`, including for a JSON number or any other value that is not a string. This step reads the whole string.
2. Its value must be below 2^1024. Otherwise `P_COORDINATE_RANGE`. An implementation may give `P_COORDINATE_RANGE` for a string of more than 309 digits without converting it, but only once step 1 has passed: a long string with a bad character anywhere in it gives `P_COORDINATE_SYNTAX`.
3. Each dictionary prime, in ascending order, is divided out as often as it divides; the number of times is that facet's exponent. If the residue is not 1, `P_COORDINATE_UNKNOWN_FACTOR`. The error does not say what the residue is.
4. The exponents must meet every family's rule. Otherwise `P_COORDINATE_CARDINALITY`, with `family` as in encode.

`"1"` decodes to the empty list. Decode and encode are inverses: encoding the decoded facets, each repeated as often as its exponent says, gives the coordinate back.

### divides

`divides(dictionary, query, coordinate)` gives `true` or `false`.

The query is decoded, then the coordinate. The first error is the result, so an error in the query is reported before an error in the coordinate. Both are always decoded: a query of `"1"` does not excuse a coordinate that fails to decode. Otherwise the result is `true` exactly when every facet of the query has an exponent in the coordinate at least as large as its exponent in the query. That is exactly "the query divides the coordinate".

- The query `"1"` divides every coordinate, `"1"` included.
- A query with a factor outside the dictionary is an error (`P_COORDINATE_UNKNOWN_FACTOR`), not `false`. So is a query that breaks a family rule, such as two facets of a `one` family (`P_COORDINATE_CARDINALITY`): no record could match it.
- Whether a record has one facet is `divides` with that facet's prime as the query, as in Kernel/2's REQ-KRN-008.
- "Any of" and "none of" are combinations of `divides` results at the query layer, not part of coordinate/0.

### check

`check(dictionary, facets, claimed)` gives ok, or an error.

1. The facets are encoded. Errors from encode come first.
2. If `claimed` is not exactly the string encode gives, `P_COORDINATE_DRIFT`. The comparison is exact: a claimed coordinate that is not canonical (`"01155"`), or not a string at all (the JSON number `1155`), gives `P_COORDINATE_DRIFT` too.

This is the check of Kernel/2's REQ-KRN-013. A profile may report coordinate/0 results under its own codes, as Kernel/2 reports drift with `P_KERNEL_COORDINATE_DRIFT`.

### extends

`extends(old, new)` gives ok, or `P_COORDINATE_NOT_EXTENSION` with the `detail` of the first check that fails:

1. `profile`: `new` has a different `profile`.
2. `family`: a family of `old`, taken in `old`'s order, is missing from `new` or has a different `values`.
3. `facet`: a facet of `old`, taken in `old`'s order, is missing from `new` (by id) or has a different prime.

A later version may only add families and facets. Every coordinate that decodes under `old` therefore decodes to the same result under `new`, and a list of `old`'s facets encodes to the same coordinate under both. A coordinate that uses a facet only `new` has gives `P_COORDINATE_UNKNOWN_FACTOR` under `old`. Every dictionary extends itself, and if `b` extends `a` and `c` extends `b`, then `c` extends `a`.

### Worked example

With the `catalogue-demo` dictionary above:

- `encode` of `record.installation`, `product.inverter`, `standard.as-nzs-4777-2` and `jurisdiction.vic` gives `"1155"` (3 × 5 × 7 × 11), whatever the order of the list.
- `decode` of `"1155"` gives those four facets, each with exponent 1, in the order of their primes: `record.installation`, `product.inverter`, `standard.as-nzs-4777-2`, `jurisdiction.vic`.
- "Installation records in Victoria under AS/NZS 4777.2" is the query `"231"`, the encoding of `record.installation`, `standard.as-nzs-4777-2` and `jurisdiction.vic`. `divides` of `"231"` and `"1155"` is `true`. Against `"3255"` (3 × 5 × 7 × 31, the same record in New South Wales) it is `false`.
- `encode` of `jurisdiction.vic` and `jurisdiction.nsw` gives `P_COORDINATE_CARDINALITY` with `family` `jurisdiction`, and so does `decode` of `"341"` (11 × 31).
- `decode` of `"97"` gives `P_COORDINATE_UNKNOWN_FACTOR`: 97 is not in the dictionary.

## Errors

An error is a JSON object with `code` and only the members its code carries. It holds no free text, so every implementation gives the same bytes for the same input. Implementations may give human-readable detail elsewhere, never in the error.

```json
{"code": "P_COORDINATE_DICTIONARY", "detail": "shape", "core": "E_DUPLICATE_KEY"}
{"code": "P_COORDINATE_UNKNOWN_FACET", "facet": "product.heat-pump"}
{"code": "P_COORDINATE_CARDINALITY", "family": "jurisdiction"}
{"code": "P_COORDINATE_NOT_EXTENSION", "detail": "facet"}
```

| Code | Members | Meaning |
|---|---|---|
| `P_COORDINATE_DICTIONARY` | `detail`; `core` only when step 1 fails | The dictionary is invalid. `detail` names the first check that fails ([Loading a dictionary](#loading-a-dictionary)). |
| `P_COORDINATE_SYNTAX` | | The coordinate is not a string in canonical decimal form. |
| `P_COORDINATE_RANGE` | | The coordinate, or the product encode computes, is 2^1024 or more. |
| `P_COORDINATE_UNKNOWN_FACET` | `facet` | A facet id that the dictionary does not hold: the first in list order. |
| `P_COORDINATE_UNKNOWN_FACTOR` | | The coordinate has a prime factor that the dictionary does not hold. |
| `P_COORDINATE_CARDINALITY` | `family` | A family's rule is broken: the first such family in the order of `families`. |
| `P_COORDINATE_DRIFT` | | A claimed coordinate is not the one its facets encode to. |
| `P_COORDINATE_NOT_EXTENSION` | `detail`: `profile`, `family` or `facet` | The new dictionary is not an extension of the old one. |

Order of errors, by operation:

| Operation | Errors, in order of precedence |
|---|---|
| load | `P_COORDINATE_DICTIONARY`, its detail by step |
| encode | `P_COORDINATE_UNKNOWN_FACET`, `P_COORDINATE_CARDINALITY`, `P_COORDINATE_RANGE` |
| decode | `P_COORDINATE_SYNTAX`, `P_COORDINATE_RANGE`, `P_COORDINATE_UNKNOWN_FACTOR`, `P_COORDINATE_CARDINALITY` |
| divides | The query's decode errors, then the coordinate's |
| check | encode's errors, then `P_COORDINATE_DRIFT` |
| extends | `P_COORDINATE_NOT_EXTENSION` with `profile`, then `family`, then `facet` |

## Tagged form

In an export or a query result, a coordinate is carried in tagged form, with its profile and dictionary:

```json
{"profile": "catalogue-demo/0", "dictionary": "sha256:005aad78a1aaae4ef493f0f8b20facec76f5dadddefc8de05903754998ca6278", "coordinate": "1155"}
```

- It has exactly these three members, all strings.
- `profile` is the `profile` of the dictionary that `dictionary` names. Two dictionaries may use the same primes; the profile id, not the primes, keeps them apart, as in [Kernel/2](../docs/kernel-2.md#33-representation-declared).
- `dictionary` is the id of the dictionary version the coordinate was computed with, or of a later version of the same profile that extends it ([Versioning](#versioning)).
- `coordinate` is a coordinate that decodes without error under that dictionary.

A reader decodes the coordinate with the dictionary that `dictionary` names, and with no other. A later version decodes every coordinate that is valid under the named one to the same facets, but it also accepts facets that the named one lacks, and a reader that holds only the later version cannot tell. Dictionaries are small and published, so a reader that does not hold the named one fetches it, or says that it does not hold it; it never decodes with another dictionary.

A tagged form is invalid if its `profile` is not the named dictionary's `profile`, or if its coordinate does not decode under the named dictionary. A reader rejects it, with the decode error in the second case.

**Several coordinates per object.** The tagged form says which dictionary a coordinate belongs to, not which of an object's coordinates it is. A profile that derives more than one coordinate for an object names each one. Kernel/2 derives `S` and `H` for every event, both under `kernel/2` and its one dictionary, so an export carries each tagged form under its coordinate name. For a unit after Kernel/2's event E4:

```json
{"S": {"profile": "kernel/2", "dictionary": "sha256:28ec5d10ee5e6e5fd8effc7515e9adec7f049f75beeccce5064d738d922cac71", "coordinate": "8321327376004027894"},
 "H": {"profile": "kernel/2", "dictionary": "sha256:28ec5d10ee5e6e5fd8effc7515e9adec7f049f75beeccce5064d738d922cac71", "coordinate": "25035387438225574245358414"}}
```

Index rows carry the coordinate name too ([Indexing](#indexing)). Which coordinates a profile derives, and what it calls them, are the profile's rules.

An index may hold the same information in its own columns: in each row the profile, the coordinate and, where a profile derives more than one coordinate per object, the coordinate name; and the dictionary id once for each profile ([Indexing](#indexing)).

## Indexing

In this section the key words MUST, MUST NOT, SHOULD, SHOULD NOT and MAY are used as in RFC 2119. Elsewhere this spec states its rules plainly.

The coordinate defines what a query means. A database answers the query from a table of factors, not by arithmetic on one integer column: SQL integers are 64-bit, and coordinates pass 2^64 quickly.

- **Exact arithmetic or none.** An index MUST NOT take a remainder of a coordinate, or convert or compare one, with fixed-width integer or floating-point arithmetic. The failure is silent. On SQLite 3.53, the product of the first 16 primes, 32589158477190044730 (65 bits), stored as text, gave the floating-point value 33.0 for `% 53` instead of 0, and the 89-bit product of the first 20 primes gave 33.0 too. SQLite converted the text to a floating-point number that does not fit a 64-bit integer, and 33 is the remainder of 2^63 − 1, the largest signed 64-bit integer, divided by 53. No error was raised. PostgreSQL's `numeric` type computes `mod` exactly at these sizes.
- **Store the text.** A stored coordinate MUST be its canonical decimal string, in a text column, never in an integer or floating-point one.
- **Store the factors.** An index SHOULD store each object's factorisation as rows (profile, object, prime, exponent), one row per prime, with an index on (profile, prime), and one row per object with its profile and coordinate. An object whose coordinate is `"1"` has no factor rows; its per-object row is how the query `"1"` finds it. An exponent vector, one column per prime as Kernel/2 allows for its 70, is equivalent. Primes are below 2^31 and exponents at most 1,023, so both fit 32-bit integers. Where a profile derives more than one coordinate per object, every row also holds the coordinate name, and the index is on (profile, name, prime).
- **Valid rows only.** Every coordinate an index stores MUST decode without error under the dictionary it records. An object whose facets do not encode gets no rows, and the index reports it with encode's error; it is never indexed under a partial coordinate.
- **Decode the query first.** An index MUST decode a query with coordinate/0 before it answers it, and for a query that does not decode it MUST return the decode error, never an empty result. Under `catalogue-demo`, the factor rows alone would quietly return no rows for `"341"` (two jurisdictions) or `"97"` (a factor outside the dictionary).
- **Answer from the factors.** An index SHOULD then answer `divides` as "has every prime of the query, each with at least the query's exponent". The query `"1"` returns every object of the profile. For example, for the query `"77"` (7 × 11):

  ```sql
  SELECT object FROM factors
   WHERE profile = 'catalogue-demo/0'
     AND ((prime = 7 AND exponent >= 1) OR (prime = 11 AND exponent >= 1))
   GROUP BY object HAVING count(*) = 2;
  ```

- **Divisibility is the reference, not the fast path.** Testing the divisibility of every stored coordinate, with exact arithmetic, reads every row of the profile: it does not make retrieval faster. It is the reference meaning of a query, and the check on the factor table. An index MUST return the same rows both ways, and the same error for a query that does not decode. Its acceptance test ([roadmap M4b](../docs/roadmap.md#m4b-coordinate-column-and-factor-table)) compares them on synthetic data that includes coordinates past 2^63 and queries that do not decode.
- **No rewritten posting lists.** An index SHOULD NOT keep the list of every object that has a facet as one stored value that is rewritten on every add: the cost of each add would grow with the list.
- **Key on (profile, coordinate),** or on (profile, name, coordinate) where a profile derives more than one coordinate per object, and record, for each profile, the id of the dictionary used to decode. Before it moves to another dictionary version, an index MUST check that the new one extends the one it holds, and MUST refuse it otherwise; its rows then stay valid unchanged.
- **Derived, never authoritative.** Index rows are derived from the objects and the profile's rules. A reader that relies on a stored coordinate recomputes it with `check`. An index MAY be dropped and rebuilt at any time; query indexes live in files of their own ([D12](../docs/decisions.md#d12-one-sqlite-file-is-the-authoritative-store)).

## Performance

- Decoding divides by each dictionary prime at most once more than its exponent, and may stop as soon as the residue is 1. That is at most 4,096 + 1,023 divisions of a number below 2^1024 by a number below 2^31: one trial division per dictionary prime, plus one per factor.
- Target: decoding a 1,024-bit coordinate with a 4,096-facet dictionary takes under 1 ms on reference hardware, as REQ-KRN-007 asks of Kernel/2's 70 facets. This is a target for implementations, not a CI test.
- Every operation needs only multiplication and division of a number below 2^1024 by a number below 2^31, comparison with 2^1024, and decimal conversion. A small hand-written routine is enough where a general big-integer library is not wanted.

## Privacy

- A coordinate hides nothing. Dictionaries are published, and anyone who holds one decodes the coordinate to its facets.
- A facet must never identify a person or hold personal information. Actor ids and approval status are never facets ([approvals.md section 3](../docs/approvals.md#3-parties-and-ids-proposed)).

## What coordinate/0 does not do

- **No ranking and no similarity.** A record matches a query or it does not. A coordinate has no meaningful order: a larger coordinate is not more of anything, and sorting by coordinate means nothing.
- **Not regulatory truth.** A coordinate decides which records a query returns, not whether a record was classified correctly or captured completely.
- **No order or time.** Multiplication ignores order. The evidence chain holds the path; a coordinate is an index over it ([Kernel/2](../docs/kernel-2.md#1-purpose-and-scope)).
- **No facet assignment.** Which facets a record has is the profile's rule.
- **No "any of" or "none of".** Those belong to the query layer.
- **Not an identifier.** A coordinate is not a fingerprint of a record and is never part of an object's id.

## Versioning

- A breaking change to this spec is `coordinate/1`, and to the dictionary format `coordinate-dictionary/1`.
- A profile's dictionary is versioned by content, and its id names each version. A dictionary has no member that names its predecessor, so the profile sets the order: it publishes its versions oldest first, and each one must extend the one before it. Every later version then extends every earlier one. A change that is not an extension needs a new profile id, such as `catalogue-demo/1`, which is a new namespace.
- Two dictionaries under one profile id that do not extend each other cannot both be versions of it. An index refuses a dictionary that does not extend the one it holds ([Indexing](#indexing)).
- A profile may be stricter. Kernel/2 fixes its dictionary: any change is a new kernel version ([Kernel/2 section 6](../docs/kernel-2.md#6-versioning)).
- A shared registry of prime ranges, for profiles that also want disjoint primes, is optional and outside this spec.

## Conformance

An implementation conforms to coordinate/0 if it gives the expected result for every case in `test-vectors/coordinate/`. The vectors are the contract ([D7](../docs/decisions.md#d7-the-vectors-are-the-contract)): `node tools/make-vectors.mjs` generates them from the independent JavaScript reference in `tools/reference/`, the Rust implementation is checked against them, and CI fails if a fresh run differs from the committed files.

### Vector files

| Path | Contents |
|---|---|
| `dictionaries.json` | `{"valid": [...], "invalid": [...]}`. Each `valid` case is `{name, id}`: load `dictionaries/valid/<name>.json`, which must succeed with that dictionary id. Each `invalid` case is `{name, error}`: load `dictionaries/invalid/<name>.json`, which must fail with that error. |
| `dictionaries/valid/<name>.json` | A valid dictionary. |
| `dictionaries/invalid/<name>.json` | The bytes under test, read as raw bytes. |
| `encode.json` | A list of `{name, dictionary, facets, expect}` or `{name, dictionary, facets, error}`. |
| `decode.json` | A list of `{name, dictionary, coordinate, expect}` or `{name, dictionary, coordinate, error}`. |
| `divides.json` | A list of `{name, dictionary, query, coordinate, expect}` or `{name, dictionary, query, coordinate, error}`. |
| `check.json` | A list of `{name, dictionary, facets, coordinate, expect}` or `{name, dictionary, facets, coordinate, error}`. `coordinate` is the claimed one. |
| `extends.json` | A list of `{name, old, new, expect}` or `{name, old, new, error}`. |

- Every case has a `name`, unique within its file, and exactly one of `expect` and `error`. Cases have no other members.
- `dictionary`, `old` and `new` are names of `valid` dictionaries.
- `facets` is an array of strings. `coordinate` and `query` are JSON values passed to the operation as they are: a string in almost every case, and a JSON number in the cases that test that a coordinate must be a string.
- Strings are JSON strings and may hold any character: `"5\n"` is the digit 5 followed by a line feed.
- `expect` is the result: for encode, a coordinate string; for decode, an array of `{"id", "exponent"}`; for divides, `true` or `false`; for check and extends, `true`, meaning ok.
- `error` is the error object ([Errors](#errors)).
- A case passes when the canonical form of the result equals the canonical form of `expect`, or the canonical form of the error equals that of `error`, byte for byte.
- The generator writes every file except those in `dictionaries/invalid/` and `catalogue-demo-reordered` with two-space indentation and a final LF, like the other vector files. Implementations compare canonical forms and must not depend on the layout. Dictionary ids are over the canonical form, never the file's bytes.
- The generator asserts that every decode case with `expect` encodes back to its coordinate, and every encode case with `expect` decodes back to its facets. The cases below are required; the generator may add more.

### Dictionary cases

Valid:

| Name | Content |
|---|---|
| `catalogue-demo` | The dictionary in [Dictionary](#dictionary). |
| `catalogue-demo-reordered` | The same dictionary written differently: members in another order, other whitespace, `\u` escapes in some strings, the prime 7 written `7.0`, and the prime 59 written `58.99999999999999999`, which parses to 59. Its id equals `catalogue-demo`'s. |
| `catalogue-demo-v2` | `catalogue-demo` with `product.heat-pump` (71), and a family `phase` (`one`) with `phase.single` (73) and `phase.three` (79). It extends `catalogue-demo`. |
| `catalogue-demo-values-changed` | `catalogue-demo` with `standard` as a `multiset` family. |
| `catalogue-demo-prime-changed` | `catalogue-demo` with `jurisdiction.nt` at 71 instead of 59. |
| `catalogue-demo-facet-removed` | `catalogue-demo` without `record.inspection`. |
| `kernel-2` | Kernel/2's dictionary: profile `kernel/2`; its seven blocks as families, each `multiset`; its 70 facets, with the identifiers of its section 2.2, at the primes its [allocation rule](../docs/kernel-2.md#21-allocation-rule-declared) gives, 2 to 349. Loading it checks REQ-KRN-001 and 002. The generator builds it from the rule and asserts that it equals the table in Kernel/2 section 2.3, which checks REQ-KRN-003 to 005. Its id is `sha256:28ec5d10ee5e6e5fd8effc7515e9adec7f049f75beeccce5064d738d922cac71`. |
| `wide-demo` | Profile `wide-demo/0`; families `flag` (`set`) and `multi` (`multiset`); the first 4,096 primes, 2 to 38,873, as `multi.p<prime>` for the 16 primes up to 53 and `flag.p<prime>` for the rest. Exactly the limit of 4,096 facets. |
| `max-prime-demo` | Profile `max-prime-demo/0`; families `big` (`multiset`) and `unused` (`one`), which has no facets; one facet, `big.p2147483647`, at 2^31 − 1, the largest prime allowed. |

Invalid. Each is `catalogue-demo` with one change, unless it says otherwise. "Appended" means added as the last facet. The cases named `<a>-before-<b>` break two checks and expect the earlier one.

| Name | Change | `detail` |
|---|---|---|
| `not-json` | A comma after the last facet. | `shape`, `core` `E_PARSE` |
| `duplicate-member` | A second `profile` member. | `shape`, `core` `E_DUPLICATE_KEY` |
| `number-range` | `record.extra` at 9007199254740992 (2^53) appended. | `shape`, `core` `E_NUMBER_RANGE` |
| `not-an-object` | The facets array alone. | `shape` |
| `member-missing` | No `facets`. | `shape` |
| `member-unknown` | A facet with a `label` member. | `shape` |
| `prime-not-a-number` | `"prime": "7"`. | `shape` |
| `shape-before-version` | `v` is `"coordinate-dictionary/1"` and the dictionary has a `note` member. | `shape` |
| `version` | `v` is `"coordinate-dictionary/1"`. | `version` |
| `version-before-profile` | `v` is `"coordinate-dictionary/1"` and `profile` is `"catalogue-demo"`. | `version` |
| `profile` | `profile` is `"catalogue-demo"`. | `profile` |
| `profile-leading-zero` | `profile` is `"catalogue-demo/00"`. | `profile` |
| `profile-trailing-newline` | `profile` is `"catalogue-demo/0\n"`. | `profile` |
| `family-name` | The family `jurisdiction` named `Jurisdiction`. | `family-name` |
| `family-name-trailing-newline` | The family `jurisdiction` named `"jurisdiction\n"`. | `family-name` |
| `name-before-values` | The family `standard` named `Standard`, with `values` `"many"`. | `family-name` |
| `values` | `standard` has `values` `"many"`. | `values` |
| `values-before-duplicate` | A second `product` family after `standard`, with `values` `"many"`. | `values` |
| `family-duplicate` | A second `product` family after `standard`. | `family-duplicate` |
| `family-order` | `product` before `jurisdiction`. | `family-order` |
| `families-before-facets` | `product` before `jurisdiction`, and `jurisdiction.nt` at 57. | `family-order` |
| `families-before-empty` | `product` before `jurisdiction`, and `facets` is `[]`. | `family-order` |
| `empty` | `facets` is `[]`. | `empty` |
| `too-many` | `wide-demo` with `flag.p38891` appended (4,097 facets), and its first facet's id changed to `multi.P2`: the count is checked before any facet. | `too-many` |
| `id-form` | `product.inverter` written `product.Inverter`. | `id-form` |
| `id-form-trailing-newline` | `jurisdiction.vic` written `"jurisdiction.vic\n"`. | `id-form` |
| `checks-within-a-facet` | `product.inverter` written `product.Inverter`, at 5.5. | `id-form` |
| `unknown-family` | `phase.single` at 71 appended. | `unknown-family` |
| `id-duplicate` | `jurisdiction.vic` at 71 appended. | `id-duplicate` |
| `not-integer` | `jurisdiction.nt` at 59.5. | `not-integer` |
| `integer-before-range` | `record.extra` at 2147483648.5 appended. | `not-integer` |
| `prime-range-low` | `record.product-listing` at 1. | `prime-range` |
| `prime-range-high` | `record.extra` at 2147483659, a prime above 2^31, appended. | `prime-range` |
| `range-before-primality` | `record.extra` at 2147483648 (2^31) appended. | `prime-range` |
| `range-before-order` | `record.extra` at 1 appended. | `prime-range` |
| `not-prime` | `jurisdiction.nt` at 57. | `not-prime` |
| `primality-before-order` | `record.extra` at 9 appended. | `not-prime` |
| `document-order` | `jurisdiction.nt` at 57, and `record.inspection`, the next facet, written `record.Inspection`. | `not-prime` |
| `prime-duplicate` | `record.inspection` at 59. | `prime-duplicate` |
| `prime-order` | `jurisdiction.vic` (11) and `product.battery` (13) swapped in the array. | `prime-order` |

### Operation cases

In these tables, 2^k stands for its decimal string, p16 for the product of the first 16 primes, 32589158477190044730 (65 bits), and p20 for the product of the first 20, 557940830126698960967415390 (89 bits). A result that is a `P_COORDINATE_` code is the error, with its member in brackets.

**`encode.json`**

| Name | Dictionary | `facets` | Result |
|---|---|---|---|
| `empty` | `catalogue-demo` | `[]` | `"1"` |
| `catalogue-example` | `catalogue-demo` | `record.installation`, `product.inverter`, `standard.as-nzs-4777-2`, `jurisdiction.vic` | `"1155"` |
| `list-order` | `catalogue-demo` | The same four, reversed | `"1155"` |
| `set-family` | `catalogue-demo` | `standard.as-nzs-4777-2`, `standard.as-nzs-5033`, `standard.as-nzs-3000` | `"3059"` |
| `unknown-facet` | `catalogue-demo` | `product.heat-pump` | `P_COORDINATE_UNKNOWN_FACET` (`product.heat-pump`) |
| `unknown-facet-trailing-newline` | `catalogue-demo` | `"jurisdiction.vic\n"` | `P_COORDINATE_UNKNOWN_FACET` (`"jurisdiction.vic\n"`) |
| `first-unknown-in-list-order` | `catalogue-demo` | `jurisdiction.vic`, `product.heat-pump`, `phase.single` | `P_COORDINATE_UNKNOWN_FACET` (`product.heat-pump`) |
| `unknown-before-cardinality` | `catalogue-demo` | `jurisdiction.vic`, `jurisdiction.nsw`, `product.heat-pump` | `P_COORDINATE_UNKNOWN_FACET` (`product.heat-pump`) |
| `one-two-facets` | `catalogue-demo` | `jurisdiction.vic`, `jurisdiction.nsw` | `P_COORDINATE_CARDINALITY` (`jurisdiction`) |
| `one-repeated` | `catalogue-demo` | `jurisdiction.vic` twice | `P_COORDINATE_CARDINALITY` (`jurisdiction`) |
| `set-repeated` | `catalogue-demo` | `standard.as-nzs-5033` twice | `P_COORDINATE_CARDINALITY` (`standard`) |
| `first-family-in-families-order` | `catalogue-demo` | `record.product-listing`, `record.installation`, `jurisdiction.vic`, `jurisdiction.nsw` | `P_COORDINATE_CARDINALITY` (`jurisdiction`) |
| `two-kinds-broken` | `catalogue-demo` | `standard.as-nzs-4777-2` twice, `product.inverter`, `product.battery` | `P_COORDINATE_CARDINALITY` (`product`) |
| `new-facets` | `catalogue-demo-v2` | `product.heat-pump`, `phase.single` | `"5183"` |
| `multiset-repeated` | `kernel-2` | `universal.entry` twice | `"4"` |
| `section-5-s-e1` to `section-5-s-e5` | `kernel-2` | The facets that hold after each event of Kernel/2 section 5 (S) | S in that table |
| `section-5-h-e1` to `section-5-h-e5` | `kernel-2` | Every facet added up to and including each event (H), repeats included | H in that table |
| `first-16-primes` | `kernel-2` | The ten `universal` facets, and `measurement` `entry`, `exit`, `unclassified`, `quantified`, `transfer` and `locked` | p16 |
| `first-20-primes` | `kernel-2` | Those, and `measurement` `committed`, `unit`, `linked` and `executed` | p20 |
| `all-70` | `kernel-2` | Every facet once | The product of the first 70 primes (467 bits) |
| `round-trip-001` to `round-trip-100` | `kernel-2` | 1 to 40 facets drawn by the generator with a fixed seed, repeats allowed | Their product |
| `2^1023` | `wide-demo` | `multi.p2` 1,023 times | 2^1023 |
| `2^1024` | `wide-demo` | `multi.p2` 1,024 times | `P_COORDINATE_RANGE` |
| `set-family-below-range` | `wide-demo` | The 122 `flag` facets from `flag.p59` to `flag.p787` | Their product (1,021 bits) |
| `set-family-range` | `wide-demo` | The 123 `flag` facets from `flag.p59` to `flag.p797` | `P_COORDINATE_RANGE` |
| `cardinality-before-range` | `wide-demo` | `multi.p2` 1,024 times and `flag.p59` twice | `P_COORDINATE_CARDINALITY` (`flag`) |
| `below-2^53` to `above-2^64` | `wide-demo` | The facets of the six decode cases of those names | Their coordinates |
| `max-prime` | `max-prime-demo` | `big.p2147483647` | `"2147483647"` |

In `first-family-in-families-order`, a walk over the facets in prime order or in list order finds `record` broken first, and in `two-kinds-broken` it finds `standard`. Only the order of `families` gives the expected family.

**`decode.json`**

| Name | Dictionary | `coordinate` | Result |
|---|---|---|---|
| `one` | `catalogue-demo` | `"1"` | `[]` |
| `catalogue-example` | `catalogue-demo` | `"1155"` | The four facets of the worked example, each with exponent 1 |
| `syntax-zero`, `syntax-leading-zero`, `syntax-plus`, `syntax-minus`, `syntax-leading-space`, `syntax-trailing-space`, `syntax-trailing-newline`, `syntax-fraction`, `syntax-exponent`, `syntax-empty`, `syntax-fullwidth-digit` | `catalogue-demo` | `"0"`, `"01"`, `"+5"`, `"-5"`, `" 5"`, `"5 "`, `"5\n"`, `"5.0"`, `"1e3"`, `""`, `"５"` (U+FF15) | `P_COORDINATE_SYNTAX` |
| `syntax-number` | `catalogue-demo` | The JSON number `5` | `P_COORDINATE_SYNTAX` |
| `syntax-before-range` | `catalogue-demo` | 400 digits with a leading zero | `P_COORDINATE_SYNTAX` |
| `syntax-late-in-long-string` | `catalogue-demo` | `1`, 400 zeros and `x` (402 characters) | `P_COORDINATE_SYNTAX` |
| `unknown-factor` | `catalogue-demo` | `"97"` | `P_COORDINATE_UNKNOWN_FACTOR` |
| `new-facet-under-old-version` | `catalogue-demo` | `"71"` | `P_COORDINATE_UNKNOWN_FACTOR` |
| `unknown-factor-before-cardinality` | `catalogue-demo` | `"33077"` (11 × 31 × 97) | `P_COORDINATE_UNKNOWN_FACTOR` |
| `one-two-facets` | `catalogue-demo` | `"341"` (11 × 31) | `P_COORDINATE_CARDINALITY` (`jurisdiction`) |
| `one-squared` | `catalogue-demo` | `"121"` | `P_COORDINATE_CARDINALITY` (`jurisdiction`) |
| `set-squared` | `catalogue-demo` | `"49"` | `P_COORDINATE_CARDINALITY` (`standard`) |
| `first-family-in-families-order` | `catalogue-demo` | `"2046"` (2 × 3 × 11 × 31) | `P_COORDINATE_CARDINALITY` (`jurisdiction`) |
| `two-kinds-broken` | `catalogue-demo` | `"3185"` (5 × 7² × 13) | `P_COORDINATE_CARDINALITY` (`product`) |
| `new-facet` | `catalogue-demo-v2` | `"71"` | `product.heat-pump`, exponent 1 |
| `old-coordinate-under-new-version` | `catalogue-demo-v2` | `"1155"` | As `catalogue-example` |
| `unclassified-as-a-whole` | `kernel-2` | `"1"` | `[]`, which Kernel/2 shows as UNCLASSIFIED (REQ-KRN-011) |
| `universal-unclassified` | `kernel-2` | `"5"` | `universal.unclassified`, exponent 1 |
| `multiset-squared` | `kernel-2` | `"4"` | `universal.entry`, exponent 2 |
| `sqlite-65-bit` | `kernel-2` | p16 | The 16 facets of `first-16-primes`, each with exponent 1 |
| `sqlite-89-bit` | `kernel-2` | p20 | The 20 facets of `first-20-primes`, each with exponent 1 |
| `section-5-h-e5` | `kernel-2` | `"1144592878288235028923541329666"` | The facets of H after E5 |
| `prime-after-dictionary` | `kernel-2` | `"353"` | `P_COORDINATE_UNKNOWN_FACTOR` |
| `round-trip-001` to `round-trip-100` | `kernel-2` | The results of the encode cases of those names | Their facets |
| `2^53`, `2^63`, `2^64` | `wide-demo` | 2^53, 2^63, 2^64 | `multi.p2`, exponent 53, 63 or 64 |
| `2^53-minus-1`, `2^53-plus-1`, `2^63-minus-1`, `2^63-plus-1`, `2^64-minus-1`, `2^64-plus-1` | `wide-demo` | 2^k − 1 and 2^k + 1 | `P_COORDINATE_UNKNOWN_FACTOR` |
| `below-2^53`, `above-2^53`, `below-2^63`, `above-2^63`, `below-2^64`, `above-2^64` | `wide-demo` | The nearest coordinates below and above 2^k that decode without error: 9007199254740990, 9007199254741096, 9223372036854775800, 9223372036854776017, 18446744073709551600 and 18446744073709552034 | Their facets |
| `2^1023` | `wide-demo` | 2^1023 | `multi.p2`, exponent 1,023 |
| `2^1024-minus-1` | `wide-demo` | 2^1024 − 1 (309 digits, in range) | `P_COORDINATE_UNKNOWN_FACTOR` |
| `2^1024`, `2^1024-plus-1` | `wide-demo` | 2^1024, 2^1024 + 1 | `P_COORDINATE_RANGE` |
| `10^308` | `wide-demo` | 1 followed by 308 zeros (309 digits) | `multi.p2` and `multi.p5`, each with exponent 308 |
| `2x10^308`, `309-nines`, `10^309` | `wide-demo` | 2 followed by 308 zeros; 309 nines; 1 followed by 309 zeros | `P_COORDINATE_RANGE` |
| `set-squared-wide` | `wide-demo` | `"3481"` (59²) | `P_COORDINATE_CARDINALITY` (`flag`) |
| `max-prime` | `max-prime-demo` | `"2147483647"` | `big.p2147483647`, exponent 1 |
| `max-prime-squared` | `max-prime-demo` | `"4611686014132420609"` | `big.p2147483647`, exponent 2 |
| `unknown-small-factor` | `max-prime-demo` | `"2"` | `P_COORDINATE_UNKNOWN_FACTOR` |

The nearest coordinates in the `below-` and `above-` cases are found by testing 2^k − 1, 2^k − 2, … and 2^k + 1, 2^k + 2, … in turn. The 2^k ± 1 cases catch an implementation that rounds to a double (2^53 + 1 becomes 2^53, which decodes) or wraps at 64 bits (2^64 + 1 becomes 1, which decodes to no facets). `syntax-late-in-long-string` catches an implementation that rejects a long string on its length before it has checked every character.

**`divides.json`**

| Name | Dictionary | `query` | `coordinate` | Result |
|---|---|---|---|---|
| `one-divides-one` | `catalogue-demo` | `"1"` | `"1"` | `true` |
| `one-divides-everything` | `catalogue-demo` | `"1"` | `"1155"` | `true` |
| `query-matches` | `catalogue-demo` | `"231"` | `"1155"` | `true` |
| `query-does-not-match` | `catalogue-demo` | `"231"` | `"3255"` | `false` |
| `query-equals-coordinate` | `catalogue-demo` | `"1155"` | `"1155"` | `true` |
| `query-unknown-factor` | `catalogue-demo` | `"97"` | `"1155"` | `P_COORDINATE_UNKNOWN_FACTOR` |
| `query-cardinality` | `catalogue-demo` | `"341"` | `"1155"` | `P_COORDINATE_CARDINALITY` (`jurisdiction`) |
| `query-syntax` | `catalogue-demo` | `"01"` | `"1155"` | `P_COORDINATE_SYNTAX` |
| `query-trailing-newline` | `catalogue-demo` | `"231\n"` | `"1155"` | `P_COORDINATE_SYNTAX` |
| `coordinate-unknown-factor` | `catalogue-demo` | `"1"` | `"97"` | `P_COORDINATE_UNKNOWN_FACTOR` |
| `coordinate-syntax` | `catalogue-demo` | `"231"` | The JSON number `1155` | `P_COORDINATE_SYNTAX` |
| `coordinate-syntax-late` | `catalogue-demo` | `"1"` | `1`, 400 zeros and `x` | `P_COORDINATE_SYNTAX` |
| `query-error-first` | `catalogue-demo` | `"-5"` | 1 followed by 400 zeros | `P_COORDINATE_SYNTAX` |
| `sqlite-65-bit` | `kernel-2` | `"53"` | p16 | `true` |
| `sqlite-89-bit` | `kernel-2` | `"53"` | p20 | `true` |
| `sqlite-65-bit-absent` | `kernel-2` | `"59"` | p16 | `false` |
| `exponent-short` | `kernel-2` | `"4"` | `"2"` | `false` |
| `exponent-enough` | `kernel-2` | `"4"` | `"8"` | `true` |
| `past-2^63` | `wide-demo` | 2^63 | 2^64 | `true` |
| `past-2^63-reversed` | `wide-demo` | 2^64 | 2^63 | `false` |
| `largest` | `wide-demo` | `"2"` | 2^1023 | `true` |

**`check.json`**

| Name | Dictionary | `facets` | `coordinate` | Result |
|---|---|---|---|---|
| `ok` | `catalogue-demo` | The four facets of the worked example | `"1155"` | `true` |
| `ok-list-order` | `catalogue-demo` | The same four, reversed | `"1155"` | `true` |
| `ok-empty` | `catalogue-demo` | `[]` | `"1"` | `true` |
| `drift-value` | `catalogue-demo` | The four | `"1156"` | `P_COORDINATE_DRIFT` |
| `drift-not-canonical` | `catalogue-demo` | The four | `"01155"` | `P_COORDINATE_DRIFT` |
| `drift-number` | `catalogue-demo` | The four | The JSON number `1155` | `P_COORDINATE_DRIFT` |
| `drift-unclassified` | `catalogue-demo` | `jurisdiction.vic` | `"1"` | `P_COORDINATE_DRIFT` |
| `unknown-facet-first` | `catalogue-demo` | `product.heat-pump` | `"71"` | `P_COORDINATE_UNKNOWN_FACET` (`product.heat-pump`) |
| `cardinality-first` | `catalogue-demo` | `jurisdiction.vic`, `jurisdiction.nsw` | `"341"` | `P_COORDINATE_CARDINALITY` (`jurisdiction`) |
| `range-first` | `wide-demo` | `multi.p2` 1,024 times | `"1"` | `P_COORDINATE_RANGE` |
| `section-5-h-e5` | `kernel-2` | H's facets after E5 | `"1144592878288235028923541329666"` | `true` |

**`extends.json`**

| Name | `old` | `new` | Result |
|---|---|---|---|
| `same` | `catalogue-demo` | `catalogue-demo` | `true` |
| `same-reordered` | `catalogue-demo` | `catalogue-demo-reordered` | `true` |
| `added` | `catalogue-demo` | `catalogue-demo-v2` | `true` |
| `family-removed` | `catalogue-demo-v2` | `catalogue-demo` | `P_COORDINATE_NOT_EXTENSION` (`family`): `phase` is missing, and that is reported before the missing facets |
| `profile` | `catalogue-demo` | `kernel-2` | `P_COORDINATE_NOT_EXTENSION` (`profile`) |
| `values-changed` | `catalogue-demo` | `catalogue-demo-values-changed` | `P_COORDINATE_NOT_EXTENSION` (`family`) |
| `prime-changed` | `catalogue-demo` | `catalogue-demo-prime-changed` | `P_COORDINATE_NOT_EXTENSION` (`facet`) |
| `facet-removed` | `catalogue-demo` | `catalogue-demo-facet-removed` | `P_COORDINATE_NOT_EXTENSION` (`facet`) |
