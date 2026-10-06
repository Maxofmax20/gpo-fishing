# FINAL FEATURE FREEZE — v5.7.0

**v5.7.0 is the final planned feature release of GPO Autofish.**

There is no v5.8, no v5.9, no v6.0 feature expansion. The architecture is
frozen. What follows v5.7.0 is maintenance, not development.

---

## 1. What future changes are allowed

Only these:

- **bug fixes** — a demonstrated defect, with a regression test
- **security fixes** — a demonstrated vulnerability, with a test that fails before it
- **compatibility fixes** — an OS, runtime, dependency or game-client change that breaks something that worked
- **dependency/security updates** — version bumps that do not change behaviour
- **crash fixes** — a reproducible crash or hang
- **evidence-driven model/data updates** — a new trained model, or new real gameplay data, where the readiness gates are actually satisfied

Anything that adds a capability, a subsystem, a configuration surface or a UI
surface is out of scope. If it is not one of the six categories above, it does
not happen.

### The bar for every one of them

A change is admissible only if it does **not**:

- weaken any readiness, review, session, split, shadow or evaluation gate
- add a production-control switch (see §3)
- introduce automatic promotion
- fabricate, synthesise or infer training data without a human verdict
- resolve a conflict silently
- make a threshold reachable that was not reachable before

"Feature-like" changes that are actually in scope: replacing a dependency that
has a CVE; adapting to a Roblox client update that moved a pixel region.

---

## 2. Why this freeze exists

This project optimises for **six months from now**, not for a better demo today.
A human must be able to:

- trust the dataset
- understand every label
- reproduce any training run
- know that a model cannot inherit another model's evidence
- know that corrupted data cannot silently enter training
- know that a failed gate cannot become a passing gate
- know that the UI cannot fake readiness
- know that security cannot be bypassed for convenience

Every rule below exists to protect one of those. Adding features is the fastest
way to break them, because each new surface is a new place for a gate to leak.

---

## 3. The invariants. These do not change.

### 3.1 No production control from vision

```
vision_to_macro = FORBIDDEN
```

There is no code path from a model prediction to a game input, and none may be
added. The real enforcement is structural: `ml_model.rs::detect()` returns
`Vec::new()` — no inference runtime is linked on the production path at all.
This is not a flag that could be flipped; it is an absence.

**No production-control switch exists anywhere in this codebase.** Adding one
requires an explicit future security review and is out of scope under this
freeze.

### 3.2 No automatic promotion

Training produces a `CANDIDATE`. Promotion to shadow is a deliberate human
action. A candidate that passes every gate still does not promote itself.

### 3.3 No fabricated training data

The collector may collect. The trainer may prepare. **Only the review system
grants training eligibility.** No path — collector label, OCR heuristic,
knowledge-base correlation, import script, or manual run — may make a row
training-eligible without a human verdict.

### 3.4 No bypass of readiness gates

Class counts, reviewed-example counts, session counts, held-out-test coverage,
macro-F1, worst-class F1, shadow volume, shadow agreement and shadow session
spread are floors, not targets. They are clamped on load *and* on save, so a
hand-edited `settings.json`, a crafted preset, or any future writer cannot
weaken them.

### 3.5 No silent conflict resolution

A `CONFLICT` stays out of training until a human resolves it. Resolution stores
the original model label, the human decision, the final canonical entity, the
resolution action, the timestamp and the reviewer. The conflict history is
never erased.

### 3.6 Fail closed

Missing, corrupt, unreadable, ambiguous, contradictory or unverifiable means
`UNKNOWN / BLOCKED`. Never `ASSUME VALID`.

An unreadable `reviews.jsonl` is **not** "zero reviews". A corrupt registry is
**not** "no candidates". An unreadable manifest is **not** "version 0". A read
error mid-file is **not** "a shorter dataset".

---

## 4. Current honest state at the freeze

```
SOFTWARE COMPLETE
MODEL NOT READY
```

These are two separate claims and both are true.

### Software

- 363 automated tests, 0 failing
- `tsc` clean, Vite build clean, clippy clean, `npm audit` 0, secret scan clean
- Release pipeline builds as a draft, verifies the signature and manifest, then
  publishes
- Backend health probe has a real timeout and cannot stack
- Dataset reads are streamed and page-capped; writes are atomic
- Review corruption fails closed; snapshot exclusions are itemised and hashed
- Candidate soak is revision-isolated and survives registry loss

### Models

| Model | Status |
|---|---|
| `state_v1` | Solved. 1.0000. Protected; do not retrain without cause. |
| `fish_v1` | 0.5824. Deployed. **No candidate exists. No shadow soak exists.** |
| fruit | 1/10 qualified. No trainer exists. `NOT_ENOUGH_DATA`. |
| sunken | `UNVERIFIED / NOT IN KB`. Not invented. |

Fish is **8/10** qualified classes and **0%** human review coverage, so its
readiness status is `NOT_ENOUGH_CLASSES` — and no model quality number can
override that.

**There is no "fish v2 candidate ready".** No such candidate has ever existed.
Any claim to that effect is false and must be removed from documentation.

---

## 5. What the next real action is

Not "train the model now" — the gates do not allow it.

```
1. Enable trace recording (Setup)
2. Play normally. Catch fish. Let sessions accumulate.
3. Review in Training > Review. Enter = correct, C = correct, U = unknown,
   S = skip, arrow keys to move, Z = undo.
4. Watch Coverage fill: fish classes need >= 20 reviewed examples and
   >= 3 independent sessions, plus a held-out TEST example each.
5. Only then does the Train button become available.
6. Train -> evaluate -> promote to shadow -> soak -> (production stays OFF)
```

The UI shows the single next action on every screen. It is derived from the
first unmet readiness stage, never from an assumption.

---

## 6. If you find a bug in this code

Open an issue with the reproduction. Fixes are welcome under §1. What is not
welcome:

- a new feature
- a relaxed threshold
- a new flag that turns a gate off
- a new path that writes `labels.jsonl` without the review projection
- a new path that reads a model and skips the class-map or weights checksum

If a fix cannot be expressed without one of those, the fix is wrong. Reopen the
design question instead.

---

## 7. The honest failure mode

If real-world data never becomes sufficient, this project still ships as
**SOFTWARE COMPLETE / MODEL NOT READY**, and that is a correct, defensible
outcome — not a failure to be papered over.

The most likely real outcome over the next months is exactly that: enough
software, not enough fish.
