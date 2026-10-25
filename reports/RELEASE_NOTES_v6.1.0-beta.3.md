# v6.1.0-beta.3 — targeted web and notification repair

Fishing logic and recorded macro playback are unchanged from v6.1.0-beta.2.

- Restored the previous dual arrow-controller layout: movement and camera, with visible Jump and Shift Lock.
- Restored camera right-button look drags using the existing validated input service.
- Restored all notification preferences in the web UI, with immediate persistent saves, stale-edit checks, failure rollback and native UI refresh.
- Notification API responses never expose bot credentials or private chat identifiers.
- Corrected the native pity-zero toggle so changing the filter no longer overwrites other alert/screenshot choices.
- Corrected Telegram remote enable/revoke handling, including commands already being polled.
- Every notification event and screenshot toggle is enforced again at queued delivery/retry; revoked choices do not leak through an older queued snapshot.
- Preserved pity-zero-only mode, provider acknowledgements, credentials and app data.

Focused coverage: four responsive/accessibility browser checks including arrow requests, reload persistence and failed saves; settings-store tests; notification fake-provider/queue tests; native HTTP tests for camera semantics and validated durable preferences. Full existing release gates also run before publication.
No real Telegram test message is sent automatically. Actual message receipt is not inferred from mocked tests.
