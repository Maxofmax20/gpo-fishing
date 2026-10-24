'use strict';
/* Shared dashboard transport. Credentials stay in tab-scoped storage, not navigation URLs.
   Image streams retain the legacy query-token protocol; ordinary API calls use Bearer. */
const GPO_TOKEN = (() => {
  if (typeof location === 'undefined') return '';
  const url = new URL(location.href);
  const supplied = url.searchParams.get('token');
  let token = supplied || '';
  try {
    if (supplied) sessionStorage.setItem('gpo-dashboard-token', supplied);
    else token = sessionStorage.getItem('gpo-dashboard-token') || '';
  } catch (_) { /* Storage-disabled browsers retain the current page credential. */ }
  if (supplied) { url.searchParams.delete('token'); history.replaceState(null, '', url); }
  return token;
})();
// Touch-gesture decisions live in gpo-gesture.js (loaded before this file);
// remote.js calls those globals with inline fallbacks if absent.

function withToken(url) {
  return url + (url.indexOf('?') >= 0 ? '&' : '?') + 'token=' + encodeURIComponent(GPO_TOKEN);
}

const _gpoReads = new Map();
function gpoGet(path) {
  if (_gpoReads.has(path)) return _gpoReads.get(path);
  const pending = gpoRead(path).finally(() => { _gpoReads.delete(path); });
  _gpoReads.set(path, pending);
  return pending;
}

async function gpoRead(path) {
  const t0 = performance.now();
  let r;
  try {
    r = await fetch(path, { headers: { Authorization: 'Bearer ' + GPO_TOKEN }, cache: 'no-store', signal: AbortSignal.timeout(5000) });
  } catch (e) {
    if (typeof gpoNoteOnline === 'function') gpoNoteOnline(false);
    throw new Error('backend unreachable');
  }
  if (!r.ok) {
    const denied = r.status === 401 || r.status === 403;
    if (typeof gpoNoteOnline === 'function') gpoNoteOnline(false, denied ? 'Authorization required — reopen Web UI from the desktop app.' : undefined);
    throw new Error(denied ? 'dashboard authorization rejected' : 'HTTP ' + r.status);
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
    r = await fetch(path, {
      method: 'POST',
      signal: AbortSignal.timeout(15000),
      headers: { 'Content-Type': 'application/json', Authorization: 'Bearer ' + GPO_TOKEN },
      body: JSON.stringify(body || {}),
    });
  } catch (e) {
    if (typeof gpoNoteOnline === 'function') gpoNoteOnline(false);
    throw new Error('backend unreachable');
  }
  let j = {};
  try { j = await r.json(); } catch (e) { throw new Error('bad backend response'); }
  const denied = r.status === 401 || r.status === 403;
  if (typeof gpoNoteOnline === 'function') gpoNoteOnline(!denied, denied ? 'Authorization required — reopen Web UI from the desktop app.' : undefined);
  if (!r.ok || j.ok === false) throw new Error((j && j.message) || ('HTTP ' + r.status));
  return j;
}

const _gpoActions = new Map();
function gpoAction(action, extra) {
  const b = Object.assign({}, extra || {}, { action });
  const key = JSON.stringify(b);
  if (_gpoActions.has(key)) return _gpoActions.get(key);
  const pending = gpoPost('/api/action', b).finally(() => _gpoActions.delete(key));
  _gpoActions.set(key, pending);
  return pending;
}

// Bottom nav: same-origin tab storage preserves authentication without URL credentials.
function gpoWireNav() {
  document.querySelectorAll('[data-nav]').forEach((a) => {
    a.href = a.getAttribute('data-nav');
  });
}

// Global backend-liveness banner. Polls only ever report success while the
// backend answers; on failure the last-known UI would otherwise freeze on
// stale "RECORDING/ACTIVE" states. This banner makes disconnects obvious.
let _gpoOnlineEl = null;
let _gpoOnline = null;
function gpoNoteOnline(ok, reason) {
  const changed = _gpoOnline !== !!ok;
  _gpoOnline = !!ok;
  if (!_gpoOnlineEl) {
    const d = document.createElement('div');
    d.id = 'gpo-offline-banner';
    d.setAttribute('role', 'status');
    d.setAttribute('aria-live', 'polite');
    d.textContent = 'OFFLINE — BACKEND UNREACHABLE, RETRYING…';
    document.body.insertBefore(d, document.querySelector('body > main'));
    _gpoOnlineEl = d;
  }
  _gpoOnlineEl.textContent = reason || 'OFFLINE — BACKEND UNREACHABLE, RETRYING…';
  _gpoOnlineEl.style.display = _gpoOnline ? 'none' : 'block';
  if (changed) document.dispatchEvent(new CustomEvent('gpo:online', { detail: _gpoOnline }));
  try {
    if (typeof gpoPaintLivePills === 'function') gpoPaintLivePills(_gpoOnline);
  } catch (e) { /* decorative only */ }
}
function gpoIsOnline() { return _gpoOnline === true; }

// Header LIVE pills (stats/studio/craft/system): static markup claims
// "LIVE 60 FPS" unconditionally. Repaint from real poll health instead —
// never 60 FPS (backend caps at 10), never LIVE when unreachable.
function gpoPaintLivePills(ok) {
  document.querySelectorAll('[data-live-pill]').forEach((pill) => {
    const spans = pill.querySelectorAll('span');
    for (const s of spans) {
      if (/LIVE|OFFLINE|CONNECTING/.test(s.textContent)) {
        const label = ok ? '● LIVE' : '○ OFFLINE';
        if (s.textContent !== label) s.textContent = label;
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
  if (el && el.textContent !== String(text)) el.textContent = text;
}

// Shared PWR button visuals: cyan ON while running, dim OFF otherwise.
function gpoPaintPwr(btn, running) {
  if (!btn) return;
  const label = running ? 'PWR ON' : 'PWR OFF';
  if (btn.textContent !== label) btn.textContent = label;
  btn.classList.toggle('opacity-60', !running);
}

// Header host line: "<ip>:3888 • v<version>".
async function gpoPaintHostline() {
  try {
    const { data } = await gpoGet('/api/status');
    document.querySelectorAll('[data-gpo="hostline"]').forEach((el) => {
      el.textContent = (data.local_ip || location.hostname || '…') + ':3888 • v' + (data.version || '—');
    });
    return data;
  } catch (e) { return null; }
}

if (typeof document !== 'undefined') {
  document.addEventListener('DOMContentLoaded', gpoWireNav);
}
