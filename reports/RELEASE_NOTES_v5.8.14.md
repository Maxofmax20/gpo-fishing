# GPO Autofish v5.8.14 — pity-zero-only result screenshots

- New optional **Pity 0 only · after store / drop** notification mode.
- Verify pity-zero evidence independently of fruit rarity. A legendary/mythical name, nonzero pity or unreadable pity cannot bypass this filter.
- Send a single result after the fruit workflow, using its after-action screenshot; suppress the earlier catch message and duplicate screenshot.
- Suppress automatic progress, spawn, bait, recovery and other unrelated queued notifications while strict mode is enabled. Explicit user-requested Test delivery still works.
- A genuinely unconfirmed pity-zero result is labelled unconfirmed, with the post-attempt screenshot when available. It never falsely claims storage succeeded.
- Avoid capturing/encoding screenshots for notifications the active filter will discard.
- Keep valuable-fruit protection, one-drop-attempt behavior and pauses for genuinely unconfirmed outcomes.
- Include v5.8.13 lighter startup, hidden-window polling improvements, dependency audit fixes and faster cached release workflow.
- Warm the production dependency cache on main so subsequent release tags can use it; GitHub does not share one tag's cache directly with another tag.

Update in place using the signed in-app updater or this release's Windows x64 setup. Existing settings, data and models are preserved. Strict notification mode defaults off for other users; enable it under Features → Notifications. Existing owner preferences can opt in without changing fruit-drop protections.
