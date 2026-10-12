# v5.8.0 validation

Local checks on 12 October 2026:

- Rust: 544 passed, zero failed, three intentionally ignored child/scenario tests. All 15 deployed model classes resolve in the fresh-install recognition catalog. Fruit, spawn-only text, unknown items, failed catches and unsupported species keep OCR recognition.
- Frontend: 137 passed; TypeScript/Vite production build passed.
- Python: 236 passed; four expected exporter deprecation warnings.
- Dashboard gestures: seven passed. npm audit: zero vulnerabilities. Version gate: all manifests and v5.8.0 agree.
- Clippy completed successfully in report mode; existing warnings remain.
- The unchanged fish weights retain SHA-256 5d49eb063ff209566152d000707f064299d1d92a477bbf4ab5f67f718508014a. Their previous real Rust-runtime replay measured 357/361 correct (98.89%) on the original frozen holdout. No retraining occurred.
- One existing workstation wall-clock benchmark exceeded its 250 ms bound under load; the isolated retry and full rerun passed.

The release workflow repeats regression checks, builds the Windows installer and requires updater artifacts before publishing. Local validation did not operate Roblox, exercise live global-hotkey failures, send real Telegram messages, or install an update. Automated tests do not establish live-game accuracy or timing.

Settings, credentials, collected samples, reviews and model weights were preserved. Local evaluation snapshots and screenshots are excluded from the published source.
