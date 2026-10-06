# Human review, canonical entities and model readiness (v5.7.0)

This document describes how the review loop actually works, what it guarantees,
and — just as importantly — what it does **not** do. Everything here is
implemented and covered by tests; nothing in this file is aspirational.

---

## 0. The four sentences that matter

> **Reviewed does not mean trained.**
> **Trained does not mean evaluated.**
> **Evaluated does not mean shadow validated.**
> **Shadow validated does not automatically mean production enabled.**

Each is enforced by a separate, deterministic gate. `vision_to_macro` is
`FORBIDDEN` and **no production-control switch exists anywhere in this
codebase** — the production vision provider's `detect()` returns an empty vec
because no inference runtime is linked into that path, so even a fully soaked
shadow model cannot act.

---

## 1. Review workflow

### The queue

`review_priority` builds a deterministic, reproducible queue. Each item gets a
weighted score and a human-readable reason:

| Flag | Weight | Reason string |
| --- | --- | --- |
| unknown prediction | 128 | `unknown prediction` |
| OCR/vision disagreement | 64 | `vision/OCR disagreement` |
| low model confidence | 32 | `low model confidence` |
| new class (<20 examples) | 16 | `new class` |
| underrepresented (<3 sessions) | 8 | `underrepresented class` |
| unseen session | 4 | `unseen session` |
| hard example | 2 | `hard example` |
| none | 0 | `ordinary` |

Conflicts are pinned above everything (score ≥ 200) because they block
training until resolved. Ties break on newest-first, then `image_id`, so the
same data always produces the same order.

Filters (status / entity / session / hard-only / disagreement-only) and sort
modes are applied **server-side** and passed to `review_priority`; the
frontend does not re-filter.

### Actions

| Action | Result | Training-eligible |
| --- | --- | --- |
| Confirm | `REVIEWED_CORRECT` when the human entity equals the model prediction, else `REVIEWED_CORRECTED` | yes, if every check passes |
| Assign + correct | `REVIEWED_CORRECTED`, or `CONFLICT` when a prior reviewed verdict disagrees | yes / **no** |
| Unknown | `REVIEWED_UNKNOWN` | **no** — reason `unknown (no entity)` |
| Skip | `REVIEWED_SKIPPED` | **no** — reason `skipped` |
| Resolve conflict | `REVIEWED_CORRECTED` + audit `RESOLVED` | recomputed from real checks |

Resolve requires the record to actually be `CONFLICT`; otherwise it would be a
back door around the rule that a second disagreeing verdict may never
silently overwrite a settled one.
| Undo | back to `UNREVIEWED`, audit `REVIEW_RESTORED` | **no** |

**A conflict is never training-eligible.** The store writes the demotion and
nothing downstream may undo it; the dataset label is not overwritten with the
disputed entity either.

### Eligibility checks (all real, all fail-closed)

```text
bad image               -> PNG missing or does not decode
unknown (no entity)     -> human set no entity
invalid canonical mapping-> entity id is not a KB entity
duplicate               -> duplicate of an already-collected image
insufficient provenance -> no session id or no capture timestamp
```

These are measured by the command layer (which can see the dataset row, the
PNG bytes and the knowledge base) and passed **into** the store. The store
never assumes them.

### Audit and recovery

Every state change appends an event to `review_audit.jsonl` **carrying the
resulting record in full**. That makes the log authoritative:

- `review_integrity` — unreadable state lines, event count, last event, and
  which images are restorable.
- `review_rebuild` — replay the audit to reconstruct the exact effective
  state. Losing `reviews.jsonl` is recoverable.
- `review_undo` - returns the row to `UNREVIEWED` (clearing the verdict
  fields so no stale entity survives), **appends** one event, and the log is
  never rewritten or truncated. The previous verdict stays visible in the
  history.

Unparseable lines in `reviews.jsonl` are preserved verbatim across writes
rather than being deleted, and a snapshot **refuses to train** while any line
is unreadable — an unreadable review file must never be mistaken for "nothing
was excluded".

Unparseable lines in `reviews.jsonl` are preserved verbatim across writes
instead of being silently deleted by the next save.

---

## 2. Reviews reach training through one channel

`snapshot_dataset()` reads `reviews.jsonl` once and writes a **filtered**
`labels.jsonl` into the job's frozen snapshot directory:

- a row with **no** review record is kept (unreviewed data keeps exactly the
  meaning it had before) but counted in `rows_unreviewed`;
- a row with `training_eligible: true` is kept;
- a row with `training_eligible: false` is **dropped** — physically absent from
  the file the trainer reads.

`fingerprint`, `rows`, `sessions` and `test_sessions` all describe the
filtered set, so a model is traceable to exactly what humans verified. A
snapshot that would contain zero rows is an error, never an empty training run.

The trainer never reads `reviews.jsonl` itself. The frozen copy exists to make
`review_fingerprint` computable and the decision auditable.

### The verdict matrix

This is the contract a reviewer relies on when they press a key. It is pinned as
a test, not just a comment.

| Verdict | In the training set? | Note |
|---|---|---|
| `CORRECT` | **yes** | model label confirmed by a human |
| `CORRECTED` | **yes** | human supplied the correct label |
| `UNREVIEWED` | **yes** | kept, counted in `rows_unreviewed` |
| `UNKNOWN` | **no** | never becomes eligible automatically |
| `SKIPPED` | **no** | never becomes eligible |
| `CONFLICT` | **no** | until explicitly resolved |
| `BAD_IMAGE` | **no** | fails the eligibility check |
| unreadable `reviews.jsonl` | **snapshot refused** | see below |

### Fail-closed on review data (v5.7.0)

`reviews.jsonl` has three distinct states and conflating any two of them is a
data-integrity failure:

| State | Meaning | Behaviour |
|---|---|---|
| **absent** | nothing reviewed yet | legal; all rows unreviewed |
| **present, fully parseable** | real review state | used |
| **present, any line unreadable** | unknown | **snapshot refused** |

The middle-right case is the one that used to fail open. One torn line made that
row's `image_id` vanish from the eligibility map, so a row a human had *skipped*
looked unreviewed and was kept — and it trained. A refused snapshot also leaves
no trainer input behind. Repair the file or run Review › Rebuild from audit.

### What a snapshot records about itself

`SnapshotMeta` is written to the job so "exactly what data produced this model"
is answerable months later, without the snapshot directory still existing:

- `fingerprint` — SHA-256 over the surviving rows' logical fields
- `labels_sha` — SHA-256 over the **frozen bytes** the trainer reads. Distinct
  from `fingerprint`: two snapshots can agree on every field and differ
  byte-for-byte, and then they are not the same training set.
- `review_fingerprint` — the review state at freeze time
- `exclusions` — **reason → count**, taken from the review record's own
  `excluded_reason`/`status`. A bare "excluded: 3" cannot be acted on; a
  reviewer cannot tell a human rejection from a data defect.
- `rows`, `rows_unreviewed`, `rows_excluded_by_review`, `sessions`,
  `test_sessions`

Identical inputs produce byte-identical metadata — that is why `exclusions` is a
sorted map rather than a hash map.

### The trainer's own second filter

`ml/gpo_train/dataset.py::review_excluded_ids` applies the review exclusion
again, as defence in depth for manual runs. It resolves `reviews.jsonl` from
**three** locations: beside the labels (the frozen snapshot), the dataset root's
parent, and the store root — because the live dataset has no copy beside
`labels.jsonl`. An unreadable review file raises rather than returning an empty
set.

### Training enforces the review floor

Every eligibility check counts *collected* data. None of them asked whether a
human had looked at it, so readiness could report `NOT_ENOUGH_REVIEW` while
`training_start` happily trained on 100% collector-labelled rows — making the
displayed blocker decorative.

`family_eligibility` now requires a minimum share of scoped rows to be both
**reviewed** and **training-eligible**, using `readiness_min_review_coverage`
(floor 0.10, clamped). Collector labels carry `annotator: "collector"` and are
the bot's own OCR+KB guesses; on their own they are not training evidence.

### Class-map integrity

The output-index → label mapping is read from the trainer's own
`config.json["vocab"]`, never re-derived by sorting evaluation keys. It only ever
agreed before because `FISH_VOCAB` happened to be alphabetical; inserting a class
would have permuted every label while the artifact checksum and output width
stayed identical — undetectable and unrecoverable.

- A class **set** mismatch between vocab and evaluated classes is **refused**.
- `vocab_sha` (SHA-256 of `classes.join("\n")`) travels into the candidate
  record and the deployed manifest, and is **verified at load**. The `.onnx`
  checksum cannot catch a permuted `classes` array, because the map lives in the
  manifest JSON.
- A manifest whose `name` does not match the file slot it occupies is refused: a
  model family cannot cross slots.

### Soak isolation

Shadow telemetry is joined on `(slot, revision)`. The revision comes from one
reader (`revision_from_manifest`) so readiness and the log can never disagree
about the join key; passing "unknown" used to mean the legacy cross-revision
union view, which let a fresh candidate inherit the incumbent's events.

Version numbers are monotonic across registry loss. The counter lives **outside**
`training/registry/` — that is the directory a user deletes to reset the app. If
there is no counter and no intact registry, numbering is **refused** rather than
guessed. `shadow_evidence_revision` additionally requires the reported metrics and
the reported soak to describe the same revision before `SHADOW_READY`.

---

## 3. Canonical entities

Every training label is a stable KB id (`fish:golden`, `fruit:yami`). Display
names are metadata. Resolution returns exactly one of:

- `Exact(id)` — only from a direct canonical/alias index hit, or from a lossy
  OCR-variant retry that matched **exactly one** entity;
- `Ambiguous(ids)` — ≥2 strict-prefix overlaps; the reviewer must pick;
- `Unknown` — nothing matched. **No entity is ever invented.**

Substring matching is anchored to a word boundary with a 4-character floor, so
`snapp` can suggest `Snapper` while `on` matches nothing.

### Alias-shadowed entities

`DEFAULT_FRUITS` lists some alias names (`Phoenix`, `Dark`, `Love`) alongside
their canonical fruit (`Tori`, `Yami`, `Mero`). The KB name index is
first-wins, so the alias is already claimed and the alias-named entity can
never be resolved. These entities are **surfaced** in Training › Drops as
`alias of fruit:tori` with a `shadowed_by` field rather than being hidden, so
the addressable-entity count stays honest. Resolution itself is unchanged —
`resolve("Dark") -> fruit:yami` is the correct answer, because `fruit:dark` is
the duplicate.

### Names that are NOT in the knowledge base

The project's authoritative KB has 141 entities (77 fruit, 41 fish, 3 bait, 20
UI terms). Of the commonly cited fishing-drop names, only **`fish:swordfish`**
exists. The following are **absent** and were deliberately not added:

```text
Blue-Lip Grouper, Tigerfin, Exotic Tigerfin, Golden Tigerfin,
Skeletal Shark, Dark Skeletal Shark, Candy Corn Squid, Jack-O'-Bite,
Fangfish, Anglerfish (the KB has fish:angelfish — a different fish),
Crimson Polka Puffer, Golden Polka Puffer, Polka Puffer,
Zebra Ribbon Angelfish, Golden Ribbon Angelfish,
Sunken Armor, Sunken Anchor, Sunken Helmet
```

`Crimson Snapper` exists only as two separate KB entries (`fish:crimson`,
`fish:snapper`), never as a combined entity. Fish carry **no** rarity
metadata — the KB has none — and nothing in the repo models seasonality, so
those fields are reported as absent rather than invented.

`wiki_url` is `None` for every bundled entity: the GPO Wiki importer builds a
real page URL, but merging deliberately refuses to overwrite bundled entries,
so the URL is discarded for them.

---

## 4. Fishing drops explorer

Each KB entity is joined with real collection data:

```text
canonical name · entity id · category · rarity · aliases
wiki source (no per-entity URL for bundled entries)
collected · reviewed · training eligible · sessions · model scope
```

`model_status` is one of `IN_SCOPE` (in the deployed model's class list),
`OUT_OF_SCOPE` (a fishing drop the model was not trained on), `N/A` (not a
fishing drop) or `ALIAS_SHADOWED` (unresolvable by name). `fishing_drop` is
true for Fish/Fruit, which come from the project's curated lists;
per-state drop mechanics are **unverified** and deliberately not claimed.

---

## 5. Model readiness

### Statuses (not collapsed)

| Status | Meaning |
| --- | --- |
| `NOT_ENOUGH_DATA` | zero qualified classes |
| `NOT_ENOUGH_CLASSES` | some classes qualify, fewer than the gate wants |
| `NOT_ENOUGH_SESSIONS` | a qualified class is short on independent sessions |
| `NOT_ENOUGH_REVIEW` | data qualifies but human review is below the floor |
| `NOT_ENOUGH_TEST` | qualified classes lack held-out TEST coverage |
| `DATA_READY` | data + review pass; training may start |
| `TRAINING` / `EVALUATING` | a real job is live (from actual job records) |
| `CANDIDATE_READY` | an evaluated candidate clears the absolute quality bars |
| `SHADOW_READY` | deployed to shadow **and** soak passes |
| `PRODUCTION_READY` | **structurally unreachable** |

### Every gate reports

```text
actual · required · difference · next action
```

for example: `qualified_classes  8/10 qualified  need 10/10  2 class(es) short`.

The `worst_class_f1` check names the offending class (`fish:pufferfish = 0.222`),
not just a number.

### The shadow gate is three gates

```text
shadow_soak_events      >= 100 events
shadow_soak_agreement   >= 80% measured OCR/vision agreement
shadow_soak_sessions    >= 3 distinct sessions
```

Volume alone is never enough: 100 events at 1% agreement fails. Soak is read
**per deployed revision** (`soak_stats_for`), so a newly promoted candidate
starts from zero telemetry instead of inheriting the incumbent's. Events are
deduplicated by `event_id`, malformed lines are counted rather than silently
dropped, and distinct sessions/entities are tracked.

### Production

`production_authorized` always fails and is marked `structural: true`. It is
displayed for honesty but is **excluded from the actionable blocker list** —
a human cannot clear it by collecting data, so listing it as a blocker would
bury the ones they can act on.

### Thresholds

All bars live in `TrainingSettings.readiness_*` (Settings › Training) and are
documented defaults: macro-F1 ≥ 0.70, worst-class F1 ≥ 0.50, ≥ 100 soak events,
≥ 80% agreement, ≥ 3 soak sessions, ≥ 50% reviewed coverage.

---

## 6. Class readiness (per entity)

```text
READY                 >=20 reviewed, >=3 sessions, TEST-split examples present
INSUFFICIENT_REVIEW   not enough reviewed examples
INSUFFICIENT_SESSIONS not enough independent sessions
INSUFFICIENT_TEST     no held-out TEST examples
NO_DATA               nothing collected
```

Each verdict carries the actual counts and the reason.

---

## 7. Coverage

Reported per entity, per session, per split (train/validation/test), per
exclusion reason, plus hard-example and unreviewed totals. Every rate is shown
with its denominator, so "reviewed N" never appears without "of M".

Two different things are both called "sessions" and are labelled distinctly:

- **dataset sessions** — distinct sessions containing that entity;
- **reviewed sessions** — distinct sessions with at least one review record.

---

## 8. Training history

`training_history.jsonl` records every decision and every factual lesson
(metrics only — no chain of thought). Each job records the dataset
fingerprint, the review fingerprint, frozen row/session counts and the frozen
test sessions, so "what did this model train on" is answerable exactly.

---

## 9. Hermes

No Hermes or AutoSkill code exists in this repository. `hermes_tasks` is a
read-only view (triggers + readiness + history) that an external orchestrator
may poll. Hermes schedules work through the existing commands. It cannot
promote, bypass evaluation, bypass canonical mapping, bypass review, enable
vision-to-macro, override UNKNOWN safety, approve corrupted weights, or ignore
a regression — because **no such code path exists**, not because of a
permission check.

---

## 10. Troubleshooting

| Symptom | Cause | Fix |
| --- | --- | --- |
| `NOT_ENOUGH_REVIEW` | fewer than 50% of scoped rows reviewed | clear the queue in Training › Review |
| A confirmed image still excluded | a check failed; the reason is on the record | read `excluded_reason` in Coverage › exclusions |
| Conflict won't train | unresolved | resolve it with a reason |
| Shadow gate stuck on events | soak is per revision; a new candidate starts at 0 | play with shadow observation on |
| `shadow_evidence_revision` fails | the evaluated candidate is not the deployed one | promote the evaluated candidate so its soak is its own |
| Status shows "Evidence: MISMATCH" | metrics and soak describe different revisions | same — the panel will not add numbers from two models together |
| "no version counter and shadow telemetry already exists" | the registry was wiped but telemetry survived, so a revision cannot be proven unused | delete `shadow_events.jsonl` too if you truly mean a full reset, or restore the registry |
| Train is refused with "human-reviewed" in the reason | the review floor is not met | review scoped rows in Training › Review; collector labels alone are not evidence |
| "alias of fruit:…" in Drops | KB name collision | use the canonical entity id |
| `unreadable_lines` > 0 | a state line could not be parsed (external edit, or a record from a newer build) | it is preserved, not lost; run Rebuild from audit |
| Training refuses with "unreadable" | `reviews.jsonl` exists but a line will not parse | repair it, or Rebuild from audit; training stays blocked until then |
| Backend shows "timed out" | `import torch` did not finish within the probe budget; the interpreter was terminated | check the interpreter and torch install; the probe never hangs the UI |
| A readiness bar will not go below its floor | by design — clamped on load and on save | raise it instead; it cannot be lowered |
| Fruit training button disabled | no fruit trainer module exists yet | needs a fruit trainer before any fruit training |

---

## 11. Known limitations

These are real and deliberately not hidden. None of them is a gate that was
weakened to hide a problem.

- Fish have no rarity metadata; the KB has no seasonal concept. Neither is
  invented.
- Per-state drop mechanics are unverified; `fishing_drop` is membership, not a
  probability.
- `vision_confidence` travels in shadow telemetry but the policy does not use
  it — `fish_v1` has no 90%-reliable operating point, so everything outside its
  8 classes is UNKNOWN.
- No fruit trainer module exists, so fruit training is blocked at the gate
  rather than failing at launch.
- `state_v1` scores a perfect 1.0000 on its held-out split. That is a *plausibility
  concern*, not a strength: the state crop is region-specific, so the model can
  identify the state from crop geometry alone. There is no leakage monitor.
  `state_v1` is protected and must not be retrained without cause.
- Sessions are minted per macro run, not per day, so consecutive same-day runs
  can land in different splits. Split purity is asserted (a session never spans
  splits), but visual independence is not guaranteed.
- Per-class regression gating is floored at 10 test examples. Below that a class
  is reported but cannot move the verdict — in either direction.
- `review_priority` must rank every unreviewed row to compute an honest
  `total_matching`, so it is O(n) per request. It runs off the UI thread, so the
  panel stays responsive, but it is not a constant-time query.
- `seed_shadow_models` never overwrites a deployed model with a higher manifest
  version, never overwrites one it cannot parse, and both promotion **and
  rollback** invalidate the cached engine. A restart therefore keeps the promoted
  weights, and the revision reported in the UI is the revision that is actually
  observing.
- Entity ids are derived from canonical names (`fruit:<lowercased name>`), so a
  rename in `fruit.rs` would orphan persisted references. Ids are stable in
  practice because the lists are curated, but there is no rename migration.
- The audit log has no rotation; it grows by roughly one record per verdict.
- `docs/knowledge_manifest_v1.json` is a pinned snapshot of KB contents; there
  is no automated drift check against the source lists.

---

## 12. Current state, honestly

```
SOFTWARE COMPLETE
MODEL NOT READY
```

| Family | Classes | Reviewed | Candidate | Shadow | Status |
|---|---|---|---|---|---|
| fish | 8/10 | 1 row / 8,609 (0 eligible) | none | not started | `NOT_ENOUGH_CLASSES` |
| fruit | 1/10 | 0 | none | none | `NOT_ENOUGH_DATA` |
| state | solved (1.0000) | — | — | — | protected |
| sunken | — | — | — | — | `UNVERIFIED / NOT IN KB` |

**No candidate has ever been registered.** `registry.json` does not exist. Any
claim that a "fish v2 candidate" is ready, or that it reached 0.80 macro-F1, is
false — those numbers exist only in unit tests with fabricated class names.

The single review on record is a CONFLICT whose human label was
`fish:tiger`; that entity is not in the KB, so the record is
`training_eligible: false, excluded_reason: "invalid canonical mapping"`. The
system correctly refused to turn a plausible-looking fish name into a canonical
entity.

Fish cannot train until ten classes each have ≥20 human-reviewed examples across
≥3 independent sessions with at least one held-out TEST example. Until then the
Train button stays disabled, and that is the system working correctly.

