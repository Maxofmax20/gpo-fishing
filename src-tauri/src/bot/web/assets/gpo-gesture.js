'use strict';
/* Touch-gesture decision core: framework-free, DOM-free, deterministic.
   Loaded as a classic script in the browser (exposes window globals) and
   required by node:test via UMD. This file is the SINGLE source of truth
   for tap-vs-drag-vs-none; gpo-remote.js only feeds it pointer positions. */
(function (root, factory) {
  var api = factory();
  if (typeof module !== 'undefined' && module.exports) {
    module.exports = api;
  }
  if (root) {
    root.GpoGesture = api;
    root.GPO_DRAG_THRESHOLD_PX = api.GPO_DRAG_THRESHOLD_PX;
    root.gpoNewTrack = api.gpoNewTrack;
    root.gpoNoteMove = api.gpoNoteMove;
    root.gpoTapUpAction = api.gpoTapUpAction;
  }
}(typeof window !== 'undefined' ? window : (typeof globalThis !== 'undefined' ? globalThis : null), function () {
  // Pixel distance that turns a touch into a drag. At or below this the
  // release is a tap; above it, a camera drag — never both, and a drag is
  // never followed by a click. Tuned for finger jitter (~12px density).
  var GPO_DRAG_THRESHOLD_PX = 12;

  // One independent track per active pointer (multitouch: tracks never
  // share state; releasing one pointer cannot end another's gesture).
  function gpoNewTrack() { return { moved: false }; }

  function gpoNoteMove(track, dxPx, dyPx) {
    if (Math.sqrt(dxPx * dxPx + dyPx * dyPx) > GPO_DRAG_THRESHOLD_PX) {
      track.moved = true;
    }
    return track.moved;
  }

  // TAP-mode left-button release decision. Cancelled gestures
  // (pointercancel, uncaptured leave, orientation change) NEVER produce
  // input — not a tap, not a drag, nothing.
  function gpoTapUpAction(track, cancelled) {
    if (cancelled) return 'none';
    return track.moved ? 'drag' : 'tap';
  }

  return {
    GPO_DRAG_THRESHOLD_PX: GPO_DRAG_THRESHOLD_PX,
    gpoNewTrack: gpoNewTrack,
    gpoNoteMove: gpoNoteMove,
    gpoTapUpAction: gpoTapUpAction,
  };
}));
