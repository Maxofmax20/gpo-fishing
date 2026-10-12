# Project repair status — 12 October 2026

This implements the findings in PROJECT_AUDIT_2026-10-12.md. The original audit remains a record of the pre-fix state. Existing trained weights, collected data and reviews were preserved. The subsequent name-recognition work records verified fish TEST metadata; see NAME_RECOGNITION_READINESS_2026-10-12.md.

Installed skills used: ML Model Training; Product Design audit and user-context workflow; Vercel React Best Practices. The existing visual design was retained.

| Finding | Implemented repair |
|---|---|
| F01 | Qualified trainer modules, equivalent production import probe, bounded single-flight caching with expiry, dependency manifests. State jobs now also produce calibration, TEST evidence and verified single-head ONNX artifacts. |
| F02 | Threshold selection consumes calibrated probabilities once; unattainable targets and empty retained sets report unavailable. Future exports explicitly use the exporter compatible with the Rust runtime. |
| F03 | Finite metrics and per-class F1/support required; both supported evaluation shapes parse support; missing evidence cannot Pass. Inconclusive comparisons preserve evaluated candidates. |
| F04 | Frozen holdout fingerprints include image bytes, labels, session IDs and ordered classes. Mismatches block improvement claims. A separate reevaluation tool measures incumbent weights on the same snapshot without retraining. |
| F05 | Reviewed manifests validate vocabulary/hash integrity, unique IDs, disjoint sessions and class support. Run IDs cannot overwrite previous runs. |
| F06 | Settings edits queue against acknowledged backend values. Conflicting saves fail visibly. Persistence precedes memory acknowledgement; normal writes hold the settings write lock. Atomic writes use unique temporary paths. |
| F07 | Parse and duplicate-check every hotkey before rebinding; failed binding/persistence restores previous shortcuts. |
| F08 | Serialize macro lifecycle transitions; generation checks cancel delayed recovery/recast; natural loop exits close sessions and stop watchdog activity. Immediate Stop also closes a session before the first loop state event. Unit tests isolate the fan-mode callback. |
| F09 | Fresh foreground checks guard gameplay actions, playback and HTTP input. Held keys/buttons release on focus loss; text input rechecks each character. |
| F10 | Unknown reel outcomes have their own counters and journal entries; confirmed catches, income and pity remain unchanged. |
| F11 | Review mutations require validated local Host, same Origin when present, JSON and a process CSRF token. Bounded workers and request limits replace unbounded connection threads. |
| F12 | Parse origins and VIP URLs by hostname; avoid unsafe byte-prefix slicing; reject oversized/truncated dashboard bodies; bounded workers and held-input expiry. Local IP reporting no longer fabricates an address. |
| F13 | Consume automatic trigger baselines only after successful launch. Completed automatic jobs are reviewed before new-trigger early exits. Concurrent ticks cannot duplicate launches; manual jobs retain manual review. |
| F14 | VPN waits require explicit verified lifecycle states. WARP disconnection permits its persistent service and checks CLI/tunnel evidence. UI polling preserves the selected engine. |
| F15 | Backend collection preflight applies to every macro start. Explicit collection-off acknowledgement persists. Start errors reach the dashboard. |
| F16 | Backend clamps timing, polling rates, watchdog delays and OCR intervals. Non-finite values cannot become infinite deadlines or busy loops; UTF-8 path truncation remains safe. |
| F17 | Shared update store separates checking from installation; explicit Install, retryable failures and version-scoped dismissal. |
| F18 | Installer resource mapping includes the review frontend. Startup resolves it from the resource directory; generated reports have an application-data location. |
| F19 | Heavy training commands use blocking workers off the command thread; polling cannot overlap; refreshes retain successful panes after partial failure. Shadow observation uses a bounded background queue with captured session identity. |
| F20 | Verify staged ONNX/manifest pairs before journaled deployment; replay interrupted commits on startup. Seeding preserves verified reevaluation metadata for unchanged weights. |
| F21 | Labels and state for shared controls, keyboard listbox navigation, modal focus containment/Escape/restore, accessible window controls and live toast announcements. |
| F22 | Opaque readable panel surface, larger minimum text, stronger muted contrast, focus outlines and reduced-motion support. Training pages use four navigation groups and remember the selected page. Missing Python shows Setup needed. |
| F23 | Current version/model figures, explicit advisory-model role, installation/data-location guidance and removal of unsupported assurance claims. Historical metrics remain historical until reevaluated. |
| F24 | CI/release use npm ci and run frontend, gesture, Python and Rust regressions; production trainer imports and dependencies are checked. |

## Verification

- Frontend production build and TypeScript checks: passed.
- Frontend regression suite: 137 passed. npm audit reported 0 vulnerabilities.
- Dashboard gesture suite: 7 passed.
- Python regression suite: 236 passed. Includes real synthetic-state ONNX export, PyTorch/ONNX agreement, single-output shape, and matching incumbent holdout metrics; no live dataset used. Four exporter deprecation warnings are expected because this exporter produces a graph supported by the current Rust runtime.
- Production Python imports: passed, including both trainer modules and export/runtime dependencies.
- Rust full suite: 518 passed, 0 failed, 3 ignored (462 unit tests and 56 integration tests passed).
- Clippy: completed successfully. Warnings remain (59 library warnings, two additional library-test warnings and seven integration-test warnings); this is not a warning-free lint result. The newly found MSRV and test-module ordering warnings were corrected.
- Git diff whitespace check: passed.
- Visual check: isolated browser fixture at 460px panel width. Grouped review navigation fits; switch/select names are exposed; keyboard selection, modal Tab containment and Escape focus restoration verified. Screenshot: UI_TRAINING_VERIFIED.png. The fixture uses mock data and cannot send game/VPN actions.

## Remaining operational checks

Live fishing, native global-hotkey registration failures, real VPN transitions, updater downloads and a clean installer launch were not exercised. Their decision paths have source and isolated regression coverage; a production smoke test is still needed. No claim is made that every possible project defect has been eliminated.

Old model manifests lack the new holdout evidence, so admission correctly remains Inconclusive until the incumbent is evaluated on the candidate's frozen snapshot. Use `python -m ml.gpo_train.reevaluate_holdout --help`. It writes a separate report by default; with the app closed, `--write-manifest` updates verified evaluation metadata only. Keep final-test evidence separate from candidate tuning.

To reproduce the visual fixture: run `npm run dev`, then open `/reports/ui-preview.html`. It is outside the production entry point and contains only mocked backend results.

## Follow-up: fish and fruit names

The user clarified that the intended recognition target is caught fish/fruits and fruit-spawn announcements. A real runtime replay exposed a downsampling mismatch missed by the original enlargement-only fixture. Pillow-compatible preprocessing raises fish TEST accuracy in the app from 42.94% to 98.89% with unchanged weights. Catch/spawn scopes prevent unrelated announcements from labeling a caught item. Full evidence and fruit-label limitations are in NAME_RECOGNITION_READINESS_2026-10-12.md.


## Fruit storage follow-up

The approved storage/pity/notification repair is implemented. See [FRUIT_STORAGE_FIXES_2026-10-12.md](FRUIT_STORAGE_FIXES_2026-10-12.md) for behaviour and limitations. Latest validation: 540 Rust tests passed, 3 ignored; 137 frontend tests passed; TypeScript/production build, Clippy, and the debug desktop rebuild passed. No live Roblox inputs or real Telegram messages were used for verification.
