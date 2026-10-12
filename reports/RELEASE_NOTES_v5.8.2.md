# GPO Autofish v5.8.2

- Removed the desktop panel's transparent outer strip, animated surface movement and native shadow border. Its opaque background now fills the window from startup through resizing.
- Removed loading text from transparent HUD and selection windows to avoid a startup line.
- Improved the five-page web dashboard for phone and desktop widths, safe-area spacing, readable labels, keyboard focus and browser zoom.
- Moved offline warnings into the page layout so they do not cover controls. Stream labels now reflect disconnection, missing Roblox and failed streams instead of continuing to claim LIVE.
- Added regression coverage for stream errors, backend disconnect/recovery and initial connection indicators.

Existing settings, credentials, samples and model weights are preserved. The sample-trained fish model remains available. Browser checks use the shipped dashboard assets with an unavailable backend; they do not establish live-game behavior or native Windows rendering on every display.
