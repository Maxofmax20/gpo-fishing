# GPO Autofish v6.0.3 — Restore v5.9.0 Fishing and UI

Restores the previously working v5.9.0 fishing cycle, rod-equipping logic, dashboard, HUD and panel presentation while retaining the newer app identity, signed updater, settings, catch data, and fruit-storage safeguards.

- Reverted the v6 fishing loop's strict rod-state recovery and cast rejection machinery that caused repeated Recovering after a small number of reels.
- Brought back v5.9.0's normal-reel fast reset with tracked rod selection and configured hotbar key. Stop/Pause input guards and a safe signed-update reel boundary remain.
- Restored v5.9.0 dashboard, HUD and panel layout; newer backend data, secured update workflow and specialized pages remain.
- Preserved the asynchronous UI's Stop/Pause synchronization fix: Stop returns after the old fishing thread is cleaned up, preventing immediate restart races.
- Retained protected-fruit rules, persistent settings, catch history and local calibration.

Automated Windows Rust, controller, frontend, browser and Python checks plus signature verification are required. Actual Roblox fishing gameplay on the user's Windows PC remains to be validated after installation.
