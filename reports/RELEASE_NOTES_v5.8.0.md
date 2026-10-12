# GPO Autofish v5.8.0

- Added live local AI fish naming using the model trained on 1,852 reviewed collected samples (15 species), with a visible Features toggle. Recording is not required.
- Fixed deployed model preprocessing: the unchanged fish weights now match their original Python holdout result, 357/361 correct (98.89%), in the Rust runtime.
- Separated catch names from overlapping fruit-spawn announcements. Fruit names and unsupported fish species continue through OCR; no fruit CNN is claimed.
- Latched pity-zero protection throughout storage, removed the second drop keypress and synchronous cloud analysis from storage, and show actual terminal storage outcomes. Unconfirmed destructive outcomes pause.
- Fixed pity-zero alerts with unknown names and legendary-only filtering, Telegram photo/text fallback and delivery-error logging. Count fruit catches once.
- Improved settings persistence, updater checks, keyboard accessibility, bot cancellation/focus guards, local review-server validation, training evaluation/provenance, model deployment recovery and VPN verification.

AI names cannot establish catch success or decide fruit storage/drop actions. Unreadable pity follows the requested nonzero fallback. Existing settings, credentials, samples and model weights are preserved. Automated replay/regression tests do not establish live-game behavior; game inputs and real Telegram delivery were not used for verification.

The Windows installer and updater manifest are published only after regression checks and updater-signature verification.
