# Contributing to GPO Autofish

Thank you for your interest in contributing to GPO Autofish!

This is a **Tauri v2 + Rust + React/TypeScript** desktop app (Windows).
There is no Python in this project.

## Development setup

Requirements: [Node.js 20+](https://nodejs.org), [Rust stable](https://rustup.rs),
WebView2 (preinstalled on Windows 11).

```bash
git clone https://github.com/Maxofmax20/gpo-fishing.git
cd gpo-fishing-macro
npm install
npm run app:dev     # Tauri dev window (Vite on :1420)
```

## Quality gates (must pass before a PR)

```bash
npm run build       # tsc --noEmit + vite build
cd src-tauri
cargo test          # Rust unit + integration tests
cargo clippy --lib --tests -- -D warnings
npm audit --audit-level=high
```

CI (`.github/workflows/ci.yml`) runs all of the above on every push/PR,
plus a secret scan. Releases are built by `.github/workflows/release.yml`
on `v*` tags.

## Release build & signing

```bash
# Local (generates %USERPROFILE%\.tauri\gpo-autofish.key on first run):
MakeItExe.bat

# Maintainer (key in .tauri\updater_key, password from environment):
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = "..."   # never commit this
.\build_release.ps1
```

`TAURI_SIGNING_PRIVATE_KEY` / `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` come from
CI secrets in releases. Never hardcode or print them. Forks must replace the
updater `pubkey` + `endpoints` in `src-tauri/tauri.conf.json`.

Releases are cut by pushing a tag (`vX.Y.Z` stable, `vX.Y.Z-beta.N` beta):
`scripts/check-versions.ps1 -Tag <tag>` must pass first (CI enforces it),
then quality gates, then the signed Tauri build. Updater behavior is covered
by `src-tauri/tests/updater.rs` — keep those green.

## Security rules for contributors

- Secrets (Telegram tokens, Discord webhook URLs, Gemini keys, `.ROBLOSECURITY`
  cookies, dashboard tokens) are DPAPI-encrypted at rest via
  `src-tauri/src/core/secrets.rs`. Never log them, never put them in tests
  (use `synthetic-` prefixed fixtures), never commit them.
- The web dashboard (`src-tauri/src/bot/web_server.rs`) binds loopback by
  default, requires the per-install token for all API/stream routes, and uses
  restricted CORS. Do not reintroduce `0.0.0.0` default bind or `*` CORS.
- Tauri `security.csp` must stay enabled. Remote `<script src>` is banned;
  all dashboard JS is served same-origin.
- Destructive actions (reset, journal clear, kill-all, account remove) need a
  `ConfirmModal` + a recoverable backup — never `window.confirm()` + delete.
- Backend errors are `Result<T, String>` and must reach the UI via toasts,
  not `catch (() => undefined)` / `console.error`-only handling.
- VPN: `Connected` requires verified tunnel evidence (process AND tunnel).
  Never set connected on spawn/exit-code/selection. New engines need an
  `evaluate_evidence` row plus unit tests. Macro VPN steps use real seconds
  (never speed-scaled) and fail loudly unless explicitly optional.
- Perception: weak evidence must yield `Unknown` with a reason — never invent
  entities. Confidence stays coarse (2dp) and evidence-named. No fake model
  weights, no network in unit tests (MediaWiki live test is `#[ignore]`).
- Knowledge imports keep provenance and never overwrite `bundled` entries
  without an explicit curator flag; renames are rejected, not merged.

## Reporting bugs

- Check existing issues to avoid duplicates.
- Include: Windows version, app version, Node/Rust versions (for build issues),
  `%AppData%\gpo-autofish\logs\gpo-autofish.log` excerpts (**redact tokens first**).
- For detection issues: Setup → Diagnostics → Run check, and paste the
  per-item results.

## Pull requests

1. Fork, create a feature branch (`git checkout -b feature/AmazingFeature`).
2. Keep the diff focused; no unrelated formatting churn.
3. Add/extend tests for behavior changes (especially `config.rs` migrations,
   `secrets.rs`, `web_server.rs` auth, `fruit.rs` parsing).
4. Ensure the quality gates above pass.
5. Open a Pull Request with before/after verification notes.

## Code style

- Rust: `cargo fmt` + Clippy clean; strong types over strings; small modules.
- TypeScript: `strict` + `noUnusedLocals`; no `any` (use real unions/generics);
  shared IPC contracts live in `src/lib/types.ts` + `src/lib/ipc.ts`.
- Never add `TODO`s, debug code, or temporary files to a PR.
