# Rebuild foundation
User-selected baseline: v5.9.0, commit58b6b6b.
Original upstream reference: arielldev/gpo-fishing main b2efab6f99b1905d64e5161772a2ee90257e3fab.
The fishing controller remains unchanged. Recorder/player, web controls and Telegram are being replaced by independent, tested services sharing a typed input contract.
Preserve app identity, calibration, saved macros, presets, models and credentials. No running app is stopped or modified during development.
