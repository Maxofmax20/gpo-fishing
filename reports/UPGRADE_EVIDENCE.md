# Upgrade evidence

Repair source: f58a640. CI run: 40106175136, all applicable quality jobs passed.
Capture geometry: 7 standalone tests passed; Windows capture/OCR cross-check and Clippy passed.
OCR request gate: 4 standalone tests passed, including delayed cancellation and stale result discard.
Reusable gpo-upgrade-team skill: validator passed, independent read-only forward test passed.
VPS-only full Tauri test was stopped: GTK/WebKit SDKs are not installed; Windows CI executes full app tests.
Running laptop app unchanged. Candidate native gameplay, display behaviour and 2h/8h AFK soak remain pending manual installation.

Performance thresholds in the plan are targets, not measured achievements. Existing production compiler settings remain unchanged until controlled Windows benchmarks justify an alternative.
