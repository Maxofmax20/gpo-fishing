[![Discord](https://img.shields.io/badge/Discord-Join%20Server-7289da?style=for-the-badge&logo=discord&logoColor=white)](https://discord.gg/unPZxXAtfb)

# 🎣 GPO Autofish - GUIDE

**Current release: v5.7.0 — final feature release. See [FINAL_FEATURE_FREEZE.md](FINAL_FEATURE_FREEZE.md).**
**Model status: fish is NOT READY (8/10 classes, 1 reviewed row out of 8,609, no candidate).** See
[Where the model actually stands](#-where-the-model-actually-stands).

**💬 Join our Discord server:** https://discord.gg/unPZxXAtfb

## 🆕 What's New in v5.7.0?

**The real data pipeline is now trustworthy end to end.** Collection can capture
real gameplay samples, a human reviews them, and only reviewed-and-eligible rows
ever reach a trainer. See
[v5.7.0](#-v570--final-feature-release) for what was fixed, and
`docs/ML_REVIEW_AND_READINESS.md` for the authoritative contract.

## 🆕 What's New in v4.3?

**Complete Rewrite - Native, Fast & Tiny:**

- ⚡ **Native Windows app** - Rewritten in Rust with Tauri. One installer under 10 MB, no Python
- 🧠 **Built-in text recognition** - Uses the OCR that ships with Windows 10/11. No 1 GB download
- 📌 **HUD pill + tray icon** - A small always-on-top pill sits on the Roblox window. No big window in the way
- 📐 **Resolution independent** - Every area and click point is saved relative to the Roblox window
- 🖱️ **On-screen editor** - Draw the bar and drop message areas directly over the game. Bar area can be auto-detected
- 👀 **Live preview** - Thumbnail and match score for the bar area so you know it works before you start
- 🧭 **Step-by-step guide** - The Setup page walks you from an empty hotbar to your first catch
- 🔄 **Imports v3 settings** - Loads your old `default_settings.json`

## What is this?

This is the **open-source version** of the GPO fishing macro that everyone uses. Unlike the closed-source version that gets flagged as a virus and isn't trustworthy, this version is:

- ✅ **Fully open source** - You can see and verify all the code
- ✅ **No viruses** - Clean, transparent, and safe
- ✅ **Improved** - Better features and reliability
- ✅ **Community-driven** - Open for contributions and review

The original closed-source macro is sketchy and often flagged by antivirus software because you can't verify what it's actually doing. This open-source version solves that problem.

**🛡️ Concerned about safety?** The whole source is here. Build it yourself with the steps below and compare. Antivirus heuristics dislike programs that send mouse input and read the screen; that is what a fishing macro does.

---

**Features:**

- **🎣 Fishing System** - Automatic fish detection and tracking with a physics-based controller
- **🍎 Devil Fruit Detection** - OCR-powered detection of devil fruit drops with keyword matching
- **🌟 Fruit Spawn Alerts** - Detects and webhooks when devil fruits spawn with exact fruit name recognition
- **📦 Auto Fruit Storage** - Automatically stores devil fruits in your fruit slots when detected
- **🔔 Discord Webhook Alerts** - Notifications for devil fruit catches, world spawns, purchases and recoveries
- **🛒 Auto-Purchase** - Configurable bait purchasing every X fish
- **🪱 Auto Bait** - Re-selects your bait before every cast
- **🎯 Auto Setup** - Zoom control and cast positioning
- **🛟 Watchdog** - Restarts a stuck loop on its own
- **💾 Presets** - Save and load full settings snapshots
- **⬆️ Auto Update** - Updates itself from GitHub Releases
- **⌨️ Global hotkey support** (F1/F2/F3/F4, all rebindable)

## 🚀 Key Features

### 🍎 Devil Fruit Detection

- **OCR Detection**: Detects devil fruit drops using Windows text recognition
- **Spawn Detection**: Detects when devil fruits spawn in the world (all 33 GPO fruits)
- **Fuzzy Matching**: Handles OCR errors with a similarity threshold
- **Auto Storage**: Automatically stores caught fruits into two hotbar slots and re-equips the rod
- **Webhook Alerts**: Discord notifications for catches and spawns

### 🎯 Auto Setup

- **Zoom Control**: Automatically zooms out/in for fishing
- **Cast Positioning**: Moves to the casting position (auto center or a custom point)
- **Menu Clearing**: Right-clicks to clear menus

### 🛒 Auto-Purchase

- **Configurable Intervals**: Buy bait every X fish caught
- **Point System**: Holds the shop key next to the bait barrel, then clicks Confirm, Quantity and an optional Cancel point
- **Auto-save**: Settings persist between sessions

### ⚡ Performance

- **Fast Detection**: Bar tracking runs in microseconds, not Python pixel loops
- **Tiny Footprint**: One small installer, no runtime to install
- **Logging**: Live activity feed in the Dashboard plus a log file in the data folder

## Installation

### 🚀 Easy Installation (Recommended)

1. **Download** the latest `GPO Autofish_x.y.z_x64-setup.exe` from Releases
2. **Run it** - No admin needed
3. **Launch GPO Autofish** - The panel opens; the HUD appears once Roblox is running

Requires Windows 10 1809 or newer. Windows OCR needs an English language pack, which is present on nearly every install. The Setup page tells you if it is missing.

### 🔧 Build the installer yourself

Requirements: [Node.js 20+](https://nodejs.org) and [Rust](https://rustup.rs). WebView2 is already on Windows 11.

1. **Download the repository** as ZIP and extract it, or `git clone https://github.com/arielldev/gpo-fishing.git`
2. **Double-click `MakeItExe.bat`** - It installs packages, builds the app and opens the folder with the installer
3. **Run the installer** it produced, same as the one from Releases

Auto-update checks GitHub Releases on launch and can be turned off in Settings.

## 🎮 Quick Start Guide

### Before you start

- **Rod in slot 1**: Put your fishing rod in the first hotbar slot (key `1`)
- **Empty inventory**: Clear everything else out of your hotbar so fruits and bait land where the bot expects them

### First Time Setup

1. **Launch**: Open Roblox, join GPO, then open GPO Autofish. The Setup page shows the window as detected
2. **Bar area**: Cast once by hand. When the blue bar shows, open Setup › Fishing bar area and press **Auto-detect**. The thumbnail turns green when matched
3. **Drop message area**: Draw it over the popup at the top middle of the screen where "New Item <Fruit>" and "A Fruit has spawned at Place" appear. Press **Read now** to confirm the OCR reads it
4. **Rod key**: Slot `1`
5. **Enable Features**: Turn on auto bait, auto buy, fruit storage and webhooks in the Features page. Each one lists its steps, keys and points. Each **Pick** opens a crosshair over Roblox
6. **Fish**: Press **F1** or the HUD play button

### Devil Fruit Storage Setup

1. **Enable Store fruits** in the Features page
2. **Set Fruit Keys**: Choose the two hotbar slots the fruit is moved through
3. **Set Fruit Point**: Pick the Store button that appears after switching to a fruit slot
4. **Set Rod Key**: Slot `1` holds your fishing rod (Setup › Rod key)
5. **Set Bait Point**: Pick the top bait in the rod menu (Features › Auto bait)

### Auto-Purchase Setup

1. **Stand next to the bait barrel** on the dock before starting
2. **Enable Auto buy bait** in the Features page and follow the numbered steps
3. **Shop key**: The bot holds it until the barrel's shop dialog opens
4. **Confirm / Quantity / Cancel**: Pick each button in the shop. Cancel is optional
5. **Amount and interval**: How much bait to type and how many fish between purchases

### Discord Webhook Setup

1. **Create Webhook**: In your Discord server → Channel Settings → Integrations → Webhooks
2. **Copy URL**: Paste the webhook URL in Features › Discord and press **Test**
3. **Configure Alerts**:
   - 🍎 Devil Fruit Catch Alerts - Notifications when you catch a fruit while fishing
   - 🌟 Devil Fruit Spawn Alerts - Notifications when fruits spawn in the world (with exact fruit name). Turning this on makes the bot read the drop message area between casts
   - 🐟 Fish Progress Updates - Regular progress reports
   - 🛒 Auto Purchase Alerts - Bait purchase confirmations
   - 🛟 Recovery Alerts - When the watchdog restarts or gives up
4. **Set Interval**: Choose how often to send fish progress updates

### Hotkeys

- **F1**: Start/Pause fishing loop
- **F2**: Edit areas on screen
- **F3**: Emergency stop and exit
- **F4**: Hide/show the HUD
- **Note**: All hotkeys work without admin privileges and can be rebound in Settings

### Performance Tips

- **Long Sessions**: Close the panel; the HUD and tray icon keep running
- **Webhook Monitoring**: Use Discord alerts for fruit spawns and catches instead of watching the screen
- **OCR Optimization**: Make the drop message area cover the whole popup for better fruit detection
- **Spawn Detection**: The bot detects all 33 GPO devil fruits automatically using fuzzy matching

---

## 🔧 Troubleshooting

### Runtime Issues

- **HUD not showing**: It only appears while Roblox is running and not minimized. Press F4 if you hid it
- **Hotkeys not working**: Another app may own the key. Rebind in Settings › Hotkeys
- **Fish detection failing**: Open Setup › Fishing bar area. If the match score is low, raise Settings › Color tolerance or redraw the area tighter around the bar
- **Devil fruit not detected**: Setup › Drop message area › Read now shows exactly what the OCR sees
- **Fruit spawns not detected**: Ensure the drop message area covers the spawn popup
- **Auto-purchase failing**: Verify the Confirm and Quantity points are set and you are standing next to the bait barrel
- **Logs**: Settings › Data folder › `logs/`

### Devil Fruit Issues

- **Fruits not being stored**: Check if OCR detected the fruit in the Dashboard activity feed
- **Storage sequence running without fruit**: Ensure the drop message area only covers the popup
- **Wrong inventory slot**: Verify the fruit keys match your hotbar
- **Rod not switching back**: Check the rod key (slot `1`) and bait point configuration

---

## 🔒 Security & data

- **Web dashboard**: binds to `127.0.0.1:3888` by default and requires a per-install token for every API/stream route. Open it from the Panel's **Web UI** button (token attached automatically). LAN access is opt-in via Settings → Web dashboard (restart required) and still requires the token.
- **Secrets at rest**: Telegram tokens, Discord webhook URLs, Gemini keys, Roblox cookies and the dashboard token are encrypted with Windows DPAPI (current user scope). Old plaintext configs migrate automatically on first launch.
- **Backups, not deletions**: settings reset and journal clear keep timestamped backups (`settings.backup-*.json`, `catches.backup-*.csv`) in the data folder (`%AppData%\gpo-autofish`). A corrupt `settings.json` is quarantined (`settings.backup-*.json`) instead of silently reset — the app shows the recovery note in Settings.
- **Diagnostics**: Setup → Diagnostics → **Run check** probes the live Roblox window (bar/drop/bait-menu/server-time capture, vision confidence, OCR samples, calibration points) with per-item pass/warn/fail. Nothing is simulated.
- **Signing**: releases are signed; `build_release.ps1` takes the key password from `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` (never committed).
- **Gates cannot be weakened by configuration** (v5.7.0): every readiness threshold is clamped on load *and* on save, so a hand-edited `settings.json`, a crafted preset, or any future writer cannot set a bar below its floor. Floors: macro-F1 ≥ 0.30, worst-class F1 ≥ 0.10, shadow agreement ≥ 0.50, reviewed share ≥ 0.10, shadow events ≥ 20, shadow sessions ≥ 1. Unmeasured evidence never satisfies a gate, whatever the threshold says.
- **Model artifacts are verified before use**: the `.onnx` checksum *and* the index→label fingerprint are checked at load. A model family cannot occupy another family's slot. A deployed manifest that exists but cannot be parsed is never overwritten by the bundled model on next launch.

## 🚀 Releases & automatic updates

- **Channels**: git tag `vX.Y.Z` publishes a stable release; `vX.Y.Z-beta.N` publishes a prerelease (beta). The app's auto-updater follows stable.
- **Pipeline** (`.github/workflows/release.yml`): version gate (`scripts/check-versions.ps1` requires tag == `package.json` == `tauri.conf.json` == `Cargo.toml`) → frontend build → Rust tests → audits → signed Tauri build → **draft release** → verify installer, `.sig`, `latest.json` and `SHA256SUMS` → upload → **publish**. The release stays a private draft until every artifact is verified and attached, so a missing signature can never leave an unsigned build publicly downloadable. Any failure stops the release.
- **Updater security**: HTTPS GitHub endpoint, minisign public key pinned in `tauri.conf.json`, private key only in GitHub Actions secrets. The updater verifies the signature before installing; a bad signature or malformed metadata aborts and the installed version stays intact.
- **One installer path only.** The signed in-app updater is the *only* thing that installs an update. The Telegram `/update` command and the web dashboard's `update` command report availability and then defer to the in-app updater — they do not download or execute anything. Previously that path took a download URL verbatim from remote JSON with no host or scheme check, never checked the HTTP status, never verified a signature, and then ran the file; it bypassed the pinned key entirely. It also compared versions with `!=`, so a downgrade or a prerelease triggered an install.
- **User experience**: the app checks 30s after launch and every 6h (never blocking, offline-safe), honoring Settings › auto-update. An update banner offers Install, Release Notes, or Later — installation always needs your click. Update failures toast an error and keep the current version.

## 🧠 Perception & VPN state

- **Perception**: OCR readings are fused with a local GPO knowledge base (fruits, fish, bait, UI terms) into structured observations with explainable evidence. Weak evidence yields `unknown` with a reason — the bot never hallucinates entities. Setup › Diagnostics shows per-item configured/detected status plus the perception verdict.
- **GPO Wiki sync (opt-in)**: Setup › Devil fruits › GPO knowledge base can import devil-fruit pages from the public Grand Piece Online Wiki API. Imports are validated, keep provenance, and never overwrite curated entries. No automatic syncing happens.
- **Learning dataset**: with trace recording on, uncertain observations are saved to `%AppData%\gpo-autofish\dataset` for later labeling (Setup › Learning dataset). Nothing trains a model automatically.
- **VPN honesty**: the VPN page shows the evidence-based state (`disconnected/connecting/verifying/connected/disconnecting/error/unknown`) with process/tunnel evidence. `Connected` appears only after a verified tunnel. Macros never touch VPN unless you append an explicit VPN step (Macros › VPN Steps); verification timeouts stop the macro unless the step is marked optional.
- **Vision models (shadow only)**: two tract-powered ONNX models ship and run observation-only — `state_v1` (WAITING/BITE/RESULT, held-out 1.0000) and `fish_v1` (8 well-supported fish, held-out 0.58; everything else resolves UNKNOWN). Predictions are logged with OCR+KB agreement telemetry and can never drive the macro (`vision_to_macro = FORBIDDEN`, production control OFF). Capability gates, manifests, and the dashboard report per-entity evidence honestly; fruit/action/confirmation coverage is still collecting.
- **ML Training Center** (Panel › Training): real dataset explorer (per-fish/per-fruit coverage with qualification reasons), training eligibility with structured blockers, manual training jobs against frozen dataset snapshots (existing Python pipeline, streamed progress, cancellation), candidate registry with deterministic current-vs-candidate comparison, promotion to shadow only on Pass (rollback preserved), review queue for uncertain rows, learning history, and opt-in automatic retraining (candidates only — production control stays OFF and has no switch).
- **Human review** (Training › Review): every PNG reviewable with its actual pixels, model/OCR context, and canonical entity search (exact / ambiguous-pick-one / no-match-never-invent). Reviews persist with full audit trail; conflicts require explicit resolution with a reason and stay out of training until resolved; training eligibility is recomputed from real checks (PNG decodes, canonical id exists in KB, provenance present). Undo is available and append-only — undoing never deletes history. The audit log carries the full record, so the effective state can be rebuilt from it (`review_rebuild`).
- **Reviews actually exclude data** (v5.6.1): a frozen training snapshot writes a **filtered** `labels.jsonl`. Anything you skipped, called unknown, flagged as a conflict, or that failed the bad-image / canonical-id gate is physically absent from what the trainer reads — not merely marked. The snapshot fails closed rather than producing an empty training set, and its fingerprint/row counts describe exactly what trains.
- **Canonical entities** (Training › Drops): stable KB-derived entity IDs are the only training labels; display names are metadata. The resolver never merges similar names (Skeletal Shark ≠ Dark Skeletal Shark ≠ shark) and short model strings never become canonical names. Substring matching is word-anchored with a length floor, and a lossy OCR-variant retry can only ever return a single unambiguous entity. Entities whose name is already claimed by another KB entry are shown as `alias of …` rather than hidden — Wet-fish rarity tiers and sunken items claimed by example drop lists were NOT added because they do not exist in the project's KB.
- **Model readiness** (Training › Readiness): deterministic, differentiated statuses per family — `NOT_ENOUGH_DATA`, `NOT_ENOUGH_CLASSES`, `NOT_ENOUGH_SESSIONS`, `NOT_ENOUGH_REVIEW`, `NOT_ENOUGH_TEST`, `DATA_READY`, `TRAINING`, `EVALUATING`, `CANDIDATE_READY`, `SHADOW_READY`. Every gate reports actual / required / difference / next action. The shadow gate requires event volume **and** measured agreement **and** session spread. PRODUCTION_READY is structurally unreachable (no authorization mechanism exists) and reported as a design note, not an actionable blocker. Reviewed ≠ trained ≠ evaluated ≠ shadow-validated ≠ production-enabled.
- **Hermes interface**: no Hermes code exists in this repo (verified). External orchestration uses the read-only `hermes_tasks` view (triggers + readiness + history) and the existing training commands; lessons are factual history records. Gates cannot be bypassed because no bypass path exists.

### 🧊 v5.7.0 — final feature release

Feature freeze. See **[FINAL_FEATURE_FREEZE.md](FINAL_FEATURE_FREEZE.md)**.

This release was a correctness, safety and performance pass over the existing
system. No new feature was added, and no gate was relaxed. What it fixed:

**Evidence can no longer be attributed to the wrong thing**

- The output-index → label map is read from the trainer's own vocabulary, not
  re-derived by sorting. It only ever coincided before because the vocabulary
  happened to be alphabetical; inserting a class would have silently permuted
  every label while the artifact checksum stayed identical. A `vocab_sha`
  fingerprint now travels with the model and is **verified at load**.
- A manifest may not occupy another family's slot. It used to be keyed by the
  manifest's self-declared `name`, so a copied manifest silently evicted the
  real model and stopped observation with no error.
- Readiness joins shadow telemetry on the same revision key the log uses, and a
  new `shadow_evidence_revision` check requires the reported metrics and the
  reported soak to describe the **same** revision before `SHADOW_READY`.
- Version numbers are monotonic across registry loss, via a counter that
  survives deleting the registry directory. With no counter and no intact
  registry, numbering is **refused** rather than guessed — a recycled revision
  would inherit a dead model's soak.
- `promote_to_shadow` now writes `per_class_f1` and `test_support`. Omitting
  them silently disabled the per-class regression gate after the first
  promotion.
- A class measured on very few held-out examples can no longer decide a
  promotion on its own.

**Bad data can no longer enter training quietly**

- The Python review filter resolved `reviews.jsonl` from the wrong directory. In
  the documented manual-run path it therefore found nothing and excluded
  nothing — every skipped, unknown and disputed row would have trained.
- **Training now enforces a human-review floor.** Readiness could report
  `NOT_ENOUGH_REVIEW` while training ran on 100% collector-labelled rows, which
  made the displayed blocker decorative. Machine labels are the bot's own
  OCR+KB guesses and are not sufficient evidence on their own.
- The collector's duplicate detection is byte-exact. It compared a 64-bit
  average hash, so a visually different frame could alias an existing image and
  the newer sample's OCR text was written onto the older image's row.
- A dataset read that hits an I/O error is counted and reported, not silently
  read as a shorter dataset.

**Security**

- The Telegram/web `/update` path no longer downloads or executes anything. It
  took a download URL verbatim from remote JSON with no host or scheme check,
  never inspected the HTTP status, never verified a signature, then executed the
  file — bypassing the pinned minisign key that protects the in-app updater. It
  now reports availability and defers to the signed updater. Its version check
  was `!=`, so a downgrade or a prerelease triggered an install.
- Readiness thresholds are clamped on **load and save**. A hand-edited
  `settings.json` or a crafted preset could previously set every bar to 0 and
  make the system report ready.
- The release workflow builds as a **draft**, verifies the installer, signature,
  manifest and SHA-256 sums, and only then publishes. It previously created a
  public release and verified afterwards.
- The trainer's integrity guards (`assert`-based) are always compiled in:
  `PYTHONOPTIMIZE` is stripped from its environment.

**Stability and responsiveness**

- The backend health probe has a real timeout, kills the interpreter on expiry
  (no orphan), and cannot stack. It used to block forever with no timeout while
  being polled every few seconds.
- Heavy review and training commands run off the UI thread.
- Dataset reads are streamed with a server-side page cap; showing 100 samples no
  longer parses the whole dataset.
- Registry writes are atomic (tmp → fsync → rename).

**A snapshot now explains itself**: dataset version, review fingerprint, the
checksum of the exact bytes the trainer reads, and *why* each row is missing
(counted by reason, taken from the human verdict). A bare "excluded: 3" cannot
be acted on.

### 🚦 Where the model actually stands

```
SOFTWARE COMPLETE
MODEL NOT READY
```

| | |
|---|---|
| `state_v1` | solved, 1.0000 — protected, do not retrain without cause |
| `fish_v1` | 0.5824 deployed — **8/10 classes, 1 reviewed row of 8,609, no candidate, no soak** |
| fruit | 1/10 qualified, no trainer exists — `NOT_ENOUGH_DATA` |
| sunken | `UNVERIFIED / NOT IN KB` — not invented |
| production | `NOT READY`, by design; no production-control switch exists |

Live dataset: 8,609 rows, 8,609 images, 1 review record (0 training-eligible),
0 registered candidates, 23,448 shadow telemetry lines.

**There is no "fish v2 candidate ready".** No such candidate has ever existed —
`registry.json` does not exist. Fish cannot train until ten classes each have ≥20
human-reviewed examples across ≥3 independent sessions with a held-out TEST
example.

## 📁 Project Structure

```
src-tauri/src/
├── core/platform/       # OS traits (window, capture, input, OCR) + Windows implementations
├── core/vision.rs       # Bar / fish / marker detection
├── core/fruit.rs        # Drop and spawn text matching
├── core/controller.rs   # Reel controller
├── bot/                 # State machine, actions, watchdog, session stats
├── config.rs            # Settings, presets, v3 import
├── webhook.rs           # Discord embeds
└── commands.rs          # Tauri command surface
src/
├── windows/             # Hud, Panel, Overlay
├── pages/               # Dashboard, Setup, Features, Settings
└── components/          # Shared UI pieces
```

Everything OS-specific sits behind traits in `src-tauri/src/core/platform/mod.rs`, so other capture or OCR backends can be added without touching the bot logic. Fruit names and drop phrases live in settings under `lexicon`, so a game update does not need a rebuild.

## 🤝 Contributing

This is an open-source project! Feel free to:

- Report bugs and issues
- Suggest new features
- Submit pull requests
- Join our Discord community

**💬 Discord:** https://discord.gg/unPZxXAtfb

## License

MIT. See [LICENSE](LICENSE).
