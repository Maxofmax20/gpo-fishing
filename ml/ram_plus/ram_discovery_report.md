# RAM++ / Semantic-Teacher Discovery Report

**Status: the teacher was NOT installed in this environment, and the data says it would not help much anyway.**
Everything below is measured on the real collected dataset, not assumed.

Dataset: `%APPDATA%\gpo-autofish\datasets\gpo-vision\v1`
Rows: 8,609 · Images: 8,609 · Human reviews: **1** (a `CONFLICT`, not eligible)
Sessions: 71 macro runs across **4 calendar days**

---

## 1. Headline finding: these crops are text panels, not fish

Measured over the 1,803 usable fish crops:

| Property | Value |
|---|---|
| Crop size | **589 × 199** (a wide drop-message panel, 100% of samples) |
| Dark (background) fraction | 0.434 |
| Bright (glyph/text ink) fraction | **0.043** |
| Mean pixel distance, **same** label | 0.1626 |
| Mean pixel distance, **different** label | 0.1719 |
| **Separation (diff − same)** | **0.0093 — only 5.4% of the cross-class distance** |
| Nearest-centroid accuracy on raw 16×16 pixels | 0.1920 (chance = 0.1250) |

The species identity is carried by the **rendered words**, not the artwork.
A nearest-neighbour classifier on the raw pixels barely beats guessing.

**Consequence for RAM++:** a semantic *image* tagger sees a dark UI panel with a
line of small text. It will return tags like `fish`, `screenshot`, `game`,
`text`. It cannot read the drop name, and the drop name is the entire signal.
RAM++ cannot improve species discrimination here, because the information is not
in the image features it was trained to look at.

---

## 2. The real problem is the ONTOLOGY, not the labels

The game's drop names are **compound**. The knowledge base holds **8 atomic
species**. The resolver picks one component arbitrarily.

Distinct in-game names actually observed in the collected data:

```
Golden Ribbon Angelfish   (many OCR variants)
Zebra Ribbon Angelfish
Candy Corn Squid
Exotic Tigerfin
Golden Tigerfin
Dark Skeletal Shark
Crimson Snapper
Golden Polka Puffer
Blue-Lip Grouper
Jack-O'-Bite
Swordfish
```

against a KB of:

```
fish:angelfish  fish:crimson  fish:golden   fish:pufferfish
fish:shark      fish:snapper  fish:squid    fish:swordfish
```

`Golden Ribbon Angelfish` contains **two** KB species words. So does
`Crimson Snapper`. `Crimson Snapper` and `Golden Polka Puffer` and
`Zebra Ribbon Angelfish` are *variants* the KB has never modelled.

**Nothing can fix this except a human decision about what the ontology should
be.** Adding these entities automatically would fabricate ground truth, which
is exactly what `FISH_VOCAB.txt` and the knowledge base currently refuse to do.

---

## 3. Triage over the real dataset (no teacher required)

Same logic the reviewer uses, applied to all 1,914 entity-bearing rows:

| Outcome | Rows | Share |
|---|---:|---:|
| OCR confirms the stored label | 884 | 46.2% |
| OCR **conflicts** with the stored label | 223 | 11.7% |
| OCR **ambiguous** between two canonical names | 621 | 32.4% |
| OCR names **no KB entity** at all | 186 | 9.7% |
| **Not confirmable by OCR alone** | **1,030** | **53.8%** |

712 rows were **OCR-variant noise**, not unknown entities, and were repaired and
explicitly *not* counted as new concepts:

```
csvordfish -> Swordfish      g0lden / g01den -> golden
anselfish  -> Angelfish      qfansfish -> Angelfish
```

**Per-class, the damage is very uneven:**

| Stored label | Rows | OCR-clean | Ambiguous | No-KB-entity | Conflict |
|---|---:|---:|---:|---:|---:|
| `fish:squid` | 301 | 234 | 16 | 30 | 21 |
| `fish:golden` | 287 | 101 | 162 | 11 | 13 |
| `fish:angelfish` | 285 | 205 | 28 | 35 | 17 |
| `fish:snapper` | 271 | **17** | **188** | 33 | 33 |
| `fish:shark` | 254 | 200 | 15 | 25 | 14 |
| `fish:crimson` | 224 | **8** | **172** | 12 | 32 |
| `fish:pufferfish` | 103 | 62 | 30 | 5 | 6 |
| `fish:swordfish` | 78 | 57 | 4 | 13 | 4 |

`fish:snapper`: **17 clean rows out of 271.** `fish:crimson`: **8 of 224.**

These two classes are effectively unlabelled — nearly every row is an ambiguous
compound name. That alone explains the 25× `squid → snapper` confusion in the
deployed model's confusion matrix, and the worst-class F1 of 0.452.

---

## 4. Unresolved visual concepts

Per the naming rule these are **never** called entities and are **never**
auto-created. Each is a request for human review.

| Concept (OCR family) | Rows | Existing KB match | Recommendation |
|---|---:|---|---|
| `Crimson Snapper` | 225 | ambiguous: `fish:crimson` \| `fish:snapper` | HUMAN REVIEW — is this a variant, or either parent? |
| `Crimson Puffer` | 216 | ambiguous: `fish:crimson` \| `fish:pufferfish` | HUMAN REVIEW |
| `Golden Puffer` / `Golden Polka Puffer` | 93 | ambiguous: `fish:golden` \| `fish:pufferfish` | HUMAN REVIEW |
| `Golden Angelfish` / `Golden Ribbon Angelfish` | 87 | ambiguous: `fish:golden` \| `fish:angelfish` | HUMAN REVIEW |
| `Exotic Tigerfin` / `Golden Tigerfin` | 31 | none | HUMAN REVIEW — one entity, variants, or OCR artefact? |
| `Zebra Ribbon Angelfish` | 3 | none | HUMAN REVIEW |
| `Candy Corn Squid` | ~130 | none | HUMAN REVIEW — is it a `fish:squid` variant? |
| `Dark Skeletal Shark` | 15 | none | HUMAN REVIEW |
| `Jack-O'-Bite` | 8 | none | HUMAN REVIEW |
| `Blue-Lip Grouper` | 4 | none | HUMAN REVIEW |

---

## 5. What the semantic teacher IS good for

Honest, and narrower than it sounds:

- **Coarse ambiguity detection** — telling "this crop is a fish drop" from "this
  is a fruit drop" from "this is a banner". Cheap, and it works.
- **Second opinion on genuinely ambiguous crops** — a human looking at a row
  flagged by three disagreeing sources pays more attention.
- **Review queue ordering** — 53.8% of rows are not OCR-confirmable; the
  teacher can rank the ones where the cheap label is most likely wrong.
- **Nothing else.** It cannot read the drop name, cannot resolve the ontology,
  and cannot substitute for a human verdict.

---

## 6. The architecture this data actually argues for

```
OCR the drop message        <- carries ~all the species signal (already exists)
        │
        ▼
Resolve against ONTOLOGY    <- the compound-name problem. HUMAN DECISION.
        │
        ├── unresolved/ambiguous ──> HUMAN REVIEW (UNKNOWN or CONFLICT)
        │
        ▼
confirmed canonical label
        │
        ▼
   clean training data ──> GPO model ──> shadow ──> production
```

Vision on these crops contributes ~5% of the between-class distance. The
highest-value work is **not** a better vision model or a bigger teacher — it is
(a) a human decision about the compound-name ontology, and (b) review of the
1,030 rows OCR cannot confirm.

This is a recommendation, not an implemented change. No feature was added.

---

## 7. What was and was not run

| Phase | Outcome |
|---|---|
| 1 — inspect existing systems | done (read-only) |
| 2 — isolated worker | **done and tested** (`ml/ram_plus/worker.py`, 8 safety tests, runs without torch) |
| 3 — model selection (RAM vs RAM++) | **NOT RUN** — torch install did not complete in this environment |
| 4 — custom GPO vocabulary | **done** (KB entities only; 18 excluded concepts pinned and test-enforced) |
| 5 — open-set contract | **done** (`RamResult` JSONL schema + strict validation) |
| 6 — inference over the dataset | **NOT RUN** — teacher unavailable |
| 7 — agreement matrix | **done** (9 classes, 11 tests) |
| 8 — never auto-create KB entities | **done, structurally** (no KB mutation path exists in the module) |
| 9 — this report | **done** |
| 10 — review UI evidence | **done** (status panel; per-row evidence lands with the RAM results file) |
| 11 — review priority | **done** (`evidence` sort mode) |
| 12 — label provenance | **done** (`LabelSource`, no teacher-derived eligible variant) |
| 13 — no pseudo-labels | **done** (no code path sets `training_eligible` from a teacher) |
| 26 — security | **done** (pinned+verified checkpoints, dataset-root path containment, no network at inference, no credentials) |

Phases 3 and 6 could not be executed: `torch` was not installed and the CUDA
wheel download did not complete. **No RAM++ output exists, and none is
fabricated.** Every number in this report comes from the real dataset or from
tests that run locally.

---

## 8. Final status

```
MODEL_NOT_READY
```

RAM++ does not change this. The binding constraint is a human decision about the
compound-name ontology plus review of 1,030 real rows. No threshold was
lowered, no entity was invented, and no teacher output was treated as truth.