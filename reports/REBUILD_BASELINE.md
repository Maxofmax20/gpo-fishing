# v5.9.0 auxiliary-service rebuild

Fishing baseline: tag v5.9.0, commit 58b6b6b. Controller and minigame vision remain unchanged.
Original reference: arielldev/gpo-fishing at b2efab6f99b1905d64e5161772a2ee90257e3fab (MIT attribution retained).

Replaced: recorder/player implementation, web HTTP service/dashboard, notification delivery and Telegram command loop.
Preserved: identity, updater public key, private settings format, existing recordings, calibrations, models and presets.

Recorder: timestamped overlapping inputs, legacy game-recording import, finite/infinite loops, speed control, explicit failures, cancellation epochs, held-input release and foreground guards.
Remote: bounded authenticated workers, dedicated max-two MJPEG streams, capture priority for fishing, measured delivered FPS, 20/30/60/120 requested limits, visible Jump, WASD, hotbar, pointer input and recorder controls.
Notifications: bounded queue, sanitized network errors, per-provider acknowledgement, capped image encoding, pity-zero-only filtering and post-workflow screenshots.

Compatibility limitations:
- Legacy recordings containing VPN actions are preserved but rejected explicitly by the new game-input player. Separate VPN controls remain available.
- Old web page routes redirect visually to the new single dashboard; removed broken advanced web controls are not simulated.
- FPS choices are requested ceilings. Delivered FPS depends on capture/encoding, network, Roblox and hardware, and is shown separately.
- Windows gameplay, Telegram phone receipt and long AFK soak require the candidate installed; automated tests do not prove those.
- The running app and private settings were not changed.
