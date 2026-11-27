# Notes for a CENTR profile (`centr/0`)

Not a spec yet, and not an official publication of any agency (see the [README](../README.md) notice). This records what a profile for the Consumer Energy National Technical Regulator (CENTR) might carry, and design pitfalls such a profile should avoid.

Context, from public sources (November 2026): the Clean Energy Regulator is preparing, subject to legislation, to take on the role of CENTR under the Consumer Energy National Technical Regulatory Framework (2026–2029). The regulator's page lists that role as including registering suppliers and accrediting installers, certifying regulated products and maintaining public lists of certified products, collecting and managing information about installed devices, and monitoring compliance and responding to potential breaches. The same page says the Department of Climate Change, Energy, the Environment and Water (DCCEEW) will lead the development of the legislation and of the National CER Technical Code ([Clean Energy Regulator](https://cer.gov.au/national-technical-regulator)). The Energy and Climate Change Ministerial Council endorsed the framework on 11 September 2026, and the framework provides for a Technical Requirements Office within DCCEEW to maintain the Code ([framework](https://www.energy.gov.au/energy-and-climate-change-ministerial-council/working-groups/electricity-working-group/consumer-energy-resources-sub-working-group/consumer-energy-national-technical-regulatory-framework)). Check each point against the linked pages before relying on it.

## What the profile would add

Under `ext["centr/0"]`, only what the core cannot say and the system of record does not already hold:

- `regime`: `legacy` or `national-code`. The framework will not apply retrospectively ([Clean Energy Regulator](https://cer.gov.au/national-technical-regulator)), so "is this Code-compliant?" and "was this lawful at the time?" are different questions.
- Code clause references for certification decisions (clause and revision).
- Jurisdiction, where it determines which licence applies.

Product, installer and installation details stay in their systems of record and are referenced through `subject` and `schema`.

## The public registers are queries

The certified product list, installer register and supplier register would be views over evidence objects, not separate databases. Publishing a list is itself an `assert` object whose content is the query result. That answers "what did the public list show on date X?" without keeping a second copy in sync.

## Design pitfalls to avoid

| Pitfall | What goes wrong | What to do instead |
|---|---|---|
| Domain enums in the schema and again in a prime dictionary | The two copies drift apart (for example `primary_role` vs `contributor_role`, or `AS-NZS-4777.2` vs `AS-NZS-4777`), so data is silently dropped from the coordinate | Point at the source schema (D2). If a profile needs codes, define them once and generate everything else from that. |
| Coordinate builder ignores unknown values | Missing data passes the consistency check | Unknown values are errors, never skipped. |
| Hand-rolled "JCS" via `json.dumps(sort_keys=True)` | Not RFC 8785 (`1.0`, `1e-07`, UTF-8 key order), so other languages compute different ids | Use the core canonicaliser; check against the vectors. |
| A gate's pass condition needs a check result that is never recorded | The gate can never pass | Every check is covered by a vector that must pass and one that must fail. |
| Signer check ignores the decision | A `refused` decision counts as approval | Approval is a signed `approve` attestation; refusal is an `assert` object, not an attestation. |
| Licence check reads an optional field unconditionally | Crashes on schema-valid records that omit the field; passes when no installer is listed | Profile rules state what each record type must contain, and the profile vectors test the absent case. |
| Attendance photos checked for presence only | Photos taken in different places, or out of order, pass the check | Check order and geographic consistency, or keep that check in the system of record and record its result as an `assert`. |
| Any open challenge blocks a status change | Anyone filing a challenge can delist a product | Only a decision with interim effect (an `assert` by a delegate) changes status. |
| Plain hash of the site address "for privacy" | Reversible by hashing every address in a public address dataset; photo geotags can reveal the site anyway | Keep personal information out of the object; use keyed hashes where linking is needed (see Privacy in the spec). |
| A prime range chosen without checking other schemes | Coordinates from two schemes that share primes can be confused when nothing says which scheme they belong to | Tag every coordinate with its dictionary's profile id and key indexes on (profile, coordinate), as Kernel/2 does. A shared registry of prime ranges is optional. |
