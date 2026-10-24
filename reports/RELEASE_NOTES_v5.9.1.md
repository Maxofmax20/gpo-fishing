# GPO Autofish v5.9.1

Repair release. Existing application identity, updater trust key and user data are preserved.

- Validate complete native capture bounds, monitor geometry and allocation arithmetic before copying pixels.
- Bound Windows OCR to one outstanding operation, cancel timed-out work and discard late results.
- Reject malformed remote clicks, drags, keys, clipboard, craft and playback requests without default actions.
- Separate bounded stream/action/control lanes; cancel remote input on Stop/Pause.
- Use the configured rod hotkey and remembered state instead of guessing slot 1 or toggling on weak negative pixels.
- Release stale tracking input, account for processing time in frame pacing, and enhance blank OCR reads.
- Keep missing fruit storage confirmation labelled unconfirmed; protected fruit is never silently dropped.
- Enforce crafting targets on the backend even with the phone page closed; counts describe attempted batches.
- Bound notification queues/screenshots and recording-frame memory. Encode recording frames on the writer thread.
- Debounce statistics writes, flushing on Pause/Stop.

## Validation
Windows Rust, frontend, browser, Node and Python quality suites pass at the tested repair source. Release repeats the quality gates.
Native Roblox, monitor hotplug and multi-hour gameplay tests have not been performed on this candidate.
This installer is supplied for manual installation. No running macro was stopped and no private preferences were changed.
