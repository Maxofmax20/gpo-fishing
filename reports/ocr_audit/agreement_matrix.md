# OCR Audit — Agreement matrix (stored / model / OCR)

14805 rows carry a model prediction.

The `fish_v1` model is a THIRD opinion here. It was not retrained, replaced or
promoted, and its predictions are never treated as ground truth. TEST rows are
included for reporting only; nothing in this pipeline tunes on them.

| stored label | model prediction | OCR interpretation | rows |
|---|---|---|---|
| `(none)` | `fish:swordfish` | `x1` | 1672 |
| `(none)` | `fish:swordfish` | `k129` | 1197 |
| `(none)` | `fish:swordfish` | `hbait ing baits y fish bait x129` | 892 |
| `(none)` | `fish:shark` | `(unresolved)` | 734 |
| `(none)` | `fish:swordfish` | `x81` | 724 |
| `(none)` | `fish:swordfish` | `(unresolved)` | 672 |
| `(none)` | `fish:squid` | `(unresolved)` | 612 |
| `(none)` | `fish:snapper` | `(unresolved)` | 491 |
| `(none)` | `fish:swordfish` | `x129` | 409 |
| `(none)` | `fish:swordfish` | `h bait ing baits y fish bait x129` | 326 |
| `(none)` | `fish:squid` | `fangfish` | 165 |
| `fish:snapper` | `fish:snapper` | `crimson snapper` | 156 |
| `(none)` | `fish:snapper` | `exotic tigerfin` | 151 |
| `fish:angelfish` | `fish:angelfish` | `zebra ribbon angelfish` | 147 |
| `(none)` | `fish:swordfish` | `bait ing baits y fish bait` | 147 |
| `(none)` | `fish:swordfish` | `hbait ing baits y fish bait x1` | 115 |
| `fish:crimson` | `fish:crimson` | `crimson polka puffer` | 107 |
| `(none)` | `fish:snapper` | `fangfish` | 89 |
| `fish:squid` | `fish:squid` | `candy corn squid` | 83 |
| `fish:shark` | `fish:shark` | `skeletal shark` | 78 |
| `(none)` | `fish:swordfish` | `sh bait ing baits y fish bait x129` | 76 |
| `(none)` | `fish:swordfish` | `hbait ing baits y fish bait` | 74 |
| `fish:golden` | `fish:golden` | `golden ribbon angelfish` | 71 |
| `(none)` | `fish:swordfish` | `anglerfish` | 62 |
| `fish:squid` | `fish:snapper` | `candy corn squid` | 60 |
| `fish:golden` | `fish:golden` | `golden tigerfin` | 58 |
| `fish:golden` | `fish:golden` | `golden polka puffer` | 57 |
| `(none)` | `fish:swordfish` | `bait ing baits y fish bait x1` | 56 |
| `fish:shark` | `fish:shark` | `skeletalshark` | 54 |
| `fish:swordfish` | `fish:swordfish` | `fish:swordfish` | 50 |
| `(none)` | `fish:swordfish` | `tormbringers hammedsamir2005 bait ing baits y fish bait x81` | 49 |
| `(none)` | `fish:squid` | `exotic tigerfin` | 47 |
| `(none)` | `fish:swordfish` | `h bait ing baits y fish bait` | 40 |
| `(none)` | `fish:angelfish` | `zebra ribbon angelfish` | 40 |
| `(none)` | `fish:shark` | `skeletalshark` | 40 |
| `fruit:yami` | `fish:golden` | `dark skeletal shark` | 38 |
| `(none)` | `fish:swordfish` | `jack o bite` | 36 |
| `(none)` | `fish:swordfish` | `fish:swordfish` | 36 |
| `(none)` | `fish:swordfish` | `129` | 34 |
| `(none)` | `fish:swordfish` | `ormbringers ammedsamir2005 bait ing baits y fish bait x81` | 34 |
| `(none)` | `fish:swordfish` | `h bait it ing baits y fish bait x129` | 34 |
| `(none)` | `fish:swordfish` | `ormbringers odlyfisheman ammedsamir2005 bait ing baits y fish bait x1` | 33 |
| `(none)` | `fish:shark` | `jack o bite` | 33 |
| `(none)` | `fish:swordfish` | `shbait ing baits y fish bait` | 31 |
| `(none)` | `fish:golden` | `(unresolved)` | 28 |

