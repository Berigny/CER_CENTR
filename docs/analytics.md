# Trends and anomaly flags: a proposed analytics layer (design note)

**Status:** draft for discussion, 28 November 2026. Design input for [M3](roadmap.md#m3-two-way-reconciliation-report-only-recorded-as-evidence) (reports as inputs), [M4](roadmap.md#m4-memory-reads-index-query-api-bundles-read-only-service) (reads and the assert intake), [M5](roadmap.md#m5-real-data-gate-trust-time-replication-privacy-operations) (real data, erasure and legal review) and the proposed [analytics track](roadmap.md#analytics-track-proposed). The points decided on 27 November 2026 are listed in the [roadmap](roadmap.md#decisions-confirmed-27-november-2026). The open questions were settled on 28 November 2026 ([section 12](#12-open-questions)); those that need a decision outside the project are marked gated. Everything else is proposed unless it is labelled **[built]**. All data is synthetic, and every evidence object in the examples verifies with the `evidence` command-line tool ([Appendix](#appendix-how-the-examples-were-checked)). Not an official publication of any agency; it describes no agency's process (see the [README](../README.md) notice).

**Scope:** whether the log can serve as the base for a layer that shows trends and flags anomalies, and how. Synthetic data only until the real-data gate (M5). The layer reads; it never writes to a system of record, makes no automated decisions, and does not score, rank or group people or households.

**Labels.** **[built]** is in the code today. **[proposed]** is a design for a later milestone. **[gated]** is a default that waits for a decision outside the project; nothing real proceeds on it until that decision is made.

## In short

- **The log is a good base for analytics, not an analytics engine.** It gives inputs that can be trusted and reproduced: what was known at each log position, on two clocks, with store report counts and reconciliation classes, labelled provenance and a record of who reviewed what. Counting and baselines run in a separate projection that can be rebuilt from the log.
- **A flag is a recorded fact against a published rule.** It says that a named measure crossed a threshold, as known at a stated log position. The detector's owner publishes its inputs, method and thresholds as a versioned `assert`, and each flag cites it by id, so anyone holding the log can recompute the flag. A flag is a prompt for review, not a finding about anyone, and has no effect outside this layer.
- **Tools inform; people decide.** A flag is a tool's statement made for an accountable person. It can add a review step under the published approval rules ([approvals.md](approvals.md)). It never approves, never settles a disagreement, and never changes an approved object's status by itself.
- **Facts, not scores.** A flag states a measure, the published rule or threshold that fired, and a result code. `analysis/flag@0` has no score member, and advisory scores about devices, product models or data quality are not planned ([section 3](#3-the-scoring-question)). Nothing scores, ranks or profiles people or households.
- **Aggregates name no one.** Trends are counts by record type, source, product model and the like, at a stated position and clock, with small counts suppressed. They are never grouped by person, installer, reviewer, gateway, household or keyed digest.
- **Tested before use.** Each detector has must-fire and must-not-fire vectors, including the simulator's injected drifts, and runs on real data only after M5 and legal review.

## Terms

- **Detector:** a fixed test over the log, with named inputs, a method, a clock, a period and thresholds, published in a rule set.
- **Flag:** a detector's recorded result: an `assert` that cites the rule set and states what was measured, the threshold and a result code.
- **Projection:** tables derived from the log behind a watermark (a log position), which can be deleted and rebuilt, like M4's index ([D12](decisions.md#d12-one-sqlite-file-is-the-authoritative-store)).

## 1. What the store already gives an analytics layer

Only M1 is built. The rest of this table is proposed in the [roadmap](roadmap.md#milestones).

| Input | What it gives | Status |
|---|---|---|
| Two clocks ([store/0](../spec/store-0.md#two-clocks), [D13](decisions.md#d13-two-clocks)) | Each object keeps its writer's claim (`at`). Each log entry adds the store's position (`seq`) and `received_at`, which never decreases. "As known at position p" is entries 1 to p, which `evidence store log` reads | **[built]** M1 |
| Checkpoints and proofs | A result can cite exactly the log it read, and a consistency proof shows that a later log extends an earlier one ([store/0](../spec/store-0.md#checkpoint-and-tree)) | **[built]** M1 |
| Store report counts | `counts.forks` (record lanes with more than one head) and `counts.at_after_received` (claims more than 300 seconds after receipt) | **[built]** M1 |
| Capture | Observed record changes; first sightings as baselines; the raw source time in `ext`; capture-run asserts carrying counts | **[proposed]** M2 |
| Reconciliation reports | One class per difference, per run (below) | **[proposed]** M3 onwards |
| Reads | `as_of` on both clocks, a faceted filter, a watermark on every answer, read labels (`observed`, `note`, `tool-stated`, `stated`) and `read-basis/0` ([turn-reader.md](turn-reader.md#the-basis-profile)) | **[proposed]** M4 |
| Disagreements | More than one head in a lane, and differences between a record and a linked statement ([cer-edge.md section 4](cer-edge.md#4-when-records-disagree)) | **[proposed]** M4 |
| Reviewer outcomes | Resolutions by a person, approved by a listed reviewer, with M3's dispositions or those of [approvals.md 7.4](approvals.md#74-resolution-by-a-person) | **[proposed]** M3, M4 |
| Injected drifts | The M2 simulator's seeded workloads, with M3's golden finding for each drift ([section 7](#7-evaluation-proposed)) | **[proposed]** M2, M3 |

The classes and codes the roadmap names:

| Milestone | Classes and codes |
|---|---|
| [M2](roadmap.md#m2-read-only-capture-from-a-synthetic-register) | `schema_changed` (capture stops for that type) |
| [M3](roadmap.md#m3-two-way-reconciliation-report-only-recorded-as-evidence) | `match`, `lagging`, `untracked_edit`, `differs`, `missing`, `unchecked` (which becomes `unreachable` after K runs), `sor_regressed`, `coalesced` (informational), `unobserved` and `resurrected`; `partial: true` names a scope whose page failed |
| M4 | `index_missing`, `index_extra` and `index_stale`: the index against the log |
| M5 | `S_REPLICA_TRUNCATED` and `S_REPLICA_FORK` between replicas; `S_KEY_UNBOUND` and `S_KEY_REVOKED` for keys |
| M6 | `shadow_match`, `shadow_differs` and `superseded`: requests against observations |
| M7 | `orphan`, `confirmed_unobserved`, `observed_unconfirmed` and `stuck_sending` |

**What it does not give.** It is not an analytics engine: M4's index answers exact, bounded reads, and keeps no time series or baselines, so the projection is a service of its own ([principle 2](#2-principles-proposed)). It is thin on personal and household data by design, because values not classified as non-personal are kept by reference or as keyed digests ([roadmap principles](roadmap.md#principles)); a detector cannot see them, and should not, because it reads only the log ([principle 9](#2-principles-proposed)). Its log cannot forget. From M5, erasure deletes only by-reference bodies, which are kept outside the log ([store/0 Privacy](../spec/store-0.md#privacy)), so until then only synthetic data. Snapshot capture misses intermediate versions ([roadmap Risks](roadmap.md#risks)), so a count of updates depends on how a source is captured. `received_at` is the store's own claim until M5 adds independent time. And the principles exclude scores of people, records and households ([approvals.md 7.6](approvals.md#76-what-is-not-proposed)); this layer adds none ([section 3](#3-the-scoring-question)).

## 2. Principles [proposed]

1. **Read only.** The layer never writes to a system of record and acts on nothing. Its only writes are its own `assert`s in the log: rule sets, flags and outcomes.
2. **A derived projection, never an authority.** Counts and baselines live in a projection built from the log behind a watermark, like M4's index. A rebuild gives the same result, and readers recompute rather than trust it ("derived, never authoritative", as in the [README](../README.md#prime-coordinates-for-faceted-queries)). The projection is a service of its own, not part of M4's index: it reads the log read-only behind its own watermark and reuses the index's watermark and rebuild code. So M4's index digest covers no analytics tables, and the projection can be switched off alone.
3. **Every result names its position and clocks:** the log, a covering checkpoint, the watermark, and the clock of each period. Anyone holding the log or an export can recompute it and get the same bytes.
4. **Rules are data.** Detectors and thresholds are a published, versioned `assert`, approved before use. A new threshold is a new version, never a silent change.
5. **A flag states a fact and the rule that fired:** the measured value, the threshold, the rule set by id and a result code. No free text about people.
6. **Tools inform; people decide.** A flag is read as `tool-stated` and routed to a person. On an object already approved it changes nothing until that person accepts it ([approvals.md 7.2](approvals.md#72-doubt-from-a-tool), [principle 10](approvals.md#2-principles)).
7. **No scores, and no rankings or groupings of people or households** ([section 3](#3-the-scoring-question)).
8. **Aggregates suppress small counts,** shown as "fewer than N" ([section 4](#4-trends-proposed)). A flag records its numbers exactly, so that it can be recomputed and checked, and so only parties who may read all of its inputs may read it ([5.4](#54-the-flag)).
9. **Privacy fails closed, and detectors read only the log.** Inputs are classified like captured columns. A detector reads entry fields such as `seq` and `received_at`; ids, `op`, `at` and codes; inline values classified as non-personal; and, where the data owner allows, keyed digests compared for equality. By-reference bodies are kept outside the log and its exports, so they are not inputs, and a rebuild after M5 erases one gives the same results. If a later decision lets a detector read bodies, its results can be recomputed only while those bodies are held, and the projection drops what it derived from an erased body at its next rebuild. No detector uses a trained model, so erasure never has a model to reach ([question 8](#12-open-questions)).
10. **Tested before use.** A detector runs on real data only once it passes its vectors, and only after M5.

<a id="3-the-scoring-question-open"></a>

## 3. The scoring question

**Decided (28 November 2026):** facts only, option (A). A flag states a measure, the published rule or threshold that fired, and a result code, and `analysis/flag@0` has no score member. Option (B) is not adopted and is not planned. Nothing scores, ranks or profiles people or households.

- Options, kept for the record:
  - (A) Flags state facts: a measure, the published rule or threshold that fired, and a result code. No scores. This fits the current principles: "No scores, weights or rankings of people, records or households" ([approvals.md 7.6](approvals.md#76-what-is-not-proposed)), and "Prioritising records for review, and any scoring of households or people, are out of scope" ([cer-edge.md section 7](cer-edge.md#7-privacy)). Under (A), flags are never ranked or put in order ([section 7](#7-evaluation-proposed)). A record-level flag is a check like any other in [approvals.md 7.1](approvals.md#71-recorded-facts-that-add-a-review-step): it can add a review step to the one record it names, and only if the deployment's approval rule says so ([section 6](#6-routing-to-people-proposed)).
  - (B), not adopted: as (A), and also model scores about devices, product models and data quality, stated by a tool as advisory values (`tool-stated`). Opt-in per detector, only after the detector passes its evaluation ([section 7](#7-evaluation-proposed)), with no effect on any status by itself, and never used to order or rank records for review. A score about one device at a home is still information about that household, so it is classified like any captured value.
- Out of scope under either option: anything that scores, ranks or profiles people or households ([approvals.md 7.6](approvals.md#76-what-is-not-proposed), [cer-edge.md section 7](cer-edge.md#7-privacy)).
- Reopening (B) would need a reasoned amendment to approvals.md 7.6, and could come only after M5 and a legal review. Scores would then also need a new flag schema version, since `analysis/flag@0` has no member for them.
- Why: under (A), anyone holding the log can recompute every flag exactly, because the rule, the threshold and the position are recorded and integer thresholds give the same answer in every implementation. A flag explains itself: "41 deletes against a threshold of 8" needs no model to interpret, and is easy to contest. It can be tested exactly with must-fire and must-not-fire vectors ([D7](decisions.md#d7-the-vectors-are-the-contract)), and needs no training data, of which there is little before real use and none before M5. A score depends on a model, its training data and often a random seed, so a verifier cannot recompute it from the log alone, and a list of scores invites ranking. A pattern that no rule names becomes a published rule with vectors, as in method 4 of [5.2](#52-methods-simplest-first), never a score.

## 4. Trends [proposed]

A trend is a count or a rate per period for a facet, computed at a stated watermark.

- **The read is on the store's clock; the period can be on either.** A trend reads entries 1 to its watermark, then counts by period on a clock it names:
  - **`received_at`:** what the store received. A past period never changes. This suits monitoring the layer itself, such as capture lag.
  - **`at`:** what writers say happened. Late arrivals fill recent periods in, so a period counts only once a published settle window has passed after it on the store's clock; until then it is labelled open. If a later arrival still changes a settled count, the new count lists the earlier one in `prev`; nothing is overwritten. This suits activity in a source, so that a capture pause or a backlog does not look like a burst, but only where the source dates each change. A delete found by a snapshot scan has no source time, because the row is gone: its `at` can only be the record's last change or the time of the scan, not when it was deleted, and after a capture pause one scan finds every delete from the pause. So a detector that counts deletes on `at` reads only a capture route that gives each delete a source time, and otherwise reports that it has no input, as in [section 7](#7-evaluation-proposed).
  - **Settle windows.** A source's settle window is fixed in the rule set (`settle_seconds`), set from that source's measured lag, and changed only by a new rule-set version when measured lag (capture lag, M3's `lagging`) shows that it is wrong. It is never computed while a detector runs, so a result depends only on the rule set and the log. A gateway's outage backlog is longer than any short window, which is why it is a must-not-fire case ([5.1](#51-kinds)). **[gated]** For a real source, its detector's owner sets the window from that source's measured lag, before AN3. Until then, synthetic rule sets use 86,400 seconds, as in [5.5](#55-examples).
  - **Zones.** Each rule set names the zone of its periods (`zone`): UTC for integrity and process detectors on the store's clock, and a named IANA zone, such as `Australia/Sydney`, for activity detectors where working days and public holidays matter. Australian zones differ and some observe daylight saving, so a local day can last 23 or 25 hours. The time-zone data version is recorded by hash, like the holiday calendar, so that implementations agree. A change of zone is a new rule-set version and restarts the detector's baselines.
- **Facets.** One short published list of non-personal operational facets serves M4's faceted filter, M4b's synthetic dictionary and AN1, so that a trend groups exactly as a query does: record type, `op`, source system, capture route (such as snapshot or change data capture), schema version, reconciliation class, check kind, tool version, product model and firmware. The list is classified like captured columns ([section 8](#8-privacy-records-and-fairness-proposed)). Domain facets, such as product model and firmware, are carried as [coordinate/0](../spec/coordinate-0.md) coordinates in [M4b](roadmap.md#m4b-coordinate-column-and-factor-table), answered from factor rows; process fields, such as `op`, capture route, schema version and tool version, stay plain columns. Actor ids and approval status are never facets ([approvals.md section 3](approvals.md#3-parties-and-ids-proposed)). A facet later found to identify a party, such as a gateway or a sole-trader installer, is dropped; trend snapshots already recorded cannot be withdrawn.
- **Never by party.** No trend is grouped by `person:` or `agent:` id, installer (a sole trader is a person), reviewer, gateway (it sits at someone's premises), household, device subject key, keyed digest or small area.
- **Small counts suppressed.** A count from 1 to N − 1 is published as "fewer than N", with N in the rule set (`suppress_below`). N is a deployment-wide minimum, published as versioned data, that a rule set may raise but not lower; the approval-spread report of [approvals.md section 11](approvals.md#11-advisory-reports-proposed) uses the same N. If a total and all but one of its parts are shown, a second part is suppressed too. Counts are published only at fixed period ends, not on request, so no one can time a release to isolate one record. That limits differencing between releases but does not stop it: a count that changes by one late arrival, while its period is open or after it has settled, differs from its earlier release by one record. So such a change is held back: the released value stands until the change reaches N, under a hold-back rule in the rule set, and the next release lists the earlier one in `prev`. A released count can then differ from the raw count at its watermark, but it is still fixed by entries 1 to the watermark, because the earlier release is in the log before it. **[gated]** The deployment and its data owners set the minimum N with the M5 legal review, using the De-identification Decision-Making Framework ([section 8](#8-privacy-records-and-fairness-proposed)), and confirm the hold-back rule, before any real count is released (AN3). Until then, synthetic values only (10 in the examples).
- **Rates and catch-up.** A rate's denominator is counted at the same watermark. A rate is suppressed whenever its numerator, its denominator or the difference between them would be, because a rate and any one of these counts give the others. First sightings at a capture start are baselines, not changes, so counts keep them apart and split changes by `op`. A change of capture route resets a baseline, because snapshot polling sees fewer updates than change data capture.
- **A published trend is a snapshot:** an `assert` whose content is the result at a recorded watermark, like a register snapshot ([M9](roadmap.md#m9-optional-centr0-profile-register-views-and-a-published-dictionary)). So "what did the trend show on date X?" has a provable answer. **[gated]** Whether any trend leaves the deployment, to the public or to named parties such as a system's owner, is for deployments and data owners to decide per trend, after the M5 legal review and before AN3. Until then, no aggregate leaves the deployment.
- **Reproducibility test.** A trend computed at position p over a store holding entries 1 to p, and over a later store read only up to p, gives byte-identical content. For the flags in [5.5](#55-examples), only the later-store half was checked: read up to entry 514, the store at 522 entries and an imported copy of it gave the same content as both flags ([Appendix](#appendix-how-the-examples-were-checked)). The appendix records no run on a store holding only entries 1 to 514.

## 5. Anomaly flags [proposed]

### 5.1 Kinds

Each example is synthetic, and each is a prompt for review: there may be a good reason.

| Kind | Synthetic example | Comes from |
|---|---|---|
| Integrity and process | Capture of `installer-app-demo` usually lands within 5 minutes; one day, 140 records arrive more than 15 minutes after their `at` | `received_at` against `at`; M3 `lagging` |
| Across systems | A commissioning record says `export_limit_kw` 5; the gateway's statement for the same keyed device digest says 10 | `statement-differs` ([approvals.md 7.1](approvals.md#71-recorded-facts-that-add-a-review-step)) |
| Device and product | A commissioning record gives `export_limit_kw` 12 for model `INV-TEST-10K`, whose published range is 0 to 10; a gateway reports a firmware hash that is not on a published list | A range or list check on one record ([5.5](#55-examples)) |
| Volume and rate | 41 commissioning records deleted in one day, against a median of 3 a day over the previous 28 days | Counts per period ([5.5](#55-examples)) |
| Sequence | A gateway's statement reads a device's settings before the commissioning record says the device was commissioned | `read_at` against `commissioned_at`, linked by keyed digest |

**Conditions about a device are detectors.** A firmware hash not on a published list, a battery capacity above a published threshold, or a feeder on a published constraint list is a device-and-product detector with a published rule, not an escalation condition of the approval rules ([approvals.md 7.1](approvals.md#71-recorded-facts-that-add-a-review-step)). Like every flag, it states facts only, never a score, and its record-level flag adds a review step only through `check-flagged` ([section 6](#6-routing-to-people-proposed)). Each needs its input in the log, classified like any captured column: firmware is inline in the example gateway statement ([cer-edge.md section 1](cer-edge.md#1-two-kinds-of-edge-object-proposed)), and a battery capacity would be a captured column that M2's mapping/0 classifies. A feeder constraint is network data that can suggest a location, so it is held as a keyed digest or by reference; a detector compares the digest for equality only where the data owner allows ([principle 9](#2-principles-proposed)), and otherwise reports that it has no input.

Two captured records about one device in different systems (installer app 5 kW, network portal 10 kW) are compared only by the optional `record-differs` condition of the approval rules, which the default rule does not name ([approvals.md 7.1](approvals.md#71-recorded-facts-that-add-a-review-step), [cer-edge.md section 4](cer-edge.md#4-when-records-disagree)). A detector for that case would add to it, not replace it, and a record-level flag names only one object ([section 6](#6-routing-to-people-proposed)). A gateway that buffered through an outage delivers days of statements at once with old `at` values, which is expected ([cer-edge.md section 6](cer-edge.md#6-moving-records-between-sites-proposed)), so its backlog is a must-not-fire vector. Its statements stay in the gateway's own log. A detector at the receiving site reads them only at positions that site has receipted, in one fixed order: by the receipt's position in the site's own log, then by position in the gateway's log. The receipt's `received_at` bounds when the site knew of them. A position across logs is a pair, a log id and a position in that log, which M4's design defines.

### 5.2 Methods, simplest first

1. **Published rules and thresholds.** A fixed test: a range, a count above a threshold, an order of events. Integer thresholds give the same answer in every implementation.
2. **Robust baselines.** Fire when a count is above median + k × MAD (the median absolute deviation) over the previous `window` periods, and at least a `floor`. One unusual period barely moves a median. Medians are compared as doubled integers, so implementations cannot round differently. MAD is zero when most periods are equal, and the floor then stops every small rise from firing. A seasonal baseline compares a Monday with recent Mondays, with public holidays from a published calendar recorded by hash. A trailing baseline absorbs a lasting change: if deletes rose to 40 a day and stayed there, the example detector would stop firing on the 15th day, though no one has decided that the new level is normal.
3. **Change-point detection.** CUSUM (a cumulative sum) adds up how far each period is above the baseline, less an allowance, and fires when the total passes a published limit. A period below the baseline plus the allowance lowers the total, but never below zero, so a long run at the baseline cannot hide a later shift. It finds a small, lasting shift that no single period shows, and records it once, so a person can decide. If median daily capture lag has been 120 seconds and becomes 300, 320, 310, 290 and 330, then with an allowance of 60 and a limit of 600 the totals are 120, 260, 390, 500 and 650, and it fires on the fifth day.
4. **Unsupervised models.** Not proposed for records, devices, people or households. At most, a method such as an isolation forest may be run offline over aggregate rows of synthetic data (one row per source system per day) to suggest a pattern, which a person then writes down as a rule with vectors. Its numbers are never recorded against anything. It is a design aid outside the layer, not a detector: every detector is a published, deterministic rule.

### 5.3 Who owns a detector

- **The tool owner publishes the rule set,** as an `assert` (`analysis/detectors@0`) that they author and approve, in the way the owner of each tool publishes that tool's list of action types ([turn-reader decision 4](turn-reader.md#4-when-a-basis-assert-is-required)). Policy for a tool stays with its owner, and is itself checkable. The deployment's administrators do not also approve it, as they approve approval rules ([approvals.md 5.3](approvals.md#53-changing-a-list-or-a-rule-set)): the deployment keeps its control through its own approval rules, which say what a flag can do, and through the review capacity it states ([section 7](#7-evaluation-proposed)).
- **Thresholds are not public by default.** "Published" here means recorded in the log for its readers, not made public. Read scopes on the rule set's system keep it to parties who may read the flags it governs and their inputs, so whoever may read a flag sees the threshold it crossed and can contest it, and a source is not told where to aim ([section 11](#11-limits-and-risks)). Releasing a detector's thresholds beyond them is a separate decision for each detector, and a threshold once released cannot be taken back. **[gated]** For real data, deployments and data owners make that decision with the M5 legal review, before AN3. Until then, thresholds go no further than those readers.
- **Versions and withdrawal.** A new version of the rule set lists the old one in `prev` and is appended with `if_heads` to the same lane, as [approvals.md 5.3](approvals.md#53-changing-a-list-or-a-rule-set) does for approval rules. The rule set in force at a position is its single current version there. A flag cites the version it was raised under, by id, and keeps citing it. A detector is withdrawn by a new version that leaves it out, and from then on it raises, continues and closes nothing. Its flags stay in the log. A record-level flag that is still current keeps `check-flagged` holding under [approvals.md 7.1](approvals.md#71-recorded-facts-that-add-a-review-step), so a step it added stays until a listed reviewer fills it. Withdrawal does not stop that: a flag keeps the rule-set version it was raised under, so withdrawal cannot quietly remove doubt, and the steps raised by a defective detector are cleared by reviewers. An aggregate episode that is raised or continuing when its detector is withdrawn ends without a new flag: advisory reports show it as ended by withdrawal, worked out from the log, because its detector is absent from the rule set in force. A person may also record an outcome for it ([section 6](#6-routing-to-people-proposed)). AN1 has vectors for both cases.
- **The deployment decides what a flag can do.** Its approval rules, not the detector's owner, say whether a flag adds a review step ([section 6](#6-routing-to-people-proposed)).
- **The tool acts under a grant.** The detector writes flags as a `service:` or `agent:` actor whose `on_behalf_of` is its owner, under an `approval/grant@0` from that owner ([approvals.md section 8](approvals.md#8-hooks-for-tools-proposed)). The assert intake refuses a flag whose grant is not valid, naming each failed `P_GRANT_*` code, and verification repeats the check. A flag without a valid grant that is in the log all the same, for example one written inside the writer process before the intake exists, is listed with the same codes and adds nothing.

### 5.4 The flag

A flag is an `assert`, schema `analysis/flag@0`, in lane `assert:analysis/flag` (a lane name stops before the `@`; [store/0](../spec/store-0.md#lanes-and-heads)).

| Member | Meaning |
|---|---|
| `detector`, `rules`, `tool` | The detector's name, its rule set's id, and a hash naming the tool, its version and its configuration, as for the form check in [approvals.md](approvals.md#8-hooks-for-tools-proposed) |
| `object`, `field` | Record level: the object checked and the field tested |
| `clock`, `period` | Aggregate: the clock of the period, and its start and end in UTC |
| `value`, `baseline`, `threshold`, `result` | What was measured, the baseline, the threshold that applied, and a result code from the rule set |
| `state`, `partial` | Aggregate: `raised`, `continuing` or `closed`, one flag per episode, closed once the condition has not held for `close_after` periods (an episode whose detector is withdrawn ends without a new flag, [5.3](#53-who-owns-a-detector)); and `true` if any input scope could not be read, as in M3 |
| `ext["read-basis/0"]`, `ext["approval/0"].grant` | The log, checkpoint and watermark the detector read, the heads of the subjects it read and, for an aggregate, the scope it read (`scopes[]`) ([turn-reader.md](turn-reader.md#the-basis-profile)); and the grant it acted under |

- **Subject.** A record-level flag goes on the subject of the record it checked, as [turn-reader decision 3](turn-reader.md#3-the-decisions-subject) does for decisions, so it shows in that subject's history and state views. An aggregate flag names no record, so it goes on the detector's own subject, and the flags of one episode chain there by `prev`, appended with `if_heads`. No subject per run is used, because it would take flags out of those views ([question 5](#12-open-questions)). So the detector's grant needs a write scope for `assert:analysis/flag` on every subject it may flag.
- **Who may read a flag.** M4's closed-by-default read scopes per `subject.system` ([roadmap](roadmap.md#m4-memory-reads-index-query-api-bundles-read-only-service)) apply to each flag's subject: whoever may read the record's system may read a record-level flag, and sees the threshold it carries. An aggregate flag, and each row of the projection, may be read only by parties who may read every input system it draws on, never by everyone who may read the detector's own system, which can be wider. So a token scoped to one system is refused a flag that draws on others, and M4's scopes need this rule, with vectors ([question 6](#12-open-questions)). **[gated]** For real data, data owners and each deployment confirm these readers before AN3. Until then, synthetic data only.
- **Later flags.** Within a lane, each detector keeps its own chain. A detector's next flag on a subject, including the first flag of a new episode, lists that detector's current flags there in `prev`. Every flag is appended with `if_heads` set to all the lane's heads, so a detector that has not seen the lane's latest flags is refused. A flag from another detector lists none of them, so two detectors' flags on one record are two heads of the lane, and each is a current check for `check-flagged` ([approvals.md 7.1](approvals.md#71-recorded-facts-that-add-a-review-step)). Checks never make each other contested ([approvals.md section 6](approvals.md#6-approval-status-proposed)). M4's disagreement view groups the heads of this lane by the flag's `detector` member, and reports a disagreement only when one detector's own chain has more than one head: two of its runs did not see each other. Separate detectors' chains are expected, not a disagreement. A later run that gets the same result for the same object under the same rule set appends no new flag: the earlier one is still current, and replacing it could take away a review step that a person had accepted ([approvals.md 7.2](approvals.md#72-doubt-from-a-tool)). These are the rules ([question 5](#12-open-questions)), and AN1 has a vector in which a repeat run appends nothing.
- **Every run is recorded,** as M3 records every reconciliation run. At a fixed cadence, each detector run appends one compact run report (working name `analysis/run@0`, a schema still to be drafted for AN1) holding counts only, such as items read and flags raised, continued and closed, with its read basis. Because a run that repeats a current flag appends no flag, the run report is what tells "ran and found nothing" from "did not run", for coverage, backtests and recall on injected drifts ([question 7](#12-open-questions)). Run reports are kept like flags ([section 8](#8-privacy-records-and-fairness-proposed)).
- **A value only where it is already inline.** A record-level flag copies a value only if that field is classified as inline in the object it checked; otherwise, as for a keyed digest compared for equality, it names the field and copies nothing ([principle 9](#2-principles-proposed)). An aggregate flag holds counts only: no personal values, keyed digests or device subject keys. Its value, baseline and threshold are exact, so that a reviewer can check them, and may be below `suppress_below` (the example's median is 3 and its threshold 8). They are not suppressed like a trend's counts, because a value of "fewer than 10" could not be compared with a threshold of 8. Instead, a flag is read only by parties who may read all of its inputs, as above ([question 4](#12-open-questions)). **[gated]** For real data, data owners and the M5 legal review confirm this before AN3. Until then, synthetic flags only.
- **No free text,** so every implementation produces the same bytes for the same inputs, as for store reports ([store/0](../spec/store-0.md#report-and-codes)).
- **A scope read.** `as_of.clock` is `recorded`, because a flag is computed from entries 1 to the watermark; the period in the content may be on either clock. The content names that clock by the member it counts on: `at`, the writers' claim, which `read-basis/0` calls `claimed`, or `received_at`, the store's, which it calls `recorded`. An aggregate reads a whole scope, not a list of subjects. Its basis names that scope in `read-basis/0`'s optional `scopes[]` member (`system`, `type`, `lane`, `filter` and `complete`), parallel to `left_out` ([turn-reader.md](turn-reader.md#the-basis-profile)), and lists in `reads` only the rule set it read. The aggregate example in [5.5](#55-examples) was made before `scopes[]` existed: its basis lists only the rule set, so the watermark and the detector's `input` fix the rest.

### 5.5 Examples

The rule set, authored and approved by the detectors' owner, a synthetic pseudonym (`{"approve":1,"author":1}`):

```json
{
  "v": "evidence/0",
  "id": "sha256:784f88aad74bcdce2cea5be4cd19c263cb8236830fd43a54c5b7a60b769fdac2",
  "op": "assert",
  "schema": { "ref": "analysis/detectors@0", "hash": "sha256:3020adf9866bb5fc2fafcdcf49cbf45a2d291436b28cf18e7757ae4ad6df3cfb" },
  "subject": { "system": "analysis-demo", "type": "detector-set", "key": "DETECTORS-TEST-0001" },
  "content": {
    "media_type": "application/json",
    "hash": "sha256:95a89d076ed4afec4cfb53e486b72191618b7b34fbfd2480bdcdffb51fd03ca9",
    "inline": {
      "detectors": [
        {
          "name": "export-limit-range",
          "method": "outside-range",
          "input": {
            "system": "installer-app-demo",
            "type": "commissioning",
            "lane": "record",
            "writer": "service:capture-installer-app-test"
          },
          "field": "export_limit_kw",
          "ranges": [ { "inverter_model": "INV-TEST-10K", "min": 0, "max": 10 } ],
          "results": [ "above-range", "below-range" ]
        },
        {
          "name": "deletes-per-day",
          "method": "median-mad",
          "input": {
            "system": "installer-app-demo",
            "type": "commissioning",
            "lane": "record",
            "op": "delete",
            "writer": "service:capture-installer-app-test"
          },
          "clock": "at",
          "settle_seconds": 86400,
          "period": "P1D",
          "zone": "UTC",
          "window": 28,
          "k": 5,
          "floor": 10,
          "close_after": 3,
          "results": [ "above-threshold" ]
        }
      ],
      "outcomes": [ "explained", "capture-defect", "source-issue", "referred", "unable-to-assess" ],
      "suppress_below": 10
    }
  },
  "actor": { "id": "person:122431c2d5c58dd7cb41e31fb4047a4e", "kind": "human" },
  "at": "2026-11-27T01:00:00Z",
  "prev": []
}
```

`export-limit-range` tests one field of each current captured record against the published range for its model. `deletes-per-day` counts captured deletes per UTC day on the writers' clock, once a day has settled for 86,400 seconds on the store's clock, against the median and MAD of the previous 28 days. It uses the writers' clock because of the catch-up case of [section 4](#4-trends-proposed): the appendix's store was built in one run, so on `received_at` its 126 deletes are one burst on one day. Its periods are UTC days because its baseline is trailing, not seasonal; a version with a seasonal baseline would name a zone ([section 4](#4-trends-proposed)).

The detector read the store at a checkpoint covering entries 1 to 514. It found one current commissioning record, `COM-TEST-000002`, which gives `export_limit_kw` 12 for model `INV-TEST-10K`, and recorded:

```json
{
  "v": "evidence/0",
  "id": "sha256:1d983a19ee21fe8f5f7da8e2aeabd706f2259b1e8d672da44350bd47f94264f0",
  "op": "assert",
  "schema": { "ref": "analysis/flag@0", "hash": "sha256:41f2251897499091b7512ff18f2d5a75e7d9b0c4b8a8afb7926f1404df604a93" },
  "subject": { "system": "installer-app-demo", "type": "commissioning", "key": "COM-TEST-000002" },
  "content": {
    "media_type": "application/json",
    "hash": "sha256:8f981cdd333ba8265c9f548bdfa952ddcd2a38c6e98d25f6676c8c99ff6b9fad",
    "inline": {
      "detector": "export-limit-range",
      "rules": "sha256:784f88aad74bcdce2cea5be4cd19c263cb8236830fd43a54c5b7a60b769fdac2",
      "tool": "sha256:bd98d17915bf260ce7496599f74d70d48c4ab5b446161bfed44e0bc38065ba83",
      "object": "sha256:d903c37a7180acd41a0113660ae3cd65d7e359e2f2f6cc3ca5a8bce324d48e0a",
      "field": "export_limit_kw",
      "value": 12,
      "threshold": { "min": 0, "max": 10 },
      "result": "above-range"
    }
  },
  "actor": { "id": "service:analysis-detectors-test", "kind": "service", "on_behalf_of": "person:122431c2d5c58dd7cb41e31fb4047a4e" },
  "at": "2026-11-27T23:44:40Z",
  "prev": [],
  "ext": {
    "approval/0": { "grant": "sha256:f5d8a59dca5138f8ed29e18f140d253d6059c06720bc4a3457a70a4c45207687" },
    "read-basis/0": {
      "as_of": {
        "clock": "recorded",
        "log": "sha256:ca89114711fc0b48527db1ada814936cb36fb28fac9eb5e97044c88e1fc3d5b9",
        "checkpoint": {
          "id": "sha256:6e06b5fdb407da884d92f1d68b9f83e9de5465b10fc8dfc1db0be1bf72b72c86",
          "size": 514,
          "root": "sha256:260324f299f99603285f254851c12021a19ba3ca0fe457180d3913619516f92c"
        },
        "watermark": {
          "seq": 514,
          "entry_hash": "sha256:51b29fc3d5ead7ea603bb1f360364d27e98d39d0319b1d8874967f6d8ea24653"
        }
      },
      "reads": [
        {
          "subject": { "system": "installer-app-demo", "type": "commissioning", "key": "COM-TEST-000002" },
          "lane": "record",
          "complete": true,
          "heads": [ { "id": "sha256:d903c37a7180acd41a0113660ae3cd65d7e359e2f2f6cc3ca5a8bce324d48e0a" } ]
        },
        {
          "subject": { "system": "analysis-demo", "type": "detector-set", "key": "DETECTORS-TEST-0001" },
          "lane": "assert:analysis/detectors",
          "complete": true,
          "heads": [ { "id": "sha256:784f88aad74bcdce2cea5be4cd19c263cb8236830fd43a54c5b7a60b769fdac2" } ]
        }
      ],
      "left_out": [],
      "truncated": false
    }
  }
}
```

It is on the record's own subject, the first flag in its lane, so `prev` is empty and it was appended with `if_heads` set to none. A later flag on this record would follow the rule for later flags in [5.4](#54-the-flag). Its basis names the record's head and the rule set's head at watermark 514, the checkpoint's size and head. It has `on_behalf_of`, so a reader labels it `tool-stated`, never `observed`. The value is copied because `export_limit_kw` is inline in this synthetic record. In a deployment, whether it is inline depends on the field's classification ([cer-edge.md section 7](cer-edge.md#7-privacy)); if the field is kept by reference or as a keyed digest, this detector has no value to test ([principle 9](#2-principles-proposed)).

From the same read, the aggregate flag:

```json
{
  "v": "evidence/0",
  "id": "sha256:50b27bf787f6f878a9e093acd6b1889b4cd6e77c58f956609adb170b48b59959",
  "op": "assert",
  "schema": { "ref": "analysis/flag@0", "hash": "sha256:41f2251897499091b7512ff18f2d5a75e7d9b0c4b8a8afb7926f1404df604a93" },
  "subject": { "system": "analysis-demo", "type": "detector", "key": "deletes-per-day" },
  "content": {
    "media_type": "application/json",
    "hash": "sha256:fcb136494cdd34a37701045d51aa93d528340016e5a327520fafef5b4cad3610",
    "inline": {
      "detector": "deletes-per-day",
      "rules": "sha256:784f88aad74bcdce2cea5be4cd19c263cb8236830fd43a54c5b7a60b769fdac2",
      "tool": "sha256:bd98d17915bf260ce7496599f74d70d48c4ab5b446161bfed44e0bc38065ba83",
      "clock": "at",
      "period": { "start": "2026-11-24T00:00:00Z", "end": "2026-11-25T00:00:00Z" },
      "value": 41,
      "baseline": { "periods": 28, "median": 3, "mad": 1 },
      "threshold": 8,
      "result": "above-threshold",
      "state": "raised",
      "partial": false
    }
  },
  "actor": { "id": "service:analysis-detectors-test", "kind": "service", "on_behalf_of": "person:122431c2d5c58dd7cb41e31fb4047a4e" },
  "at": "2026-11-27T23:44:40Z",
  "prev": [],
  "ext": {
    "approval/0": { "grant": "sha256:f5d8a59dca5138f8ed29e18f140d253d6059c06720bc4a3457a70a4c45207687" },
    "read-basis/0": {
      "as_of": {
        "clock": "recorded",
        "log": "sha256:ca89114711fc0b48527db1ada814936cb36fb28fac9eb5e97044c88e1fc3d5b9",
        "checkpoint": {
          "id": "sha256:6e06b5fdb407da884d92f1d68b9f83e9de5465b10fc8dfc1db0be1bf72b72c86",
          "size": 514,
          "root": "sha256:260324f299f99603285f254851c12021a19ba3ca0fe457180d3913619516f92c"
        },
        "watermark": {
          "seq": 514,
          "entry_hash": "sha256:51b29fc3d5ead7ea603bb1f360364d27e98d39d0319b1d8874967f6d8ea24653"
        }
      },
      "reads": [
        {
          "subject": { "system": "analysis-demo", "type": "detector-set", "key": "DETECTORS-TEST-0001" },
          "lane": "assert:analysis/detectors",
          "complete": true,
          "heads": [ { "id": "sha256:784f88aad74bcdce2cea5be4cd19c263cb8236830fd43a54c5b7a60b769fdac2" } ]
        }
      ],
      "left_out": [],
      "truncated": false
    }
  }
}
```

The previous 28 days have median 3 and MAD 1, so the threshold is 3 + 5 × 1 = 8, and 41 is above it and above the floor of 10. The flag records the computed threshold; the floor is in the rule set it cites. In two other builds of the store, 9 deletes on that day gave no flag (above the threshold, below the floor) and 10 gave one. It is the first of an episode, on the detector's own subject; a later `continuing` or `closed` flag would list it in `prev`. It names no record, installer or device, and adds no review step to any record.

## 6. Routing to people [proposed]

- **A record-level flag is a check.** It names one object and gives a result code, which is what [approvals.md 7.1](approvals.md#71-recorded-facts-that-add-a-review-step) calls a check. If the deployment's approval rule names `analysis/flag` in `checks`, opts in the detector and the result codes that need review (`flag_results`), and lists the detector owner's role in `checks_from`, then `check-flagged` holds. Each detector is opted in one by one, and the published default rule opts in none, so a detector adds no review step until a deployment names it, which it does only once the detector has passed its vectors ([section 7](#7-evaluation-proposed)). A flag whose detector owner is not listed in a `checks_from` role is listed with `P_CHECK_NOTED` and adds nothing. On an object not yet approved, a review step is added at once. On an object already approved, the flag goes to its `on_behalf_of` as a `review-prompt`, and adds the step only once that person accepts it ([approvals.md 7.2](approvals.md#72-doubt-from-a-tool)).
- **No condition of its own.** Record-level flags reuse `check-flagged`. A separate condition would be needed only if flags had to say something `check-flagged` cannot, such as a flag that names more than one object. So a record-level flag names exactly one object. A detector about more than one object, such as the sequence kind of [5.1](#51-kinds), adds no step itself: its flags go into an advisory report, and a step comes only from a condition the approval rules already have, such as `statement-differs`, where one covers the case.
- **An aggregate flag adds no step to any record.** It goes into an advisory report, like those of [approvals.md section 11](approvals.md#11-advisory-reports-proposed), for the people who look after the capture service or the source data.
- **A person records the outcome.** For a record-level flag, a listed reviewer fills the review step with `approve`, or records a position that a person settles with a resolution ([approvals.md 7.4](approvals.md#74-resolution-by-a-person)). For an aggregate flag, a person records an outcome: an `assert` of its own schema, `analysis/outcome@0`, by a human actor, approved by a listed reviewer, naming the flag and a code from the rule set's `outcomes`, in the pattern of M3's resolutions. It does not reuse `approval/resolution@0`, which settles positions and must name every open one: an aggregate flag has no positions, and its codes come from the rule set, as M3 keeps its own dispositions ([approvals.md 7.4](approvals.md#74-resolution-by-a-person)). An outcome is about the flag, never about a person; it is not an instruction to anyone.
- **Disputing a flag.** Any party can record a position about a flag, as about any other object ([approvals.md 7.3](approvals.md#73-positions)). A position never removes the flag or the review step it added; a listed reviewer still decides. A party who already sees a record's approval status learns of a record-level flag that adds a review step through that status (`next` and review prompts), with content limited to codes and the key and field names, as M3's notifications are. An aggregate flag names no record, so no party is told of it. **[gated]** Deployments and data owners decide, with the M5 legal review, how a party named in a flagged record learns of a flag, before AN3. Until then, this default applies, on synthetic data only ([question 6](#12-open-questions)).
- **Nothing is settled by counting flags.** Several conditions that add the same step add it once. The number of flags on a record is not a score, and no flag removes or overrides an approval.

## 7. Evaluation [proposed]

- **Backtests as known at each position.** A backtest runs a detector at each past position p over entries 1 to p only: selecting by `seq` or `received_at`, never by `at` alone; working out heads at p; ending trailing windows at p; using the mapping and normalisation in force at p, and either the rule set in force at p (to replay past flags) or the version under test (to evaluate it); and counting a reviewer's outcome only from its own position. One test catches look-ahead: the detector at p over a store holding only entries 1 to p, and over a later store read only up to p, gives byte-identical output.
- **The simulator's drifts are labelled test cases.** M3 already requires a golden finding for each injected drift ([roadmap M3](roadmap.md#m3-two-way-reconciliation-report-only-recorded-as-evidence)), from `untracked_edit[status]` for an edit made with change tracking off to `sor_regressed` for a restore from backup. AN1 adds a burst of deletes, a bulk correction, an out-of-range setting, a firmware change, a slow rise in lag and a gateway's outage backlog. Each names the detector that must fire, the latest position by which it must, and the flag it must give; every other detector must stay quiet.
- **Must-fire and must-not-fire vectors per detector.** Must fire: each injected drift, and the first value that should fire: just above a threshold, or at the floor if that is higher (10 deletes in [5.5](#55-examples)). Must not fire: a clean run, values exactly at a threshold, values above a threshold but below the floor (9 deletes), a known weekly pattern, one late arrival, the catch-up after a capture pause, a capture start, and M3's normalisation cases (6.6 against 6.600, dates, time zones, NULL). The vectors are the contract ([D7](decisions.md#d7-the-vectors-are-the-contract)). The `analysis/detectors@0` and `analysis/flag@0` shapes and the pure methods (outside a range, median and MAD, CUSUM, and small-count suppression) are formats others must reproduce, so they get an independent JavaScript reference and shared vectors; run outputs and trend snapshots are reports, with Rust-only golden files ([roadmap decision 5](roadmap.md#decisions-confirmed-26-november-2026)).
- **Precision at the review capacity people have.** The deployment states how many flags reviewers can look at in a period, as published data that backtests cite, so that the check is reproducible; it is never stated per reviewer. A backtest gives each detector's expected flags. If the total is above capacity, a new rule version narrows a scope or raises a threshold; flags are never ranked to shorten the list. Precision is the share of reviewed flags that reviewers found worth following up; an unreviewed flag is unknown, not false. Counts go with the ratio: 9 of 12 is not a firm 75 per cent. **[gated]** Each deployment states its own capacity figure before AN3. Until then, backtests cite a synthetic figure.
- **Base rates.** If 1 record in 10,000 has an edit made outside change tracking, a detector that catches 99 per cent of them with a 1 per cent false-alarm rate raises about 10,098 flags per million records, of which 99 are real. So the false-alarm rate, not the catch rate, sets the workload: most flags are real only if it is below the base rate. False alarms also add up across scopes: a detector run separately for each of 500 product models, each with one false alarm in 1,000 days, raises about one false flag every two days, and the capacity check above counts them all. An exact test such as M3's `untracked_edit` (the hash differs but the version is unchanged) has almost no false alarms while its normalisation is right, which is why published rules come first ([5.2](#52-methods-simplest-first)).
- **Recall** is measured on injected drifts, where the truth is known, and after M5 estimated by reviewing a random sample of unflagged items. The seed's hash is recorded before the period and the seed is published after it, so the sample can be neither chosen nor foreseen. For rare events such a sample bounds misses rather than measuring them: if none of 3,000 sampled items is a miss, the miss rate is probably below 1 in 1,000, which says little at a base rate of 1 in 10,000.
- **Reviewer outcomes as labels, with care.** Outcomes are kept with the detector version that raised the flag, and never summarised per reviewer. A reviewer who is shown a flag is more likely to agree with it, so outcomes on flagged items can make precision look higher than it is. As a check, some flagged items are mixed into the random sample of unflagged items and reviewed without saying which were flagged; that review is for evaluation only and fills no review step. Flags drive reviews and reviews make labels, so kinds a detector never flags never get labels. The random sample of unflagged items and a fixed synthetic test set keep that loop open, and a detector is never tuned and evaluated on the same period.
- **Drift of the detector itself.** When a source's schema version, mapping or capture route changes, a baseline detector reports that it has no baseline for the new inputs instead of running on them, until `window` settled periods exist under the new inputs. For `deletes-per-day` that is 28 days in which a burst of deletes cannot be flagged; fixed tests such as `export-limit-range` still run. Before a new detector version is used, old and new run over the same positions and the comparison is published. A change that no version records, such as a slow shift in how a source is used, may show only in a detector's own flags and outcomes, so these are counted per period for each detector version, and a lasting change in them is a reason to review the rule set.
- **Reproducible runs.** The same rule set, tool hash, log, watermark and seed give the same content hash, as M3 requires of its reports. A flag's own `at` differs between runs, so runs are compared by content hash, not by id. A detector is used on real data only once it passes its vectors, and not before M5.

## 8. Privacy, records and fairness [proposed]

- **No household attributes.** Flags and trends never carry a household's address, occupants or circumstances. Device settings and measurements are still information about a household ([cer-edge.md section 7](cer-edge.md#7-privacy)), so a detector uses them only as classified, and never builds a series for one device or premises.
- **Joins on keyed digests stay with the data owner.** A join on equal digests links records across systems and can identify more than either source. Joins run only where the data owner allows, for parties entitled to both sources; outputs never carry digests; and counts from joins are suppressed like any other. If a link-domain key leaks, such joins can be reversed ([evidence/0 Privacy](../spec/evidence-0.md#privacy)).
- **Proxies.** When settings change at a premises can suggest when people are home; owning a battery or a charger can suggest income; an export limit can suggest a network area. So features and groupings are published, classified like captured columns (M2's mapping/0), and never describe a household.
- **Fairness.** A detector that flags one product model or source more often than others may reflect how it was built rather than the records. Backtests report flags and outcomes per facet, at the levels of [section 4](#4-trends-proposed), never per person.
- **Erasure.** From M5, erasure deletes a by-reference body and appends a redaction record. A projection that reads only the log is unchanged by it ([principle 9](#2-principles-proposed)). A flag holds no body, only ids, codes, numbers and hashes.
- **Flags, trends and the projection may be records.** National Archives guidance treats AI-generated content created or received by Australian Government agencies as Commonwealth records ([National Archives](https://naa.gov.au/node/3660)), and DTA guidance says that prompts, inputs and raw outputs of AI "may constitute Commonwealth records under the Archives Act 1983" ([DTA](https://www.digital.gov.au/ai/impact-assessment-tool/guidance/transparency-and-explainability)). The detectors proposed here are rule-based, but their outputs may be records all the same. Where they are Commonwealth records, disposal follows section 24 of the Archives Act 1983, not APP 11.2 ([cer-edge.md section 7](cer-edge.md#7-privacy)). Rule sets, flags, outcomes and trend snapshots hold their content inline in the log, which cannot forget ([store/0 Privacy](../spec/store-0.md#privacy)). They have no by-reference body, so erasure does not reach them, and they cannot be disposed of one by one: they are kept for as long as the log and its copies are, and so are run reports. Only the projection can be deleted on its own, and it can be rebuilt from the log. **[gated]** How long the log and its copies are kept, and under which records authority, is for the M5 legal review and the records authority of the body that holds them to decide, before any real-data flag is written (AN3). Until then, synthetic flags and run reports are kept for as long as the log and its copies ([question 7](#12-open-questions)).

**Law and policy to check at the real-data gate.** M5's legal review already covers the Privacy Act 1988 (APP 11), the Archives Act 1983 and the Freedom of Information Act 1982 ([roadmap M5](roadmap.md#m5-real-data-gate-trust-time-replication-privacy-operations)). Its scope also includes at least these public sources. The scope is set now, and the review is commissioned before M5 starts. **[gated]** How each source applies is for the review to conclude, and no real data reaches a detector until it has concluded. This is not legal advice; whether any of them applies depends on who holds the data and how flags are used.

| Source | Why it matters here |
|---|---|
| APP 1.7 to 1.9, inserted by Schedule 1, Part 15 ("Automated decisions and privacy policies") of the [Privacy and Other Legislation Amendment Act 2024](https://www.legislation.gov.au/C2024A00128/asmade), from 10 December 2026 ([OAIC](https://www.oaic.gov.au/news/media-centre/new-resources-on-transparency-for-use-of-ai-and-automated-decision-making)) | An APP entity's privacy policy must describe its use of computer programs, using personal information, to make decisions, or to do things substantially and directly related to making them, that could reasonably be expected to significantly affect an individual's rights or interests. It is a transparency duty, not a permission or a ban. Whether a flag that a person then reviews is in scope is for each holder to check against the OAIC's guidance; a flag's recorded rule, threshold and inputs would help describe it |
| [APP 6](https://www.oaic.gov.au/privacy/australian-privacy-principles/australian-privacy-principles-guidelines/chapter-6-app-6-use-or-disclosure-of-personal-information) and [APP 10](https://www.oaic.gov.au/privacy/australian-privacy-principles/australian-privacy-principles-guidelines/chapter-10-app-10-quality-of-personal-information) | Re-using records for analytics may be a secondary use under APP 6. APP 10 asks for "such steps (if any) as are reasonable in the circumstances" to make information used or disclosed "accurate, up-to-date, complete and relevant": one reason every result states the position and clock of its inputs |
| The OAIC's [Guide to data analytics and the APPs](https://www.oaic.gov.au/privacy/privacy-guidance-for-organisations-and-government-agencies/more-guidance/guide-to-data-analytics-and-the-australian-privacy-principles) (2018) and the [Australian Government Agencies Privacy Code](https://www.oaic.gov.au/privacy/privacy-registers/privacy-codes/privacy-australian-government-agencies-governance-app-code-2017) | "Use de-identified data where possible". Agencies covered by the Code conduct a privacy impact assessment for "high privacy risk projects" |
| The [De-identification Decision-Making Framework](https://www.oaic.gov.au/privacy/privacy-guidance-for-organisations-and-government-agencies/handling-personal-information/de-identification-decision-making-framework) (OAIC and CSIRO's Data61, 2017; the OAIC notes that parts may be out of date) | Low counts carry higher disclosure risk, and a suppressed cell can be recovered by arithmetic or by comparing releases: the reasons for "fewer than N" and fixed release points |
| The Commonwealth Ombudsman's [Automated Decision-Making Better Practice Guide](https://www.ombudsman.gov.au/__data/assets/pdf_file/0025/317437/Automated-Decision-Making-Better-Practice-Guide-March-2025.pdf) (March 2025, with the OAIC and the Attorney-General's Department) | Better practice for agencies that use automated systems in administrative decisions. Its principles apply whether a system is built in-house or by a contractor |
| The Digital Transformation Agency's [Policy for the responsible use of AI in government](https://www.digital.gov.au/ai/ai-in-government-policy) | Applies to non-corporate Commonwealth entities, except the defence portfolio and certain national security agencies. Whether a detector is an AI use case under it turns on the policy's own definition **[gated]**: the body that holds the data decides for each detector at the M5 legal review, before AN3. If it is, the policy asks for an accountable owner for each use case in its scope, and a published AI transparency statement. Each rule set already names its accountable owner, and could be published as the basis of such a statement |

## 9. What is not proposed

- No scores. A flag states facts only, and `analysis/flag@0` has no score member ([section 3](#3-the-scoring-question)). Advisory scores about devices, product models or data quality are not adopted and not planned; reopening them would need a reasoned amendment to [approvals.md 7.6](approvals.md#76-what-is-not-proposed). No scores, weights or rankings of people, records or households, and no scoring, ranking or profiling of people or households.
- No automated decision or action, and no write to any system. A flag routes a question to a person; it decides, gates and assigns nothing, and never approves, changes or removes a record or an approval.
- No flag that changes an approved object's status by itself ([approvals.md 7.2](approvals.md#72-doubt-from-a-tool)), and no outcome computed by counting flags, positions or objections.
- No free-text judgement about any person, and no trend grouped by person, installer, reviewer, gateway, household, device or keyed digest.
- No recall, ranking or similarity search over the log ([roadmap Risks](roadmap.md#risks)).

## 10. Where this lands

| Milestone | What this note uses or adds |
|---|---|
| [M1](roadmap.md#m1-verifiable-store-append-only-log-checkpoints-offline-verification) **[built]** | Two clocks, positions, checkpoints and proofs; `counts.at_after_received`; `evidence store log` to read up to a position |
| [M2](roadmap.md#m2-read-only-capture-from-a-synthetic-register) | The simulator's seeded workloads; capture/0's baselines and raw source times |
| [M3](roadmap.md#m3-two-way-reconciliation-report-only-recorded-as-evidence) | Reconciliation classes per run, as inputs and as labelled test cases; resolutions as reviewer outcomes |
| [M4](roadmap.md#m4-memory-reads-index-query-api-bundles-read-only-service) | `as_of` on both clocks, watermarks, the faceted filter (with coordinates and factor rows from M4b), read labels, `read-basis/0` with its `scopes[]` member, the approval-status and disagreement views, read scopes that keep an aggregate flag to readers of all its inputs, and the position across logs (a log id and a position) for receipted gateway statements. Late in M4 (early M6 at the latest): the assert intake, through which flags are appended, and which refuses a flag without a valid grant |
| [M5](roadmap.md#m5-real-data-gate-trust-time-replication-privacy-operations) | Real data only after legal review, whose scope includes the sources of [section 8](#8-privacy-records-and-fairness-proposed); independent time; key binding, without which no approval of a rule set and no outcome is accountability evidence; erasure, which leaves a projection that reads only the log unchanged |
| [M9](roadmap.md#m9-optional-centr0-profile-register-views-and-a-published-dictionary) | Trends published as snapshot asserts, like register snapshots, where a deployment decides to publish any ([section 4](#4-trends-proposed)). Prime coordinates as facets come earlier, through coordinate/0 and M4b |

**Analytics track [proposed].** The steps are planned only in the [roadmap's analytics track](roadmap.md#analytics-track-proposed):

| Step | Content | When |
|---|---|---|
| AN1 | The `analysis/detectors@0` and `analysis/flag@0` shapes, with no score member, and the run report; a projection, a service of its own, rebuilt from the log behind a watermark; rule-based detectors with must-fire and must-not-fire vectors on the simulator's workloads, including the added drifts of [section 7](#7-evaluation-proposed), a repeat run that appends nothing, and withdrawal; an independent JavaScript reference for the shapes and pure methods. Synthetic data only | With M4 |
| AN2 | Trends with suppression and held-back changes; record-level flags routed to review through `check-flagged` in the approval-status view, for detectors a deployment opts in; aggregate flags in advisory reports, with episodes ended by withdrawal; outcomes recorded by people as `analysis/outcome@0` | After the assert intake (late in M4, early M6 at the latest) |
| AN3 | Real data, only after M5 and legal review, and with the agreement of each system's owner, once the gated points of [section 12](#12-open-questions) are decided. No model scores: option (B) of [section 3](#3-the-scoring-question) is not adopted | After M5 |

The analytics track is not yet sized; each step adds to the milestone it sits under. AN1 is to be sized inside M4, with everything else now landing on M4, before M4 planning; AN2 once the intake's timing is fixed; AN3 after M5. No estimate is given here.

## 11. Limits and risks

- **A flag can be misread as a finding.** Every flag, trend and report says that it is a prompt for review with no effect outside this layer.
- **Alert fatigue and escalation.** With too many flags, reviewers stop reading them ([approvals.md section 14](approvals.md#14-limits-and-risks)). Volume is kept within stated capacity, with one flag per episode, and dismissals and time to review are measured per detector, never per reviewer, but nothing here prevents a bad rule.
- **Thresholds can be aimed at.** A source that knows a threshold can stay just under it. So thresholds are not public by default, but whoever may read a flag sees the threshold it crossed ([5.3](#53-who-owns-a-detector), [question 4](#12-open-questions)).
- **Before M5, nothing here is accountability evidence.** A rule set's approval and a reviewer's outcome show what was recorded, not who stood behind a key.
- **The prototype is a design aid.** The script that computed the examples' flags is not a specification and not conformance code.

## 12. Open questions

Settled on 28 November 2026. Each item keeps its question. Its status line gives the decision, or, where a party outside the project decides, says who decides and what waits for it.

1. Scores: facts only (A), or also advisory scores about devices, product models and data quality as an opt-in per detector after M5 (B), which needs an amendment to [approvals.md 7.6](approvals.md#76-what-is-not-proposed)?
   - **Decided (28 November 2026):** facts only, option (A) ([section 3](#3-the-scoring-question)).
2. Who owns and who approves a detector's rule set: its owner alone (as for a grant), or also the deployment's two administrators? Who states review capacity?
   - **Decided (28 November 2026):** the detector's owner alone authors and approves the rule set; the deployment's administrators do not also approve it. The deployment keeps its control through its approval rules and through the review capacity it states, as published data that backtests cite, never per reviewer ([5.3](#53-who-owns-a-detector), [section 7](#7-evaluation-proposed)).
   - **Gated:** each deployment states its own capacity figure, before AN3. Until then, backtests cite a synthetic figure.
3. Which flags add a review step: reuse `check-flagged`, with `analysis/flag` named in `checks`, or a condition of its own? Which result codes and roles count?
   - **Decided (28 November 2026):** reuse `check-flagged`; there is no condition of its own, so a record-level flag names exactly one object. A deployment opts in each detector and the result codes that need review one by one, and lists the detector owner's role in `checks_from`; the published default rule opts in no detector, and a flag from an owner in no `checks_from` role is `P_CHECK_NOTED`. Conditions about a device, such as firmware, battery capacity or feeder, are detectors of this kind ([5.1](#51-kinds), [section 6](#6-routing-to-people-proposed)).
4. Are thresholds public, which helps scrutiny, or shown only to parties entitled to see them, so that a source cannot aim just under them? Is any aggregate published, and with what N?
   - **Decided (28 November 2026):** thresholds are not public by default; whoever may read a flag and its inputs sees the threshold it crossed. A flag keeps its numbers exact and is read only by parties who may read all of its inputs. N is a deployment-wide minimum, published as versioned data, that a rule set may raise but not lower, and the approval-spread report uses the same N. A late arrival that changes a released count is held back until the change reaches N ([section 4](#4-trends-proposed), [5.3](#53-who-owns-a-detector), [5.4](#54-the-flag)).
   - **Gated:** deployments and data owners, with the M5 legal review, set the minimum N, decide per trend whether any aggregate leaves the deployment, decide per detector whether any threshold is released, and confirm the exact numbers in flags and the hold-back rule, before AN3. Until then, no aggregate or threshold leaves the deployment, and all values are synthetic.
5. Flag subject and lane: the record's own subject for a record-level flag and the detector's own subject for an aggregate one, or a subject per run? Within a lane, does each detector keep its own chain, and does a run that repeats a current flag append nothing, so that one detector's flag never stops another's from being a current check ([approvals.md 7.1](approvals.md#71-recorded-facts-that-add-a-review-step))? How does M4's disagreement view tell separate detectors' chains from a disagreement? Does an outcome reuse `approval/resolution@0`?
   - **Decided (28 November 2026):** the record's own subject for a record-level flag and the detector's own subject for an aggregate one; no subject per run. Each detector keeps its own chain in the lane, and a run that repeats a current flag appends nothing. M4's disagreement view groups the lane's heads by the flag's `detector` member and reports a disagreement only when one detector's own chain has more than one head. An outcome is its own schema, `analysis/outcome@0`, not `approval/resolution@0` ([5.4](#54-the-flag), [section 6](#6-routing-to-people-proposed)).
6. Where does the projection live (beside the M4 index, or in a service of its own), and who may read it and the flags: closed-by-default read scopes per system, as M4 proposes?
   - **Decided (28 November 2026):** in a service of its own, reading the log read-only behind its own watermark and reusing the index's watermark and rebuild code ([principle 2](#2-principles-proposed)). M4's read scopes per `subject.system` apply to each flag's subject; an aggregate flag, and each projection row, may be read only by parties who may read every input system it draws on. A party who already sees a record's approval status learns of a record-level flag that adds a step through that status, with codes and key and field names only ([5.4](#54-the-flag), [section 6](#6-routing-to-people-proposed)).
   - **Gated:** data owners and each deployment confirm these readers, and, with the M5 legal review, decide how a party named in a flagged record learns of a flag, before AN3. Until then, these defaults apply, on synthetic data only.
7. Record every detector run, as M3 records every reconciliation run, or only runs that raise, continue or close a flag? How long are flags and run reports kept, and under which records authority?
   - **Decided (28 November 2026):** every run, as a compact run report at a fixed cadence, with counts only ([5.4](#54-the-flag)).
   - **Gated:** the M5 legal review and the records authority of the body that holds them decide how long flags and run reports are kept, and under which authority, before any real-data flag is written (AN3). Until then, synthetic flags and run reports are kept for as long as the log and its copies; only the projection can be deleted on its own ([section 8](#8-privacy-records-and-fairness-proposed)).
8. Under option (B), how does erasure reach a model trained on erased data: retrain, withdraw, or train only on values classified as non-personal?
   - **Does not arise (28 November 2026):** option (B) is not adopted, so no detector uses a trained model, and detectors read only the log ([principle 9](#2-principles-proposed)).
9. Are detector rule sets and flags formats that others must reproduce, with an independent JavaScript reference, or reports with Rust-only golden files ([roadmap decision 5](roadmap.md#decisions-confirmed-26-november-2026))?
   - **Decided (28 November 2026):** split. The `analysis/detectors@0` and `analysis/flag@0` shapes and the pure methods (outside a range, median and MAD, CUSUM, and small-count suppression) get an independent JavaScript reference and shared vectors; run outputs and trend snapshots get Rust-only golden files ([section 7](#7-evaluation-proposed)).
10. Periods: UTC days (as in the example) or local days in a named zone? Australian zones differ, and some observe daylight saving. Settle windows: fixed per source, or measured from each source's lag and published?
    - **Decided (28 November 2026):** each rule set names its zone: UTC for integrity and process detectors on the store's clock, and a named IANA zone for activity detectors where working days and holidays matter, with the time-zone data version recorded by hash. A settle window is fixed per source in the rule set, set from that source's measured lag, and changed only by a new rule-set version ([section 4](#4-trends-proposed)).
    - **Gated:** for a real source, its detector's owner sets the settle window from that source's measured lag, before AN3. Until then, synthetic rule sets use 86,400 seconds.

## Appendix: how the examples were checked

All data is synthetic. The `evidence` command-line tool was built from this repository (its code is unchanged since commit `a6bbb94`) and run in a scratch directory. Seeds are development keys from `evidence keygen`, made for these runs only. Keyed digests are HMAC-SHA256 under the published test key `synthetic-link-domain-test-key`, of device serials (`SN-TEST-000002`, and `SN-TEST-000101` to `SN-TEST-000226` for the deleted records) and installer references (`installer-app-demo/user/U-TEST-0042`, and `U-TEST-0101` to `U-TEST-0112`). The two `analysis/` schema hashes are SHA-256 of placeholder schema files made for these examples; the grant and the commissioning records use those of [approvals.md](approvals.md) and [cer-edge.md](cer-edge.md). The `tool` value is `evidence content-hash` of a one-line synthetic tool description. None of these files is in this repository. Each JSON block above was copied out and checked: `evidence verify` gives `ok` with the id shown.

- **Store.** `evidence store init st --name analysis-demo`, then, each with `store append … --if-heads` and `store attest`: the rule set (`sha256:784f88aa…`, with the owner's `author` and `approve` lines); the owner's grant to the detector tool (`sha256:f5d8a59d…`), whose write scope names `analysis/flag` for the two subjects used; 126 commissioning records created by the capture service, with `at` on 1 and 2 October 2026; their 126 deletes, each appended with `--if-heads` set to its record, with `at` from 27 October to 24 November (28 days of 1 to 5 deletes, with median 3 and MAD 1, then 41 on 24 November); and `COM-TEST-000002` (`sha256:d903c37a…`). Each captured object carries capture's `author` line. Then `evidence store checkpoint`, covering entries 1 to 514.
- **Detector.** A short script, not in this repository, read `evidence store log` up to entry 514, read-only, and computed both detectors. One current record was in scope (the other 126 were deleted), and it was out of range. Delete counts were taken per day on `at`, for days settled at entry 514's `received_at`; only 24 November fired. On `received_at`, all 126 deletes fall on the day the store was built.
- **Flags.** Both flags were sealed, signed `author` by the tool, and appended with `--if-heads none`, at entries 517 and 519, with their lines at 518 and 520. They cite the checkpoint (`sha256:6e06b5fd…`), whose id depends on the store key and the time, so their ids differ between builds. The rule set and the grant cite no checkpoint, and have the same ids on every build.

| Command (`obj/` and `att/` are the sealed files) | Result |
|---|---|
| `evidence verify obj/detectors-v1.json --attestations att/detectors-v1.jsonl --require author --require approve` (and the grant) | exit 0, `ok`, `{"approve":1,"author":1}` |
| `evidence verify obj/flag-record.json --attestations att/flag-record.jsonl --require author` (and the aggregate flag) | exit 0, `ok`, `{"author":1}` |
| `evidence verify obj/flag-record.json --store st` (and the aggregate flag) | exit 0, `ok` |
| `evidence store heads st installer-app-demo commissioning COM-TEST-000002 --lane assert:analysis/flag` (and `analysis-demo detector deletes-per-day`) | One head each: the flag |
| `evidence store checkpoint st …`, then `evidence store verify st` | exit 0, `ok`, 522 entries, forks 0, `at_after_received` 0, no findings, covered size 520 |
| The script again at entry 514, on the store holding 522 entries | The same content as both flags (equal `content.hash`) |
| `evidence store export`, `store verify` of the export, `store import`, then the script at entry 514 on the imported store | `ok`; the same content as both flags |
| Two other builds of the store, one with 9 deletes on 24 November and `export_limit_kw` 10, one with 10 deletes and 11 | No flag from the first; both flags from the second |
