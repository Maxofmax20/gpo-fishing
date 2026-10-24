# Upgrade evidence

## Published repair and optional beta

- v5.9.2: 614 native Windows tests passed; 157 frontend tests passed. Installer checksum and existing updater signature independently verified.
- v5.9.3-beta.1 source a1ae80c: 634 native Windows tests passed, 3 ignored. Frontend, browser, Python and Clippy checks passed in release run 40131245121. Installer signature and manifest independently verified; publication status is checked separately.
- Telegram bot identity and configured chat read-only checks passed. One explicitly authorized diagnostic message was accepted by Telegram. No credential values are included here.
- Installed Windows application was observed as v5.9.0. It was not closed, restarted, upgraded or reconfigured by this work.

## Broader UI/performance branch

This branch includes task-first navigation, on-demand previews, bounded history pagination, real remote-dashboard states, capture prioritization, bounded controller metrics, durable outcome delivery records and crafting checkpoints. Its latest combined Windows CI must pass before a broader installer is published.

Controlled browser checks cover the remote pages and their icons/layouts. They are not native Windows DPI or Roblox gameplay tests.

The compiler-profile benchmark workflow is opt-in. Production compiler settings remain unchanged; build-speed and runtime targets are not measured achievements.

## Remaining device verification

Actual Roblox gameplay, installer-to-installer handoff, native display behaviour and 2h/8h AFK soak remain pending voluntary installation. Preserve the running v5.9.0 session and provide installers rather than forcing an upgrade.
