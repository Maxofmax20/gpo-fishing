# GPO Autofish v6.0.4 — Full v5.9.0 Rollback Base

This is an **official in-app upgrade number** carrying the **original v5.9.0 full application source** as its starting point. The higher version number is needed so installations already on v6.0.3 can receive the rollback using the cryptographically signed updater.

## Restored from the exact v5.9.0 tag
- Complete v5.9.0 fishing behavior (cast, bait, rod handling, catch/recast timing, catch tracking and fruit-storage rules).
- Complete v5.9.0 desktop/HUD/website UI, navigation and settings behavior; no transplanted v6 experimental controller or menu-detection logic.
- v5.9.0's plugins, storage layout, optional functionality, and default settings.

## Small, isolated improvements on that baseline
- Stop/Pause UI commands now finish fishing thread cleanup **before reporting success**. This removes the Stop-then-Start race without modifying the v5.9.0 fishing algorithm.
- Before parsing and potentially rewriting the older settings format, the app makes a one-time, non-overwriting local backup of existing `settings.json`, `stats.json`, and `catches.csv` in `backup-before-v590-restore`. These files remain on your PC.
- Added regression tests for stopping and restarting from Recovering, and original user-data preservation.

## Verification and limits
CI must pass Windows Rust, controller, Python, frontend and browser checks, and the signed Windows updater artifact must be validated before publication. Actual Roblox gameplay is **not verified** from the build server. Validate multiple catch/store/recast cycles on the Windows PC. The v6-only fields are preserved in the backup but not guaranteed to remain editable in the older v5.9.0 settings schema.
