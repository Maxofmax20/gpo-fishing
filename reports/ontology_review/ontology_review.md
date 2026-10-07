# In-game ontology review

Every concept the game appears to name, with the evidence behind it.

Names below are what the GAME appears to be writing. The current knowledge base's
candidates are shown as evidence, not as an answer: the ontology under review is
exactly the thing in question, so it must not bias the answer.

## How to read this

- **Occurrences** - rows whose freshly-read text matched this name.
- **Sessions / Days** - how many independent macro runs and calendar days
  produced it. Recurrence is the strongest evidence that a name is real
  game output rather than one OCR slip.
- **Current KB candidates** - what today's knowledge base *could* claim.
  Shown for reference only.
- **Stored labels** - what the dataset currently calls these rows.
  **Evidence, not truth**: those labels were produced under the ontology
  now in question.
- **Decision** - PENDING until a human rules. Nothing is pre-selected.

## Totals

| property | value |
|---|---|
| Distinct observed game names | 77 |
| …from raw OCR strings | 157 |
| Rows considered on entity frames | 2866 |
| Rows skipped as banner/system text | 304 |
| Rows skipped (not an entity frame) | 11939 |
| Concept families | 48 |
| Decisions recorded | 0 |
| Decisions pending | 77 |


### Review queue by priority band

| band | concepts | criterion |
|---|---|---|
| P0 | 11 | >=100 rows, >=10 sessions, >=3 days |
| P1 | 3 | >=25 rows, >=5 sessions |
| P2 | 16 | >=5 rows |
| P3 | 47 | singleton |


## P0 — 11 concept(s)

### Zebra Ribbon Angelfish

| property | value |
|---|---|
| Occurrences | 308 |
| Sessions | 46 |
| Calendar days | 5 |
| Priority | P0 (308 occurrences across 46 sessions / 5 days) |
| Concept id | `concept:zebra-ribbon-angelfish` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| angelfish | zebra, ribbon | family:angelfish |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:angelfish` | 205 |
| `fish:crimson` | 21 |
| `fish:golden` | 5 |
| `ui:new-item` | 5 |
| `fish:shark` | 4 |
| `fish:squid` | 3 |
| `fruit:yami` | 3 |
| `fish:snapper` | 2 |
| `fish:pufferfish` | 1 |
| `fruit:paw` | 1 |


**OCR variants seen:**

- `zebra ribbon angelfish` ×262
- `zebra ribbonangelfish` ×44
- `zebraribbon angelfish` ×2

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `606020203e34beb6` | 2026-10-02 | 2026-10-02_232819_2c7e6408 | `New Item <Zebra Ribbon Angelfish>` | `fish:angelfish` | 0.913 | highest OCR confidence |
| 2 | `602020203e3ebe9e` | 2026-10-02 | 2026-10-02_230043_e51ff1a7 | `New Item <Zebra Ribbon Angelfish>` | `fish:angelfish` | 0.911 | different session |
| 3 | `30301010be80ffff` | 2026-10-03 | 2026-10-03_104057_39d5add1 | `New Item <Zebra Ribbon Angelfish>` | `fish:angelfish` | 0.911 | different calendar day |
| 4 | `3b373f3e3e3e007f` | 2026-10-06 | 2026-10-06_175729_f925385b | `Ne Item <zebra RibbonAngelfish>` | - | 0.873 | different OCR variant |
| 5 | `202020203e3ebe9e` | 2026-10-02 | 2026-10-02_232819_2c7e6408 | `New Item <Zebra Ribbon Angelfish>` | `fruit:paw` | 0.913 | different stored label |
| 6 | `606020203e303eb0` | 2026-10-02 | 2026-10-02_232819_2c7e6408 | `New Item <Zebra Ribbon Angelfish>` | `fish:angelfish` | 0.913 | additional coverage |


_6 image(s) spanning 4 session(s), 3 day(s), 2 OCR variant(s), 3 stored label(s)_

**Decision:** **PENDING**

---

### Skeletalshark

| property | value |
|---|---|
| Occurrences | 293 |
| Sessions | 43 |
| Calendar days | 5 |
| Priority | P0 (293 occurrences across 43 sessions / 5 days) |
| Concept id | `concept:skeletalshark` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| skeletalshark | (none - single word) | family:skeletalshark |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:shark` | 170 |
| `fish:snapper` | 10 |
| `fish:squid` | 10 |
| `fish:angelfish` | 4 |
| `fish:crimson` | 1 |
| `fish:golden` | 1 |
| `fish:pufferfish` | 1 |
| `fish:swordfish` | 1 |
| `fruit:yami` | 1 |
| `ui:new-item` | 1 |


**OCR variants seen:**

- `skeletal shark` ×88
- `skeletalshark` ×205

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `6020a0e0fcf0f0f0` | 2026-10-05 | 2026-10-05_172944_49fd118f | `New Item <skeletal Shark` | `fish:shark` | 0.882 | highest OCR confidence |
| 2 | `606030303c34beb0` | 2026-10-05 | 2026-10-05_211733_183ded9a | `New Item skeletal Shark-` | `fish:shark` | 0.882 | different session |
| 3 | `606030303f3e3ebe` | 2026-10-02 | 2026-10-02_232819_2c7e6408 | `New Item skeletal Shark-` | `fish:squid` | 0.881 | different calendar day |
| 4 | `e0606060643e3e3e` | 2026-10-03 | 2026-10-03_052246_57c522fe | `NewItemskeletalShark` | - | 0.872 | different OCR variant |
| 5 | `677e3e3e06017f07` | 2026-10-04 | 2026-10-04_220833_0f8210ed | `NewItemskeletalShark` | `fish:angelfish` | 0.868 | different stored label |
| 6 | `606020203e3ebeb6` | 2026-10-02 | 2026-10-02_232819_2c7e6408 | `New Item skeletal Shark-` | `fish:shark` | 0.879 | additional coverage |


_6 image(s) spanning 5 session(s), 4 day(s), 2 OCR variant(s), 4 stored label(s)_

**Decision:** **PENDING**

---

### Crimson Polka Puffer

| property | value |
|---|---|
| Occurrences | 280 |
| Sessions | 39 |
| Calendar days | 5 |
| Priority | P0 (280 occurrences across 39 sessions / 5 days) |
| Concept id | `concept:crimson-polka-puffer` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| puffer | crimson, polka | family:puffer |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:crimson` | 161 |
| `fish:pufferfish` | 60 |
| `fish:angelfish` | 23 |
| `fish:golden` | 10 |
| `fish:shark` | 5 |
| `fish:squid` | 4 |
| `fish:snapper` | 2 |
| `ui:new-item` | 2 |
| `fish:swordfish` | 1 |
| `fruit:kilo` | 1 |
| `ui:spawned` | 1 |


**OCR variants seen:**

- `crimson polka puffer` ×247
- `crimson polkapuffer` ×24
- `crimsonpolka puffer` ×9

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `e02020203e3034be` | 2026-10-03 | 2026-10-03_223708_0ca663cf | `New Item <crimson Polka Puffer>` | `fish:golden` | 0.914 | highest OCR confidence |
| 2 | `30101090be9effff` | 2026-10-02 | 2026-10-02_215405_f5d40eba | `New Item <Crimson Polka Puffer>` | `fish:angelfish` | 0.914 | different session |
| 3 | `e1303c333f3f1f9f` | 2026-10-06 | 2026-10-06_225424_a78e14f2 | `Nev Item <crimson Polka Puffer>` | `fish:crimson` | 0.914 | different calendar day |
| 4 | `606060603e303e00` | 2026-10-03 | 2026-10-03_095208_9aa2f3b7 | `New Item <crimson PolkaPuffer>` | `fish:crimson` | 0.893 | different OCR variant |
| 5 | `e02020203e3a3cbe` | 2026-10-03 | 2026-10-03_223708_0ca663cf | `New Item <crimson Polka Puffer>` | `fish:pufferfish` | 0.914 | different stored label |
| 6 | `20301090be80ffff` | 2026-10-03 | 2026-10-03_115211_8fa9d196 | `New Item <Crimson Polka Puffer>` | `fish:angelfish` | 0.914 | additional coverage |


_6 image(s) spanning 5 session(s), 3 day(s), 2 OCR variant(s), 4 stored label(s)_

**Decision:** **PENDING**

---

### Crimson Snapper

| property | value |
|---|---|
| Occurrences | 276 |
| Sessions | 41 |
| Calendar days | 5 |
| Priority | P0 (276 occurrences across 41 sessions / 5 days) |
| Concept id | `concept:crimson-snapper` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| snapper | crimson | family:snapper |


**Current KB candidates:**

- `fish:crimson`
- `fish:snapper`

_AMBIGUOUS: contains more than one canonical entity, so today's KB has no single answer_

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:snapper` | 199 |
| `fish:squid` | 13 |
| `fish:crimson` | 12 |
| `fish:shark` | 9 |
| `fish:angelfish` | 6 |
| `fish:golden` | 2 |
| `fish:pufferfish` | 2 |
| `fish:swordfish` | 2 |
| `fruit:yami` | 2 |
| `ui:new-item` | 2 |


_210 of 276 rows have a stored label equal to one KB candidate. That means the label agrees with one COMPONENT of the name — it does NOT mean the name is confirmed._

**OCR variants seen:**

- `crimson snapper` ×249
- `crimsonsnapper` ×27

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `30301016bc9fffff` | 2026-10-03 | 2026-10-03_115211_8fa9d196 | `New Item <crimson Snapper>` | `fish:snapper` | 0.887 | highest OCR confidence |
| 2 | `6066262b3e3ebe90` | 2026-10-04 | 2026-10-04_202745_08b4b13a | `New Item <Crimson SnaPper>` | `fish:snapper` | 0.881 | different session |
| 3 | `30303b123e9f80df-1` | 2026-10-06 | 2026-10-06_212246_fe47a4fa | `New Item <Crimson SnaPper>` | `fish:snapper` | 0.880 | different calendar day |
| 4 | `60603337363e3e10` | 2026-10-03 | 2026-10-03_171113_094bcfa0 | `Nev Item<crimsonSnapper>` | `fish:squid` | 0.849 | different OCR variant |
| 5 | `008080d7ffdede46` | 2026-10-04 | 2026-10-04_160014_94aaa3c9 | `New Item <crimson Snapper>` | `fish:shark` | 0.871 | different stored label |
| 6 | `30303810be9e80df` | 2026-10-06 | 2026-10-06_212246_fe47a4fa | `New Item <Crimson SnaPper>` | `fish:snapper` | 0.880 | additional coverage |


_6 image(s) spanning 5 session(s), 3 day(s), 2 OCR variant(s), 3 stored label(s)_

**Decision:** **PENDING**

---

### Exotic Tigerfin

| property | value |
|---|---|
| Occurrences | 262 |
| Sessions | 36 |
| Calendar days | 5 |
| Priority | P0 (262 occurrences across 36 sessions / 5 days) |
| Concept id | `concept:exotic-tigerfin` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| tigerfin | exotic | family:tigerfin |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:snapper` | 10 |
| `fish:shark` | 8 |
| `fish:squid` | 8 |
| `fish:angelfish` | 2 |
| `fish:golden` | 2 |
| `ui:new-item` | 2 |
| `fish:crimson` | 1 |
| `fish:pufferfish` | 1 |
| `fish:swordfish` | 1 |
| `fish:tiger` | 1 |
| `fruit:yami` | 1 |


**OCR variants seen:**

- `exotic tigerfin` ×245
- `exotictigerfin` ×17

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `e0e060203c3b3e3e` | 2026-10-04 | 2026-10-04_120009_b970ca36 | `New Item <Exotic Tigerfin>` | `fish:shark` | 0.920 | highest OCR confidence |
| 2 | `606023233f3f3ebf` | 2026-10-02 | 2026-10-02_230043_e51ff1a7 | `New Item <Exotic Tigerfin>` | - | 0.916 | different session |
| 3 | `30303010bc94ffff` | 2026-10-03 | 2026-10-03_115211_8fa9d196 | `New Item <Exotic Tigerfin>` | `fish:snapper` | 0.910 | different calendar day |
| 4 | `e1303f33073f1f9f` | 2026-10-06 | 2026-10-06_225424_a78e14f2 | `New Item <ExoticTigerfin` | - | 0.869 | different OCR variant |
| 5 | `30303010bc94ffeb` | 2026-10-03 | 2026-10-03_115211_8fa9d196 | `New Item <Exotic Tigerfin>` | `fish:pufferfish` | 0.909 | different stored label |
| 6 | `33311f1f3f9fd9c0` | 2026-10-04 | 2026-10-04_225639_0180cb9d | `New Item <Exotic Tigerfin>` | - | 0.910 | additional coverage |


_6 image(s) spanning 5 session(s), 4 day(s), 2 OCR variant(s), 4 stored label(s)_

**Decision:** **PENDING**

---

### Candy Corn Squid

| property | value |
|---|---|
| Occurrences | 241 |
| Sessions | 44 |
| Calendar days | 5 |
| Priority | P0 (241 occurrences across 44 sessions / 5 days) |
| Concept id | `concept:candy-corn-squid` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| squid | candy, corn | family:squid |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:squid` | 195 |
| `fish:snapper` | 12 |
| `fish:shark` | 5 |
| `fish:crimson` | 4 |
| `fish:angelfish` | 2 |
| `fish:golden` | 1 |
| `fish:pufferfish` | 1 |
| `fish:swordfish` | 1 |


**OCR variants seen:**

- `candy corn squid` ×210
- `candy cornsquid` ×3
- `candycorn squid` ×20
- `candycornsquid` ×8

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `303030103c9fd8c8` | 2026-10-04 | 2026-10-04_225639_0180cb9d | `New Item <Candy Corn Squid>` | `fish:squid` | 0.899 | highest OCR confidence |
| 2 | `00e0f0f0fcfeffff` | 2026-10-03 | 2026-10-03_132921_cfb209d0 | `New Item <Candy Corn Squid>` | `fish:squid` | 0.891 | different session |
| 3 | `303030303e9e809f-9` | 2026-10-06 | 2026-10-06_212246_fe47a4fa | `New Item <Candy Com Squid>` | `fish:squid` | 0.883 | different calendar day |
| 4 | `333c3a3e96809fff` | 2026-10-04 | 2026-10-04_073807_04bff67c | `Nev Item<CandyCorn Squid` | `fish:squid` | 0.869 | different OCR variant |
| 5 | `e06060603c22343e` | 2026-10-03 | 2026-10-03_052246_57c522fe | `New Item <Candy Corn Squid>` | `fish:shark` | 0.880 | different stored label |
| 6 | `6066223b3e3ebe80` | 2026-10-04 | 2026-10-04_202745_08b4b13a | `New Item <Candy Corn Squid>` | `fish:squid` | 0.883 | additional coverage |


_6 image(s) spanning 6 session(s), 3 day(s), 2 OCR variant(s), 2 stored label(s)_

**Decision:** **PENDING**

---

### Swordfish

| property | value |
|---|---|
| Occurrences | 114 |
| Sessions | 39 |
| Calendar days | 5 |
| Priority | P0 (114 occurrences across 39 sessions / 5 days) |
| Concept id | `concept:swordfish` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| swordfish | (none - single word) | family:swordfish |


**Current KB candidates:**

- `fish:swordfish`

_EXACT_ENTITY: is exactly one canonical entity_

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:swordfish` | 51 |
| `fish:snapper` | 4 |
| `fish:golden` | 2 |
| `fish:squid` | 2 |
| `ui:new-item` | 2 |
| `fish:shark` | 1 |


_51 of 114 rows have a stored label equal to one KB candidate. That means the label agrees with one COMPONENT of the name — it does NOT mean the name is confirmed._

**OCR variants seen:**

- `swordfish` ×114

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `e86060607c602034` | 2026-10-04 | 2026-10-04_201848_52e11685 | `New Item <Swordfish>` | `fish:swordfish` | 0.877 | highest OCR confidence |
| 2 | `f03030303c3030b0-6` | 2026-10-06 | 2026-10-06_225424_a78e14f2 | `New Item <Swordfish>` | - | 0.877 | different session |
| 3 | `e06030303c3030b0` | 2026-10-02 | 2026-10-02_232819_2c7e6408 | `New Item <Swordfish>` | `fish:swordfish` | 0.874 | different calendar day |
| 4 | `606060603c343e00` | 2026-10-03 | 2026-10-03_095208_9aa2f3b7 | `New Item <Swordfish>` | `fish:snapper` | 0.856 | different stored label |
| 5 | `f03030303c3030b0-7` | 2026-10-06 | 2026-10-06_225424_a78e14f2 | `New Item <Swordfish>` | - | 0.875 | additional coverage |
| 6 | `f03030303c3030b0-4` | 2026-10-06 | 2026-10-06_225424_a78e14f2 | `New Item <Swordfish>` | - | 0.875 | additional coverage |


_6 image(s) spanning 4 session(s), 4 day(s), 1 OCR variant(s), 3 stored label(s)_

**Decision:** **PENDING**

---

### Golden Ribbon Angelfish

| property | value |
|---|---|
| Occurrences | 114 |
| Sessions | 36 |
| Calendar days | 5 |
| Priority | P0 (114 occurrences across 36 sessions / 5 days) |
| Concept id | `concept:golden-ribbon-angelfish` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| angelfish | golden, ribbon | family:angelfish |


**Current KB candidates:**

- `fish:angelfish`
- `fish:golden`

_AMBIGUOUS: contains more than one canonical entity, so today's KB has no single answer_

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:golden` | 76 |
| `fish:angelfish` | 14 |
| `fish:crimson` | 1 |
| `fish:snapper` | 1 |
| `fruit:yami` | 1 |


_87 of 114 rows have a stored label equal to one KB candidate. That means the label agrees with one COMPONENT of the name — it does NOT mean the name is confirmed._

**OCR variants seen:**

- `golden ribbon angelfish` ×109
- `golden ribbonangelfish` ×5

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `30809ac7ffc24242` | 2026-10-04 | 2026-10-04_160014_94aaa3c9 | `New Item <Golden Ribbon Angelfish-` | `fish:golden` | 0.915 | highest OCR confidence |
| 2 | `e06060203e30303e` | 2026-10-03 | 2026-10-03_110949_15b78581 | `New Item <Golden Ribbon Angelfish-` | `fish:crimson` | 0.915 | different session |
| 3 | `606020203e32007f-3` | 2026-10-06 | 2026-10-06_175729_f925385b | `New Item <Golden Ribbon Angelfish>` | `fish:golden` | 0.911 | different calendar day |
| 4 | `30303030be9eb1e1` | 2026-10-03 | 2026-10-03_161417_bb4b5817 | `New Item <Golden RibbonAngelfish-` | `fish:golden` | 0.907 | different OCR variant |
| 5 | `30303090beffe3e3` | 2026-10-03 | 2026-10-03_161417_bb4b5817 | `New Item <Golden Ribbon Angelfish-` | `fish:angelfish` | 0.906 | different stored label |
| 6 | `308094c7ffc24242` | 2026-10-04 | 2026-10-04_160014_94aaa3c9 | `New Item <Golden Ribbon Angelfish-` | `fish:golden` | 0.914 | additional coverage |


_6 image(s) spanning 4 session(s), 3 day(s), 2 OCR variant(s), 3 stored label(s)_

**Decision:** **PENDING**

---

### Golden Polka Puffer

| property | value |
|---|---|
| Occurrences | 111 |
| Sessions | 30 |
| Calendar days | 5 |
| Priority | P0 (111 occurrences across 30 sessions / 5 days) |
| Concept id | `concept:golden-polka-puffer` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| puffer | golden, polka | family:puffer |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:golden` | 68 |
| `fish:pufferfish` | 25 |
| `fish:crimson` | 2 |
| `fish:squid` | 2 |
| `fish:angelfish` | 1 |
| `fish:shark` | 1 |
| `fish:swordfish` | 1 |
| `fruit:yami` | 1 |


**OCR variants seen:**

- `golden polka puffer` ×110
- `golden polkapuffer` ×1

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `38383030be809eff` | 2026-10-04 | 2026-10-04_073807_04bff67c | `Nev Item <Golden Polka Puffer-` | `fish:golden` | 0.916 | highest OCR confidence |
| 2 | `30303030be9e80df-7` | 2026-10-06 | 2026-10-06_212246_fe47a4fa | `New Item <Golden Polka Puffer>` | `fish:golden` | 0.912 | different session |
| 3 | `f06020203e3e3e3e` | 2026-10-03 | 2026-10-03_052246_57c522fe | `New Item <Golden Polka Puffer>` | `fish:golden` | 0.904 | different calendar day |
| 4 | `e0607833173f3e3e` | 2026-10-05 | 2026-10-05_132601_bb362a56 | `New Item <Golden PolkaPuffer?` | `fish:golden` | 0.812 | different OCR variant |
| 5 | `e46420203e3036b0` | 2026-10-02 | 2026-10-02_232819_2c7e6408 | `Ne Item Golden Polka Puffer-` | `fish:pufferfish` | 0.903 | different stored label |
| 6 | `38181090bc9fffe2` | 2026-10-03 | 2026-10-03_132921_cfb209d0 | `New Item <Golden Polka Puffer?` | `fish:golden` | 0.904 | additional coverage |


_6 image(s) spanning 6 session(s), 5 day(s), 2 OCR variant(s), 2 stored label(s)_

**Decision:** **PENDING**

---

### Dark Skeletal Shark

| property | value |
|---|---|
| Occurrences | 102 |
| Sessions | 35 |
| Calendar days | 5 |
| Priority | P0 (102 occurrences across 35 sessions / 5 days) |
| Concept id | `concept:dark-skeletal-shark` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| shark | dark, skeletal | family:shark |


**Current KB candidates:**

- `fish:shark`
- `fruit:dark`

_AMBIGUOUS: contains more than one canonical entity, so today's KB has no single answer_

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fruit:yami` | 57 |
| `fish:shark` | 30 |
| `fish:crimson` | 4 |
| `fish:snapper` | 2 |
| `fish:angelfish` | 1 |
| `fish:golden` | 1 |
| `fish:pufferfish` | 1 |
| `fish:squid` | 1 |


_17 of 102 rows have a stored label equal to one KB candidate. That means the label agrees with one COMPONENT of the name — it does NOT mean the name is confirmed._

**OCR variants seen:**

- `dark skeletal shark` ×48
- `dark skeletalshark` ×3
- `darkskeletal shark` ×26
- `darkskeletalshark` ×25

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `f07060607ce0e0e0` | 2026-10-05 | 2026-10-05_190403_c9429492 | `New Item <Dark Skeletal Shark-` | `fruit:yami` | 0.915 | highest OCR confidence |
| 2 | `e06060203e30343e` | 2026-10-04 | 2026-10-04_120009_b970ca36 | `New Item <Dark Skeletal Shark-` | `fish:crimson` | 0.911 | different session |
| 3 | `606060203e3e3e00` | 2026-10-03 | 2026-10-03_095208_9aa2f3b7 | `New Item <Dark Skeletal Shark-` | `fish:crimson` | 0.902 | different calendar day |
| 4 | `32323010be809fff` | 2026-10-03 | 2026-10-03_104057_39d5add1 | `New Item <DarkSkeletal Shark-` | `fruit:yami` | 0.894 | different OCR variant |
| 5 | `30303010be80ffff` | 2026-10-03 | 2026-10-03_104057_39d5add1 | `New Item <Dark Skeletal Shark-` | `fish:golden` | 0.889 | different stored label |
| 6 | `e6686c603e202034` | 2026-10-05 | 2026-10-05_190403_c9429492 | `New Item <Dark Skeletal Shark-` | `fruit:yami` | 0.913 | additional coverage |


_6 image(s) spanning 4 session(s), 3 day(s), 2 OCR variant(s), 3 stored label(s)_

**Decision:** **PENDING**

---

### Golden Tigerfin

| property | value |
|---|---|
| Occurrences | 102 |
| Sessions | 30 |
| Calendar days | 5 |
| Priority | P0 (102 occurrences across 30 sessions / 5 days) |
| Concept id | `concept:golden-tigerfin` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| tigerfin | golden | family:tigerfin |


**Current KB candidates:**

- `fish:golden`
- `fish:tiger`

_AMBIGUOUS: contains more than one canonical entity, so today's KB has no single answer_

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:golden` | 72 |
| `fish:squid` | 3 |
| `fish:shark` | 2 |
| `fish:snapper` | 2 |
| `fish:tiger` | 1 |
| `ui:new-item` | 1 |


_3 of 102 rows have a stored label equal to one KB candidate. That means the label agrees with one COMPONENT of the name — it does NOT mean the name is confirmed._

**OCR variants seen:**

- `golden tigerfin` ×96
- `goldentigerfin` ×6

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `7b7b272f2f3e1e00` | 2026-10-04 | 2026-10-04_202745_08b4b13a | `Nev Item <Golden Tigerfin` | - | 0.928 | highest OCR confidence |
| 2 | `e2e262223c203030` | 2026-10-04 | 2026-10-04_120009_b970ca36 | `New Item <Golden Tigerfin` | `fish:golden` | 0.905 | different session |
| 3 | `e06060223c303030` | 2026-10-03 | 2026-10-03_110949_15b78581 | `New Item <Golden Tigerfinz` | `fish:golden` | 0.896 | different calendar day |
| 4 | `e0303033373f3f1f-16` | 2026-10-06 | 2026-10-06_225424_a78e14f2 | `New Item <GoldenTigerfin` | `fish:golden` | 0.869 | different OCR variant |
| 5 | `e0e060203c303030` | 2026-10-04 | 2026-10-04_112359_6e9b9aa5 | `New Item <Golden Tigerfin?` | `fish:snapper` | 0.898 | different stored label |
| 6 | `e0e060607c202034` | 2026-10-04 | 2026-10-04_141428_4c6bc299 | `New Item <Golden Tigerfin-` | `fish:golden` | 0.903 | additional coverage |


_6 image(s) spanning 6 session(s), 3 day(s), 2 OCR variant(s), 3 stored label(s)_

**Decision:** **PENDING**

---

## P1 — 3 concept(s)

### Fangfish

| property | value |
|---|---|
| Occurrences | 59 |
| Sessions | 27 |
| Calendar days | 4 |
| Priority | P1 (59 occurrences across 27 sessions) |
| Concept id | `concept:fangfish` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| fangfish | (none - single word) | family:fangfish |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:squid` | 11 |
| `fish:angelfish` | 9 |
| `fish:snapper` | 9 |
| `fish:shark` | 4 |
| `fish:crimson` | 3 |
| `fish:swordfish` | 3 |
| `fish:pufferfish` | 2 |
| `fish:golden` | 1 |


**OCR variants seen:**

- `fangfish` ×59

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `e06060607c60783e` | 2026-10-05 | 2026-10-05_234514_efb8fcc9 | `Nev Item <fangfish>` | `fish:shark` | 0.855 | highest OCR confidence |
| 2 | `303030101e9fd8c8` | 2026-10-04 | 2026-10-04_225639_0180cb9d | `Ne Item <Fangfish>` | - | 0.851 | different session |
| 3 | `606020303e3ebe80` | 2026-10-03 | 2026-10-03_220117_1e7efb16 | `Nev Item <Fangfish` | `fish:angelfish` | 0.843 | different calendar day |
| 4 | `e06060607c60687c` | 2026-10-05 | 2026-10-05_204832_5353f44f | `Ne Item <fangfish>` | `fish:squid` | 0.843 | different stored label |
| 5 | `e06060607ce0e0e0` | 2026-10-05 | 2026-10-05_201952_14b664e4 | `Ne Item <fangfish>` | `fish:shark` | 0.845 | additional coverage |
| 6 | `606030303e3ebe80` | 2026-10-03 | 2026-10-03_220117_1e7efb16 | `Ne Item <fangfish>` | - | 0.833 | additional coverage |


_6 image(s) spanning 5 session(s), 3 day(s), 1 OCR variant(s), 4 stored label(s)_

**Decision:** **PENDING**

---

### Tigerfin

| property | value |
|---|---|
| Occurrences | 52 |
| Sessions | 5 |
| Calendar days | 4 |
| Priority | P1 (52 occurrences across 5 sessions) |
| Concept id | `concept:tigerfin` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| tigerfin | (none - single word) | family:tigerfin |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:snapper` | 2 |
| `fish:shark` | 1 |
| `ui:new-item` | 1 |


**OCR variants seen:**

- `tigerfin` ×52

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `7c6424303c30007f` | 2026-10-06 | 2026-10-06_175729_f925385b | `New Item <Tigerfin>` | - | 0.898 | highest OCR confidence |
| 2 | `e0e060203c383c3e` | 2026-10-04 | 2026-10-04_120009_b970ca36 | `New Item <Tigerfin>` | - | 0.897 | different session |
| 3 | `e02036333f3f3ebe` | 2026-10-03 | 2026-10-03_223708_0ca663cf | `New Item <Tigerfin>` | `fish:snapper` | 0.896 | different calendar day |
| 4 | `e12036333f3f3ebe` | 2026-10-03 | 2026-10-03_223708_0ca663cf | `New Item <Tigerfin>` | `fish:shark` | 0.873 | different stored label |
| 5 | `e2e060203c3b3e3e` | 2026-10-04 | 2026-10-04_120009_b970ca36 | `New Item <Tigerfin>` | - | 0.897 | additional coverage |
| 6 | `606020303ebe80ff-2` | 2026-10-06 | 2026-10-06_175729_f925385b | `New Item <Tigerfin>` | - | 0.896 | additional coverage |


_6 image(s) spanning 3 session(s), 3 day(s), 1 OCR variant(s), 3 stored label(s)_

**Decision:** **PENDING**

---

### Sgolden Tigerfin

| property | value |
|---|---|
| Occurrences | 33 |
| Sessions | 18 |
| Calendar days | 5 |
| Priority | P1 (33 occurrences across 18 sessions) |
| Concept id | `concept:sgolden-tigerfin` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| tigerfin | sgolden | family:tigerfin |


**Current KB candidates:**

- `fish:golden`
- `fish:tiger`

_AMBIGUOUS: contains more than one canonical entity, so today's KB has no single answer_

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:golden` | 20 |
| `fish:shark` | 2 |
| `fish:angelfish` | 1 |


_20 of 33 rows have a stored label equal to one KB candidate. That means the label agrees with one COMPONENT of the name — it does NOT mean the name is confirmed._

**OCR variants seen:**

- `sgolden tigerfin` ×33

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `303030303fbe9e9f` | 2026-10-04 | 2026-10-04_070602_b6f12619 | `New Item sGolden Tigerfin` | `fish:golden` | 0.907 | highest OCR confidence |
| 2 | `f03030303c303092-1` | 2026-10-06 | 2026-10-06_225424_a78e14f2 | `New Item sGolden Tigerfin` | `fish:golden` | 0.906 | different session |
| 3 | `e86060203c303030` | 2026-10-05 | 2026-10-05_161430_f023a515 | `New Item sGolden Tigerfin` | `fish:golden` | 0.905 | different calendar day |
| 4 | `607077777e3e3e3e` | 2026-10-03 | 2026-10-03_051036_cf6004f2 | `New Item sGolden Tigerfin?` | - | 0.860 | different stored label |
| 5 | `f83030303c30309a` | 2026-10-06 | 2026-10-06_225424_a78e14f2 | `New Item sGolden Tigerfin` | `fish:golden` | 0.906 | additional coverage |
| 6 | `e26266233c203030` | 2026-10-05 | 2026-10-05_161430_f023a515 | `New Item sGolden Tigerfin` | `fish:golden` | 0.905 | additional coverage |


_6 image(s) spanning 4 session(s), 4 day(s), 1 OCR variant(s), 2 stored label(s)_

**Decision:** **PENDING**

---

## P2 — 16 concept(s)

### Sgolden Ribbon Angelfish

| property | value |
|---|---|
| Occurrences | 19 |
| Sessions | 11 |
| Calendar days | 5 |
| Priority | P2 (19 occurrences) |
| Concept id | `concept:sgolden-ribbon-angelfish` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| angelfish | sgolden, ribbon | family:angelfish |


**Current KB candidates:**

- `fish:angelfish`
- `fish:golden`

_AMBIGUOUS: contains more than one canonical entity, so today's KB has no single answer_

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:golden` | 13 |
| `fish:angelfish` | 4 |


_1 of 19 rows have a stored label equal to one KB candidate. That means the label agrees with one COMPONENT of the name — it does NOT mean the name is confirmed._

**OCR variants seen:**

- `sgolden ribbon angelfish` ×18
- `sgolden ribbonangelfish` ×1

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `606060203e363200` | 2026-10-04 | 2026-10-04_202745_08b4b13a | `New Item sGolden Ribbon Angelfish-` | `fish:golden` | 0.897 | highest OCR confidence |
| 2 | `e22020203e30303e` | 2026-10-03 | 2026-10-03_223708_0ca663cf | `New Item sGolden Ribbon Angelfish-` | `fish:golden` | 0.886 | different session |
| 3 | `f06020203e303eb0` | 2026-10-02 | 2026-10-02_232819_2c7e6408 | `New Item sGolden Ribbon Angelfish-` | `fish:golden` | 0.881 | different calendar day |
| 4 | `602021a0c0fefefe` | 2026-10-04 | 2026-10-04_202745_08b4b13a | `New Item sGolden RibbonAngelfish-` | `fish:angelfish` | 0.845 | different OCR variant |
| 5 | `6060303c267f7f7f` | 2026-10-04 | 2026-10-04_220833_0f8210ed | `New Item sGolden Ribbon Angelfish-` | - | 0.834 | different stored label |
| 6 | `e06060607e60787c` | 2026-10-05 | 2026-10-05_204832_5353f44f | `New Item sGolden Ribbon Angelfish>` | `fish:golden` | 0.879 | additional coverage |


_6 image(s) spanning 5 session(s), 4 day(s), 2 OCR variant(s), 3 stored label(s)_

**Decision:** **PENDING**

---

### Sdark Skeletal Shark

| property | value |
|---|---|
| Occurrences | 16 |
| Sessions | 11 |
| Calendar days | 5 |
| Priority | P2 (16 occurrences) |
| Concept id | `concept:sdark-skeletal-shark` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| shark | sdark, skeletal | family:shark |


**Current KB candidates:**

- `fish:shark`
- `fruit:dark`

_AMBIGUOUS: contains more than one canonical entity, so today's KB has no single answer_

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fruit:yami` | 15 |
| `fish:swordfish` | 1 |


**OCR variants seen:**

- `sdark skeletal shark` ×15
- `sdarkskeletalshark` ×1

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `f8e060603e203030` | 2026-10-04 | 2026-10-04_120009_b970ca36 | `New Item sDark Skeletal Shark-` | `fruit:yami` | 0.905 | highest OCR confidence |
| 2 | `f03030303e3030b0` | 2026-10-06 | 2026-10-06_225424_a78e14f2 | `New Item sDark Skeletal Shark-` | `fruit:yami` | 0.900 | different session |
| 3 | `786c6c283c202000` | 2026-10-03 | 2026-10-03_095208_9aa2f3b7 | `New Item sDark Skeletal Shark-` | `fruit:yami` | 0.900 | different calendar day |
| 4 | `3e989edf47464242` | 2026-10-04 | 2026-10-04_160014_94aaa3c9 | `New Item sDarkskeletalshark-` | `fruit:yami` | 0.833 | different OCR variant |
| 5 | `606020203e343ebe` | 2026-10-03 | 2026-10-03_091319_6fbb96f4 | `New Item sDark Skeletal Shark-` | `fish:swordfish` | 0.886 | different stored label |
| 6 | `e06060607e203422` | 2026-10-03 | 2026-10-03_051036_cf6004f2 | `New Item sDark Skeletal Shark-` | `fruit:yami` | 0.896 | additional coverage |


_6 image(s) spanning 6 session(s), 3 day(s), 2 OCR variant(s), 2 stored label(s)_

**Decision:** **PENDING**

---

### Blve Lipgrouper

| property | value |
|---|---|
| Occurrences | 14 |
| Sessions | 5 |
| Calendar days | 4 |
| Priority | P2 (14 occurrences) |
| Concept id | `concept:blve-lipgrouper` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| lipgrouper | blve | family:lipgrouper |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:angelfish` | 1 |
| `fish:golden` | 1 |
| `fish:shark` | 1 |
| `fish:snapper` | 1 |
| `fish:swordfish` | 1 |
| `fruit:light` | 1 |


**OCR variants seen:**

- `blve lip grouper` ×4
- `blve lipgrouper` ×10

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `e0e060203c303c3e` | 2026-10-04 | 2026-10-04_120009_b970ca36 | `New Item <Blve-Lip Grouper>` | `fish:shark` | 0.903 | highest OCR confidence |
| 2 | `606020303c34be80` | 2026-10-03 | 2026-10-03_220117_1e7efb16 | `Nev Item <Blve-Lip GroUper>` | `fish:snapper` | 0.884 | different session |
| 3 | `70673e3e3e3e003f-1` | 2026-10-06 | 2026-10-06_175729_f925385b | `NevItem<Blve-LipGrouper>` | - | 0.857 | different calendar day |
| 4 | `e02020203c3034be` | 2026-10-03 | 2026-10-03_223708_0ca663cf | `New Item <Blve-Lip Grouper>` | `fish:swordfish` | 0.873 | different stored label |
| 5 | `e0607073677e3e3e` | 2026-10-05 | 2026-10-05_234514_efb8fcc9 | `Nev Item<Blve-LipGrouper>` | `fish:golden` | 0.851 | additional coverage |
| 6 | `7f3f3f3e3e3e003f` | 2026-10-06 | 2026-10-06_175729_f925385b | `NevItem <Blve-LipGrouper>` | - | 0.850 | additional coverage |


_6 image(s) spanning 5 session(s), 4 day(s), 2 OCR variant(s), 5 stored label(s)_

**Decision:** **PENDING**

---

### Tigerfinz

| property | value |
|---|---|
| Occurrences | 13 |
| Sessions | 3 |
| Calendar days | 3 |
| Priority | P2 (13 occurrences) |
| Concept id | `concept:tigerfinz` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| tigerfinz | (none - single word) | family:tigerfinz |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**OCR variants seen:**

- `tigerfinz` ×13

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `60203e3e3ebe80ff` | 2026-10-06 | 2026-10-06_175729_f925385b | `New Item <Tigerfinz` | - | 0.895 | highest OCR confidence |
| 2 | `e02030232f3f3ebe` | 2026-10-03 | 2026-10-03_223708_0ca663cf | `Ne Item <Tigerfinz` | - | 0.850 | different session |
| 3 | `e0607433273f3e3e` | 2026-10-04 | 2026-10-04_120009_b970ca36 | `Nev Item<Tigerfinz` | - | 0.798 | different calendar day |
| 4 | `602020303ebe80ff-4` | 2026-10-06 | 2026-10-06_175729_f925385b | `New Item <Tigerfinz` | - | 0.863 | additional coverage |
| 5 | `20203e363ebe80ff-5` | 2026-10-06 | 2026-10-06_175729_f925385b | `New Item <Tigerfinz` | - | 0.860 | additional coverage |
| 6 | `202038363ebe80ff` | 2026-10-06 | 2026-10-06_175729_f925385b | `New Item <Tigerfinz` | - | 0.859 | additional coverage |


_6 image(s) spanning 3 session(s), 3 day(s), 1 OCR variant(s), 1 stored label(s)_

**Decision:** **PENDING**

---

### Sgolden Polka Puffer

| property | value |
|---|---|
| Occurrences | 12 |
| Sessions | 8 |
| Calendar days | 3 |
| Priority | P2 (12 occurrences) |
| Concept id | `concept:sgolden-polka-puffer` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| puffer | sgolden, polka | family:puffer |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:golden` | 7 |
| `fish:pufferfish` | 3 |
| `fish:shark` | 1 |


**OCR variants seen:**

- `sgolden polka puffer` ×12

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `e0e060603c202030` | 2026-10-04 | 2026-10-04_120009_b970ca36 | `New Item sGolden Polka Puffer?` | `fish:golden` | 0.900 | highest OCR confidence |
| 2 | `7c7878783c302020` | 2026-10-03 | 2026-10-03_051036_cf6004f2 | `New Item sGolden Polka Puffer?` | `fish:golden` | 0.877 | different session |
| 3 | `e0783327273e3e1e` | 2026-10-02 | 2026-10-02_230043_e51ff1a7 | `Nev Item sGolden Polka Puffer-` | `fish:pufferfish` | 0.857 | different calendar day |
| 4 | `203927272f3e3e3e` | 2026-10-03 | 2026-10-03_091319_6fbb96f4 | `Nev Item sGolden Polka Puffer?` | - | 0.860 | different stored label |
| 5 | `e0607f23273e3e3e` | 2026-10-03 | 2026-10-03_052246_57c522fe | `Nev Item sGolden Polka Puffer?` | `fish:pufferfish` | 0.869 | additional coverage |
| 6 | `706060203e303000` | 2026-10-04 | 2026-10-04_202745_08b4b13a | `New Item sGolden Polka Puffer` | `fish:golden` | 0.850 | additional coverage |


_6 image(s) spanning 6 session(s), 3 day(s), 1 OCR variant(s), 3 stored label(s)_

**Decision:** **PENDING**

---

### Candy Cormn Squid

| property | value |
|---|---|
| Occurrences | 10 |
| Sessions | 6 |
| Calendar days | 4 |
| Priority | P2 (10 occurrences) |
| Concept id | `concept:candy-cormn-squid` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| squid | candy, cormn | family:squid |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:squid` | 10 |


**OCR variants seen:**

- `candy cormn squid` ×10

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `303038303e9e80df-8` | 2026-10-06 | 2026-10-06_212246_fe47a4fa | `New Item <Candy Cormn Squid>` | `fish:squid` | 0.859 | highest OCR confidence |
| 2 | `202030303ebe80ff` | 2026-10-06 | 2026-10-06_175729_f925385b | `New Item <Candy Cormn Squid>` | `fish:squid` | 0.834 | different session |
| 3 | `606020303c3036be` | 2026-10-03 | 2026-10-03_091319_6fbb96f4 | `New Item <Candy Cormn Squid>` | `fish:squid` | 0.829 | different calendar day |
| 4 | `303038303e9e80df` | 2026-10-06 | 2026-10-06_212246_fe47a4fa | `New Item <Candy Cormn Squid>` | `fish:squid` | 0.850 | additional coverage |
| 5 | `303030303c9e809f-4` | 2026-10-06 | 2026-10-06_212246_fe47a4fa | `New Item <Candy Cormn Squid>` | `fish:squid` | 0.848 | additional coverage |
| 6 | `303030303e9e809f-2` | 2026-10-06 | 2026-10-06_212246_fe47a4fa | `New Item <Candy Cormn Squid>` | `fish:squid` | 0.847 | additional coverage |


_6 image(s) spanning 3 session(s), 2 day(s), 1 OCR variant(s), 1 stored label(s)_

**Decision:** **PENDING**

---

### Anglerfish

| property | value |
|---|---|
| Occurrences | 8 |
| Sessions | 7 |
| Calendar days | 2 |
| Priority | P2 (8 occurrences) |
| Concept id | `concept:anglerfish` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| anglerfish | (none - single word) | family:anglerfish |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:snapper` | 2 |
| `fish:squid` | 2 |
| `fish:angelfish` | 1 |
| `fish:shark` | 1 |
| `fish:swordfish` | 1 |


**OCR variants seen:**

- `anglerfish` ×8

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `e06060603c22303e` | 2026-10-03 | 2026-10-03_052246_57c522fe | `New Item <Anglerfish-` | `fish:squid` | 0.893 | highest OCR confidence |
| 2 | `e0e060203c30343e` | 2026-10-04 | 2026-10-04_120009_b970ca36 | `New Item <Anglerfish-` | `fish:shark` | 0.889 | different session |
| 3 | `e0e060607c202030` | 2026-10-04 | 2026-10-04_141428_4c6bc299 | `New Item <Anglerfish-` | `fish:snapper` | 0.870 | different stored label |
| 4 | `e06060203c3a3e3e` | 2026-10-03 | 2026-10-03_110949_15b78581 | `New Item <Anglerfish-` | `fish:squid` | 0.871 | additional coverage |
| 5 | `3f3e3e00085f4b4f` | 2026-10-04 | 2026-10-04_145954_42e87519 | `Ne Item<Anglerfish-` | - | 0.828 | additional coverage |
| 6 | `e0606063643e3e3e` | 2026-10-03 | 2026-10-03_052246_57c522fe | `New Item <Anglerfish-` | `fish:snapper` | 0.818 | additional coverage |


_6 image(s) spanning 5 session(s), 2 day(s), 1 OCR variant(s), 4 stored label(s)_

**Decision:** **PENDING**

---

### Candy Comsquid

| property | value |
|---|---|
| Occurrences | 8 |
| Sessions | 5 |
| Calendar days | 2 |
| Priority | P2 (8 occurrences) |
| Concept id | `concept:candy-comsquid` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| comsquid | candy | family:comsquid |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:squid` | 5 |
| `fish:snapper` | 1 |
| `fish:swordfish` | 1 |


**OCR variants seen:**

- `candy comsquid` ×3
- `candycom squid` ×2
- `candycomsquid` ×3

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `633e3e3e3e01ffff` | 2026-10-04 | 2026-10-04_220833_0f8210ed | `Ne Item<Candy ComSquid>` | `fish:squid` | 0.826 | highest OCR confidence |
| 2 | `e0e06061633f3f3e` | 2026-10-04 | 2026-10-04_141428_4c6bc299 | `Ne Item<CandyCom Squid-` | `fish:swordfish` | 0.821 | different session |
| 3 | `c0606063677e7e7e` | 2026-10-05 | 2026-10-05_201952_14b664e4 | `Nev Item<Candy ComSquid>` | `fish:snapper` | 0.808 | different calendar day |
| 4 | `2020a0e0c0f6fefe` | 2026-10-05 | 2026-10-05_172944_49fd118f | `Nev Item<CandyComSquid>` | `fish:squid` | 0.807 | different OCR variant |
| 5 | `2020e0f0c2fefeff` | 2026-10-05 | 2026-10-05_185058_8e07ab89 | `Nev Item<CandyComSquid>` | - | 0.769 | different stored label |
| 6 | `777e3e3e06013f07` | 2026-10-04 | 2026-10-04_220833_0f8210ed | `Ne Item<Candy ComSquid>` | `fish:squid` | 0.802 | additional coverage |


_6 image(s) spanning 5 session(s), 2 day(s), 3 OCR variant(s), 4 stored label(s)_

**Decision:** **PENDING**

---

### Jack O Bite

| property | value |
|---|---|
| Occurrences | 7 |
| Sessions | 7 |
| Calendar days | 2 |
| Priority | P2 (7 occurrences) |
| Concept id | `concept:jack-o-bite` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| bite | jack, o | family:bite |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:snapper` | 3 |
| `fish:crimson` | 1 |
| `fish:golden` | 1 |
| `fish:pufferfish` | 1 |
| `fish:shark` | 1 |


**OCR variants seen:**

- `jack o bite` ×7

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `30303010bc90ffff` | 2026-10-03 | 2026-10-03_115211_8fa9d196 | `New Item Jack-O'-Bite-` | `fish:snapper` | 0.879 | highest OCR confidence |
| 2 | `e06060203c3f3e3e` | 2026-10-04 | 2026-10-04_120009_b970ca36 | `New Item -Jack-O'-Bite-` | `fish:shark` | 0.871 | different session |
| 3 | `606060603e3e3e00` | 2026-10-03 | 2026-10-03_095208_9aa2f3b7 | `New Item -Jack-o'-Bite-` | `fish:crimson` | 0.870 | different stored label |
| 4 | `e06060603c202030` | 2026-10-03 | 2026-10-03_052246_57c522fe | `New Item -Jack-O'-Bite-` | `fish:snapper` | 0.866 | additional coverage |
| 5 | `406060607c6401ff` | 2026-10-03 | 2026-10-03_084835_be6e8c8b | `New Item -Jack-O'-Bite-` | `fish:golden` | 0.860 | additional coverage |
| 6 | `606060203c3e3210` | 2026-10-04 | 2026-10-04_202745_08b4b13a | `New Item <Jack-O'-Bite-` | `fish:snapper` | 0.842 | additional coverage |


_6 image(s) spanning 6 session(s), 2 day(s), 1 OCR variant(s), 4 stored label(s)_

**Decision:** **PENDING**

---

### Candy Con Squid

| property | value |
|---|---|
| Occurrences | 7 |
| Sessions | 5 |
| Calendar days | 3 |
| Priority | P2 (7 occurrences) |
| Concept id | `concept:candy-con-squid` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| squid | candy, con | family:squid |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:squid` | 6 |


**OCR variants seen:**

- `candy con squid` ×5
- `candy consquid` ×2

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `0000a000befefefe` | 2026-10-04 | 2026-10-04_160014_94aaa3c9 | `New Item <Candy Con Squid>` | `fish:squid` | 0.835 | highest OCR confidence |
| 2 | `e0606060652e3e3e` | 2026-10-05 | 2026-10-05_190403_c9429492 | `Ne Item<Candy ConSquid>` | `fish:squid` | 0.828 | different session |
| 3 | `e1303f33073f1f9f-1` | 2026-10-06 | 2026-10-06_225424_a78e14f2 | `New Item<Candy Con Squid>` | `fish:squid` | 0.814 | different calendar day |
| 4 | `20a0f4f2c2fefeff` | 2026-10-04 | 2026-10-04_145954_42e87519 | `Nev Item<Candy Con Squid` | - | 0.803 | different stored label |
| 5 | `e060606064e0e0fe` | 2026-10-05 | 2026-10-05_190403_c9429492 | `Ne Item<Candy ConSquid>` | `fish:squid` | 0.825 | additional coverage |
| 6 | `36383e9ac2feffff` | 2026-10-04 | 2026-10-04_145954_42e87519 | `Nev Item<Candy Con Squid` | `fish:squid` | 0.808 | additional coverage |


_6 image(s) spanning 4 session(s), 3 day(s), 2 OCR variant(s), 2 stored label(s)_

**Decision:** **PENDING**

---

### Exotictigerfinz

| property | value |
|---|---|
| Occurrences | 7 |
| Sessions | 5 |
| Calendar days | 4 |
| Priority | P2 (7 occurrences) |
| Concept id | `concept:exotictigerfinz` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| exotictigerfinz | (none - single word) | family:exotictigerfinz |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:crimson` | 1 |


**OCR variants seen:**

- `exotictigerfinz` ×7

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `2020e0f0c0fefefe` | 2026-10-05 | 2026-10-05_185058_8e07ab89 | `Nev Item<ExoticTigerfinz` | `fish:crimson` | 0.859 | highest OCR confidence |
| 2 | `3f3e1e020c3f7f7f` | 2026-10-04 | 2026-10-04_145954_42e87519 | `Nev Item<ExoticTigerfinz` | - | 0.850 | different session |
| 3 | `787c7f3f2e3e3e00` | 2026-10-03 | 2026-10-03_095208_9aa2f3b7 | `Nev Item<ExoticTigerfinz` | - | 0.843 | different calendar day |
| 4 | `333f3f3e1e003f47` | 2026-10-05 | 2026-10-05_185058_8e07ab89 | `New Item<ExoticTigerfinz` | - | 0.851 | additional coverage |
| 5 | `0008dee2c27fffff` | 2026-10-04 | 2026-10-04_160014_94aaa3c9 | `New Item<ExoticTigerfinz` | - | 0.835 | additional coverage |
| 6 | `fefefec20002ffff` | 2026-10-04 | 2026-10-04_160014_94aaa3c9 | `New Item <ExoticTigerfinz` | - | 0.822 | additional coverage |


_6 image(s) spanning 4 session(s), 3 day(s), 1 OCR variant(s), 2 stored label(s)_

**Decision:** **PENDING**

---

### Ccrimson Polka Puffer

| property | value |
|---|---|
| Occurrences | 7 |
| Sessions | 4 |
| Calendar days | 3 |
| Priority | P2 (7 occurrences) |
| Concept id | `concept:ccrimson-polka-puffer` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| puffer | ccrimson, polka | family:puffer |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:crimson` | 3 |
| `fish:pufferfish` | 2 |


**OCR variants seen:**

- `ccrimson polka puffer` ×6
- `ccrimson polkapuffer` ×1

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `e0606433273f3f3e` | 2026-10-05 | 2026-10-05_190403_c9429492 | `Nev Item ccrimson PolkaPuffer>` | - | 0.828 | highest OCR confidence |
| 2 | `7f3f3f3e163e003f-1` | 2026-10-06 | 2026-10-06_175729_f925385b | `Nev Item ccrimson Polka Puffer>` | `fish:pufferfish` | 0.816 | different session |
| 3 | `3f3f1e9086bf0140` | 2026-10-03 | 2026-10-03_164415_39a04dc5 | `NewItem cCrimson Polka Puffer>` | `fish:crimson` | 0.815 | different calendar day |
| 4 | `7f3f3f3e063e003f` | 2026-10-06 | 2026-10-06_175729_f925385b | `Nev Item ccrimson Polka Puffer>` | `fish:pufferfish` | 0.814 | additional coverage |
| 5 | `ffbebe90801f0100` | 2026-10-03 | 2026-10-03_164415_39a04dc5 | `NewItem ccrimson Polka Puffer>` | `fish:crimson` | 0.805 | additional coverage |
| 6 | `79273f3e3ebe80ff` | 2026-10-06 | 2026-10-06_175729_f925385b | `Nev Item ccrimson Polka Puffer>` | - | 0.797 | additional coverage |


_6 image(s) spanning 3 session(s), 3 day(s), 2 OCR variant(s), 3 stored label(s)_

**Decision:** **PENDING**

---

### Scandy Corn Squid

| property | value |
|---|---|
| Occurrences | 7 |
| Sessions | 2 |
| Calendar days | 2 |
| Priority | P2 (7 occurrences) |
| Concept id | `concept:scandy-corn-squid` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| squid | scandy, corn | family:squid |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:squid` | 6 |
| `fish:golden` | 1 |


**OCR variants seen:**

- `scandy corn squid` ×7

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `606020203c3e809e` | 2026-10-05 | 2026-10-05_172944_49fd118f | `New Item sCandy Corn Squid>` | `fish:squid` | 0.841 | highest OCR confidence |
| 2 | `3e109f6747424242` | 2026-10-04 | 2026-10-04_160014_94aaa3c9 | `New Item sCandy Corn Squid` | `fish:squid` | 0.786 | different session |
| 3 | `3e109f4747424242` | 2026-10-04 | 2026-10-04_160014_94aaa3c9 | `New Item sCandy Corn Squid>` | `fish:golden` | 0.768 | different stored label |
| 4 | `606030303c3e80de` | 2026-10-05 | 2026-10-05_172944_49fd118f | `New Item sCandy Corn Squid>` | `fish:squid` | 0.841 | additional coverage |
| 5 | `60632f3e363e001f` | 2026-10-05 | 2026-10-05_172944_49fd118f | `Nev ItemsCandy Corn Squid>` | `fish:squid` | 0.781 | additional coverage |
| 6 | `3e109f6f47424242` | 2026-10-04 | 2026-10-04_160014_94aaa3c9 | `Nev Item sCandy Corn Squid>` | `fish:squid` | 0.755 | additional coverage |


_6 image(s) spanning 2 session(s), 2 day(s), 1 OCR variant(s), 2 stored label(s)_

**Decision:** **PENDING**

---

### Sswordfish

| property | value |
|---|---|
| Occurrences | 6 |
| Sessions | 6 |
| Calendar days | 4 |
| Priority | P2 (6 occurrences) |
| Concept id | `concept:sswordfish` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| sswordfish | (none - single word) | family:sswordfish |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:swordfish` | 5 |


**OCR variants seen:**

- `sswordfish` ×6

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `e0e060603e3f3f3e` | 2026-10-04 | 2026-10-04_141428_4c6bc299 | `New Item sSwordfish>` | `fish:swordfish` | 0.853 | highest OCR confidence |
| 2 | `f03020303e323c3e` | 2026-10-05 | 2026-10-05_161430_f023a515 | `New Item sSwordfish-` | `fish:swordfish` | 0.848 | different session |
| 3 | `303030303c9e809f-10` | 2026-10-06 | 2026-10-06_212246_fe47a4fa | `New Item sSwordfish=` | `fish:swordfish` | 0.844 | different calendar day |
| 4 | `e02036233f3f3ebe` | 2026-10-03 | 2026-10-03_223708_0ca663cf | `New Item sSwordfish>` | - | 0.831 | different stored label |
| 5 | `303030303d3c9e9f` | 2026-10-04 | 2026-10-04_070602_b6f12619 | `New Item sSwordfish>` | `fish:swordfish` | 0.846 | additional coverage |
| 6 | `787870203c303e00` | 2026-10-03 | 2026-10-03_095208_9aa2f3b7 | `New Item sSwordfish>` | `fish:swordfish` | 0.831 | additional coverage |


_6 image(s) spanning 6 session(s), 4 day(s), 1 OCR variant(s), 2 stored label(s)_

**Decision:** **PENDING**

---

### Candy Corm Squid

| property | value |
|---|---|
| Occurrences | 6 |
| Sessions | 5 |
| Calendar days | 4 |
| Priority | P2 (6 occurrences) |
| Concept id | `concept:candy-corm-squid` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| squid | candy, corm | family:squid |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:squid` | 4 |


**OCR variants seen:**

- `candy corm squid` ×6

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `303030303c9e909f-2` | 2026-10-06 | 2026-10-06_212246_fe47a4fa | `New Item <Candy Corm Squid>` | `fish:squid` | 0.872 | highest OCR confidence |
| 2 | `e0606060647e7e7e` | 2026-10-05 | 2026-10-05_210511_66017b30 | `New Item<Candy Corm Squid>` | `fish:squid` | 0.807 | different session |
| 3 | `e0203023273f3ebe` | 2026-10-03 | 2026-10-03_223708_0ca663cf | `New Item <Candy Corm Squid>` | - | 0.799 | different calendar day |
| 4 | `607033273f3ebe9e` | 2026-10-02 | 2026-10-02_230043_e51ff1a7 | `New Item<Candy Corm Squid>` | `fish:squid` | 0.782 | additional coverage |
| 5 | `e0603020253e3c3e` | 2026-10-03 | 2026-10-03_223708_0ca663cf | `New Item<Candy Corm Squid>` | `fish:squid` | 0.772 | additional coverage |
| 6 | `607033373f3e3e9e` | 2026-10-02 | 2026-10-02_232819_2c7e6408 | `New Item<Candy Corm Squid-` | - | 0.739 | additional coverage |


_6 image(s) spanning 5 session(s), 4 day(s), 1 OCR variant(s), 2 stored label(s)_

**Decision:** **PENDING**

---

### Exotietigerfin

| property | value |
|---|---|
| Occurrences | 5 |
| Sessions | 4 |
| Calendar days | 3 |
| Priority | P2 (5 occurrences) |
| Concept id | `concept:exotietigerfin` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| exotietigerfin | (none - single word) | family:exotietigerfin |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:squid` | 1 |


**OCR variants seen:**

- `exotie tigerfin` ×2
- `exotietigerfin` ×3

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `e0607863263e3e3e` | 2026-10-03 | 2026-10-03_052246_57c522fe | `Nev Item <ExotieTigerfin>` | - | 0.828 | highest OCR confidence |
| 2 | `606033372e3ebe90` | 2026-10-03 | 2026-10-03_171113_094bcfa0 | `New Item <Exotie Tigerfin>` | - | 0.820 | different session |
| 3 | `e07033372e3e3e3e` | 2026-10-02 | 2026-10-02_232819_2c7e6408 | `New Item <Exotie Tigerfin>` | - | 0.807 | different calendar day |
| 4 | `e0707c33273f3e3e` | 2026-10-04 | 2026-10-04_120009_b970ca36 | `New Item<ExotieTigerfin` | `fish:squid` | 0.794 | different stored label |
| 5 | `e0607b23273e3e3e` | 2026-10-03 | 2026-10-03_052246_57c522fe | `Nev Item <ExotieTigerfin>` | - | 0.755 | additional coverage |


_5 image(s) spanning 4 session(s), 3 day(s), 2 OCR variant(s), 2 stored label(s)_

**Decision:** **PENDING**

---

## P3 — 47 concept(s)

### Neitemskeletalshark

| property | value |
|---|---|
| Occurrences | 3 |
| Sessions | 3 |
| Calendar days | 3 |
| Priority | P3 (singleton: one observation) |
| Concept id | `concept:neitemskeletalshark` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| neitemskeletalshark | (none - single word) | family:neitemskeletalshark |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**OCR variants seen:**

- `neitemskeletalshark` ×3

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `6079333f273e3e00` | 2026-10-03 | 2026-10-03_171113_094bcfa0 | `NeItemskeletalShark` | - | 0.815 | highest OCR confidence |
| 2 | `e060606343fefefe` | 2026-10-05 | 2026-10-05_234514_efb8fcc9 | `NeItemskeletalShark` | - | 0.806 | different session |
| 3 | `e0606433273f3e3e` | 2026-10-04 | 2026-10-04_120009_b970ca36 | `NeItemskeletalShark` | - | 0.796 | different calendar day |


_3 image(s) spanning 3 session(s), 3 day(s), 1 OCR variant(s), 1 stored label(s)_

**Decision:** **PENDING**

---

### Scrimson Polka Puffer

| property | value |
|---|---|
| Occurrences | 3 |
| Sessions | 3 |
| Calendar days | 2 |
| Priority | P3 (singleton: one observation) |
| Concept id | `concept:scrimson-polka-puffer` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| puffer | scrimson, polka | family:puffer |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:crimson` | 2 |
| `fruit:yami` | 1 |


**OCR variants seen:**

- `scrimson polka puffer` ×2
- `scrimson polkapuffer` ×1

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `e06060603e202036` | 2026-10-03 | 2026-10-03_052246_57c522fe | `New Item sCrimson PolkaPuffer>` | `fruit:yami` | 0.863 | highest OCR confidence |
| 2 | `e0f0f0f0fee0feff` | 2026-10-03 | 2026-10-03_131603_72bb9ad3 | `New Item sCrimson Polka Puffer>` | `fish:crimson` | 0.831 | different session |
| 3 | `e0606323363e3e3e` | 2026-10-02 | 2026-10-02_230043_e51ff1a7 | `Nev Item scrimson Polka Puffer>` | `fish:crimson` | 0.807 | different calendar day |


_3 image(s) spanning 3 session(s), 2 day(s), 2 OCR variant(s), 2 stored label(s)_

**Decision:** **PENDING**

---

### Blve Lip Grovper

| property | value |
|---|---|
| Occurrences | 3 |
| Sessions | 2 |
| Calendar days | 2 |
| Priority | P3 (singleton: one observation) |
| Concept id | `concept:blve-lip-grovper` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| grovper | blve, lip | family:grovper |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:golden` | 1 |
| `fruit:light` | 1 |


**OCR variants seen:**

- `blve lip grovper` ×3

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `602020203ebe80ff-4` | 2026-10-06 | 2026-10-06_175729_f925385b | `Ne Item <Blve-Lip Grovper>` | - | 0.914 | highest OCR confidence |
| 2 | `e02020203c3e3ebe` | 2026-10-03 | 2026-10-03_223708_0ca663cf | `New Item <Blve-Lip Grovper>` | `fish:golden` | 0.892 | different session |
| 3 | `e0602020253e3c3e` | 2026-10-03 | 2026-10-03_223708_0ca663cf | `Nev Item <Blve-Lip Grovper>` | `fruit:light` | 0.868 | different stored label |


_3 image(s) spanning 2 session(s), 2 day(s), 1 OCR variant(s), 3 stored label(s)_

**Decision:** **PENDING**

---

### Spooksville Stues

| property | value |
|---|---|
| Occurrences | 3 |
| Sessions | 1 |
| Calendar days | 1 |
| Priority | P3 (singleton: one observation) |
| Concept id | `concept:spooksville-stues` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| stues | spooksville | family:stues |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**OCR variants seen:**

- `spooksville stues` ×3

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `0100000337efffff` | 2026-10-05 | 2026-10-05_183847_8c2f390c | `SPOOKSVILLE STUeS` | - | 0.593 | highest OCR confidence |
| 2 | `01000003ffefffff` | 2026-10-05 | 2026-10-05_183847_8c2f390c | `SPOOKSVILLE STUeS` | - | 0.591 | additional coverage |
| 3 | `010000033fefffff` | 2026-10-05 | 2026-10-05_183847_8c2f390c | `SPOOKSVILLE STUeS` | - | 0.586 | additional coverage |


_3 image(s) spanning 1 session(s), 1 day(s), 1 OCR variant(s), 1 stored label(s)_

**Decision:** **PENDING**

---

### Blve Lip Groper

| property | value |
|---|---|
| Occurrences | 2 |
| Sessions | 2 |
| Calendar days | 2 |
| Priority | P3 (singleton: one observation) |
| Concept id | `concept:blve-lip-groper` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| groper | blve, lip | family:groper |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**OCR variants seen:**

- `blve lip groper` ×1
- `blve lipgroper` ×1

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `71273f3e3e3e003f-1` | 2026-10-06 | 2026-10-06_175729_f925385b | `Nev Item <Blve-Lip Groper>` | - | 0.843 | highest OCR confidence |
| 2 | `e0303833273e3e3e` | 2026-10-03 | 2026-10-03_223708_0ca663cf | `Nev Item<Blve-LipGroper>` | - | 0.834 | different session |


_2 image(s) spanning 2 session(s), 2 day(s), 2 OCR variant(s), 1 stored label(s)_

**Decision:** **PENDING**

---

### Jack 0 Bite

| property | value |
|---|---|
| Occurrences | 2 |
| Sessions | 2 |
| Calendar days | 2 |
| Priority | P3 (singleton: one observation) |
| Concept id | `concept:jack-0-bite` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| bite | jack, 0 | family:bite |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:shark` | 1 |
| `fish:squid` | 1 |


**OCR variants seen:**

- `jack 0 bite` ×2

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `30301090bcd0ffff` | 2026-10-02 | 2026-10-02_215405_f5d40eba | `New Item Jack-0'-Bite-` | `fish:squid` | 0.878 | highest OCR confidence |
| 2 | `30303010bc80dfff` | 2026-10-03 | 2026-10-03_104057_39d5add1 | `New Item -Jack-0'-Bite-` | `fish:shark` | 0.831 | different session |


_2 image(s) spanning 2 session(s), 2 day(s), 1 OCR variant(s), 2 stored label(s)_

**Decision:** **PENDING**

---

### Rimson Snapper

| property | value |
|---|---|
| Occurrences | 2 |
| Sessions | 2 |
| Calendar days | 1 |
| Priority | P3 (singleton: one observation) |
| Concept id | `concept:rimson-snapper` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| snapper | rimson | family:snapper |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:snapper` | 2 |


**OCR variants seen:**

- `rimson snapper` ×1
- `rimsonsnapper` ×1

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `f0303433373f3f1f` | 2026-10-06 | 2026-10-06_225424_a78e14f2 | `Nev Item <rimson Snapper>` | `fish:snapper` | 0.841 | highest OCR confidence |
| 2 | `61673f3a3e3e003f` | 2026-10-06 | 2026-10-06_175729_f925385b | `Nev Item<rimsonSnapper>` | `fish:snapper` | 0.841 | different session |


_2 image(s) spanning 2 session(s), 1 day(s), 2 OCR variant(s), 1 stored label(s)_

**Decision:** **PENDING**

---

### Zzebra Ribbon Angelfish

| property | value |
|---|---|
| Occurrences | 2 |
| Sessions | 2 |
| Calendar days | 2 |
| Priority | P3 (singleton: one observation) |
| Concept id | `concept:zzebra-ribbon-angelfish` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| angelfish | zzebra, ribbon | family:angelfish |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:angelfish` | 2 |


**OCR variants seen:**

- `zzebra ribbon angelfish` ×2

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `303012153f9fdfc1` | 2026-10-04 | 2026-10-04_225639_0180cb9d | `New Item <Zzebra Ribbon Angelfish>` | `fish:angelfish` | 0.847 | highest OCR confidence |
| 2 | `e02036203f3f3ebe` | 2026-10-03 | 2026-10-03_223708_0ca663cf | `New Item <Zzebra Ribbon Angelfish>` | `fish:angelfish` | 0.844 | different session |


_2 image(s) spanning 2 session(s), 2 day(s), 1 OCR variant(s), 1 stored label(s)_

**Decision:** **PENDING**

---

### Exotitigerfin

| property | value |
|---|---|
| Occurrences | 2 |
| Sessions | 1 |
| Calendar days | 1 |
| Priority | P3 (singleton: one observation) |
| Concept id | `concept:exotitigerfin` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| exotitigerfin | (none - single word) | family:exotitigerfin |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**OCR variants seen:**

- `exotitigerfin` ×2

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `6070333f2e3e3e10` | 2026-10-03 | 2026-10-03_171113_094bcfa0 | `New Item<ExotiTigerfin>` | - | 0.810 | highest OCR confidence |
| 2 | `606123272f3e3e00` | 2026-10-03 | 2026-10-03_171113_094bcfa0 | `New Item<ExotiTigerfin>` | - | 0.806 | additional coverage |


_2 image(s) spanning 1 session(s), 1 day(s), 1 OCR variant(s), 1 stored label(s)_

**Decision:** **PENDING**

---

### Neyitemskeletalshark

| property | value |
|---|---|
| Occurrences | 2 |
| Sessions | 1 |
| Calendar days | 1 |
| Priority | P3 (singleton: one observation) |
| Concept id | `concept:neyitemskeletalshark` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| neyitemskeletalshark | (none - single word) | family:neyitemskeletalshark |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**OCR variants seen:**

- `neyitemskeletalshark` ×2

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `7b3f3f3e2e3e003f` | 2026-10-06 | 2026-10-06_175729_f925385b | `NeyItemskeletalShark` | - | 0.815 | highest OCR confidence |
| 2 | `7b373f3e2e3e003f-1` | 2026-10-06 | 2026-10-06_175729_f925385b | `NeyItemskeletalShark` | - | 0.813 | additional coverage |


_2 image(s) spanning 1 session(s), 1 day(s), 1 OCR variant(s), 1 stored label(s)_

**Decision:** **PENDING**

---

### Rimson Polka Puffer

| property | value |
|---|---|
| Occurrences | 2 |
| Sessions | 1 |
| Calendar days | 1 |
| Priority | P3 (singleton: one observation) |
| Concept id | `concept:rimson-polka-puffer` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| puffer | rimson, polka | family:puffer |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:crimson` | 2 |


**OCR variants seen:**

- `rimson polka puffer` ×2

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `3f3f1e9684bfc3c1` | 2026-10-03 | 2026-10-03_164415_39a04dc5 | `Nev Item <rimson Polka Puffer>` | `fish:crimson` | 0.822 | highest OCR confidence |
| 2 | `3f3f1e8284bfcbcb` | 2026-10-03 | 2026-10-03_164415_39a04dc5 | `Nev Item <rimson Polka Puffer>` | `fish:crimson` | 0.821 | additional coverage |


_2 image(s) spanning 1 session(s), 1 day(s), 1 OCR variant(s), 1 stored label(s)_

**Decision:** **PENDING**

---

### Town Of Beginnings Safezone Protected

| property | value |
|---|---|
| Occurrences | 2 |
| Sessions | 1 |
| Calendar days | 1 |
| Priority | P3 (singleton: one observation) |
| Concept id | `concept:town-of-beginnings-safezone-protected` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| protected | town, of, beginnings, safezone | family:protected |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**OCR variants seen:**

- `town of beginnings safezone protected` ×2

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `2878ffffffffe700` | 2026-10-05 | 2026-10-05_184251_41c96424 | `TOWN OF BEGINNINGS SAFEZONE PROTECTED` | - | 0.855 | highest OCR confidence |
| 2 | `0030ffffffffe700` | 2026-10-05 | 2026-10-05_184251_41c96424 | `TOWN OF BEGINNINGS SAFEZONE PROTECTED` | - | 0.854 | additional coverage |


_2 image(s) spanning 1 session(s), 1 day(s), 1 OCR variant(s), 1 stored label(s)_

**Decision:** **PENDING**

---

### Agomu Has Spavned At Tradinghub

| property | value |
|---|---|
| Occurrences | 1 |
| Sessions | 1 |
| Calendar days | 1 |
| Priority | P3 (singleton: one observation) |
| Concept id | `concept:agomu-has-spavned-at-tradinghub` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| tradinghub | agomu, has, spavned, at | family:tradinghub |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fruit:gomu` | 1 |


**OCR variants seen:**

- `agomu has spavned at tradinghub` ×1

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `3e3e9e94889f8fc0` | 2026-10-03 | 2026-10-03_132921_cfb209d0 | `AGOMU has Spavned at TRADINGHUB` | `fruit:gomu` | 0.863 | highest OCR confidence |


_1 image(s) spanning 1 session(s), 1 day(s), 1 OCR variant(s), 1 stored label(s)_

**Decision:** **PENDING**

---

### Andy Con Squid

| property | value |
|---|---|
| Occurrences | 1 |
| Sessions | 1 |
| Calendar days | 1 |
| Priority | P3 (singleton: one observation) |
| Concept id | `concept:andy-con-squid` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| squid | andy, con | family:squid |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:squid` | 1 |


**OCR variants seen:**

- `andy con squid` ×1

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `303010d1c0ffffff` | 2026-10-04 | 2026-10-04_225639_0180cb9d | `New Item<andy Con Squid>` | `fish:squid` | 0.823 | highest OCR confidence |


_1 image(s) spanning 1 session(s), 1 day(s), 1 OCR variant(s), 1 stored label(s)_

**Decision:** **PENDING**

---

### Andycorn Squid

| property | value |
|---|---|
| Occurrences | 1 |
| Sessions | 1 |
| Calendar days | 1 |
| Priority | P3 (singleton: one observation) |
| Concept id | `concept:andycorn-squid` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| squid | andycorn | family:squid |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:squid` | 1 |


**OCR variants seen:**

- `andycorn squid` ×1

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `e0f0e202020fffff` | 2026-10-04 | 2026-10-04_160014_94aaa3c9 | `Nev Item<andyCorn Squid` | `fish:squid` | 0.821 | highest OCR confidence |


_1 image(s) spanning 1 session(s), 1 day(s), 1 OCR variant(s), 1 stored label(s)_

**Decision:** **PENDING**

---

### Crimson Polka Pyffer

| property | value |
|---|---|
| Occurrences | 1 |
| Sessions | 1 |
| Calendar days | 1 |
| Priority | P3 (singleton: one observation) |
| Concept id | `concept:crimson-polka-pyffer` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| pyffer | crimson, polka | family:pyffer |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:pufferfish` | 1 |


**OCR variants seen:**

- `crimson polka pyffer` ×1

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `ff7f7e7800302020` | 2026-10-05 | 2026-10-05_234514_efb8fcc9 | `Ne Item <crimson Polka Pyffer?` | `fish:pufferfish` | 0.823 | highest OCR confidence |


_1 image(s) spanning 1 session(s), 1 day(s), 1 OCR variant(s), 1 stored label(s)_

**Decision:** **PENDING**

---

### Czebra Ribbon Angelfish

| property | value |
|---|---|
| Occurrences | 1 |
| Sessions | 1 |
| Calendar days | 1 |
| Priority | P3 (singleton: one observation) |
| Concept id | `concept:czebra-ribbon-angelfish` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| angelfish | czebra, ribbon | family:angelfish |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:angelfish` | 1 |


**OCR variants seen:**

- `czebra ribbon angelfish` ×1

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `3f1e1e9e801f0700` | 2026-10-03 | 2026-10-03_131603_72bb9ad3 | `New Item czebra Ribbon Angelfish` | `fish:angelfish` | 0.860 | highest OCR confidence |


_1 image(s) spanning 1 session(s), 1 day(s), 1 OCR variant(s), 1 stored label(s)_

**Decision:** **PENDING**

---

### Darkskeletalsharkmaskblveprint

| property | value |
|---|---|
| Occurrences | 1 |
| Sessions | 1 |
| Calendar days | 1 |
| Priority | P3 (singleton: one observation) |
| Concept id | `concept:darkskeletalsharkmaskblveprint` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| darkskeletalsharkmaskblveprint | (none - single word) | family:darkskeletalsharkmaskblveprint |


**Current KB candidates:**

- `fish:shark`
- `fruit:dark`

_AMBIGUOUS: contains more than one canonical entity, so today's KB has no single answer_

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:shark` | 1 |


_1 of 1 rows have a stored label equal to one KB candidate. That means the label agrees with one COMPONENT of the name — it does NOT mean the name is confirmed._

**OCR variants seen:**

- `darkskeletalsharkmaskblveprint` ×1

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `e0606061237e3e3e` | 2026-10-03 | 2026-10-03_050857_436d1ee0 | `NevItemDarkSkeletalSharkMaskBlveprint` | `fish:shark` | 0.894 | highest OCR confidence |


_1 image(s) spanning 1 session(s), 1 day(s), 1 OCR variant(s), 1 stored label(s)_

**Decision:** **PENDING**

---

### Darksketetalshark

| property | value |
|---|---|
| Occurrences | 1 |
| Sessions | 1 |
| Calendar days | 1 |
| Priority | P3 (singleton: one observation) |
| Concept id | `concept:darksketetalshark` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| darksketetalshark | (none - single word) | family:darksketetalshark |


**Current KB candidates:**

- `fish:shark`
- `fruit:dark`

_AMBIGUOUS: contains more than one canonical entity, so today's KB has no single answer_

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:shark` | 1 |


_1 of 1 rows have a stored label equal to one KB candidate. That means the label agrees with one COMPONENT of the name — it does NOT mean the name is confirmed._

**OCR variants seen:**

- `darksketetalshark` ×1

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `797e7e7c443c013f` | 2026-10-03 | 2026-10-03_084835_be6e8c8b | `New Item<Darksketetalshark-` | `fish:shark` | 0.788 | highest OCR confidence |


_1 image(s) spanning 1 session(s), 1 day(s), 1 OCR variant(s), 1 stored label(s)_

**Decision:** **PENDING**

---

### Droppedbomb Vill Despavnin10 Minutes

| property | value |
|---|---|
| Occurrences | 1 |
| Sessions | 1 |
| Calendar days | 1 |
| Priority | P3 (singleton: one observation) |
| Concept id | `concept:droppedbomb-vill-despavnin10-minutes` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| minutes | droppedbomb, vill, despavnin10 | family:minutes |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**OCR variants seen:**

- `droppedbomb vill despavnin10 minutes` ×1

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `c5f370141c1ddddf` | 2026-10-05 | 2026-10-05_161430_f023a515 | `DroppedBomb vill despavnin10 minutes.` | - | 0.877 | highest OCR confidence |


_1 image(s) spanning 1 session(s), 1 day(s), 1 OCR variant(s), 1 stored label(s)_

**Decision:** **PENDING**

---

### Exotietigerfinz

| property | value |
|---|---|
| Occurrences | 1 |
| Sessions | 1 |
| Calendar days | 1 |
| Priority | P3 (singleton: one observation) |
| Concept id | `concept:exotietigerfinz` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| exotietigerfinz | (none - single word) | family:exotietigerfinz |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**OCR variants seen:**

- `exotietigerfinz` ×1

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `6079673f263e3e00` | 2026-10-03 | 2026-10-03_095208_9aa2f3b7 | `Nev Item<ExotieTigerfinz` | - | 0.825 | highest OCR confidence |


_1 image(s) spanning 1 session(s), 1 day(s), 1 OCR variant(s), 1 stored label(s)_

**Decision:** **PENDING**

---

### Exotistigerfin

| property | value |
|---|---|
| Occurrences | 1 |
| Sessions | 1 |
| Calendar days | 1 |
| Priority | P3 (singleton: one observation) |
| Concept id | `concept:exotistigerfin` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| exotistigerfin | (none - single word) | family:exotistigerfin |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**OCR variants seen:**

- `exotistigerfin` ×1

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `a0e0f672427e7f7f` | 2026-10-04 | 2026-10-04_145954_42e87519 | `Nev Item<ExotisTigerfin` | - | 0.801 | highest OCR confidence |


_1 image(s) spanning 1 session(s), 1 day(s), 1 OCR variant(s), 1 stored label(s)_

**Decision:** **PENDING**

---

### Golden Polko Puffer

| property | value |
|---|---|
| Occurrences | 1 |
| Sessions | 1 |
| Calendar days | 1 |
| Priority | P3 (singleton: one observation) |
| Concept id | `concept:golden-polko-puffer` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| puffer | golden, polko | family:puffer |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:pufferfish` | 1 |


**OCR variants seen:**

- `golden polko puffer` ×1

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `f0f070783ca0e0e0` | 2026-10-04 | 2026-10-04_141428_4c6bc299 | `New Item Golden Polko Puffer?` | `fish:pufferfish` | 0.886 | highest OCR confidence |


_1 image(s) spanning 1 session(s), 1 day(s), 1 OCR variant(s), 1 stored label(s)_

**Decision:** **PENDING**

---

### Golden Tigerfinz Newitem

| property | value |
|---|---|
| Occurrences | 1 |
| Sessions | 1 |
| Calendar days | 1 |
| Priority | P3 (singleton: one observation) |
| Concept id | `concept:golden-tigerfinz-newitem` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| newitem | golden, tigerfinz | family:newitem |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**OCR variants seen:**

- `golden tigerfinz newitem` ×1

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `3f3e3e128c3f0141` | 2026-10-03 | 2026-10-03_161417_bb4b5817 | `<Golden Tigerfinz NewItem` | - | 0.744 | highest OCR confidence |


_1 image(s) spanning 1 session(s), 1 day(s), 1 OCR variant(s), 1 stored label(s)_

**Decision:** **PENDING**

---

### Goldentigerfinz

| property | value |
|---|---|
| Occurrences | 1 |
| Sessions | 1 |
| Calendar days | 1 |
| Priority | P3 (singleton: one observation) |
| Concept id | `concept:goldentigerfinz` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| goldentigerfinz | (none - single word) | family:goldentigerfinz |


**Current KB candidates:**

- `fish:golden`
- `fish:tiger`

_AMBIGUOUS: contains more than one canonical entity, so today's KB has no single answer_

**OCR variants seen:**

- `goldentigerfinz` ×1

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `f0303033273f3f1f-3` | 2026-10-06 | 2026-10-06_225424_a78e14f2 | `New Item <GoldenTigerfinz` | - | 0.801 | highest OCR confidence |


_1 image(s) spanning 1 session(s), 1 day(s), 1 OCR variant(s), 1 stored label(s)_

**Decision:** **PENDING**

---

### Keletal Shark

| property | value |
|---|---|
| Occurrences | 1 |
| Sessions | 1 |
| Calendar days | 1 |
| Priority | P3 (singleton: one observation) |
| Concept id | `concept:keletal-shark` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| shark | keletal | family:shark |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:shark` | 1 |


**OCR variants seen:**

- `keletal shark` ×1

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `e06060607c202036` | 2026-10-03 | 2026-10-03_052246_57c522fe | `New Item keletal Shark>` | `fish:shark` | 0.866 | highest OCR confidence |


_1 image(s) spanning 1 session(s), 1 day(s), 1 OCR variant(s), 1 stored label(s)_

**Decision:** **PENDING**

---

### Neitem Fangfish

| property | value |
|---|---|
| Occurrences | 1 |
| Sessions | 1 |
| Calendar days | 1 |
| Priority | P3 (singleton: one observation) |
| Concept id | `concept:neitem-fangfish` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| fangfish | neitem | family:fangfish |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**OCR variants seen:**

- `neitem fangfish` ×1

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `607033372f3e3e16` | 2026-10-02 | 2026-10-02_232819_2c7e6408 | `NeItem<fangfish` | - | 0.853 | highest OCR confidence |


_1 image(s) spanning 1 session(s), 1 day(s), 1 OCR variant(s), 1 stored label(s)_

**Decision:** **PENDING**

---

### Nevltem Exotictigerfin

| property | value |
|---|---|
| Occurrences | 1 |
| Sessions | 1 |
| Calendar days | 1 |
| Priority | P3 (singleton: one observation) |
| Concept id | `concept:nevltem-exotictigerfin` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| exotictigerfin | nevltem | family:exotictigerfin |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**OCR variants seen:**

- `nevltem exotictigerfin` ×1

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `ffffffbe823e1c1e` | 2026-10-06 | 2026-10-06_175729_f925385b | `Nevltem<ExoticTigerfin>` | - | 0.785 | highest OCR confidence |


_1 image(s) spanning 1 session(s), 1 day(s), 1 OCR variant(s), 1 stored label(s)_

**Decision:** **PENDING**

---

### Neyitem Crimsonpolkapuffer

| property | value |
|---|---|
| Occurrences | 1 |
| Sessions | 1 |
| Calendar days | 1 |
| Priority | P3 (singleton: one observation) |
| Concept id | `concept:neyitem-crimsonpolkapuffer` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| crimsonpolkapuffer | neyitem | family:crimsonpolkapuffer |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:crimson` | 1 |


**OCR variants seen:**

- `neyitem crimsonpolkapuffer` ×1

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `fcfc7c7c407c2424` | 2026-10-05 | 2026-10-05_234514_efb8fcc9 | `NeyItem <crimsonPolkaPuffer?` | `fish:crimson` | 0.812 | highest OCR confidence |


_1 image(s) spanning 1 session(s), 1 day(s), 1 OCR variant(s), 1 stored label(s)_

**Decision:** **PENDING**

---

### Neylitem Crimson Snapper

| property | value |
|---|---|
| Occurrences | 1 |
| Sessions | 1 |
| Calendar days | 1 |
| Priority | P3 (singleton: one observation) |
| Concept id | `concept:neylitem-crimson-snapper` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| snapper | neylitem, crimson | family:snapper |


**Current KB candidates:**

- `fish:crimson`
- `fish:snapper`

_AMBIGUOUS: contains more than one canonical entity, so today's KB has no single answer_

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:snapper` | 1 |


_1 of 1 rows have a stored label equal to one KB candidate. That means the label agrees with one COMPONENT of the name — it does NOT mean the name is confirmed._

**OCR variants seen:**

- `neylitem crimson snapper` ×1

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `7f7ffff8c0fcf8c0` | 2026-10-03 | 2026-10-03_164415_39a04dc5 | `NeylItem<crimson Snapper>` | `fish:snapper` | 0.789 | highest OCR confidence |


_1 image(s) spanning 1 session(s), 1 day(s), 1 OCR variant(s), 1 stored label(s)_

**Decision:** **PENDING**

---

### Nw Ne

| property | value |
|---|---|
| Occurrences | 1 |
| Sessions | 1 |
| Calendar days | 1 |
| Priority | P3 (singleton: one observation) |
| Concept id | `concept:nw-ne` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| ne | nw | family:ne |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**OCR variants seen:**

- `nw ne` ×1

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `c0f0f030141c19df` | 2026-10-05 | 2026-10-05_190403_c9429492 | `NW NE` | - | 0.567 | highest OCR confidence |


_1 image(s) spanning 1 session(s), 1 day(s), 1 OCR variant(s), 1 stored label(s)_

**Decision:** **PENDING**

---

### Sanglerfish

| property | value |
|---|---|
| Occurrences | 1 |
| Sessions | 1 |
| Calendar days | 1 |
| Priority | P3 (singleton: one observation) |
| Concept id | `concept:sanglerfish` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| sanglerfish | (none - single word) | family:sanglerfish |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:angelfish` | 1 |


**OCR variants seen:**

- `sanglerfish` ×1

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `797e7e7c6c3c017f` | 2026-10-03 | 2026-10-03_084835_be6e8c8b | `New Item SAnglerfish` | `fish:angelfish` | 0.802 | highest OCR confidence |


_1 image(s) spanning 1 session(s), 1 day(s), 1 OCR variant(s), 1 stored label(s)_

**Decision:** **PENDING**

---

### Scandyconsquid

| property | value |
|---|---|
| Occurrences | 1 |
| Sessions | 1 |
| Calendar days | 1 |
| Priority | P3 (singleton: one observation) |
| Concept id | `concept:scandyconsquid` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| scandyconsquid | (none - single word) | family:scandyconsquid |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:squid` | 1 |


**OCR variants seen:**

- `scandyconsquid` ×1

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `e0e06021273f3e3e` | 2026-10-04 | 2026-10-04_120009_b970ca36 | `New ItemsCandyConSquid>` | `fish:squid` | 0.801 | highest OCR confidence |


_1 image(s) spanning 1 session(s), 1 day(s), 1 OCR variant(s), 1 stored label(s)_

**Decision:** **PENDING**

---

### Sdarksketetalshark

| property | value |
|---|---|
| Occurrences | 1 |
| Sessions | 1 |
| Calendar days | 1 |
| Priority | P3 (singleton: one observation) |
| Concept id | `concept:sdarksketetalshark` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| sdarksketetalshark | (none - single word) | family:sdarksketetalshark |


**Current KB candidates:**

- `fish:shark`
- `fruit:dark`

_AMBIGUOUS: contains more than one canonical entity, so today's KB has no single answer_

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fruit:yami` | 1 |


**OCR variants seen:**

- `sdarksketetalshark` ×1

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `602828b8c8fefefe` | 2026-10-04 | 2026-10-04_202745_08b4b13a | `New Item sDarkSketetalshark?` | `fruit:yami` | 0.809 | highest OCR confidence |


_1 image(s) spanning 1 session(s), 1 day(s), 1 OCR variant(s), 1 stored label(s)_

**Decision:** **PENDING**

---

### Sgolden Ribbonangetfish

| property | value |
|---|---|
| Occurrences | 1 |
| Sessions | 1 |
| Calendar days | 1 |
| Priority | P3 (singleton: one observation) |
| Concept id | `concept:sgolden-ribbonangetfish` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| ribbonangetfish | sgolden | family:ribbonangetfish |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:angelfish` | 1 |


**OCR variants seen:**

- `sgolden ribbonangetfish` ×1

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `60e0e0e0e0f64000` | 2026-10-04 | 2026-10-04_202745_08b4b13a | `New Item sGolden RibbonAngetfish?` | `fish:angelfish` | 0.852 | highest OCR confidence |


_1 image(s) spanning 1 session(s), 1 day(s), 1 OCR variant(s), 1 stored label(s)_

**Decision:** **PENDING**

---

### Spooksville Astuo Candy Cornsquid

| property | value |
|---|---|
| Occurrences | 1 |
| Sessions | 1 |
| Calendar days | 1 |
| Priority | P3 (singleton: one observation) |
| Concept id | `concept:spooksville-astuo-candy-cornsquid` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| cornsquid | spooksville, astuo, candy | family:cornsquid |


**Current KB candidates:**

- `fish:squid`
- `fruit:uo`

_AMBIGUOUS: contains more than one canonical entity, so today's KB has no single answer_

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:squid` | 1 |


_1 of 1 rows have a stored label equal to one KB candidate. That means the label agrees with one COMPONENT of the name — it does NOT mean the name is confirmed._

**OCR variants seen:**

- `spooksville astuo candy cornsquid` ×1

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `00000003437fffff` | 2026-10-05 | 2026-10-05_183558_be8759b3 | `SPOOKSVILLE ASTUO Nev Item<Candy CornSqu` | `fish:squid` | 0.541 | highest OCR confidence |


_1 image(s) spanning 1 session(s), 1 day(s), 1 OCR variant(s), 1 stored label(s)_

**Decision:** **PENDING**

---

### Spooksville Astuo Crimson Snapper

| property | value |
|---|---|
| Occurrences | 1 |
| Sessions | 1 |
| Calendar days | 1 |
| Priority | P3 (singleton: one observation) |
| Concept id | `concept:spooksville-astuo-crimson-snapper` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| snapper | spooksville, astuo, crimson | family:snapper |


**Current KB candidates:**

- `fish:crimson`
- `fish:snapper`

_AMBIGUOUS: contains more than one canonical entity, so today's KB has no single answer_

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:crimson` | 1 |


_1 of 1 rows have a stored label equal to one KB candidate. That means the label agrees with one COMPONENT of the name — it does NOT mean the name is confirmed._

**OCR variants seen:**

- `spooksville astuo crimson snapper` ×1

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `00000003c37fffff` | 2026-10-05 | 2026-10-05_183558_be8759b3 | `SPOOKSVILLE aSTUO Nev Item<crimson Snapp` | `fish:crimson` | 0.532 | highest OCR confidence |


_1 image(s) spanning 1 session(s), 1 day(s), 1 OCR variant(s), 1 stored label(s)_

**Decision:** **PENDING**

---

### Spooksville Astuo Zebra Ribbon Angelfish

| property | value |
|---|---|
| Occurrences | 1 |
| Sessions | 1 |
| Calendar days | 1 |
| Priority | P3 (singleton: one observation) |
| Concept id | `concept:spooksville-astuo-zebra-ribbon-angelfish` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| angelfish | spooksville, astuo, zebra, ribbon | family:angelfish |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**OCR variants seen:**

- `spooksville astuo zebra ribbon angelfish` ×1

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `00008081c3ffffff` | 2026-10-05 | 2026-10-05_183558_be8759b3 | `SPOOKSVILLE ASTUO Nev Item <zebra Ribbon` | - | 0.548 | highest OCR confidence |


_1 image(s) spanning 1 session(s), 1 day(s), 1 OCR variant(s), 1 stored label(s)_

**Decision:** **PENDING**

---

### Spooksville Stucs

| property | value |
|---|---|
| Occurrences | 1 |
| Sessions | 1 |
| Calendar days | 1 |
| Priority | P3 (singleton: one observation) |
| Concept id | `concept:spooksville-stucs` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| stucs | spooksville | family:stucs |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**OCR variants seen:**

- `spooksville stucs` ×1

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `000000037f7fffff` | 2026-10-05 | 2026-10-05_183558_be8759b3 | `SPOOKSVILLE STUCS` | - | 0.641 | highest OCR confidence |


_1 image(s) spanning 1 session(s), 1 day(s), 1 OCR variant(s), 1 stored label(s)_

**Decision:** **PENDING**

---

### Spooksville Stucs Nevitemskeletalshark

| property | value |
|---|---|
| Occurrences | 1 |
| Sessions | 1 |
| Calendar days | 1 |
| Priority | P3 (singleton: one observation) |
| Concept id | `concept:spooksville-stucs-nevitemskeletalshark` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| nevitemskeletalshark | spooksville, stucs | family:nevitemskeletalshark |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**OCR variants seen:**

- `spooksville stucs nevitemskeletalshark` ×1

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `01000003437fffff` | 2026-10-05 | 2026-10-05_183558_be8759b3 | `SPOOKSVILLE STUCS NevItemskeletalshark` | - | 0.622 | highest OCR confidence |


_1 image(s) spanning 1 session(s), 1 day(s), 1 OCR variant(s), 1 stored label(s)_

**Decision:** **PENDING**

---

### Spooksville Stues Skeletalshark

| property | value |
|---|---|
| Occurrences | 1 |
| Sessions | 1 |
| Calendar days | 1 |
| Priority | P3 (singleton: one observation) |
| Concept id | `concept:spooksville-stues-skeletalshark` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| skeletalshark | spooksville, stues | family:skeletalshark |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:shark` | 1 |


**OCR variants seen:**

- `spooksville stues skeletalshark` ×1

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `01000003477fffff` | 2026-10-05 | 2026-10-05_183847_8c2f390c | `SPOOKSVILLE STUeS New Item <skeletalshar` | `fish:shark` | 0.564 | highest OCR confidence |


_1 image(s) spanning 1 session(s), 1 day(s), 1 OCR variant(s), 1 stored label(s)_

**Decision:** **PENDING**

---

### Spooksville Stues Zebra Ribbon Angelfish

| property | value |
|---|---|
| Occurrences | 1 |
| Sessions | 1 |
| Calendar days | 1 |
| Priority | P3 (singleton: one observation) |
| Concept id | `concept:spooksville-stues-zebra-ribbon-angelfish` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| angelfish | spooksville, stues, zebra, ribbon | family:angelfish |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**OCR variants seen:**

- `spooksville stues zebra ribbon angelfish` ×1

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `01000003077fffff` | 2026-10-05 | 2026-10-05_183847_8c2f390c | `SPOOKSVILLE STUeS Nev Item <Zebra Ribbon` | - | 0.565 | highest OCR confidence |


_1 image(s) spanning 1 session(s), 1 day(s), 1 OCR variant(s), 1 stored label(s)_

**Decision:** **PENDING**

---

### Ssvordfish

| property | value |
|---|---|
| Occurrences | 1 |
| Sessions | 1 |
| Calendar days | 1 |
| Priority | P3 (singleton: one observation) |
| Concept id | `concept:ssvordfish` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| ssvordfish | (none - single word) | family:ssvordfish |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:swordfish` | 1 |


**OCR variants seen:**

- `ssvordfish` ×1

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `3e3e017f47830080` | 2026-10-02 | 2026-10-02_221654_5ef6f5f9 | `New ItemsSvordfish` | `fish:swordfish` | 0.800 | highest OCR confidence |


_1 image(s) spanning 1 session(s), 1 day(s), 1 OCR variant(s), 1 stored label(s)_

**Decision:** **PENDING**

---

### Stigerfin

| property | value |
|---|---|
| Occurrences | 1 |
| Sessions | 1 |
| Calendar days | 1 |
| Priority | P3 (singleton: one observation) |
| Concept id | `concept:stigerfin` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| stigerfin | (none - single word) | family:stigerfin |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**OCR variants seen:**

- `stigerfin` ×1

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `e0607c3f2f3f3e3e` | 2026-10-04 | 2026-10-04_120009_b970ca36 | `Nev Item sTigerfin>` | - | 0.815 | highest OCR confidence |


_1 image(s) spanning 1 session(s), 1 day(s), 1 OCR variant(s), 1 stored label(s)_

**Decision:** **PENDING**

---

### Svordfish

| property | value |
|---|---|
| Occurrences | 1 |
| Sessions | 1 |
| Calendar days | 1 |
| Priority | P3 (singleton: one observation) |
| Concept id | `concept:svordfish` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| svordfish | (none - single word) | family:svordfish |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:swordfish` | 1 |


**OCR variants seen:**

- `svordfish` ×1

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `3f3f3e160e3f0101` | 2026-10-03 | 2026-10-03_164415_39a04dc5 | `New ItemSvordfish` | `fish:swordfish` | 0.799 | highest OCR confidence |


_1 image(s) spanning 1 session(s), 1 day(s), 1 OCR variant(s), 1 stored label(s)_

**Decision:** **PENDING**

---

### Szebra Ribbonangelfish

| property | value |
|---|---|
| Occurrences | 1 |
| Sessions | 1 |
| Calendar days | 1 |
| Priority | P3 (singleton: one observation) |
| Concept id | `concept:szebra-ribbonangelfish` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| ribbonangelfish | szebra | family:ribbonangelfish |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:angelfish` | 1 |


**OCR variants seen:**

- `szebra ribbonangelfish` ×1

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `607c3737273e3e1e` | 2026-10-02 | 2026-10-02_232819_2c7e6408 | `New Item szebra RibbonAngelfish>` | `fish:angelfish` | 0.797 | highest OCR confidence |


_1 image(s) spanning 1 session(s), 1 day(s), 1 OCR variant(s), 1 stored label(s)_

**Decision:** **PENDING**

---

### Zebra Ribbon Angelfisha

| property | value |
|---|---|
| Occurrences | 1 |
| Sessions | 1 |
| Calendar days | 1 |
| Priority | P3 (singleton: one observation) |
| Concept id | `concept:zebra-ribbon-angelfisha` |


**Decomposition (linguistic evidence only, not an ontology claim):**

| base name | modifiers | family |
|---|---|---|
| angelfisha | zebra, ribbon | family:angelfisha |


**Current KB candidates:** none — the knowledge base has no entity for this name.

**Stored labels (evidence, not truth):**

| label | rows |
|---|---|
| `fish:angelfish` | 1 |


**OCR variants seen:**

- `zebra ribbon angelfisha` ×1

**Representative evidence:**

| # | image id | day | session | OCR text | stored label | conf | chosen because |
|---|---|---|---|---|---|---|---|
| 1 | `3f1f1e9e809ebfc3` | 2026-10-02 | 2026-10-02_215405_f5d40eba | `Nev Item zebra Ribbon Angelfisha` | `fish:angelfish` | 0.838 | highest OCR confidence |


_1 image(s) spanning 1 session(s), 1 day(s), 1 OCR variant(s), 1 stored label(s)_

**Decision:** **PENDING**

---
