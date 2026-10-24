# GPO Autofish v6.0.2 — Fishing Recovery and Restart Hotfix

- Preserve previously verified rod state across ordinary completed reels; a positively selected different hotbar slot still invalidates it.
- Replace permanent one-shot rod recovery with three spaced, bounded attempts. Reset the counter on verified success, or pause with calibration guidance instead of remaining in Recovering forever.
- Stop and Pause now await fishing-thread lifecycle cleanup before returning to the UI, preventing immediate Stop then Start from racing the old loop.
- Add tests for retained equipment evidence, bounded recovery and restarting from Recovering.
- Preserve settings, calibration, session history, protected-fruit handling and updater identity.

Requires Windows Rust/frontend/Python/controller CI and signed installer verification. Real Roblox gameplay and long AFK behavior remain unverified from the VPS.
