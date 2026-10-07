# OCR Audit — Compound names

A compound name contains two or more whole-word canonical entities, so **no
single stored label can be correct for it** without a human decision.

This is the case that produced the original contamination: the collector's
resolver picked one component arbitrarily.

**39 distinct concept(s) affected.**

## `crimson snapper`

| property | value |
|---|---|
| rows | 245 |
| compound rows | 245 |
| sessions / days | 40 / 5 |
| canonical candidates | `fish:crimson` ×245, `fish:snapper` ×245 |
| entity fragments found | `crimson -> fish:crimson`; `snapper -> fish:snapper` |
| stored labels it was filed under | `fish:snapper` ×181, `fish:squid` ×11, `fish:crimson` ×10, `fish:shark` ×9, `fish:angelfish` ×4, `fish:swordfish` ×2, `fish:golden` ×2, `fish:pufferfish` ×2, `ui:new-item` ×2 |
| mean OCR confidence | 0.8365 |
| example image ids | `000000003cffffff`, `008080d7ffdede46`, `008080dfffdede46`, `1e009f4f47424242`, `1e029f4f47424242`, `202020103e7f7fff` |
| OCR variants seen | `Nev Item <crimson Snapper>` ×59; `New Item <Crimson Snapper>` ×55; `New Item <crimson Snapper>` ×43; `New Item <Crimson SnaPper>` ×21; `New Item <crimson SnaPper>` ×15 |


## `golden ribbon angelfish`

| property | value |
|---|---|
| rows | 105 |
| compound rows | 105 |
| sessions / days | 35 / 5 |
| canonical candidates | `fish:angelfish` ×105, `fish:golden` ×105 |
| entity fragments found | `golden -> fish:golden`; `angelfish -> fish:angelfish` |
| stored labels it was filed under | `fish:golden` ×71, `fish:angelfish` ×13, `fish:crimson` ×1, `fish:snapper` ×1, `fruit:yami` ×1 |
| mean OCR confidence | 0.8634 |
| example image ids | `030303030387070f`, `10109090fedffffe`, `202020203e3ebebe`, `202033253e3ebebe`, `2020a000feffdfdf`, `20301090fe80ffff` |
| OCR variants seen | `New Item <Golden Ribbon Angelfish-` ×45; `New Item <Golden Ribbon Angelfish>` ×18; `New Item Golden Ribbon Angelfish-` ×10; `Nev Item <Golden Ribbon Angelfish-` ×8; `Nev Item Golden Ribbon Angelfish-` ×5 |


## `dark skeletal shark`

| property | value |
|---|---|
| rows | 48 |
| compound rows | 48 |
| sessions / days | 21 / 5 |
| canonical candidates | `fish:shark` ×48, `fruit:dark` ×48 |
| entity fragments found | `dark -> fruit:dark`; `shark -> fish:shark` |
| stored labels it was filed under | `fruit:yami` ×38, `fish:crimson` ×3, `fish:shark` ×3, `fish:golden` ×1, `fish:snapper` ×1, `fish:pufferfish` ×1 |
| mean OCR confidence | 0.8746 |
| example image ids | `109090c7ffc64646`, `128092e7ffc24242`, `20203e323ebe80ff-2`, `282820203e3ebebe`, `28383010be80dfff`, `30301090beffffff` |
| OCR variants seen | `New Item <Dark Skeletal Shark-` ×43; `New Item <Dark skeletal Shark-` ×3; `Nev Item <Dark skeletal Shark-` ×2 |


## `sgolden tigerfin`

| property | value |
|---|---|
| rows | 31 |
| compound rows | 31 |
| sessions / days | 16 / 4 |
| canonical candidates | `fish:golden` ×31, `fish:tiger` ×31 |
| entity fragments found | `golden -> fish:golden`; `tiger -> fish:tiger` |
| stored labels it was filed under | `fish:golden` ×18, `fish:shark` ×2, `fish:angelfish` ×1 |
| mean OCR confidence | 0.8615 |
| example image ids | `2030f0f0fcf0f0ff`, `30109090bcdfffee`, `303030303fbe9e9f`, `3030f0f0fcf0e0ff`, `30333f3a1e9f809f-7`, `373d17171f1f9980` |
| OCR variants seen | `New Item sGolden Tigerfin` ×12; `New Item sGolden Tigerfin?` ×12; `New Item sGolden Tigerfin>` ×3; `New Item sGolden Tigerfin-` ×2; `Nev Item sGolden Tigerfin?` ×1 |


## `crimsonsnapper`

| property | value |
|---|---|
| rows | 23 |
| compound rows | 23 |
| sessions / days | 10 / 4 |
| canonical candidates | `fish:crimson` ×23, `fish:snapper` ×23 |
| entity fragments found | `crimson -> fish:crimson`; `snapper -> fish:snapper` |
| stored labels it was filed under | `fish:snapper` ×14, `fruit:yami` ×2, `fish:angelfish` ×2, `fish:squid` ×1 |
| mean OCR confidence | 0.8197 |
| example image ids | `303f373f3f3f3f80`, `33273f3e3e1e007f`, `333f3e3e16801f3f`, `333f3e3e96809f0f`, `3f3f1e9084bf0140`, `60602337363ebe90` |
| OCR variants seen | `Nev Item <crimsonSnapper>` ×10; `Nev Item<crimsonSnapper>` ×4; `Nev Item<crimsonSnapper?` ×2; `New Item <crimsonSnapper>` ×2; `New Item<CrimsonSnapper>` ×1 |


## `darkskeletalshark`

| property | value |
|---|---|
| rows | 22 |
| compound rows | 22 |
| sessions / days | 11 / 5 |
| canonical candidates | `fish:shark` ×22, `fruit:dark` ×22 |
| entity fragments found | `dark -> fruit:dark`; `shark -> fish:shark` |
| stored labels it was filed under | `fish:shark` ×13, `fruit:yami` ×3, `fish:snapper` ×1, `fish:angelfish` ×1 |
| mean OCR confidence | 0.8221 |
| example image ids | `203c37373f3f3f1f`, `31393337273e1e00`, `33343a3e9680dfff`, `333e3ebe9680dfff`, `333f3e1e1e801f3f`, `373e3e1e809f3f01` |
| OCR variants seen | `New Item <DarkskeletalShark-` ×8; `Nev Item <DarkskeletalShark-` ×4; `New Item <Darkskeletalshark-` ×3; `New Item<DarkSkeletalShark-` ×2; `New Item<DarkskeletalShark-` ×2 |


## `goldentigerfin`

| property | value |
|---|---|
| rows | 6 |
| compound rows | 6 |
| sessions / days | 3 / 2 |
| canonical candidates | `fish:golden` ×6, `fish:tiger` ×6 |
| entity fragments found | `golden -> fish:golden`; `tiger -> fish:tiger` |
| stored labels it was filed under | `fish:golden` ×3 |
| mean OCR confidence | 0.853 |
| example image ids | `373c3a3e9e80dfff`, `b03033370f3e1f1f`, `e0303030363f38bf`, `e0303033373f3f1f-16`, `f0303031373f3e3f`, `f0303c33073f1f9f` |
| OCR variants seen | `New Item <GoldenTigerfin` ×3; `New Item<GoldenTigerfin` ×1; `Nev Item GoldenTigerfin` ×1; `Nev Item <GoldenTigerfin` ×1 |


## `ne item golden ribbon angelfish`

| property | value |
|---|---|
| rows | 3 |
| compound rows | 3 |
| sessions / days | 2 / 2 |
| canonical candidates | `fish:angelfish` ×3, `fish:golden` ×3 |
| entity fragments found | `golden -> fish:golden`; `angelfish -> fish:angelfish` |
| stored labels it was filed under | `fish:golden` ×2 |
| mean OCR confidence | 0.8647 |
| example image ids | `30303337171f9f9f`, `e0303031333f3e9f-2`, `f0303c31133f3f9f` |
| OCR variants seen | `Ne Item <Golden Ribbon Angelfish-` ×2; `Ne Item <Golden Ribbon Angelfish>` ×1 |


## `nevitem crimsonsnapper`

| property | value |
|---|---|
| rows | 3 |
| compound rows | 3 |
| sessions / days | 2 / 2 |
| canonical candidates | `fish:crimson` ×3, `fish:snapper` ×3 |
| entity fragments found | `crimson -> fish:crimson`; `snapper -> fish:snapper` |
| stored labels it was filed under | `fish:snapper` ×1, `fish:crimson` ×1 |
| mean OCR confidence | 0.8337 |
| example image ids | `273e3e3e06017f0f`, `3e3e7e7f477f7c7c`, `777e3e3e02013f07` |
| OCR variants seen | `NevItem<crimsonSnapper` ×2; `NevItem<crimsonSnapper?` ×1 |


## `newitem crimson snapper`

| property | value |
|---|---|
| rows | 2 |
| compound rows | 2 |
| sessions / days | 2 / 2 |
| canonical candidates | `fish:crimson` ×2, `fish:snapper` ×2 |
| entity fragments found | `crimson -> fish:crimson`; `snapper -> fish:snapper` |
| stored labels it was filed under | `fish:squid` ×1, `fish:crimson` ×1 |
| mean OCR confidence | 0.821 |
| example image ids | `333f3f3e1e001f47`, `3f3f3e1e163f4141` |
| OCR variants seen | `NewItem<crimson Snapper>` ×2 |


## `sgolden tigerfinz`

| property | value |
|---|---|
| rows | 2 |
| compound rows | 2 |
| sessions / days | 2 / 2 |
| canonical candidates | `fish:golden` ×2, `fish:tiger` ×2 |
| entity fragments found | `golden -> fish:golden`; `tiger -> fish:tiger` |
| stored labels it was filed under | `fish:golden` ×2 |
| mean OCR confidence | 0.8652 |
| example image ids | `2020007f7fc7c2c2`, `3e1a1f4f43424242` |
| OCR variants seen | `New Item sGolden Tigerfinz` ×2 |


## `a bomb has at trading hub zebra ribbon angelfish`

| property | value |
|---|---|
| rows | 1 |
| compound rows | 1 |
| sessions / days | 1 / 1 |
| canonical candidates | `fish:angelfish` ×1, `fruit:bomb` ×1 |
| entity fragments found | `bomb -> fruit:bomb`; `angelfish -> fish:angelfish` |
| stored labels it was filed under | `fruit:bomb` ×1 |
| mean OCR confidence | 0.8834 |
| example image ids | `e060607e7e606020` |
| OCR variants seen | `A BOMB has Spawned at TRADING HUB New Item <Zebra Ribbon Angelfish>` ×1 |


## `a gomu has spavned at trading hub candy corn squid`

| property | value |
|---|---|
| rows | 1 |
| compound rows | 1 |
| sessions / days | 1 / 1 |
| canonical candidates | `fish:squid` ×1, `fruit:gomu` ×1 |
| entity fragments found | `gomu -> fruit:gomu`; `squid -> fish:squid` |
| stored labels it was filed under | `fruit:gomu` ×1 |
| mean OCR confidence | 0.8736 |
| example image ids | `e060607c3e202030` |
| OCR variants seen | `A GOMU has Spavned at TRADING HUB New Item <Candy Corn Squid>` ×1 |


## `a heal has at trading hub crimson snapper`

| property | value |
|---|---|
| rows | 1 |
| compound rows | 1 |
| sessions / days | 1 / 1 |
| canonical candidates | `fish:crimson` ×1, `fish:snapper` ×1, `fruit:heal` ×1 |
| entity fragments found | `heal -> fruit:heal`; `crimson -> fish:crimson`; `snapper -> fish:snapper` |
| stored labels it was filed under | `fruit:heal` ×1 |
| mean OCR confidence | 0.8731 |
| example image ids | `e060607c7e606060` |
| OCR variants seen | `A HEAL has Spawned at TRADING HUB New Item <crimson SnaPper>` ×1 |


## `a heal has spavned at trading hub golden ribbon angelfish`

| property | value |
|---|---|
| rows | 1 |
| compound rows | 1 |
| sessions / days | 1 / 1 |
| canonical candidates | `fish:angelfish` ×1, `fish:golden` ×1, `fruit:heal` ×1 |
| entity fragments found | `heal -> fruit:heal`; `golden -> fish:golden`; `angelfish -> fish:angelfish` |
| stored labels it was filed under | `fruit:heal` ×1 |
| mean OCR confidence | 0.8303 |
| example image ids | `3e929f074b424242` |
| OCR variants seen | `A HEAL has Spavned at TRADING HUB New Item <Golden Ribbon Angelfish` ×1 |


## `a healhasspavned at trading hub newitemcandycornsquid`

| property | value |
|---|---|
| rows | 1 |
| compound rows | 1 |
| sessions / days | 1 / 1 |
| canonical candidates | `fish:squid` ×1, `fruit:heal` ×1 |
| entity fragments found | `heal -> fruit:heal`; `squid -> fish:squid` |
| stored labels it was filed under | `fish:squid` ×1 |
| mean OCR confidence | 0.851 |
| example image ids | `303333141e9f809f` |
| OCR variants seen | `A HEALhasSpavned at TRADING HUB NewItemCandyCornSquid>` ×1 |


## `a kilo has at trading hub zebra ribbon angelfish`

| property | value |
|---|---|
| rows | 1 |
| compound rows | 1 |
| sessions / days | 1 / 1 |
| canonical candidates | `fish:angelfish` ×1, `fruit:kilo` ×1 |
| entity fragments found | `kilo -> fruit:kilo`; `angelfish -> fish:angelfish` |
| stored labels it was filed under | `fruit:kilo` ×1 |
| mean OCR confidence | 0.8565 |
| example image ids | `e060203e3e303030` |
| OCR variants seen | `A KILO has Spawned at TRADING HUB New Item <Zebra Ribbon Angelfish>` ×1 |


## `a spin has at trading hub eeing eye you drop checkyour backpa legendary pity 24 40`

| property | value |
|---|---|
| rows | 1 |
| compound rows | 1 |
| sessions / days | 1 / 1 |
| canonical candidates | `bait:legendary` ×1, `fruit:spin` ×1 |
| entity fragments found | `spin -> fruit:spin`; `eei -> fish:eel`; `legendary -> bait:legendary` |
| stored labels it was filed under | `fruit:spin` ×1 |
| mean OCR confidence | 0.8294 |
| example image ids | `fe7e7c7e3e202030` |
| OCR variants seen | `A SPIN has Spawned at TRADING HUB eeing Eye: YOU GOT A DEVIL FRUIT DROP,CHECKYOUR BACKPA LEGENDARY PITY: 24/40` ×1 |


## `a yomi has at trading hub crimson snapper`

| property | value |
|---|---|
| rows | 1 |
| compound rows | 1 |
| sessions / days | 1 / 1 |
| canonical candidates | `fish:crimson` ×1, `fish:snapper` ×1, `fruit:yomi` ×1 |
| entity fragments found | `yomi -> fruit:yomi`; `crimson -> fish:crimson`; `snapper -> fish:snapper` |
| stored labels it was filed under | `fish:snapper` ×1 |
| mean OCR confidence | 0.8528 |
| example image ids | `303010bc9c84ffff` |
| OCR variants seen | `A YOMI has Spawned at TRADING HUB New Item <Crimson SnaPper>` ×1 |


## `agomu has spavned at trading hub newitem exotictigerfin`

| property | value |
|---|---|
| rows | 1 |
| compound rows | 1 |
| sessions / days | 1 / 1 |
| canonical candidates | `fish:tiger` ×1, `fruit:gomu` ×1 |
| entity fragments found | `gomu -> fruit:gomu`; `tiger -> fish:tiger` |
| stored labels it was filed under | `ui:spawned` ×1 |
| mean OCR confidence | 0.8573 |
| example image ids | `e03030211f3f3f1f` |
| OCR variants seen | `AGOMU has Spavned at TRADING HUB NewItem<ExoticTigerfin>` ×1 |


## `asukehas spavned attrading hub newitem candyconsquid`

| property | value |
|---|---|
| rows | 1 |
| compound rows | 1 |
| sessions / days | 1 / 1 |
| canonical candidates | `fish:squid` ×1, `fruit:suke` ×1 |
| entity fragments found | `suke -> fruit:suke`; `squid -> fish:squid` |
| stored labels it was filed under | `fish:squid` ×1 |
| mean OCR confidence | 0.7994 |
| example image ids | `20e0e0c0c8feffff` |
| OCR variants seen | `ASUKEhas Spavned atTRADING HUB NewItem<CandyConSquid>` ×1 |


## `darksketetalshark`

| property | value |
|---|---|
| rows | 1 |
| compound rows | 1 |
| sessions / days | 1 / 1 |
| canonical candidates | `fish:shark` ×1, `fruit:dark` ×1 |
| entity fragments found | `dark -> fruit:dark`; `shark -> fish:shark` |
| stored labels it was filed under | `fish:shark` ×1 |
| mean OCR confidence | 0.7884 |
| example image ids | `797e7e7c443c013f` |
| OCR variants seen | `New Item<Darksketetalshark-` ×1 |


## `eeing eye you drop check your backpa legendarypity 80 40`

| property | value |
|---|---|
| rows | 1 |
| compound rows | 1 |
| sessions / days | 1 / 1 |
| canonical candidates | `bait:legendary` ×1, `fish:eel` ×1 |
| entity fragments found | `eei -> fish:eel`; `legendary -> bait:legendary` |
| stored labels it was filed under | `bait:legendary` ×1 |
| mean OCR confidence | 0.7221 |
| example image ids | `3e3f3713101f9989` |
| OCR variants seen | `eeing Eye: YOU GOT A DEVIL FRUIT DROP, CHECK YOUR  BACKPA LEGENDARYPITY: 80/40` ×1 |


## `goldentigerfinz`

| property | value |
|---|---|
| rows | 1 |
| compound rows | 1 |
| sessions / days | 1 / 1 |
| canonical candidates | `fish:golden` ×1, `fish:tiger` ×1 |
| entity fragments found | `golden -> fish:golden`; `tiger -> fish:tiger` |
| stored labels it was filed under |  |
| mean OCR confidence | 0.801 |
| example image ids | `f0303033273f3f1f-3` |
| OCR variants seen | `New Item <GoldenTigerfinz` ×1 |


## `kilo a dedt fruit 1 1 5 063 golden staff agoldenstaff 35 441 bomi s log pose alogposethatcan trackmuttiple locations`

| property | value |
|---|---|
| rows | 1 |
| compound rows | 1 |
| sessions / days | 1 / 1 |
| canonical candidates | `fish:golden` ×1, `fruit:kilo` ×1 |
| entity fragments found | `golden -> fish:golden`; `kilo -> fruit:kilo` |
| stored labels it was filed under |  |
| mean OCR confidence | 0.5401 |
| example image ids | `1f03070f03671f03` |
| OCR variants seen | `kilo A dedt fruit 1/1 5.063 Golden Staff Agoldenstaff 35,441 Bomi's Log Pose AlogPosethatcan trackmuttiple locations,` ×1 |


## `ne item crimson snapper`

| property | value |
|---|---|
| rows | 1 |
| compound rows | 1 |
| sessions / days | 1 / 1 |
| canonical candidates | `fish:crimson` ×1, `fish:snapper` ×1 |
| entity fragments found | `crimson -> fish:crimson`; `snapper -> fish:snapper` |
| stored labels it was filed under | `fish:snapper` ×1 |
| mean OCR confidence | 0.8007 |
| example image ids | `333f3e1e96809fcf` |
| OCR variants seen | `Ne Item<crimson Snapper` ×1 |


## `ne item darkskeletalshark`

| property | value |
|---|---|
| rows | 1 |
| compound rows | 1 |
| sessions / days | 1 / 1 |
| canonical candidates | `fish:shark` ×1, `fruit:dark` ×1 |
| entity fragments found | `dark -> fruit:dark`; `shark -> fish:shark` |
| stored labels it was filed under | `fruit:yami` ×1 |
| mean OCR confidence | 0.8417 |
| example image ids | `3f3e1e9e841f8700` |
| OCR variants seen | `Ne Item <DarkskeletalShark-` ×1 |


## `ne legendary pity 28 40 youcan only store one of each fruit dropped bomb vill despavn in 10minutes`

| property | value |
|---|---|
| rows | 1 |
| compound rows | 1 |
| sessions / days | 1 / 1 |
| canonical candidates | `bait:legendary` ×1, `fruit:bomb` ×1 |
| entity fragments found | `legendary -> bait:legendary`; `bomb -> fruit:bomb` |
| stored labels it was filed under |  |
| mean OCR confidence | 0.6531 |
| example image ids | `e1e36120001c195f` |
| OCR variants seen | `NE LEGENDARY PITY: 28/40 Youcan only store one of each fruit! Dropped Bomb vill despavn in 10minutes.` ×1 |


## `nevitem darkskeletalshark`

| property | value |
|---|---|
| rows | 1 |
| compound rows | 1 |
| sessions / days | 1 / 1 |
| canonical candidates | `fish:shark` ×1, `fruit:dark` ×1 |
| entity fragments found | `dark -> fruit:dark`; `shark -> fish:shark` |
| stored labels it was filed under | `fish:shark` ×1 |
| mean OCR confidence | 0.8158 |
| example image ids | `7079773f263e3e00` |
| OCR variants seen | `NevItem <DarkskeletalShark-` ×1 |


## `nevitem golden ribbon angelfish`

| property | value |
|---|---|
| rows | 1 |
| compound rows | 1 |
| sessions / days | 1 / 1 |
| canonical candidates | `fish:angelfish` ×1, `fish:golden` ×1 |
| entity fragments found | `golden -> fish:golden`; `angelfish -> fish:angelfish` |
| stored labels it was filed under | `fish:golden` ×1 |
| mean OCR confidence | 0.8126 |
| example image ids | `757f3737023e3e12` |
| OCR variants seen | `NevItem <Golden Ribbon Angelfish-` ×1 |


## `nevitemdarkskeletalsharkmaskblveprint`

| property | value |
|---|---|
| rows | 1 |
| compound rows | 1 |
| sessions / days | 1 / 1 |
| canonical candidates | `fish:shark` ×1, `fruit:dark` ×1 |
| entity fragments found | `dark -> fruit:dark`; `shark -> fish:shark` |
| stored labels it was filed under | `fish:shark` ×1 |
| mean OCR confidence | 0.894 |
| example image ids | `e0606061237e3e3e` |
| OCR variants seen | `NevItemDarkSkeletalSharkMaskBlveprint` ×1 |


## `newitem crimsonsnapper`

| property | value |
|---|---|
| rows | 1 |
| compound rows | 1 |
| sessions / days | 1 / 1 |
| canonical candidates | `fish:crimson` ×1, `fish:snapper` ×1 |
| entity fragments found | `crimson -> fish:crimson`; `snapper -> fish:snapper` |
| stored labels it was filed under | `fish:snapper` ×1 |
| mean OCR confidence | 0.8278 |
| example image ids | `673e3e3e3e01ffdf` |
| OCR variants seen | `NewItem<crimsonSnapper>` ×1 |


## `newitem darkskeletalshark`

| property | value |
|---|---|
| rows | 1 |
| compound rows | 1 |
| sessions / days | 1 / 1 |
| canonical candidates | `fish:shark` ×1, `fruit:dark` ×1 |
| entity fragments found | `dark -> fruit:dark`; `shark -> fish:shark` |
| stored labels it was filed under | `fruit:yami` ×1 |
| mean OCR confidence | 0.8072 |
| example image ids | `3f3e3e9084bf0100` |
| OCR variants seen | `NewItem<DarkskeletalShark-` ×1 |


## `neylitem crimson snapper`

| property | value |
|---|---|
| rows | 1 |
| compound rows | 1 |
| sessions / days | 1 / 1 |
| canonical candidates | `fish:crimson` ×1, `fish:snapper` ×1 |
| entity fragments found | `crimson -> fish:crimson`; `snapper -> fish:snapper` |
| stored labels it was filed under | `fish:snapper` ×1 |
| mean OCR confidence | 0.7892 |
| example image ids | `7f7ffff8c0fcf8c0` |
| OCR variants seen | `NeylItem<crimson Snapper>` ×1 |


## `sdarkskeletalshark`

| property | value |
|---|---|
| rows | 1 |
| compound rows | 1 |
| sessions / days | 1 / 1 |
| canonical candidates | `fish:shark` ×1, `fruit:dark` ×1 |
| entity fragments found | `dark -> fruit:dark`; `shark -> fish:shark` |
| stored labels it was filed under | `fruit:yami` ×1 |
| mean OCR confidence | 0.8333 |
| example image ids | `3e989edf47464242` |
| OCR variants seen | `New Item sDarkskeletalshark-` ×1 |


## `sdarksketetalshark`

| property | value |
|---|---|
| rows | 1 |
| compound rows | 1 |
| sessions / days | 1 / 1 |
| canonical candidates | `fish:shark` ×1, `fruit:dark` ×1 |
| entity fragments found | `dark -> fruit:dark`; `shark -> fish:shark` |
| stored labels it was filed under | `fruit:yami` ×1 |
| mean OCR confidence | 0.8092 |
| example image ids | `602828b8c8fefefe` |
| OCR variants seen | `New Item sDarkSketetalshark?` ×1 |


## `sgolden ribbonangelfish`

| property | value |
|---|---|
| rows | 1 |
| compound rows | 1 |
| sessions / days | 1 / 1 |
| canonical candidates | `fish:angelfish` ×1, `fish:golden` ×1 |
| entity fragments found | `golden -> fish:golden`; `angelfish -> fish:angelfish` |
| stored labels it was filed under | `fish:angelfish` ×1 |
| mean OCR confidence | 0.8455 |
| example image ids | `602021a0c0fefefe` |
| OCR variants seen | `New Item sGolden RibbonAngelfish-` ×1 |


## `spooksville astuo candy cornsquid`

| property | value |
|---|---|
| rows | 1 |
| compound rows | 1 |
| sessions / days | 1 / 1 |
| canonical candidates | `fish:squid` ×1, `fruit:uo` ×1 |
| entity fragments found | `uo -> fruit:uo`; `squid -> fish:squid` |
| stored labels it was filed under | `fish:squid` ×1 |
| mean OCR confidence | 0.541 |
| example image ids | `00000003437fffff` |
| OCR variants seen | `SPOOKSVILLE ASTUO Nev Item<Candy CornSquid>` ×1 |


## `spooksville astuo crimson snapper`

| property | value |
|---|---|
| rows | 1 |
| compound rows | 1 |
| sessions / days | 1 / 1 |
| canonical candidates | `fish:crimson` ×1, `fish:snapper` ×1 |
| entity fragments found | `uo -> fruit:uo`; `crimson -> fish:crimson`; `snapper -> fish:snapper` |
| stored labels it was filed under | `fish:crimson` ×1 |
| mean OCR confidence | 0.532 |
| example image ids | `00000003c37fffff` |
| OCR variants seen | `SPOOKSVILLE aSTUO Nev Item<crimson Snapper` ×1 |

