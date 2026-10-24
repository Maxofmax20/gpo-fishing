# GPO Autofish v5.8.13 — lighter startup and faster releases

- Load non-dashboard pages and browser research routes only when opened. Measured dashboard startup JavaScript falls from 504,060 to approximately 287,418 bytes (43%). This is code-loading measurement, not a promised RAM, FPS or fishing-accuracy improvement.
- Remove hidden-window polling intervals and refresh when visible again.
- Cache Rust dependencies and npm/pip downloads. Run frontend, Python and Windows Rust quality checks concurrently.
- Build a private signed draft in parallel. Publication still waits for all quality gates, signed updater artifacts and checksum verification.
- Use faster test-only dependency compilation. Production optimization, model inference and panic recovery remain unchanged.
- Update vulnerable frontend build/test dependencies within existing supported version ranges.
- Preserve application identity, settings, models and signed in-app update verification.

Use Settings → Check for updates, or install the Windows x64 setup over the existing installation. Do not uninstall first.

The first build must populate new caches; later builds can reuse them. Cold and warm release timings must be measured separately. Live Roblox performance and Windows UI behavior still require a hardware session.
