# GPO Autofish v6.0.0 beta 1

**Optional signed Windows prerelease for testing.** Your existing stable installation can remain unchanged. This is not yet the automatic stable update channel.

## Fishing recovery
- Fix a common post-fruit stall by rechecking late drop confirmation even when the unconfirmed-outcome recovery preference is off. The extra check only reads the screen and never sends a second Backspace/drop. If outcome remains uncertain, normal conservative pause rules still apply.
- After storing/dropping, require positive local evidence that the configured fishing rod is held before casting. Avoid blind rod-slot toggles.
- Recognize sustained positive evidence of a rejected cast (bait menu remained open or a different slot was visibly selected) and recover rather than waiting through a full bite timeout. Ambiguous visuals are not treated as cast failure.
- Retain protected-fruit policies, pity-zero safeguards, and instant Stop/Pause/focus-loss cancellation.

## v6 workflow changes
- Compact desktop workflow and refreshed controls, reduced optional capture/render overhead, stale stream expiration, backend catch-history pagination, and demand-driven hardware polling.
- Preserve user profile, settings, calibration, lifetime totals, telemetry and updater signing identity.
- Includes the v5.9.2 fishing/playback fixes and v5.9.3-beta.1 Telegram pity evidence and safe signed-update handoff.

## Verification and rollout
- Candidate requires the full GitHub Windows Rust, frontend, Python, controller, browser and signature/update-artifact gates. A passed CI build cannot verify actual Roblox gameplay or the complete installer handoff on the user's PC.
- **Do not call this stable** until real fruit-store/drop/recast and extended AFK gameplay have been confirmed on Windows.
- Prerelease builds are installed manually from the GitHub Release; the in-app stable updater uses the latest stable release endpoint.
- If the user chooses not to test v6 yet, use the current official stable release v5.9.2.
