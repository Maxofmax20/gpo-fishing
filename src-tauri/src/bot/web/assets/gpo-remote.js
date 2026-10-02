'use strict';
/* REMOTE DECK wiring: live stream, touch pads, macro + utility actions. */
(function () {
  let streamFps = 8;      // backend caps MJPEG at 10fps
  let streamScale = 720;
  let tapMode = true;     // TAP = tap-to-click, PAD = drag camera on stream
  let lastStatus = null;

  function toast(m) {
    if (typeof showToast === 'function') { try { showToast(m); return; } catch (e) {} }
    gpoToast(m);
  }

  function streamUrl() {
    return withToken('/api/stream?fps=' + streamFps + '&scale=' + streamScale + '&q=70') + '&t=' + Date.now();
  }

  function reloadStream() {
    const img = document.getElementById('gpo-stream');
    if (img) img.src = streamUrl();
  }

  async function sendKey(key, down, tap) {
    try {
      await gpoPost('/api/key', { key, down: !!down, tap: !!tap });
    } catch (e) { toast('Key failed: ' + e.message); }
  }

  function holdButton(el, key) {
    if (!el) return;
    const down = (e) => { e.preventDefault(); sendKey(key, true); };
    const up = () => { sendKey(key, false); };
    el.addEventListener('pointerdown', down);
    el.addEventListener('pointerup', up);
    el.addEventListener('pointercancel', up);
    el.addEventListener('pointerleave', up);
  }

  function bindPads() {
    const byLabel = (t) => document.querySelector('[aria-label="' + t + '"]');
    holdButton(byLabel('Walk Up'), 'w');
    holdButton(byLabel('Walk Left'), 'a');
    holdButton(byLabel('Walk Right'), 'd');
    holdButton(byLabel('Walk Down'), 's');
    const sprint = document.getElementById('center-run-btn') || document.getElementById('sprint-toggle');
    holdButton(sprint, 'shift');
    // Camera pad: short look-drags from the stream center.
    const cam = { 'Camera Up': [0, -1], 'Camera Left': [-1, 0], 'Camera Right': [1, 0], 'Camera Down': [0, 1] };
    Object.keys(cam).forEach((label) => {
      const el = byLabel(label);
      if (!el) return;
      let timer = 0;
      const step = () => {
        const img = document.getElementById('gpo-stream');
        const r = img ? img.getBoundingClientRect() : { width: 640, height: 360 };
        const cx = 0.5, cy = 0.5;
        const dx = cam[label][0] * 0.18, dy = cam[label][1] * 0.18;
        gpoPost('/api/drag', { x1: cx - dx, y1: cy - dy, x2: cx + dx, y2: cy + dy, ms: 220 }).catch(() => {});
      };
      el.addEventListener('pointerdown', (e) => { e.preventDefault(); step(); timer = setInterval(step, 450); });
      const stop = () => { clearInterval(timer); };
      el.addEventListener('pointerup', stop);
      el.addEventListener('pointercancel', stop);
      el.addEventListener('pointerleave', stop);
    });
  }

  function bindStreamTouch() {
    const img = document.getElementById('gpo-stream');
    if (!img) return;
    img.style.touchAction = 'none';
    let dragStart = null;
    const rel = (e) => {
      const r = img.getBoundingClientRect();
      return {
        x: Math.min(1, Math.max(0, (e.clientX - r.left) / r.width)),
        y: Math.min(1, Math.max(0, (e.clientY - r.top) / r.height)),
      };
    };
    img.addEventListener('pointerdown', (e) => {
      e.preventDefault();
      const p = rel(e);
      if (tapMode) {
        gpoPost('/api/click', { rx: p.x, ry: p.y }).catch(() => {});
      } else {
        dragStart = p;
      }
    });
    img.addEventListener('pointermove', (e) => {
      if (!tapMode && dragStart && e.buttons) {
        const p = rel(e);
        gpoPost('/api/mouse', { rx: p.x, ry: p.y }).catch(() => {});
      }
    });
    const endDrag = (e) => {
      if (!tapMode && dragStart) {
        const p = rel(e);
        gpoPost('/api/drag', { x1: dragStart.x, y1: dragStart.y, x2: p.x, y2: p.y, ms: 250 }).catch(() => {});
        dragStart = null;
      }
    };
    img.addEventListener('pointerup', endDrag);
    img.addEventListener('pointercancel', () => { dragStart = null; });
  }

  function bindStreamControls() {
    // FPS cycle 5 -> 8 -> 10 (backend DoD cap).
    const anchor = document.getElementById('stream-fps-anchor');
    const fpsBtn = anchor ? anchor.closest('button') : null;
    const fpsSteps = [5, 8, 10];
    if (fpsBtn) {
      fpsBtn.addEventListener('click', () => {
        streamFps = fpsSteps[(fpsSteps.indexOf(streamFps) + 1) % fpsSteps.length];
        const spans = fpsBtn.querySelectorAll('span');
        spans.forEach((s) => { if (/^\d+$/.test(s.textContent.trim())) s.textContent = streamFps; });
        reloadStream();
      });
    }
    // Quality cycle 480 -> 720 -> 1080.
    const qSteps = [480, 720, 1080];
    document.querySelectorAll('button').forEach((b) => {
      const t = b.textContent || '';
      if (/720p/.test(t) && !b.dataset.gpoBound) {
        b.dataset.gpoBound = '1';
        b.addEventListener('click', () => {
          streamScale = qSteps[(qSteps.indexOf(streamScale) + 1) % qSteps.length];
          b.innerHTML = b.innerHTML.replace(/\d{3}p/, streamScale + 'p');
          reloadStream();
        });
      }
    });
    const tapBadge = document.getElementById('tap-mode-label');
    if (tapBadge && tapBadge.parentElement) {
      tapBadge.parentElement.style.cursor = 'pointer';
      tapBadge.parentElement.addEventListener('click', () => {
        tapMode = !tapMode;
        tapBadge.textContent = tapMode ? 'TAP' : 'PAD';
        toast(tapMode ? 'Tap-to-click mode' : 'Drag-camera mode');
      });
    }
    const refresh = document.querySelector('[aria-label="Refresh stream"]');
    if (refresh) refresh.addEventListener('click', reloadStream);
    const fs = document.querySelector('[aria-label="Fullscreen stream"]');
    if (fs) fs.addEventListener('click', () => {
      const card = document.getElementById('gpo-stream');
      const host = card ? card.closest('div.relative') : document.documentElement;
      if (document.fullscreenElement) document.exitFullscreen().catch(() => {});
      else if (host && host.requestFullscreen) host.requestFullscreen().catch(() => {});
    });
  }

  async function toggleMacro() {
    try {
      const running = lastStatus && lastStatus.is_running && !lastStatus.paused;
      await gpoAction(running ? 'pause' : 'start');
      await refresh();
    } catch (e) { toast('Macro toggle failed: ' + e.message); }
  }

  function paintMacroBtn(running) {
    const btn = document.getElementById('main-macro-btn');
    if (!btn) return;
    if (running) {
      btn.innerHTML = '<span class="material-symbols-outlined text-[24px] animate-spin">sync</span><span>STOP MACRO // HARVESTING</span>';
      btn.classList.remove('bg-primary-container', 'hover:bg-primary', 'text-on-primary', 'border-primary/40');
      btn.classList.add('bg-error-container', 'text-on-error-container', 'border-error/40', 'macro-active-pulse');
    } else {
      btn.innerHTML = '<span class="material-symbols-outlined text-[24px]">play_arrow</span><span id="macro-btn-label">START MACRO</span>';
      btn.classList.add('bg-primary-container', 'hover:bg-primary', 'text-on-primary', 'border-primary/40');
      btn.classList.remove('bg-error-container', 'text-on-error-container', 'border-error/40', 'macro-active-pulse');
    }
  }

  function bindActions() {
    const macroBtn = gpoRebind(document.getElementById('main-macro-btn'));
    if (macroBtn) macroBtn.addEventListener('click', toggleMacro);
    const pwr = gpoRebind(document.getElementById('pwr-btn'));
    if (pwr) pwr.addEventListener('click', toggleMacro);
    const alerts = gpoRebind(document.getElementById('alerts-btn'));
    if (alerts) alerts.addEventListener('click', async () => {
      try { const j = await gpoAction('toggle_spawn'); toast(j.message); await refresh(); }
      catch (e) { toast('Alerts toggle failed: ' + e.message); }
    });
    const wire = (id, action, okMsg) => {
      const el = document.getElementById(id);
      if (!el) return;
      el.addEventListener('click', async () => {
        try { const j = await gpoAction(action); toast(okMsg || j.message); await refresh(); }
        catch (e) { toast('Failed: ' + e.message); }
      });
    };
    wire('recast-btn', 'recast');
    wire('buy-bait-btn', 'buy_bait');
    wire('update-btn', 'update', 'Update check started — watch Telegram/app for progress.');
    const cast = document.getElementById('sfx-cast-btn');
    if (cast) cast.addEventListener('click', async () => {
      try { await gpoAction('recast'); } catch (e) { toast('Recast failed: ' + e.message); }
    });
    const unmute = gpoRebind(document.getElementById('unmute-btn'));
    if (unmute) unmute.addEventListener('click', async () => {
      try {
        const muted = lastStatus && lastStatus.muted;
        const j = await gpoAction(muted ? 'unmute' : 'mute');
        toast(j.message);
        await refresh();
      } catch (e) { toast('Mute toggle failed: ' + e.message); }
    });
  }

  function paint(d) {
    lastStatus = d;
    const running = d.is_running && !d.paused;
    paintMacroBtn(running);
    gpoPaintPwr(document.getElementById('pwr-btn'), running);
    gpoSetText('macro-status-text', running ? ('● ' + d.state + ' • harvesting') : 'Ready • macro idle');
    gpoSetText('macro-profile-text', 'Profile: GPO Farm v' + (d.version || '—'));
    gpoSetText('stream-event-text', d.last_spawn || 'No spawn alerts yet');
    const muted = !!d.muted;
    gpoSetText('unmute-text', muted ? 'MUTE' : 'UNMUTE');
    const ui = document.getElementById('unmute-icon');
    if (ui) ui.textContent = muted ? 'volume_off' : 'volume_up';
    gpoSetText('tele-sys', 'FISH ' + d.fish + ' • ' + d.state.toUpperCase());
    gpoSetText('tele-gpu', d.fan && d.fan.gpu_temp != null ? ('GPU ' + d.fan.gpu_temp + '°C') : 'GPU —');
  }

  async function refresh() {
    try {
      const { data } = await gpoGet('/api/status');
      paint(data);
    } catch (e) { /* offline: keep last frame */ }
  }

  document.addEventListener('DOMContentLoaded', () => {
    reloadStream();
    bindPads();
    bindStreamTouch();
    bindStreamControls();
    bindActions();
    gpoPaintHostline().then((d) => { if (d) paint(d); });
    refresh();
    setInterval(() => { if (!document.hidden) refresh(); }, 2500);
    const release = () => { gpoPost('/api/key', { key: 'release_all' }).catch(() => {}); };
    document.addEventListener('visibilitychange', () => {
      if (document.hidden) release();
      else reloadStream();
    });
    window.addEventListener('pagehide', release);
  });
})();
