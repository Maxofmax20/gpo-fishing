# Human review, canonical entities and model readiness (v5.6.1)

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
| "alias of fruit:…" in Drops | KB name collision | use the canonical entity id |
| `unreadable_lines` > 0 | a state line could not be parsed (external edit, or a record from a newer build) | it is preserved, not lost; run Rebuild from audit |
| Training refuses with "unreadable" | `reviews.jsonl` exists but a line will not parse | repair it, or Rebuild from audit; training stays blocked until then |
| Fruit training button disabled | no fruit trainer module exists yet | needs a fruit trainer before any fruit training |

---

## 11. Known limitations

- Fish have no rarity metadata; the KB has no seasonal concept. Neither is
  invented.
- Per-state drop mechanics are unverified; `fishing_drop` is membership, not a
  probability.
- `vision_confidence` travels in shadow telemetry but the policy does not use
  it — `fish_v1` has no 90%-reliable operating point, so everything outside its
  8 classes is UNKNOWN.
- No fruit trainer module exists, so fruit training is blocked at the gate
  rather than failing at launch.
- `seed_shadow_models` never overwrites a deployed model with a higher manifest
  version, and promotion invalidates the cached engine. A restart therefore
  keeps the promoted weights, and the revision reported in the UI is the
  revision that is actually observing.
- Entity ids are derived from canonical names (`fruit:<lowercased name>`), so a
  rename in `fruit.rs` would orphan persisted references. Ids are stable in
  practice because the lists are curated, but there is no rename migration.
- The audit log has no rotation; it grows by roughly one record per verdict.
- `docs/knowledge_manifest_v1.json` is a pinned snapshot of KB contents; there
  is no automated drift check against the source lists.
