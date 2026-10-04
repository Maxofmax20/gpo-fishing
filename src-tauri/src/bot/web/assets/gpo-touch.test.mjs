// Touch gesture decision tests (node:test, no DOM).
// Covers the shipped decision core in gpo-gesture.js: tap vs drag vs none,
// threshold boundary, cancellation, and multitouch pointer independence.
// The UMD module sets window/globalThis globals when loaded without a CJS
// `module` object (ESM import path); the test reads them from globalThis.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import './gpo-gesture.js';

const core = globalThis.GpoGesture;
assert.ok(core, 'gpo-gesture.js must expose globalThis.GpoGesture');

test('threshold is a sane finger-jitter value', () => {
  assert.equal(core.GPO_DRAG_THRESHOLD_PX, 12);
});

test('clean release without movement is a tap', () => {
  const t = core.gpoNewTrack();
  assert.equal(core.gpoTapUpAction(t, false), 'tap');
});

test('movement past the threshold turns the release into a drag, never a tap', () => {
  const t = core.gpoNewTrack();
  assert.equal(core.gpoNoteMove(t, 3, 4), false); // 5px: still tap
  assert.equal(core.gpoTapUpAction(t, false), 'tap');
  assert.equal(core.gpoNoteMove(t, 10, 10), true); // ~14px: drag
  assert.equal(core.gpoTapUpAction(t, false), 'drag');
});

test('exactly-at-threshold stays a tap (strictly-greater rule)', () => {
  const t = core.gpoNewTrack();
  core.gpoNoteMove(t, 6, 8); // 10px
  assert.equal(core.gpoTapUpAction(t, false), 'tap');
  const t2 = core.gpoNewTrack();
  core.gpoNoteMove(t2, 12, 0); // exactly 12px: NOT > 12
  assert.equal(core.gpoTapUpAction(t2, false), 'tap');
  // Callers pass origin-relative displacement, so slow drifts accumulate
  // across move events until the total crosses the threshold.
  core.gpoNoteMove(t2, 12.5, 0);
  assert.equal(core.gpoTapUpAction(t2, false), 'drag');
});

test('cancelled gestures never produce input, even when moved', () => {
  const t = core.gpoNewTrack();
  core.gpoNoteMove(t, 100, 100);
  assert.equal(core.gpoTapUpAction(t, true), 'none');
  const t2 = core.gpoNewTrack();
  assert.equal(core.gpoTapUpAction(t2, true), 'none');
});

test('multitouch pointers evolve independently', () => {
  const finger1 = core.gpoNewTrack();
  const finger2 = core.gpoNewTrack();
  core.gpoNoteMove(finger1, 50, 0); // finger 1 drags (camera)
  assert.equal(core.gpoTapUpAction(finger1, false), 'drag');
  assert.equal(core.gpoTapUpAction(finger2, false), 'tap'); // finger 2 still a tap
  assert.equal(core.gpoTapUpAction(finger2, true), 'none'); // finger 2 cancelled
  assert.equal(core.gpoTapUpAction(finger1, false), 'drag'); // finger 1 unaffected
});

test('long hold without movement stays a single tap (no repeat logic here)', () => {
  const t = core.gpoNewTrack();
  // Holding still = no moves recorded = one tap on release. Repeats would
  // be a separate repeat-timer concern, never implied by this core.
  assert.equal(core.gpoTapUpAction(t, false), 'tap');
});
