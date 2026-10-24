import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { JSDOM } from 'jsdom';

const html = readFileSync(new URL('../system.html', import.meta.url), 'utf8');
const script = readFileSync(new URL('./gpo-system.js', import.meta.url), 'utf8');
const tick = () => new Promise(resolve => setImmediate(resolve));

async function page(hardware = true) {
  const dom = new JSDOM(html, { url: 'http://localhost/system', runScripts: 'outside-only' });
  const w = dom.window;
  if (w.document.readyState === 'loading') await new Promise(resolve => w.document.addEventListener('DOMContentLoaded', resolve, { once: true }));
  const calls = [], pending = [], reads = [];
  w.setInterval = () => 0;
  for (const name of ['gpoNoteOnline', 'gpoPaintHostline', 'gpoPaintPwr', 'gpoRebind', 'gpoToast']) w[name] = () => {};
  w.gpoSetText = (id, value) => { const e = w.document.getElementById(id); if (e) e.textContent = value; };
  w.gpoEsc = value => String(value || '');
  w.gpoGet = async path => { reads.push(path); return { data: { volume: 20, brightness: 40, fan: hardware ? { success: true, current_mode: "balance", fan_level_pct: 50 } : null, keyboard_light: hardware ? { success: true, status: "High" } : null, collection_enabled: true, dataset: {}, readiness: {}, model: {} } }; };
  w.gpoAction = (action, body) => { calls.push({ action, ...body }); return new Promise(resolve => pending.push(resolve)); };
  w.gpoPost = (path, body) => { calls.push({ path, ...body }); return new Promise(resolve => pending.push(resolve)); };
  w.eval(script);
  w.document.dispatchEvent(new w.Event('DOMContentLoaded'));
  await tick();
  const sliders = [...w.document.querySelectorAll('[role="slider"]')];
  for (const el of sliders) { el.setPointerCapture = () => {}; el.getBoundingClientRect = () => ({ left: 0, width: 100 }); }
  const pointer = (el, name, x) => el.dispatchEvent(new w.MouseEvent(name, { clientX: x, button: 0, bubbles: true }));
  return { w, calls, pending, reads, sliders, pointer, close: () => w.close() };
}

test('opening laptop controls does not scan the dataset; Refresh requests it explicitly', async () => {
  const p = await page();
  try {
    assert.deepEqual(p.reads, ['/api/status']);
    p.w.document.getElementById('ml-refresh-btn').click();
    await tick();
    assert.deepEqual(p.reads, ['/api/status', '/api/ml/status']);
  } finally { p.close(); }
});

test('all laptop mode buttons have visible names and embedded icons', async () => {
  const p = await page();
  try {
    const buttons = [...p.w.document.querySelectorAll('[data-fanmode], [data-rgb]')];
    assert.equal(buttons.length, 8);
    for (const button of buttons) { assert.ok(button.textContent.trim()); assert.ok(button.querySelector('svg')); }
  } finally { p.close(); }
});

test('drag feedback is immediate; sends one final fan command with the correct field', async () => {
  const p = await page();
  try {
    p.pointer(p.sliders[0], 'pointerdown', 10);
    p.pointer(p.sliders[0], 'pointermove', 78);
    assert.equal(p.w.document.getElementById('fan-speed-fill').style.width, '78%');
    assert.equal(p.calls.length, 0);
    p.pointer(p.sliders[0], 'pointerup', 80);
    assert.deepEqual(p.calls, [{ path: '/api/fan/set', pct: 80 }]);
    p.pending.shift()({}); await tick();
  } finally { p.close(); }
});

test('slow hardware commands coalesce later edits without resetting the slider', async () => {
  const p = await page();
  try {
    const el = p.sliders[1];
    p.pointer(el, 'pointerdown', 30); p.pointer(el, 'pointerup', 30);
    p.pointer(el, 'pointerdown', 50); p.pointer(el, 'pointerup', 50);
    p.pointer(el, 'pointerdown', 90); p.pointer(el, 'pointerup', 90);
    assert.equal(p.calls.length, 1);
    assert.equal(p.w.document.getElementById('volume-fill').style.width, '90%');
    p.pending.shift()({}); await tick(); await tick();
    assert.deepEqual(p.calls, [{ action: 'set_volume', value: 30 }, { action: 'set_volume', value: 90 }]);
    p.pending.shift()({}); await tick();
  } finally { p.close(); }
});

test('cancelled drags send no command; keyboard controls work', async () => {
  const p = await page();
  try {
    const el = p.sliders[2];
    p.pointer(el, 'pointerdown', 70); p.pointer(el, 'pointercancel', 70);
    assert.equal(p.calls.length, 0);
    await tick();
    el.dispatchEvent(new p.w.KeyboardEvent('keydown', { key: 'End', bubbles: true }));
    assert.deepEqual(p.calls, [{ action: 'set_brightness', value: 100 }]);
    p.pending.shift()({}); await tick();
  } finally { p.close(); }
});

test('keyboard light buttons cannot launch overlapping Cycle commands', async () => {
  const p = await page();
  try {
    const cycle = p.w.document.querySelector('[data-rgb="cycle"]');
    cycle.click();
    cycle.click();
    p.w.document.querySelector('[data-rgb="low"]').click();
    assert.equal(p.calls.length, 1);
    assert.equal(p.calls[0].action, 'cycle');
    assert.ok([...p.w.document.querySelectorAll('[data-rgb]')].every(b => b.disabled));
    p.pending.shift()({ status: 'low' }); await tick();
    assert.ok([...p.w.document.querySelectorAll('[data-rgb]')].every(b => !b.disabled));
  } finally { p.close(); }
});

test('web collection stop sends an explicit preference without stopping fishing', async () => {
  const p = await page();
  try {
    const button = p.w.document.getElementById('ml-collection-btn');
    assert.equal(button.textContent, 'Stop collecting data');
    button.click(); button.click();
    assert.deepEqual(p.calls, [{ action: 'set_collection', enabled: false }]);
    p.pending.shift()({ message: 'Collection stopped' }); await tick();
  } finally { p.close(); }
});

test('unsupported hardware controls cannot submit device commands', async () => {
  const fixture = await page(false);
  const w = fixture.w;
  try {
    assert.ok([...w.document.querySelectorAll('[data-fanmode], [data-rgb], #turbo-toggle')].every(button => button.disabled));
    const slider = w.document.querySelector('[aria-label="fan speed"]');
    assert.equal(slider.getAttribute('aria-disabled'), 'true');
    slider.dispatchEvent(new w.KeyboardEvent('keydown', { key: 'End', bubbles: true }));
    assert.equal(fixture.calls.length, 0);
  } finally { w.close(); }
});
