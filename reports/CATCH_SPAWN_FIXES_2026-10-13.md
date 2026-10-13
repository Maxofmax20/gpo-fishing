# Fish count and spawn observation repair

Local logs repeatedly show "Nev Item Shark", "Nev Item <Exotic Tiserfin»", "Nev Item czebra Ribbon Angelfish" and "—uev 'Item-cGndy.Com Squid", followed by an unverified-catch message. The parser previously recognized the exact "new item" title, so these completed catches never reached the confirmed accounting path or local fish-name model.

Catch verification now recognizes a bounded set of observed two-word title variants. It retains failure precedence and does not establish success from a fish/fruit name or a spawn announcement alone. An incomplete "Nev i" stays unverified. Confirmed accounting is shared through one production helper and still occurs once per catch.

Verification previously silently reduced configured catch-read gaps to 40 milliseconds and used only one attempt when fruit features were off. It now honors the configured gap and retry count for fish accounting too, avoiding an unnecessary wait after the final read.

Spawn observation previously ran both inside the waiting-for-bite loop and in a background watcher. It also called Gemini synchronously while holding a settings read lock. The control-loop observer was removed. Local spawn events/statistics are emitted immediately by the existing watcher, and optional Gemini enrichment is carried with the notification job to its worker. The worker snapshots Gemini settings before any network call. It cannot revise fishing actions, catch accounting or fruit protection.

Catch and spawn clauses are scoped independently even when OCR merges them onto one line. Spawn subjects cannot become caught fruit names, and bracketed catch names cannot become spawn names. Original pity evidence remains available for protection.

Local Rust library regressions passed: 495 passed, two existing ignored. New replay tests verify the real logged catch title, one confirmed catch, configured settling delay with fruit features disabled, no spawn capture on the fishing control thread, and unchanged tracking state/counters with zero input events after a spawn observation. A queue test enables Gemini with dummy credentials but starts no transport worker, confirming enrichment context is handed off without external requests. Existing storage/pity and notification regressions remain included. These are fake-input/replay tests; live-game behavior and real message delivery were not exercised.

The logs also contain a separate pause after an unconfirmed fruit-storage outcome. That safety behavior and the user's protected-catch policy are preserved. A fruit-spawn announcement is an observation, not a caught fruit or a storage request.

The v5.8.4 release gate exposed a pre-existing temporary-directory collision between concurrent training-projection integration tests: timestamp-only directory names allowed one test to remove the other's reviews. The fixture now includes a process ID and atomic sequence number. All three projection tests and 30 repeated concurrent runs passed locally. Main CI on the runtime repair also passed. The failed v5.8.4 release was not published; the final release is v5.8.5.
