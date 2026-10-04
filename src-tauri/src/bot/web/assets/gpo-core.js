'use strict';
/* GPO Cyberdeck shared core: auth plumbing, API helpers, nav, status polling.
   No external requests. Token travels only in ?token= (needed for <img>).
   Pure gesture helpers below are shared with node:test (guarded browser-only
   top-level code at the bottom). */
const GPO_TOKEN = (typeof location !== 'undefined')
  ? new URLSearchParams(location.search).get('token') || ''
  : '';
// Touch-gesture decisions live in gpo-gesture.js (loaded before this file);
// remote.js calls those globals with inline fallbacks if absent.

function withToken(url) {
  return url + (url.indexOf('?') >= 0 ? '&' : '?') + 'token=' + encodeURIComponent(GPO_TOKEN);
}

async function gpoGet(path) {
  const t0 = performance.now();
  let r;
  try {
    r = await fetch(withToken(path), { cache: 'no-store' });
  } catch (e) {
    if (typeof gpoNoteOnline === 'function') gpoNoteOnline(false);
    throw new Error('backend unreachable');
  }
  if (!r.ok) {
    if (typeof gpoNoteOnline === 'function') gpoNoteOnline(false);
    throw new Error('HTTP ' + r.status);
  }
  let data;
  try {
    data = await r.json();
  } catch (e) {
    if (typeof gpoNoteOnline === 'function') gpoNoteOnline(false);
    throw new Error('bad backend response');
  }
  if (typeof gpoNoteOnline === 'function') gpoNoteOnline(true);
  return { data, ms: Math.max(1, Math.round(performance.now() - t0)) };
}

async function gpoPost(path, body) {
  let r;
  try {
    r = await fetch(withToken(path), {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body || {}),
    });
  } catch (e) {
    if (typeof gpoNoteOnline === 'function') gpoNoteOnline(false);
    throw new Error('backend unreachable');
  }
  let j = {};
  try { j = await r.json(); } catch (e) { /* non-JSON */ }
  if (!r.ok || j.ok === false) throw new Error((j && j.message) || ('HTTP ' + r.status));
  if (typeof gpoNoteOnline === 'function') gpoNoteOnline(true);
  return j;
}

function gpoAction(action, extra) {
  const b = extra || {};
  b.action = action;
  return gpoPost('/api/action', b);
}

// Bottom nav: keep the token across tab switches.
function gpoWireNav() {
  document.querySelectorAll('[data-nav]').forEach((a) => {
    a.href = a.getAttribute('data-nav') + '?token=' + encodeURIComponent(GPO_TOKEN);
  });
}

// Global backend-liveness banner. Polls only ever report success while the
// backend answers; on failure the last-known UI would otherwise freeze on
// stale "RECORDING/ACTIVE" states. This banner makes disconnects obvious.
let _gpoOnlineEl = null;
let _gpoOnline = true;
function gpoNoteOnline(ok) {
  _gpoOnline = !!ok;
  if (!_gpoOnlineEl) {
    const d = document.createElement('div');
    d.id = 'gpo-offline-banner';
    d.style.cssText = 'position:fixed;left:50%;transform:translateX(-50%);bottom:76px;z-index:9998;padding:6px 14px;border-radius:9999px;background:rgba(147,0,10,.92);border:1px solid #ffb4ab;color:#ffdad6;font:700 11px "Space Mono",monospace;letter-spacing:.08em;display:none;max-width:88vw;text-align:center;pointer-events:none;';
    d.textContent = 'OFFLINE — BACKEND UNREACHABLE, RETRYING…';
    document.body.appendChild(d);
    _gpoOnlineEl = d;
  }
  _gpoOnlineEl.style.display = _gpoOnline ? 'none' : 'block';
  try {
    if (typeof gpoPaintLivePills === 'function') gpoPaintLivePills(_gpoOnline);
  } catch (e) { /* decorative only */ }
}
function gpoIsOnline() { return _gpoOnline; }

// Header LIVE pills (stats/studio/craft/system): static markup claims
// "LIVE 60 FPS" unconditionally. Repaint from real poll health instead —
// never 60 FPS (backend caps at 10), never LIVE when unreachable.
function gpoPaintLivePills(ok) {
  document.querySelectorAll('[data-live-pill]').forEach((pill) => {
    const spans = pill.querySelectorAll('span');
    for (const s of spans) {
      if (/LIVE|OFFLINE/.test(s.textContent)) {
        s.textContent = ok ? '● LIVE' : '○ OFFLINE';
      }
    }
    pill.classList.toggle('opacity-50', !ok);
  });
}

// Fallback toast when a page has no toast element of its own.
let _gpoToastEl = null;
let _gpoToastTimer = 0;
function gpoToast(msg) {
  if (!_gpoToastEl) {
    const d = document.createElement('div');
    d.style.cssText = 'position:fixed;top:96px;left:50%;transform:translateX(-50%);z-index:9999;padding:8px 16px;border-radius:9999px;background:rgba(49,53,64,.95);border:1px solid rgba(76,215,246,.4);color:#dfe2f1;font:700 11px "Space Mono",monospace;letter-spacing:.08em;transition:opacity .3s;opacity:0;pointer-events:none;max-width:86vw;text-align:center;';
    document.body.appendChild(d);
    _gpoToastEl = d;
  }
  _gpoToastEl.textContent = msg;
  _gpoToastEl.style.opacity = '1';
  clearTimeout(_gpoToastTimer);
  _gpoToastTimer = setTimeout(() => { _gpoToastEl.style.opacity = '0'; }, 2000);
}

function gpoFmtRuntime(totalS) {
  totalS = Math.max(0, Math.floor(totalS || 0));
  const h = String(Math.floor(totalS / 3600)).padStart(2, '0');
  const m = String(Math.floor((totalS % 3600) / 60)).padStart(2, '0');
  const s = String(totalS % 60).padStart(2, '0');
  return h + ':' + m + ':' + s;
}

// "2026-09-13 11:04:17" (local PC time) -> "18m ago".
function gpoAgo(ts) {
  if (!ts) return '—';
  const t = new Date(String(ts).replace(' ', 'T')).getTime();
  if (isNaN(t)) return String(ts);
  const s = Math.max(0, Math.floor((Date.now() - t) / 1000));
  if (s < 60) return s + 's ago';
  if (s < 3600) return Math.floor(s / 60) + 'm ago';
  if (s < 86400) return Math.floor(s / 3600) + 'h ago';
  return Math.floor(s / 86400) + 'd ago';
}

function gpoEsc(s) {
  return String(s == null ? '' : s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
}

// Strip all listeners by cloning (used to neutralize mock-only handlers
// before attaching the real backend wiring on the same button).
function gpoRebind(el) {
  if (!el) return null;
  const clone = el.cloneNode(true);
  el.replaceWith(clone);
  return clone;
}

function gpoSetText(id, text) {
  const el = document.getElementById(id);
  if (el) el.textContent = text;
}

// Shared PWR button visuals: cyan ON while running, dim OFF otherwise.
function gpoPaintPwr(btn, running) {
  if (!btn) return;
  btn.textContent = running ? 'PWR ON' : 'PWR OFF';
  btn.classList.toggle('opacity-60', !running);
}

// Header host line: "<ip>:3888 • v<version>".
async function gpoPaintHostline() {
  try {
    const { data } = await gpoGet('/api/status');
    document.querySelectorAll('[data-gpo="hostline"]').forEach((el) => {
      el.textContent = (data.local_ip || '…') + ':3888 • v' + (data.version || '—');
    });
    return data;
  } catch (e) { return null; }
}

if (typeof document !== 'undefined') {
  document.addEventListener('DOMContentLoaded', gpoWireNav);
}
