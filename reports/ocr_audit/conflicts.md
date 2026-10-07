# OCR Audit — Conflicts and unresolved rows

Rows where the freshly-read text does not agree with the stored label, or
does not resolve to any single entity. Grouped by concept and ordered by how
many rows share each interpretation.

**3291 of 14805 rows (22.2%) are in conflict or unresolved.**

No stored label was modified. Every row below still carries its original
`entity_id` in `evidence.jsonl`.

| rows | stored label | OCR interpretation | audit status | candidates | sessions |
|---|---|---|---|---|---|
| 265 | `(none)` | `fangfish` | UNKNOWN_ENTITY | — | 41 |
| 200 | `(none)` | `exotic tigerfin` | UNKNOWN_ENTITY | — | 34 |
| 181 | `fish:snapper` | `crimson snapper` | COMPOUND_AMBIGUOUS | `fish:crimson`, `fish:snapper` | 37 |
| 147 | `fish:angelfish` | `zebra ribbon angelfish` | UNKNOWN_ENTITY | — | 42 |
| 143 | `fish:squid` | `candy corn squid` | UNKNOWN_ENTITY | — | 38 |
| 143 | `fish:crimson` | `crimson polka puffer` | UNKNOWN_ENTITY | — | 34 |
| 96 | `(none)` | `jack o bite` | UNKNOWN_ENTITY | — | 31 |
| 93 | `(none)` | `anglerfish` | UNKNOWN_ENTITY | — | 34 |
| 78 | `fish:shark` | `skeletal shark` | UNKNOWN_ENTITY | — | 25 |
| 71 | `fish:golden` | `golden ribbon angelfish` | COMPOUND_AMBIGUOUS | `fish:angelfish`, `fish:golden` | 32 |
| 61 | `fish:golden` | `golden polka puffer` | UNKNOWN_ENTITY | — | 22 |
| 61 | `fish:golden` | `golden tigerfin` | UNKNOWN_ENTITY | — | 24 |
| 54 | `fish:shark` | `skeletalshark` | UNKNOWN_ENTITY | — | 21 |
| 42 | `fish:pufferfish` | `crimson polka puffer` | UNKNOWN_ENTITY | — | 17 |
| 41 | `(none)` | `tigerfin` | UNKNOWN_ENTITY | — | 5 |
| 40 | `(none)` | `zebra ribbon angelfish` | UNKNOWN_ENTITY | — | 20 |
| 40 | `(none)` | `skeletalshark` | UNKNOWN_ENTITY | — | 19 |
| 38 | `fruit:yami` | `dark skeletal shark` | COMPOUND_AMBIGUOUS | `fish:shark`, `fruit:dark` | 18 |
| 31 | `fish:squid` | `ne item candy corn squid` | UNKNOWN_ENTITY | — | 14 |
| 29 | `(none)` | `blve lip grouper` | UNKNOWN_ENTITY | — | 5 |
| 23 | `fish:pufferfish` | `golden polka puffer` | UNKNOWN_ENTITY | — | 12 |
| 23 | `fish:angelfish` | `zebra ribbonangelfish` | UNKNOWN_ENTITY | — | 8 |
| 22 | `fish:shark` | `newitemskeletalshark` | UNKNOWN_ENTITY | — | 13 |
| 22 | `(none)` | `crimson snapper` | COMPOUND_AMBIGUOUS | `fish:crimson`, `fish:snapper` | 15 |
| 20 | `(none)` | `newitemskeletalshark` | UNKNOWN_ENTITY | — | 14 |
| 19 | `fish:angelfish` | `crimson polka puffer` | UNKNOWN_ENTITY | — | 13 |
| 19 | `fish:crimson` | `zebra ribbon angelfish` | UNKNOWN_ENTITY | — | 13 |
| 18 | `(none)` | `nevitemskeletalshark` | UNKNOWN_ENTITY | — | 12 |
| 18 | `fish:golden` | `sgolden tigerfin` | COMPOUND_AMBIGUOUS | `fish:golden`, `fish:tiger` | 10 |
| 18 | `(none)` | `golden ribbon angelfish` | COMPOUND_AMBIGUOUS | `fish:angelfish`, `fish:golden` | 9 |
| 17 | `fish:angelfish` | `ne item zebra ribbon angelfish` | UNKNOWN_ENTITY | — | 5 |
| 15 | `(none)` | `golden tigerfin` | UNKNOWN_ENTITY | — | 10 |
| 14 | `(none)` | `exotictigerfin` | UNKNOWN_ENTITY | — | 10 |
| 14 | `fish:snapper` | `crimsonsnapper` | COMPOUND_AMBIGUOUS | `fish:crimson`, `fish:snapper` | 8 |
| 13 | `fruit:yami` | `sdark skeletal shark` | UNKNOWN_ENTITY | — | 9 |
| 13 | `(none)` | `candy corn squid` | UNKNOWN_ENTITY | — | 9 |
| 13 | `fish:angelfish` | `golden ribbon angelfish` | COMPOUND_AMBIGUOUS | `fish:angelfish`, `fish:golden` | 11 |
| 13 | `fish:shark` | `darkskeletal shark` | UNKNOWN_ENTITY | — | 11 |
| 13 | `fish:shark` | `darkskeletalshark` | COMPOUND_AMBIGUOUS | `fish:shark`, `fruit:dark` | 8 |
| 13 | `fish:golden` | `sgolden ribbon angelfish` | UNKNOWN_ENTITY | — | 7 |
| 12 | `(none)` | `tigerfinz` | UNKNOWN_ENTITY | — | 3 |
| 11 | `fish:squid` | `crimson snapper` | COMPOUND_AMBIGUOUS | `fish:crimson`, `fish:snapper` | 9 |
| 11 | `fish:angelfish` | `zebra ribbon angelfishz` | UNKNOWN_ENTITY | — | 6 |
| 11 | `fish:squid` | `candycorn squid` | UNKNOWN_ENTITY | — | 10 |
| 11 | `(none)` | `blve lipgrouper` | UNKNOWN_ENTITY | — | 2 |
| 11 | `fruit:yami` | `darkskeletal shark` | UNKNOWN_ENTITY | — | 7 |
| 10 | `(none)` | `ne item fangfish` | UNKNOWN_ENTITY | — | 9 |
| 10 | `fish:squid` | `candy cormn squid` | UNKNOWN_ENTITY | — | 6 |
| 10 | `(none)` | `golden polka puffer` | UNKNOWN_ENTITY | — | 8 |
| 10 | `fish:snapper` | `candy corn squid` | UNKNOWN_ENTITY | — | 6 |
| 10 | `(none)` | `sgolden tigerfin` | COMPOUND_AMBIGUOUS | `fish:golden`, `fish:tiger` | 7 |
| 10 | `(none)` | `exotic tigerfinz` | UNKNOWN_ENTITY | — | 7 |
| 10 | `fish:crimson` | `crimson snapper` | COMPOUND_AMBIGUOUS | `fish:crimson`, `fish:snapper` | 8 |
| 9 | `fish:snapper` | `exotic tigerfin` | UNKNOWN_ENTITY | — | 8 |
| 9 | `fish:shark` | `crimson snapper` | COMPOUND_AMBIGUOUS | `fish:crimson`, `fish:snapper` | 6 |
| 9 | `(none)` | `crimson polka puffer` | UNKNOWN_ENTITY | — | 6 |
| 9 | `fish:snapper` | `fangfish` | UNKNOWN_ENTITY | — | 8 |
| 9 | `fish:golden` | `crimson polka puffer` | UNKNOWN_ENTITY | — | 6 |
| 9 | `fish:shark` | `nevitemskeletalshark` | UNKNOWN_ENTITY | — | 6 |
| 9 | `(none)` | `zebra ribbonangelfish` | UNKNOWN_ENTITY | — | 3 |


## Highest-priority individual rows

Ranked by disagreement, recurrence across independent sessions, and OCR
confidence. **Ordering only — this score never changes a label.**

| priority | image_id | stored | OCR text | status | candidates | session | reasons |
|---|---|---|---|---|---|---|---|
| 116 | `20203e323ebe80ff-2` | `fruit:yami` | `dark skeletal shark` | COMPOUND_AMBIGUOUS | `fish:shark`, `fruit:dark` | session_2026-10-06_175729_f925385b | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 116 | `30301090beffffff` | `fruit:yami` | `dark skeletal shark` | COMPOUND_AMBIGUOUS | `fish:shark`, `fruit:dark` | session_2026-10-03_164415_39a04dc5 | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 116 | `30303010be80ffff` | `fish:golden` | `dark skeletal shark` | COMPOUND_AMBIGUOUS | `fish:shark`, `fruit:dark` | session_2026-10-03_104057_39d5add1 | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 116 | `3f3e3e10963f0101` | `None` | `golden ribbon angelfish` | COMPOUND_AMBIGUOUS | `fish:angelfish`, `fish:golden` | session_2026-10-03_161417_bb4b5817 | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 116 | `606020203e367efe` | `fruit:yami` | `dark skeletal shark` | COMPOUND_AMBIGUOUS | `fish:shark`, `fruit:dark` | session_2026-10-04_202745_08b4b13a | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 116 | `606021213e3ebebe` | `fruit:yami` | `dark skeletal shark` | COMPOUND_AMBIGUOUS | `fish:shark`, `fruit:dark` | session_2026-10-02_232819_2c7e6408 | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 116 | `606060203e3e3e00` | `fish:crimson` | `dark skeletal shark` | COMPOUND_AMBIGUOUS | `fish:shark`, `fruit:dark` | session_2026-10-03_095208_9aa2f3b7 | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 116 | `606060e0fce0f0f0` | `fish:snapper` | `dark skeletal shark` | COMPOUND_AMBIGUOUS | `fish:shark`, `fruit:dark` | session_2026-10-04_202745_08b4b13a | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 116 | `606420203e34beb6` | `fruit:yami` | `dark skeletal shark` | COMPOUND_AMBIGUOUS | `fish:shark`, `fruit:dark` | session_2026-10-02_232819_2c7e6408 | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 116 | `796160203e3e3200` | `fruit:yami` | `dark skeletal shark` | COMPOUND_AMBIGUOUS | `fish:shark`, `fruit:dark` | session_2026-10-04_202745_08b4b13a | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 116 | `7c7cb4303c200000` | `fruit:yami` | `dark skeletal shark` | COMPOUND_AMBIGUOUS | `fish:shark`, `fruit:dark` | session_2026-10-04_202745_08b4b13a | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 116 | `b03030303f3c9e9f` | `fruit:yami` | `dark skeletal shark` | COMPOUND_AMBIGUOUS | `fish:shark`, `fruit:dark` | session_2026-10-04_070602_b6f12619 | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 116 | `e02020203e3030b6` | `fruit:yami` | `dark skeletal shark` | COMPOUND_AMBIGUOUS | `fish:shark`, `fruit:dark` | session_2026-10-03_223708_0ca663cf | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 116 | `e0203023373f3ebe` | `None` | `golden ribbon angelfish` | COMPOUND_AMBIGUOUS | `fish:angelfish`, `fish:golden` | session_2026-10-03_223708_0ca663cf | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 116 | `e03030313f3f3f9f-1` | `fruit:yami` | `dark skeletal shark` | COMPOUND_AMBIGUOUS | `fish:shark`, `fruit:dark` | session_2026-10-06_225424_a78e14f2 | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 116 | `e06060203e30343e` | `fish:crimson` | `dark skeletal shark` | COMPOUND_AMBIGUOUS | `fish:shark`, `fruit:dark` | session_2026-10-04_120009_b970ca36 | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 116 | `e06060203e303eb0` | `fruit:yami` | `dark skeletal shark` | COMPOUND_AMBIGUOUS | `fish:shark`, `fruit:dark` | session_2026-10-02_232819_2c7e6408 | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 116 | `e06060603e20303e` | `fish:pufferfish` | `dark skeletal shark` | COMPOUND_AMBIGUOUS | `fish:shark`, `fruit:dark` | session_2026-10-03_052246_57c522fe | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 116 | `e06066203f3f3ebe` | `fruit:yami` | `dark skeletal shark` | COMPOUND_AMBIGUOUS | `fish:shark`, `fruit:dark` | session_2026-10-03_110949_15b78581 | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 116 | `e07020203e303030` | `fruit:yami` | `dark skeletal shark` | COMPOUND_AMBIGUOUS | `fish:shark`, `fruit:dark` | session_2026-10-03_223708_0ca663cf | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 116 | `e0e060603e3f3e3e` | `fruit:yami` | `dark skeletal shark` | COMPOUND_AMBIGUOUS | `fish:shark`, `fruit:dark` | session_2026-10-04_141428_4c6bc299 | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 116 | `e0e0a0203c303010` | `fruit:yami` | `dark skeletal shark` | COMPOUND_AMBIGUOUS | `fish:shark`, `fruit:dark` | session_2026-10-02_232819_2c7e6408 | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 116 | `e36062233e3e3e3e` | `fruit:yami` | `dark skeletal shark` | COMPOUND_AMBIGUOUS | `fish:shark`, `fruit:dark` | session_2026-10-03_052246_57c522fe | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 116 | `e6686c603e202034` | `fruit:yami` | `dark skeletal shark` | COMPOUND_AMBIGUOUS | `fish:shark`, `fruit:dark` | session_2026-10-05_190403_c9429492 | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 116 | `e87820203e3030b0` | `fruit:yami` | `dark skeletal shark` | COMPOUND_AMBIGUOUS | `fish:shark`, `fruit:dark` | session_2026-10-02_232819_2c7e6408 | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 116 | `e8e8ecc83c002000` | `fruit:yami` | `dark skeletal shark` | COMPOUND_AMBIGUOUS | `fish:shark`, `fruit:dark` | session_2026-10-03_051036_cf6004f2 | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 116 | `f03030303e303030-8` | `fruit:yami` | `dark skeletal shark` | COMPOUND_AMBIGUOUS | `fish:shark`, `fruit:dark` | session_2026-10-06_225424_a78e14f2 | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 116 | `f03030303e30349e` | `fruit:yami` | `dark skeletal shark` | COMPOUND_AMBIGUOUS | `fish:shark`, `fruit:dark` | session_2026-10-06_225424_a78e14f2 | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 116 | `f03030303e30349e-1` | `fruit:yami` | `dark skeletal shark` | COMPOUND_AMBIGUOUS | `fish:shark`, `fruit:dark` | session_2026-10-06_225424_a78e14f2 | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 116 | `f07060607ce0e0e0` | `fruit:yami` | `dark skeletal shark` | COMPOUND_AMBIGUOUS | `fish:shark`, `fruit:dark` | session_2026-10-05_190403_c9429492 | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 115 | `109090c7ffc64646` | `fruit:yami` | `dark skeletal shark` | COMPOUND_AMBIGUOUS | `fish:shark`, `fruit:dark` | session_2026-10-04_160014_94aaa3c9 | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 115 | `128092e7ffc24242` | `fruit:yami` | `dark skeletal shark` | COMPOUND_AMBIGUOUS | `fish:shark`, `fruit:dark` | session_2026-10-04_160014_94aaa3c9 | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 115 | `203c3f3f073e3e9e` | `None` | `golden ribbon angelfish` | COMPOUND_AMBIGUOUS | `fish:angelfish`, `fish:golden` | session_2026-10-02_232819_2c7e6408 | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 115 | `282820203e3ebebe` | `fruit:yami` | `dark skeletal shark` | COMPOUND_AMBIGUOUS | `fish:shark`, `fruit:dark` | session_2026-10-03_091319_6fbb96f4 | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 115 | `28383010be80dfff` | `fruit:yami` | `dark skeletal shark` | COMPOUND_AMBIGUOUS | `fish:shark`, `fruit:dark` | session_2026-10-04_073807_04bff67c | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 115 | `30303030be809eff` | `fish:crimson` | `dark skeletal shark` | COMPOUND_AMBIGUOUS | `fish:shark`, `fruit:dark` | session_2026-10-04_073807_04bff67c | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 115 | `30303030be80dfff` | `fish:shark` | `dark skeletal shark` | COMPOUND_AMBIGUOUS | `fish:shark`, `fruit:dark` | session_2026-10-04_073807_04bff67c | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 115 | `308090c7ffc64646` | `fruit:yami` | `dark skeletal shark` | COMPOUND_AMBIGUOUS | `fish:shark`, `fruit:dark` | session_2026-10-04_160014_94aaa3c9 | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 115 | `31393337273e1e00` | `fish:shark` | `darkskeletalshark` | COMPOUND_AMBIGUOUS | `fish:shark`, `fruit:dark` | session_2026-10-03_171113_094bcfa0 | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 115 | `33311090bcffe3e3` | `fruit:yami` | `dark skeletal shark` | COMPOUND_AMBIGUOUS | `fish:shark`, `fruit:dark` | session_2026-10-03_164415_39a04dc5 | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 115 | `333e3ebe9680dfff` | `fish:snapper` | `darkskeletalshark` | COMPOUND_AMBIGUOUS | `fish:shark`, `fruit:dark` | session_2026-10-04_073807_04bff67c | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 115 | `338381c3ffc24242` | `fruit:yami` | `dark skeletal shark` | COMPOUND_AMBIGUOUS | `fish:shark`, `fruit:dark` | session_2026-10-04_160014_94aaa3c9 | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 115 | `373e3e1e809f3f01` | `fish:angelfish` | `darkskeletalshark` | COMPOUND_AMBIGUOUS | `fish:shark`, `fruit:dark` | session_2026-10-02_215405_f5d40eba | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 115 | `373f3e9e809effc1` | `fish:shark` | `darkskeletalshark` | COMPOUND_AMBIGUOUS | `fish:shark`, `fruit:dark` | session_2026-10-03_115211_8fa9d196 | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 115 | `3838333f3f3e3e1e` | `None` | `golden ribbon angelfish` | COMPOUND_AMBIGUOUS | `fish:angelfish`, `fish:golden` | session_2026-10-02_232819_2c7e6408 | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 115 | `383838103e809fef` | `fruit:yami` | `dark skeletal shark` | COMPOUND_AMBIGUOUS | `fish:shark`, `fruit:dark` | session_2026-10-02_214651_50738306 | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 115 | `3c3f3f3f1f3e3e80` | `None` | `golden ribbon angelfish` | COMPOUND_AMBIGUOUS | `fish:angelfish`, `fish:golden` | session_2026-10-03_220117_1e7efb16 | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 115 | `3f3f3f3e3e3e003f-1` | `None` | `crimson snapper` | COMPOUND_AMBIGUOUS | `fish:crimson`, `fish:snapper` | session_2026-10-06_175729_f925385b | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 115 | `603833373f3f3e1e` | `fish:shark` | `darkskeletalshark` | COMPOUND_AMBIGUOUS | `fish:shark`, `fruit:dark` | session_2026-10-02_232819_2c7e6408 | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 115 | `606020203e32feff` | `fruit:yami` | `dark skeletal shark` | COMPOUND_AMBIGUOUS | `fish:shark`, `fruit:dark` | session_2026-10-03_211416_8d611afe | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 115 | `606020603e303eb0` | `fruit:yami` | `dark skeletal shark` | COMPOUND_AMBIGUOUS | `fish:shark`, `fruit:dark` | session_2026-10-02_232819_2c7e6408 | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 115 | `60676e7e647e013d` | `fish:shark` | `darkskeletalshark` | COMPOUND_AMBIGUOUS | `fish:shark`, `fruit:dark` | session_2026-10-05_215306_5858c673 | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 115 | `60743337273e3e1e` | `None` | `crimson snapper` | COMPOUND_AMBIGUOUS | `fish:crimson`, `fish:snapper` | session_2026-10-02_232819_2c7e6408 | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 115 | `607c373f273e3e00` | `None` | `crimson snapper` | COMPOUND_AMBIGUOUS | `fish:crimson`, `fish:snapper` | session_2026-10-03_171113_094bcfa0 | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 115 | `637e3e3c00013f07` | `None` | `golden ribbon angelfish` | COMPOUND_AMBIGUOUS | `fish:angelfish`, `fish:golden` | session_2026-10-04_220833_0f8210ed | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 115 | `647c3727073e3e1e` | `None` | `golden ribbon angelfish` | COMPOUND_AMBIGUOUS | `fish:angelfish`, `fish:golden` | session_2026-10-02_232819_2c7e6408 | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 115 | `666636223e303090` | `fruit:yami` | `dark skeletal shark` | COMPOUND_AMBIGUOUS | `fish:shark`, `fruit:dark` | session_2026-10-02_232819_2c7e6408 | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 115 | `677e3e3e0201ff87` | `None` | `golden ribbon angelfish` | COMPOUND_AMBIGUOUS | `fish:angelfish`, `fish:golden` | session_2026-10-04_220833_0f8210ed | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 115 | `70783b37073e3e16` | `None` | `golden ribbon angelfish` | COMPOUND_AMBIGUOUS | `fish:angelfish`, `fish:golden` | session_2026-10-02_232819_2c7e6408 | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |
| 115 | `70797f3f363e3e00` | `None` | `golden ribbon angelfish` | COMPOUND_AMBIGUOUS | `fish:angelfish`, `fish:golden` | session_2026-10-03_095208_9aa2f3b7 | compound name with 2 candidates; a candidate belongs to a different KB category than the stored label; seen in |

