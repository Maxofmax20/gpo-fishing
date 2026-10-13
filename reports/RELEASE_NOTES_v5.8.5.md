# GPO Autofish v5.8.5

- Fixed stopped fish counters when Windows OCR reads a catch banner as "Nev Item", "uev Item" or the observed "itemv" variant. Confirmed catches are counted once; spawn-only announcements and unreadable text remain separate.
- Catch verification now uses the configured retry count and delay even when fruit storage and fruit alerts are disabled, giving incomplete first readings time to settle.
- Removed spawn capture/OCR from the fishing control loop. The existing background spawn observer handles announcements, while optional Gemini message enrichment runs in the notification worker without holding settings locks during network requests.
- Isolated catch and spawn clauses even when OCR merges both into one line, so the spawned fruit cannot be mistaken for the caught fish.
- Added regressions for real logged OCR text, catch accounting, spawn-only state/counter/input invariants, observer isolation and asynchronous enrichment dispatch.

Includes the v5.8.2 desktop edge/startup and web dashboard repairs. Existing fruit protection, storage settings, credentials, collected samples and model weights are preserved. Automated replay and regression tests do not establish live-game behavior.
