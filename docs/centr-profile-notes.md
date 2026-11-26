# Notes for a CENTR profile (`centr/0`)

Not a spec yet. This records what a profile for the Consumer Energy National Technical Regulator (CENTR) might carry, and what the review of the earlier CENTR draft (`evidence.centr/v0`) found, so the profile avoids the same problems.

Context, as publicly described in late 2026: CENTR is a proposed function of the Clean Energy Regulator, subject to legislation. It would certify products, register suppliers, accredit installers, collect information about installed devices, and monitor compliance against the National CER Technical Code. The Code is developed by the Department of Climate Change, Energy, the Environment and Water (DCCEEW). Before relying on any of this, confirm names and functions, including the name of DCCEEW's Code office, against the framework document.

## What the profile would add

Under `ext["centr/0"]`, only what the core cannot say and the system of record does not already hold:

- `regime`: `legacy` or `national-code`. The framework is not retrospective, so "is this Code-compliant?" and "was this lawful at the time?" are different questions.
- Code clause references for certification decisions (clause and revision).
- Jurisdiction, where it determines which licence applies.

Product, installer and installation details stay in their systems of record and are referenced through `subject` and `schema`.

## The public registers are queries

The certified product list, installer register and supplier register would be views over evidence objects, not separate databases. Publishing a list is itself an `assert` object whose content is the query result. That answers "what did the public list show on date X?" without keeping a second copy in sync.

## Pitfalls found in the earlier draft

| Earlier draft | What went wrong | What to do instead |
|---|---|---|
| Domain enums in the schema and again in a prime dictionary | They disagreed (`primary_role` vs `contributor_role`, `AS-NZS-4777.2` vs `AS-NZS-4777`), so data was silently dropped from the coordinate | Point at the source schema (D2). If a profile needs codes, define them once and generate everything else from that. |
| Coordinate builder ignored unknown edges | Missing data passed the consistency check | Unknown values are errors, never skipped. |
| Hand-rolled "JCS" via `json.dumps(sort_keys=True)` | Not RFC 8785 (`1.0`, `1e-07`, UTF-8 key order), so other languages compute different ids | Use the core canonicaliser; check against the vectors. |
| Gate's `passed` never recorded `schema_valid` | The gate could never pass | Every check is covered by a vector that must pass and one that must fail. |
| Signer check ignored the decision | A `refused` decision counted as approval | Approval is a signed `approve` attestation; refusal is an `assert` object, not an attestation. |
| Licence check read `spatial` unconditionally | Crashed on schema-valid records; passed when no installer was listed | Profile rules state what each record type must contain, and the profile vectors test the absent case. |
| Attendance photos checked for presence only | Photos geotagged in three cities, out of order, passed | Check order and geographic consistency, or keep that check in the system of record and record its result as an `assert`. |
| Any open challenge blocked issuance | Anyone filing a challenge could delist a product | Only a decision with interim effect (an `assert` by a delegate) changes status. |
| Plain hash of the site address "for privacy" | Reversible by hashing every address in the national address file; geotags revealed the site anyway | Keep personal information out of the object; use keyed hashes where linking is needed (see Privacy in the spec). |
| Structural primes 211–269 | Overlap the DSS kernel's reserved multi-modal (MMF) prime range | If prime coordinates come back, register the namespace in the DSS kernel's `prime-views.md` first. |
