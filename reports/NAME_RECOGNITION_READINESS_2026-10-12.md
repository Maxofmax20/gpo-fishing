# Fish and fruit name recognition — 12 October 2026

Scope confirmed by the user: identify caught fish/fruit names AND fruit-spawn announcements. Fishing-bar classification is outside this request.

## What was found and fixed

The store contains 35,345 images from 97 sessions. Of those, 22,420 are fishing-bar frames, 12,437 are result crops and 488 have no state label. The total image count is not the number of labeled fish or fruit examples.

The reviewed fish universe contains 1,852 usable images for 15 species, plus 12 Anglerfish images below the training gate. There are no human-reviewed fruit labels. The 229 collector-labeled fruit rows are not trustworthy training targets: 153 of the 189 Yami rows have OCR text mentioning Dark Skeletal Shark, and 35 rows mention a fruit-spawn announcement. Visual inspection confirmed examples of fish catches incorrectly tagged as fruits and announcements overlaid on catches. Original data and human reviews were preserved.

**Runtime preprocessing was the major fish-model failure.** Training used Pillow BILINEAR with antialiasing when shrinking screenshots; the app used point interpolation. On the SAME original frozen TEST images, the deployed fish weights achieved 98.89% in Python but only 42.94% through the app runtime. The corrected separable resize reproduces Pillow's widened kernel and uint8 rounding. The app now achieves 98.89% on the same 361 images, matching Python. No weights were retrained.

A new synthetic downsampling fixture tests actual result-screen dimensions (589×199 → 96×96), with tensor agreement below 1e-5. The earlier fixture only tested a small image being enlarged and missed this defect. Read-only `verify_model_runtime` exercises both deployed ONNX models through the real Rust engine; it sends no gameplay inputs.

Catch names and spawn names now have separate text scopes. An announcement about Horo above a Dark Skeletal Shark catch cannot label that catch as Horo/Yami. A Pika catch preceding a Suke announcement retains each name independently. Collector entity matching uses the caught-item scope while retaining the original OCR text for review.

## Fish status

- Trained image model: fish_v1 revision 4, 15 species.
- Original held-out TEST: 361 images from seven sessions.
- Accuracy: 98.89%; macro-F1: 0.98664; worst class F1: 0.94737 (Swordfish).
- TEST ECE: 0.10055. The previous manifest's 0.08432 was not the recomputed TEST calibration result.
- Weights SHA-256: 5d49eb063ff209566152d000707f064299d1d92a477bbf4ab5f67f718508014a.
- Frozen holdout fingerprint: df595c5817f5d0ec4ca59f08e6e3b12c901542dfc7582bb676bcbbf1703ee27d.
- The original reviewed split was preserved. The live dataset currently has different splits; 127 of its fish TEST images were in the original model's TRAIN split, so the live split was NOT used to claim unseen-model accuracy.
- Verified evidence was added to the live and bundled fish manifests. Original manifests and settings are backed up under `ml/output/readiness-20261012/backups`.
- Anglerfish is recognized by the name vocabulary but is not a trained CNN class. Its 12 reviewed images do not clear the 20-example class gate; sufficient TRAIN/VAL/TEST support is also needed before extending the model.

## Fruit status

The current application reads named fruit drops and spawn announcements with OCR and its name/alias vocabulary. That is NOT a trained fruit image classifier. No fruit ONNX model exists, and this dataset has no verified fruit labels from which to train one honestly. Wrong collector labels were not promoted into ground truth.

`reports/fruit_name_review_candidates.json` provides the 229 suspect records for inspection; it is explicitly not a training manifest or a human-review log. A source-backed vocabulary reference is in `reports/name_vocabulary_2026-10-12.json`.

A fruit image classifier needs actual examples labeled by species AND event type (caught fruit versus spawn announcement), with separate sessions for TRAIN, VAL and TEST. The vocabulary alone does not provide these examples. A generic "you got a devil fruit" banner without the fruit's name cannot identify the species from text alone.

The GPO wiki was checked for name references: [Fishing Bestiary](https://grand-piece-online.fandom.com/wiki/Fishing/Bestiary) and [Devil Fruits](https://grand-piece-online.fandom.com/wiki/Devil_Fruits). The fetched bestiary lists 16 species, including Anglerfish. Existing fruit aliases include common names such as Heal/Chiyu, Bomb/Bomu, Paw/Nikyu and Smoke/Moku. The wiki distinguishes implemented fruits from teased ones; an unverified or teased name must not be treated as a trained class.

## Configuration and use

The installed Python executable and this project root were configured as the training backend. Its dependencies and production trainer imports were checked. Shadow model inference was already enabled and remains enabled. No macro, VPN or game inputs were started.

As of v5.8.0, AI fish recognition is enabled by default in Features. The trained local fish CNN names independently confirmed fish catches using the same image that supplied the OCR evidence, even with recording disabled. Unknown items, fruit catches and unsupported known species retain OCR names. Predictions never establish catch success or decide fruit storage/drop actions. Fruit catch and spawn names still use OCR. Confidence is informational; no rejection threshold is claimed.

## Verification

- Real Rust-runtime replay: fish 357/361 correct (98.89%).
- Pillow downsampling and model loading tests: 13 passed.
- Python regression suites: 236 passed (four expected exporter deprecation warnings). Legacy state-head reevaluation regression: passed; existing multi-head state artifacts can be measured by their named state output.
- Full Rust regressions: 518 passed, zero failures, three ignored. Overlapping catch/spawn names and all 16 wiki fish names passed.
- Frontend production build: passed.
- Tauri desktop build: passed (`--debug --no-bundle`). The executable is `src-tauri/target/debug/gpo-autofish.exe`; its frontend is included. No installer was generated and no live-game smoke test was run.
- Clippy: completed successfully; existing warnings remain (59 library warnings plus test warnings).
- Git diff whitespace check: passed.

Detailed artifacts are under `ml/output/readiness-20261012`: original fish holdout manifest, evaluation evidence, before/after runtime predictions, image-case list and backups. `reports/fruit-candidate-inspection.png` is a diagnostic contact sheet, not a verified training-label source.

To use this build on this computer, run `src-tauri/target/debug/gpo-autofish.exe`. The current fish weights are already in the app model store. Features → AI fish recognition enables their live naming path; the app log records AI names, confidence and model revision. Training Center → Shadow also exposes advisory diagnostics. Fruit catch/spawn names use OCR; a trained fruit image classifier is not claimed.
