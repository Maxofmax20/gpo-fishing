# GPO Autofish v5.8.12 — fruit-drop recast reliability

This maintenance release addresses fishing sessions unexpectedly pausing after a duplicate or unprotected fruit is dropped.

## Fixed
- Fruit-drop confirmation now waits for delayed in-game banners: at least 1,400 ms and up to nine screen reads after the one permitted Backspace/drop action, rather than only three reads across a 350 ms default window.
- A delayed genuine `Dropped ... will despawn` banner can now confirm the operation so fishing continues normally instead of falsely reporting an unconfirmed drop.
- Added a regression replay with a lingering duplicate banner and delayed drop confirmation.

## Unchanged safeguards
- The macro never repeats a drop input to force confirmation.
- Legendary, mythical and pity-zero fruit protection remains enforced.
- Truly unconfirmed storage/drop results, failed storage, cancellation or an explicitly enabled protected-fruit pause still halt for inspection, rather than risking an unwanted item loss.
- Saved settings, existing datasets and bundled ML weights are not reset.

## Verification and limitations
- GitHub Actions runs automated frontend, Python and Windows Rust tests before creating release assets.
- The release pipeline requires a signed Windows NSIS installer, updater signature, `latest.json` feed and checksums before publishing.
- Automated regressions do not replace an uninterrupted live Roblox session; monitor one duplicate-fruit drop after installing.

## Update
Use **Settings → Check for updates** in GPO Autofish, or install the Windows x64 setup executable from this release. The app's latest-release updater feed is designed to pick up this stable version after it is published.
