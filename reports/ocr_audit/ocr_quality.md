# OCR Audit — OCR quality

## Confidence distribution

| statistic | value |
|---|---|
| n | 12235 |
| min | 0.5 |
| p10 | 0.5227 |
| median | 0.6536 |
| p90 | 0.8517 |
| max | 0.9341 |
| mean | 0.6786 |


## Correction rules that fired

Each rule exists to fix a NAMED failure mode and is covered by a unit test.
No rule was derived from frequency alone.

| rule | times applied |
|---|---|
| `banner_ne` | 896 |
| `sordfish` | 96 |
| `rn_as_m` | 14 |
| `item` | 1 |


## Most common single-character disagreements

Observed between the collector's stored text and this audit's fresh read, on
rows where both are non-empty. These are OBSERVATIONS for a human to judge.
This pipeline adds no rule from this table automatically.

| stored char | fresh char | count |
|---|---|---|
| `v` | `w` | 2180 |
| `n` | ` ` | 942 |
| `e` | `r` | 811 |
| `r` | `i` | 801 |
| `i` | `n` | 746 |
| `t` | `e` | 676 |
| ` ` | `r` | 667 |
| `t` | `i` | 662 |
| ` ` | `e` | 661 |
| `o` | `n` | 650 |
| ` ` | `p` | 626 |
| `e` | ` ` | 616 |
| `a` | ` ` | 600 |
| `f` | `i` | 569 |
| `o` | ` ` | 541 |
| `n` | `g` | 492 |
| `m` | ` ` | 478 |
| `e` | `i` | 461 |
| ` ` | `i` | 455 |
| `c` | `r` | 453 |
| `<` | `c` | 452 |
| `a` | `n` | 451 |
| ` ` | `o` | 444 |
| ` ` | `t` | 440 |
| ` ` | `a` | 429 |
| `i` | `m` | 421 |
| `m` | `s` | 402 |
| `o` | `l` | 402 |
| `s` | `o` | 394 |
| `i` | ` ` | 381 |


## Rejected low-confidence detections

0 detection box(es) fell below the confidence floor and
were excluded from the reading. They are still recorded in the cache so the
suppression is auditable rather than silent.
