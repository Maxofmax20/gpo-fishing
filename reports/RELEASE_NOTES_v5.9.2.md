# GPO Autofish 5.9.2

- Automatic bait selection uses local detection rather than waiting for network vision. Explicit manual scans remain available.
- Catch verification has a bounded observation window, including a late check. Missing banners remain unconfirmed and never inflate catch or pity counters.
- Rod selection is re-established after catches. Rod and reset slots must differ to prevent accidental unequipping.
- Recorded playback hands focus to Roblox once at startup, works independently of fishing, and ignores an already-held launch hotkey. Later focus loss still stops playback.
- OCR cancellation retains single-request gating and rejects late results.

Existing settings and data are preserved. This release does not claim live Roblox or extended AFK verification.
