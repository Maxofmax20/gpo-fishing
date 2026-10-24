# GPO Autofish product repair audit — v5.9.0

## Coverage and dispositions

| Surface | Disposition | Validation |
| --- | --- | --- |
| Dashboard and HUD | Preserved; consistent red calibration colors; catch/recovery labels distinguish observation from success | Frontend and native regression suites; live gameplay pending |
| Setup, guide and overlay | Preserved points/regions and native calibration; no default calibration overwrite | Typecheck/build; physical DPI/focus checks pending |
| Features and settings | Offline settings accessible; ordinary-fruit recovery is explicit and disabled by default; duplicate Multi-Roblox toggle removed | Navigation/component tests; native policy tests |
| Journal | Preserved history; bounded server tail for polling, full history remains on disk | Unicode/large-log test |
| Recorder, VPN and Multi-Roblox | Preserved under Advanced; Multi-Roblox waits for persisted settings before claiming success | Existing unit/API suites; external account/provider tests pending |
| Training and review | Preserved under Advanced; capped thumbnail cache with shared requests; suppress empty collection completion toasts | Existing training/review suite; cache inspected |
| Desktop updater | Genuine download progress, failure/retry/dismissal regressions retained | Updater tests; signed artifact checks required |
| Web Remote/Stats/Studio/Craft/System | Consistent local compiled CSS, layout, accessible names/contrast, connection and rejection states | 15 Chromium scenarios at 360/768/1280px, WCAG serious/critical checks |
| Web authentication | Public shell remains data-free; API Bearer authentication; token moved from visible URL to session storage; image query authentication retained; no-referrer policy | Unauthorized/reconnect regression checks; backend auth tests |
| Web command/settings contracts | Invalid commands return failure; settings save before memory commit; refresh desktop settings after web changes | Native malformed-value/save-failure tests |
| Fishing verification | Bounded read-only drop recheck; one drop input; high-contrast catch OCR fallback with strict parser | Native pixel/replay/policy tests; genuine fruit event pending |
| Telemetry | Bounded 1024-event channel; lossy visual updates cannot block fishing; critical state/result messages retain ordering; visible-window reading delivery | Native compilation/regressions; long Windows soak pending |

## Removed or consolidated

- Removed the browser runtime Tailwind compiler; replaced with generated, embedded CSS and an explicit dynamic-class safelist.
- Removed the duplicate Multi-Roblox control from Features; Advanced > Multi Roblox remains the authoritative UI.
- Renamed the non-functional “Calibrate Timers” promise to “Check connection”: it measures API latency rather than pretending to calibrate gameplay.
- Removed misleading initial sample statistics, the hard-coded laptop model, hard-coded 100 pity limit and unconditional success messages for failed telemetry/crafting operations.
- No saved macros, presets, credentials, calibration points, logs, models or training records were deleted.

## Release criteria and limitations

Publication requires the exact source to pass frontend, browser, Python, Windows Rust and Clippy checks and signed-installer metadata verification. Windows gameplay, hardware operations, DPI/overlay behavior and a two-hour soak are separate acceptance checks. Browser tests use controlled API responses; they are not evidence of a real Windows device operation. Native optimization remains unchanged until controlled latency/accuracy measurements justify changing LTO.

Three moderate advisories remain in Vitest development-only dependencies; npm audit fix currently cannot resolve them within the published dependency range. High/critical audit gate remains enabled. No public persistent Vitest server is run.
