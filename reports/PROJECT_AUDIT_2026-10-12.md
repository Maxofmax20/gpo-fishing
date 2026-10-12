# Project audit — 12 October 2026

## Result and scope

The frontend builds, and the frontend and Python tests exercised in this audit pass. There are nevertheless important correctness defects in training, evaluation, settings persistence, bot lifecycle, and local HTTP writes. Fix these before investing in additional model training or a visual redesign.

This is an audit, not an implementation: application source, settings, dataset labels, model weights, and review records were not changed. The user selected a whole-project audit and improvements to the existing UI rather than a redesign.

Coverage: React/Tauri interfaces, shared controls, settings and hotkeys, fishing state machine and input, training supervisor, Python trainers and evaluation, candidate admission/promotion, deployed model metadata, VPN state, both local HTTP surfaces, packaging, documentation, and CI. Visual inspection was limited to the existing application's VPN page; subsequent captures were unavailable. UI observations elsewhere are source-based. Live fishing, network/VPN changes, training jobs, model promotion, updates, and notification delivery were not exercised.

Evidence labels below distinguish a reproduced defect from a source-confirmed path and an outstanding verification need. P1 means fix first because incorrect state, input, persistent data, or trust decisions are affected. P2 means a functional, usability, or maintenance issue that should follow.

## Current model facts

- Both the bundled and local deployed fish manifests name revision **4**, contain **15 canonical classes**, and report **361 test examples**, **7 test sessions**, **accuracy 0.9889197**, and **macro-F1 0.9866436**.
- The state manifest reports revision 1, three classes, macro-F1 1.0, and 391 test examples.
- A local candidate registry exists. The README's claims of eight classes, 0.5824 accuracy, and no registry are stale.
- These are recorded held-out metrics, not a live-game accuracy measurement. The confidence and comparison defects below affect what conclusions can safely be drawn from those records. There was no finding of proven train/test leakage in the existing reviewed dataset: its committed audit reports no byte-identical or pixel-identical groups crossing splits. Perceptual-hash similarities require image review and are not proof of leakage.
- Model inference remains shadow/advisory code. No change should let model predictions control the fishing macro as part of this repair pass.

## Findings

### Training and model evaluation

**F01 — P1 — Training can report available and then fail to import its trainer.** Source-confirmed and import failure reproduced.

`src-tauri/src/core/training.rs:774,833,1218` and `src-tauri/src/commands_training.rs:52`: backend validation requires a repository root containing `ml/gpo_train/fish_train.py`, but launch uses that root as the working directory and executes `python -m gpo_train.fish_train`. The package resides under `ml`, and no Python search-path adjustment is made. `gpo_train` was not importable from this project root in the audit environment. A separately installed package or inherited PYTHONPATH can mask the defect.

Fix: use repository-qualified modules (`ml.gpo_train.fish_train` and `ml.gpo_train.train`) with the repository root as cwd. Probe imports using the same executable, cwd, module, and environment as launch, including required exporter/runtime dependencies. Preserve frozen dataset/image environment overrides. Do not switch blindly to the v2 manual trainer, which uses a different input contract.

**F02 — P1 — Rejection-threshold calibration applies softmax twice.** Reproduced using the actual helper extracted without importing the training stack.

`ml/gpo_train/fish_train.py:167,353` and `fish_train_v2.py:290`: callers pass calibrated probabilities to `pick_rejection_threshold`, which treats them as logits and applies softmax again. Test retention compares the returned threshold with singly calibrated probabilities. Example: correct `[0.99,0.01]` and incorrect `[0.65,0.35]` yield threshold 0.60 and reported validation kept-accuracy 100%, but the test rule retains both with 50% accuracy. Also, when no threshold satisfies the target, the helper returns its default zero threshold instead of reporting no valid operating point.

Fix: define the helper explicitly over calibrated probabilities, transform logits once, use identical validation/test/runtime retention logic, and represent an unsatisfied target as unavailable. Recompute rejection metadata after correction; the ordinary argmax accuracy need not change. Add known-confidence and unsatisfiable-target regression tests.

**F03 — P1 — Missing per-class evidence bypasses comparison checks.** Source-confirmed.

`src-tauri/src/core/registry.rs:57,522`: support is parsed only from `per_entity`, although F1 is accepted from both `per_entity` and `per_class`. Regression checks visit only classes for which both models supply F1, and thin-support checks run only for support strictly between zero and ten. Missing F1/support and explicit zero support can therefore avoid the advertised evidence floor. Missing rejection/calibration files are also accepted, and the kept-accuracy check is conditional on both sides having that value.

Fix: require finite, in-range core metrics and per-class F1 plus minimum test support for every required class before a comparison can Pass. Read support consistently from supported trainer shapes. Old records with unknown evidence should be Inconclusive until evaluated, rather than silently judged. State explicitly which optional metrics are advisory and which are admission requirements.

**F04 — P1 — Candidate improvement is calculated across potentially different test sets.** Source-confirmed contract gap.

`src-tauri/src/core/registry.rs:38,522`: comparison subtracts saved metrics and checks test example/session counts, without matching test IDs, labels, image hashes, or split identity. An easier new holdout of equal or greater size can appear to improve the model.

Fix: compare incumbent and candidate on one frozen, versioned holdout and carry its fingerprint in both evaluations. A mismatch must be Inconclusive. Keep model selection and threshold tuning on validation; do not use repeated final-test results to tune training.

**F05 — P2 — The separate v2 CLI trusts its input manifest too much.** Source-confirmed boundary gap; no claim that the current manifest is corrupt.

`ml/gpo_train/fish_train_v2.py:132–169`: checks cover vocabulary width, nonempty splits, and total examples, but do not independently enforce disjoint sessions, TRAIN examples per class, or verify the supplied dataset/vocabulary fingerprints. Output directories also reuse an existing run ID and overwrite artifacts.

Fix: share the existing frozen-input validation contract with the manual trainer, verify integrity and session purity before training, require TRAIN/VAL/TEST support, and reject accidental run-directory reuse. Keep explicitly permitted diagnostic runs distinguishable from promotion-eligible runs.

### Application state and fishing reliability

**F06 — P1 — Settings failures are hidden and concurrent whole-settings saves can diverge.** Source-confirmed.

`src/lib/store.ts:161` optimistically changes settings, catches errors without rethrowing, and does not restore or refresh them. Ordinary pages do not surface the stored global error. `src-tauri/src/commands.rs:67` changes backend memory before persistence and hotkey registration succeed. Multiple full-blob writers can overwrite unrelated changes; each finally block independently clears the single saving flag.

Fix: serialize settings mutations, apply them to the latest acknowledged settings, return normalized settings from the backend, and keep memory/file/UI consistent. Surface save failures and allow dependent actions such as Enable collection + Start to stop on rejection. Use a revision/conflict check for independent windows or remote writers.

**F07 — P1 — Failed hotkey rebinding can remove working shortcuts.** Source-confirmed.

`src-tauri/src/hotkeys.rs:17` unregisters all shortcuts before parsing and registering replacements. A malformed, duplicate, or unavailable binding can leave a partial set; settings may already have been saved. This includes emergency-stop access.

Fix: validate the full proposed binding set first, reject duplicates, and restore the previous registered set on any registration failure. Commit settings only with a consistent binding outcome. Test failure on a later binding, not only the first.

**F08 — P1 — Start, stop, recovery, and disconnect are not one serialized lifecycle.** Source-confirmed concurrency/state defects.

`src-tauri/src/bot/mod.rs:75,124,139,267` and `bot/machine.rs:452`: start uses a check followed later by setting running; commands spawn independent threads. Two starts can both proceed. A watchdog recovery sleeps during backoff and then unconditionally starts another loop, allowing a stop during backoff to be undone. Disconnect can set the displayed state to Paused and return while leaving running true and the Bot paused flag false. The UI and watchdog then disagree about whether work is active.

Fix: serialize lifecycle requests, use a run generation/cancellation token, cancel recovery when the generation changes, and finalize every loop exit through one state/session/input cleanup path. Test simultaneous start requests, stop during backoff, and disconnect without reconnect.

**F09 — P1 — Foreground checks do not protect the whole reel or action sequence.** Source-confirmed; live unintended-input behavior was not exercised.

`bot/machine.rs:497–619` checks liveness inside the tracking loop but does not recheck foreground before `hold_mouse`. `bot/actions.rs` sends multi-step input after an earlier focus check. Alt-Tab during a reel or shop sequence can therefore let later input reach another foreground application.

Fix: immediately release held input on target-window/focus loss and wait without advancing the sequence. Guard input operations against the current target and active run generation, including remote/recorded macro paths. Test focus loss after mouse-down and between action steps with a mock input backend.

**F10 — P2 — Unknown catches inflate confirmed fishing statistics.** Source-confirmed.

`bot/machine.rs:110–135` handles every verdict except Failed as success, including Unknown. It increments fish, pity, purchase progress, and journal entries when OCR is unavailable or inconclusive, even though the log adds an unverified tag.

Fix: track unverified outcomes separately from confirmed catches, make journal/status wording explicit, and define whether scheduling uses completed reels or confirmed catches. Preserve completed-reel behavior where needed without labeling uncertain evidence as confirmed fish.

### Local HTTP and URL validation

**F11 — P1 — Review write endpoints lack the claimed CSRF protection.** Source-confirmed; foreign-origin text/plain acceptance reproduced against the extracted parser on an isolated temporary listener. No malicious write was sent to the live service.

`src-tauri/src/ontology_web.rs:176–240,308,441`: Request retains no Origin, Host, or Content-Type. The decision, undo, and label-confirmation routes deserialize JSON from any POST body. A valid JSON body sent as `text/plain` is not rejected. Omitting CORS response headers does not itself prevent write side effects. This matters especially for `/api/labels/confirm`, which writes human reviews in the app data store, rather than only an ontology proposal.

Fix: enforce the local Host and same-origin write policy, require the expected content type and a session CSRF credential for mutation routes, and reject unauthorized requests before reading or acting on input. Add foreign-origin, absent-credential, and simple-content-type tests using isolated fixtures. Browser local-network restrictions may reduce some attack paths but are not a substitute for server checks.

**F12 — P2 — String-prefix URL checks and byte slicing accept invalid boundaries.** Reproduced with extracted production helper functions.

`bot/web_server.rs:132,169,247`: `http://localhost.evil.example` passes the loopback CORS check. A malformed multibyte header can panic when sliced at an arbitrary byte boundary. `bot/machine.rs:197` accepts a VIP URL containing `roblox.com` anywhere, including `https://roblox.com.evil.example/games/123`. Dashboard token authentication still exists; these findings do not demonstrate a token bypass or arbitrary shell execution.

Fix: use parsed host/origin boundaries and exact allowlists; parse headers without unchecked UTF-8 slicing. Reject oversized/incomplete requests rather than silently truncating declared bodies. Bound concurrent clients/streams to avoid unbounded thread growth.

### Scheduling, VPN, updates, and packaging

**F13 — P2 — Automatic training consumes triggers before it can launch a job.** Source-confirmed.

`src-tauri/src/commands_training.rs:1228–1263` advances the fingerprint, row/session counters, readiness state, and cooldown before checking auto-enabled, deferral, or later backend eligibility. Small additions are measured since each check rather than accumulated since a successful launch; a deferred coverage/new-data trigger can disappear at the next unchanged-data check.

Fix: distinguish observed state from consumed training state, persist pending triggers, accumulate additions, and consume a trigger/cooldown only after a job is accepted. Test additions spread over several ticks, fishing deferral, backend failure, and a retry without new data.

**F14 — P2 — VPN disconnect verification confuses service lifetime with tunnel state.** Source-confirmed and consistent with the existing visible WARP timeout.

`src-tauri/src/vpn.rs:1142`: WARP disconnect calls its CLI and then waits for `warp-svc.exe` to disappear. The background service can remain running while disconnected. The check also looks for any tunnel rather than only the selected engine. Other external engines have an empty expected-process list, allowing success without proving their tunnel is gone. `wait_for(false)` checks only the connected boolean, so Error or Unknown can satisfy an intended verified-disconnected wait.

Fix: verify engine-specific disconnected evidence; for WARP use CLI/tunnel state while allowing the service to stay running. Treat unknown/error evidence as unverified, and make timeout diagnostics describe the observed problem. In `VpnPage.tsx:60`, stop polling from overwriting the user's chosen next engine before they connect.

**F15 — P2 — Collection-start behavior depends on which button is used.** Source-confirmed.

`src/pages/Dashboard.tsx:42`, `windows/Hud.tsx:89`, and `src-tauri/src/hotkeys.rs:51`: only Dashboard Start implements the collection prompt/preflight. HUD, hotkey, and remote paths call the bot directly. Turning collection off also prompts again on each fresh dashboard start.

Fix: persist the collection preference explicitly, centralize required preflight/start policy in the backend, and have every entry point report the same outcome. Keep ordinary fishing available with collection disabled and make the active recording state visible.

**F16 — P2 — Runtime numeric settings lack general backend validation.** Source-confirmed.

`src-tauri/src/config.rs:1027` clamps training thresholds but does not validate all runtime timings and regions. Several paths pass user-controlled float values directly to `Duration::from_secs_f32`, including catch waits, OCR scan intervals, watchdog heartbeat, and recovery backoff. Negative values from imported/hand-edited settings can panic despite UI slider limits.

Fix: validate finite, bounded runtime values and relative geometry at every load/save/import boundary, with actionable field errors or documented safe normalization. UI constraints alone are insufficient.

**F17 — P2 — Update check installs immediately, and failed banner installation cannot retry.** Source-confirmed.

`src/pages/Settings.tsx:63,381` labels an action Check but downloads and installs as soon as an update is found. `src/windows/Panel.tsx:90–100,177` stores an error in the same updating variable used to disable the install button, leaving it disabled after failure. Dismissal also persists across subsequently detected versions.

Fix: separate checking, available, downloading, installing, and failed states; show version/notes and an explicit Install action; restore Retry after errors; scope Later to the dismissed version. Retain signature verification.

**F18 — P2 — Installed review pages depend on checkout-relative files.** Source-confirmed packaging gap; clean-installer behavior remains to be verified.

`src-tauri/src/ontology_web.rs:71,603` searches for a physical `dist` tree. Workstation data searches checkout-relative reports, and label batches/thumbnails are also repository artifacts. `src-tauri/tauri.conf.json` lists only models as extra bundle resources. Embedded Tauri frontend assets do not automatically supply the separate HTTP server with a filesystem `dist` directory.

Fix: serve the built frontend from embedded assets or explicitly packaged resources, use known app resource/data roots, and package the supported review reference artifacts. Keep writable decisions in app data. Check an installation launched from an arbitrary working directory without the source checkout; incomplete optional data should produce an actionable setup screen.

**F19 — P2 — Background work can stall or overwhelm the UI and fishing loop.** Source-confirmed execution paths; no latency profile was taken.

`commands_training.rs:452` still performs backend probing and dataset/snapshot work in a synchronous command. `src/lib/useVisiblePoll.ts` does not prevent overlapping asynchronous polls or late stale responses. Training Center refresh groups use Promise.all, so one required read failure discards successful results. `bot/machine.rs:571` calls shadow inference and event-file writing synchronously during tracking.

Fix: move heavyweight commands to bounded workers, permit at most one poll per resource, ignore outdated responses, and show partial/error states per panel. Run shadow observation through a bounded worker queue with immutable frame/revision context; dropped observations must be counted and must never affect input decisions. Measure loop timing before and after.

**F20 — P2 — Atomic individual model files are not an atomic model deployment.** Source-confirmed crash window.

`src-tauri/src/core/registry.rs:723,756` replaces weights before writing the corresponding manifest. A crash between those operations leaves a mismatched pair; checksum verification correctly refuses it, but there is no single committed deployment pointer covering both files. Seeding also writes the pair individually.

Fix: stage and verify both files in a versioned directory, atomically switch the active revision pointer, and keep the previous verified revision available. On startup recover interrupted operations without silently downgrading or relabeling telemetry. Test interruption between each deployment step.

### UI/UX and delivery quality

**F21 — P2 — Shared controls are incompletely accessible.** Source-confirmed.

`src/components/ConfirmModal.tsx` lacks dialog semantics, focus trapping/restoration, and Escape handling; backdrop dismissal also ignores busy. `primitives.tsx` switches have no accessible label contract, and CustomSelect lacks listbox semantics and arrow-key/ Escape selection behavior. Several icon-only window/dashboard controls lack accessible names. Toasts have no live-region announcement.

Fix: accessible shared dialog/select/switch contracts, labeled icon controls, clear keyboard focus, status announcements, and guarded dismissal during noncancelable work. Add keyboard tests for actual user flows and modal focus behavior.

**F22 — P2 — Readability and navigation need refinement within the existing design.** Visual observation plus source review.

The visible panel allowed substantial background text to show through. `src/index.css` uses transparent page roots and an 86%-opaque glass background, while muted text is small and low-contrast. The default 460-pixel panel has nine icon navigation items; Training has fifteen wrapped tabs plus dense metrics. The visible VPN page displayed a stale disconnect error next to current tunnel evidence, leaving the next action unclear.

Fix: use an opaque main content background while retaining the compact HUD style; increase secondary-text contrast and readable text size; group Training navigation into Overview, Data & Review, Jobs & Models, and Settings without dropping existing pages; remember the selected subpage. Use a roomier Training layout within the resizable panel, with a compact fallback. Present current status, last-action error, and next action as separate clearly named information. Validate minimum width, normal width, and Windows scaling rather than claiming visual QA from code alone.

**F23 — P2 — Model/version documentation is misleading and contains unsupported certainty.** Source-confirmed.

`README.md:5–6,321–337` is inconsistent with app version 5.7.1, revision-4 model metadata, and the existing local registry. The security section has a paragraph accidentally joined across unrelated topics. SOFTWARE COMPLETE and blanket safety claims overstate the evidence, given the defects above.

Fix: separate historical release notes from current status, source current class/version facts from maintained artifacts, and date any local dataset/readiness snapshot. Explain model scope, shadow-only behavior, and evidence limits without presenting one developer's app-data state as a universal installation state.

**F24 — P2 — CI does not run the frontend or Python regression suites.** Source-confirmed.

`.github/workflows/ci.yml` and release quality gates build TypeScript and run Rust tests, but do not run npm test or the Python training/ontology/OCR tests. Dependency installation uses npm install rather than a lockfile-enforcing npm ci. A stub-trainer integration test does not prove that the configured production Python module is importable.

Fix: run frontend, Python, web gesture, and Rust unit/integration suites in CI; add the production-module import check; use npm ci and pinned Python requirements for reproducibility. Separate report-only legacy lints from explicit enforceable checks on changed code. Never print matching secret values in security checks.

## Repair sequence and acceptance criteria

1. **State/input and write integrity:** F06–F08, F09, F11. Settings/file/registered keys must agree after failure; one macro loop may exist; Stop cancels recovery; focus loss releases held input; foreign write requests cannot create review records.
2. **Trustworthy model workflow:** F01–F05, F13, F15, F20. Launch from a clean checkout without inherited PYTHONPATH; use one confidence transform; missing support blocks admission; compare on identical frozen holdouts; pending jobs survive deferral; promotion and restart preserve a verified artifact/manifest pair.
3. **Remaining reliability:** F10, F12, F14, F16–F19. Unknown evidence has honest counters; malformed requests do not panic; WARP service can remain alive after verified disconnect; invalid runtime values are rejected; updates can retry; installed review pages work without checkout files; work queues/polls stay bounded.
4. **UI and maintenance:** F21–F24. Keep existing features/style; make main content readable, keyboard accessible, and usable at minimum width and 100/125/150/200% scaling; run all meaningful suites in CI; synchronize current documentation with artifact facts.

Do not relax review/data/evaluation/shadow gates, enable production model control, replace the state model merely because its score is perfect, or retrain the fish model just to regenerate reports. Correct evaluation metadata and compare verified artifacts first.

## Checks completed and limitations

- `npm test`: **127 tests passed** across three files.
- `npm run typecheck`: **passed**.
- `npm run build`: **passed**.
- `python -m pytest ml/gpo_train/tests ml/gpo_ontology_review/tests ml/gpo_ocr_audit/tests ml/ram_plus/test_worker.py -q`: **229 tests passed**.
- `node --test src-tauri/src/bot/web/assets/gpo-touch.test.mjs`: **7 tests passed**.
- `npm audit --json`: **0 reported vulnerabilities** in the resolved npm dependency tree at audit time. This does not cover Rust/Python dependencies or application security.
- Targeted extracted-production-helper probes reproduced F02 and F12 and verified the parser acceptance in F11 without changing application data.
- Rust validation: **not completed**. `cargo test --lib --locked` remained in the cold dependency build; the resumed `-j 2` build also remained in Windows/tract compilation and was stopped. No Rust test pass or source compilation failure is established by these interrupted builds. Re-run unit and integration tests before implementing or shipping repairs.
- No live fishing, training, promotion, update installation, VPN operations, webhook delivery, or clean-installer execution was performed.

Passing existing tests is evidence about covered behavior, not proof that the findings above are absent. Several missing checks are specifically identified for new regression coverage.
