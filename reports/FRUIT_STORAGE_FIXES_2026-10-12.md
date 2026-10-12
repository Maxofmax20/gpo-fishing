# Fruit storage, pity protection, and notifications — 12 October 2026

Implemented the approved repair plan. No saved settings, credentials, training images, reviews, or model weights were changed by this repair.

## Behaviour

- Fruit handling uses local OCR. Gemini requests and message rewriting no longer block catch/storage control.
- Original catch text supplies pity independently of name-scoped text. Detected zero stays attached to the catch and its policy. Spaced ratios and generic Devil Fruit catches are supported.
- As requested, unreadable pity is treated as nonzero. An unknown species name alone does not protect an otherwise unprotected, recognized fruit catch.
- Store observation honors configured settling and observation delays. Duplicate/full-storage evidence is separate from a confirmed ground-drop banner.
- The workflow sends at most one Backspace press, only on an unprotected duplicate/full-storage outcome. Positive storage/drop confirmation stops storage inputs. A drop that remains unconfirmed pauses for inspection.
- Protected fruit that cannot be stored is kept without Backspace and pauses. If the game reports that a protected fruit has already dropped, the app reports the actual drop and pauses; it does not falsely claim that the fruit was kept.
- Every started storage action receives a terminal event, including cancellation, missing configuration, and focus loss. The dashboard exits Storing fruit before screenshot encoding or queued notification delivery.
- Pity-zero storage outcomes qualify for legendary-only fruit alerts even when the name/rarity is unknown. Notification eligibility uses the same catch context as protection. Alerts name the actual stored/kept/dropped/failed/unknown/cancelled outcome.
- Telegram photo failures fall back to text. Long photo captions become a separate text message. Delivery failures are redacted and forwarded to the app log.
- Post-catch handling no longer increments or records a fruit that the confirmed-catch path already counted.
- Settings descriptions explain unreadable-pity handling, protected-fruit pausing, and pity-zero notification eligibility.

## Verification

- Full Rust suite: **540 passed, 0 failed, 3 ignored** (484 unit + 56 integration tests).
- Frontend: **137 passed**. TypeScript and production build passed.
- Clippy completed successfully, with 62 warnings. This repair does not claim a warning-free repository.
- Desktop rebuild passed: `npm run tauri -- build --debug --no-bundle -- --jobs 1`. Updated executable: `src-tauri/target/debug/gpo-autofish.exe` (frontend embedded; no installer generated).
- Git whitespace check passed.

Regression evidence is in `fruit-storage-rust-tests.log`, `fruit-storage-frontend-tests.log`, `fruit-storage-frontend-build.log`, `fruit-storage-clippy.log`, and `fruit-storage-desktop-build.log` in this directory.

Tests use scripted OCR/capture, recorded input, and fake notification transports. They do not send actual game inputs or Telegram messages. No live-game storage test has been performed. The existing no-failure-banner store-success contract remains in use after completed configured store clicks; tests do not independently establish that game contract.

Fish recognition remains the existing advisory model and fruit recognition remains OCR. No model was retrained for these repairs.
