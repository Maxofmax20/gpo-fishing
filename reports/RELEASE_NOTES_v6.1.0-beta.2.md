# v6.1.0-beta.2 — v5.9.0 fishing baseline, rebuilt auxiliary services

This candidate starts from the v5.9.0 fishing code rather than the later fishing changes. The minigame controller and vision are unchanged from v5.9.0.

- Rebuilt the recorder/player around timestamped input events. Playback no longer depends on the fishing-running flag or the launch key remaining held.
- Added foreground checks, cancellation epochs and held-key release. Stop invalidates already queued web inputs.
- Replaced the old multi-page web implementation with one compact rounded remote, visible Jump/Space, WASD, hotbar, pointer controls and recording/playback.
- Added requested stream limits of 20, 30, 60 and 120 FPS and a separate delivered-FPS indicator. Capture yields to fishing; requested rates are not a hardware performance guarantee.
- Rebuilt bounded notification delivery, sanitized errors, provider acknowledgement and post-workflow pity-zero filtering/screenshots.
- Rebuilt authenticated Telegram commands and removed the old command dispatcher.
- Rounded the native panel and made the recorder visible outside advanced mode.
- Preserved existing credentials, calibration, signing/updater identity, models and saved recordings. Corrupt recording files are rejected before edits.

Known limits: VPN-containing legacy recordings are preserved but explicitly rejected by the new game-input player. Use the separate VPN controls. Removed broken advanced web controls are not simulated.
Real Roblox gameplay, long AFK fishing and new-build Telegram phone delivery remain unverified until this candidate is installed. Keep v5.9.0 available for rollback.
This prerelease does not replace the running app automatically.

Independent review rejected ambiguous full-screen pity evidence. Only explicit, non-conflicting Legendary Pity labels are accepted; generic and Fruit Pity values cannot trigger a zero alert.
