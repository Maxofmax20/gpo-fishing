# GPO Autofish v5.8.6

- Made the main desktop window translucent while keeping text and controls fully opaque. The panel still fills its client area without an extra transparent edge.
- Moved local-IP, screen-brightness and GPU/fan queries out of dashboard status requests. Status uses cached readings while one background worker refreshes hardware information.
- Shared simultaneous reads of the same endpoint and added a five-second GET deadline, preventing slow requests from accumulating. Remote state refreshes every second; unchanged controls no longer recreate their icon markup.
- Reduced the bundled icon font from 4,003,088 to 120,936 bytes, preloaded it, and versioned font URLs. Replaced an unsupported icon name. All five dashboard pages rendered their icons correctly in a mobile-width browser check.

Validation: 137 frontend tests, 13 dashboard/gesture tests, and 496 Rust library tests passed (two existing ignored). A fake-input HTTP regression verifies three status responses within one second with zero input events. Browser checks used an idle mock backend; live game/network latency and native desktop transparency were not visually verified.