# GPO Autofish v6.0.1 — Cast Recovery Hotfix

Signed Windows update for the existing app, including v6.0.0-beta.1 installations. Saved settings, calibrations, catch history, and update signing identity are preserved.

- The Fishing Baits menu can stay visible during a valid cast. A visible menu alone no longer causes an immediate cast rejection and fast recovery loop.
- Cast rejection requires consecutive local evidence that a different toolbar slot is selected.
- Three consecutive confirmed rejected casts pause the bot with rod-slot and bait-calibration guidance instead of retrying forever.
- Three consecutive bait-selection errors also pause safely instead of casting without a confirmed selection action.
- Repeated bait-menu OCR was removed from the timing-sensitive post-cast observation path.
- Existing protected-fruit and Stop/Pause behavior is retained.

Automated CI and signed installer verification are required. Actual Roblox gameplay and long AFK tests remain unverified on the Linux VPS. Use the in-app updater to install v6.0.1 when it appears, or use the official signed release installer.
