# GPO Autofish v6.0.5 — v5.9.0 Stability Diagnostics

A conservative update **built on the complete v5.9.0 fishing and UI baseline**, not on the experimental v6 controller. The 6.0.5 version number exists for signed in-app update compatibility.

- Add a regression test requiring 20 successive simulated fishing minigames to end normally without entering `Recovering`. This guards against changes that break v5.9.0 fishing behavior.
- Record average and p95 capture/vision timing in **existing optional reel trace data**, computed at the end of each trace without adding new capture or timing operations on the critical fishing path.
- Preserve ability to read old trace metrics with the newly added fields defaulting safely.
- Retain the v5.9.0 fishing/controller/rod/bait/UI, the Stop→Start lifecycle fix, and the one-time backup of existing settings/stats/catches introduced in v6.0.4.
- Leave gameplay timings and bait clicks unchanged; no automatic high-speed tuning or indefinite recovery attempts.

The optional trace metrics are intended to guide later optimization from evidence instead of blindly adjusting the working fishing loop. Full automated CI and signed Windows installer verification are required before publication. Real Windows/Roblox behavior is not proven by simulated tests; verify live fishing before extended unattended use.
