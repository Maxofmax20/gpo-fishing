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
    return withToken('/api/stream?fps=' + streamFps + '&scale=' + streamScale + '&q=80') + '&t=' + Date.now();
  }

  // "No game" overlay: the MJPEG endpoint replays the last frame when
  // Roblox is gone, so stale pixels must never impersonate a live stream.
  function paintStreamOverlay(live) {
    const img = document.getElementById('gpo-stream');
    if (!img || !img.parentElement) return;
    let ov = document.getElementById('gpo-stream-overlay');
    if (!ov) {
      ov = document.createElement('div');
      ov.id = 'gpo-stream-overlay';
      ov.className = 'absolute inset-0 z-10 flex-col items-center justify-center gap-2 bg-surface-dim/85 backdrop-blur-sm';
      ov.style.display = 'none';
      ov.innerHTML =
        '<span class="material-symbols-outlined text-[#f59e0b] text-[28px]">videocam_off</span>' +
        '<span class="font-label-md text-label-md text-on-surface uppercase font-bold tracking-wider">Roblox not detected</span>' +
        '<span class="font-body-sm text-body-sm text-on-surface-variant px-6 text-center">Start Roblox on the PC — showing last captured frame.</span>';
      img.parentElement.appendChild(ov);
    }
    ov.style.display = live ? 'none' : 'flex';
  }

  function reloadStream() {
    const img = document.getElementById('gpo-stream');
    if (!img) return;
    img.style.display = '';
    img.src = streamUrl();
  }

  function hideBrokenStream() {
    const img = document.getElementById('gpo-stream');
    if (!img) return;
    // Empty or failed source: hide the broken-image glyph (the
    // not-detected overlay covers the area with a real explanation).
    if (!img.getAttribute('src')) img.style.display = 'none';
    img.addEventListener('error', () => { img.style.display = 'none'; });
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
    // Shift Lock is a TOGGLE in Roblox (tap Shift), not a hold-to-sprint:
    // both deck buttons flip it and show the resulting state.
    let shiftLock = false;
    const paintShift = () => {
      document.querySelectorAll('[data-shiftlock]').forEach((b) => {
        b.classList.toggle('border-tertiary', shiftLock);
        b.classList.toggle('text-tertiary', shiftLock);
      });
      const t = document.getElementById('shift-toggle-label');
      if (t) t.textContent = shiftLock ? 'LOCKED' : 'SHIFT LOCK';
    };
    const tapShiftLock = async (el) => {
      try {
        await gpoPost('/api/key', { key: 'shift', tap: true });
        shiftLock = !shiftLock;
        paintShift();
        toast(shiftLock ? 'Shift Lock ON' : 'Shift Lock OFF');
      } catch (e) { toast('Shift Lock failed: ' + e.message); }
    };
    ['center-run-btn', 'sprint-toggle'].forEach((id) => {
      const el = document.getElementById(id);
      if (el) {
        el.setAttribute('data-shiftlock', '1');
        el.setAttribute('aria-label', 'Toggle Shift Lock');
        el.addEventListener('click', () => tapShiftLock(el));
      }
    });
    // Camera pad: short RIGHT-button look-drags from the stream center.
    // Roblox rotates the camera on right-drag; left-drag does nothing here,
    // which is why the old left-drag binding appeared dead.
    const cam = { 'Camera Up': [0, -1], 'Camera Left': [-1, 0], 'Camera Right': [1, 0], 'Camera Down': [0, 1] };
    Object.keys(cam).forEach((label) => {
      const el = byLabel(label);
      if (!el) return;
      let timer = 0;
      const step = () => {
        const cx = 0.5, cy = 0.5;
        const dx = cam[label][0] * 0.18, dy = cam[label][1] * 0.18;
        gpoPost('/api/drag', { x1: cx - dx, y1: cy - dy, x2: cx + dx, y2: cy + dy, ms: 220, button: 'right' }).catch(() => {});
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

  function closeMenus() {
    [['fps-toggle', 'fps-menu'], ['quality-toggle', 'quality-menu']].forEach(([t, m]) => {
      const menu = document.getElementById(m);
      if (menu) { menu.classList.add('hidden'); menu.classList.remove('flex'); }
    });
  }

  function paintStreamLabels() {
    gpoSetText('fps-label', streamFps);
    gpoSetText('stream-fps-label', streamFps + ' FPS');
    gpoSetText('quality-label', streamScale + 'p');
    const pill = document.getElementById('header-live-pill');
    if (pill) pill.textContent = 'LIVE ' + streamFps + ' FPS • ' + streamScale + 'p';
    document.querySelectorAll('#fps-menu [data-fps]').forEach((b) => {
      const on = parseInt(b.getAttribute('data-fps'), 10) === streamFps;
      b.classList.toggle('bg-primary-container', on);
      b.classList.toggle('text-on-primary', on);
      b.classList.toggle('text-on-surface-variant', !on);
    });
    document.querySelectorAll('#quality-menu [data-scale]').forEach((b) => {
      const on = parseInt(b.getAttribute('data-scale'), 10) === streamScale;
      b.classList.toggle('bg-primary-container', on);
      b.classList.toggle('text-on-primary', on);
      b.classList.toggle('text-on-surface-variant', !on);
    });
  }

  function bindStreamControls() {
    // Explicit dropdowns: pick FPS (backend DoD cap 10) and scale directly.
    const fpsToggle = document.getElementById('fps-toggle');
    const fpsMenu = document.getElementById('fps-menu');
    if (fpsToggle && fpsMenu) {
      fpsToggle.addEventListener('click', (e) => {
        e.stopPropagation();
        const open = fpsMenu.classList.contains('hidden');
        closeMenus();
        if (open) { fpsMenu.classList.remove('hidden'); fpsMenu.classList.add('flex'); }
      });
      fpsMenu.querySelectorAll('[data-fps]').forEach((b) => {
        b.addEventListener('click', () => {
          streamFps = parseInt(b.getAttribute('data-fps'), 10) || 8;
          paintStreamLabels();
          closeMenus();
          reloadStream();
        });
      });
    }
    const qToggle = document.getElementById('quality-toggle');
    const qMenu = document.getElementById('quality-menu');
    if (qToggle && qMenu) {
      qToggle.addEventListener('click', (e) => {
        e.stopPropagation();
        const open = qMenu.classList.contains('hidden');
        closeMenus();
        if (open) { qMenu.classList.remove('hidden'); qMenu.classList.add('flex'); }
      });
      qMenu.querySelectorAll('[data-scale]').forEach((b) => {
        b.addEventListener('click', () => {
          streamScale = parseInt(b.getAttribute('data-scale'), 10) || 720;
          paintStreamLabels();
          closeMenus();
          reloadStream();
        });
      });
    }
    document.addEventListener('click', closeMenus);
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
    // Fruit slot: taps hotbar key 2 (matches the deck's [2] FRUIT mapping).
    const skill = document.getElementById('sfx-skill-btn');
    if (skill) skill.addEventListener('click', async () => {
      try {
        await gpoPost('/api/key', { key: '2', tap: true });
        toast('Sent key 2 (fruit slot)');
      } catch (e) { toast('Slot key failed: ' + e.message); }
    });
    // Emote button: the game exposes no emote API to drive — say so instead
    // of pretending the tap did something.
    const emote = document.querySelector('[aria-label="Emote Wave"]');
    if (emote) emote.addEventListener('click', () => {
      toast('Emotes have no game API — nothing sent');
    });
    // Type-on-PC + clipboard dialog (chat bubble button).
    const typeDlg = document.getElementById('type-dialog');
    const typeBox = document.getElementById('type-text');
    const openType = document.getElementById('type-open-btn');
    if (openType && typeDlg) openType.addEventListener('click', () => {
      typeDlg.style.display = 'grid';
      if (typeBox) typeBox.focus();
    });
    const closeType = document.getElementById('type-close-btn');
    if (closeType && typeDlg) closeType.addEventListener('click', () => { typeDlg.style.display = 'none'; });
    if (typeDlg) typeDlg.addEventListener('click', (e) => { if (e.target === typeDlg) typeDlg.style.display = 'none'; });
    const typeSend = document.getElementById('type-send-btn');
    if (typeSend && typeBox) typeSend.addEventListener('click', async () => {
      const text = typeBox.value || '';
      if (!text.trim()) { toast('Type something first'); return; }
      try {
        const j = await gpoPost('/api/type', { text });
        toast('Typed ' + j.typed + ' chars on PC');
        typeBox.value = '';
      } catch (e) { toast('Typing failed: ' + e.message); }
    });
    const clipGet = document.getElementById('clip-get-btn');
    if (clipGet && typeBox) clipGet.addEventListener('click', async () => {
      try {
        const j = await gpoGet('/api/clipboard/get');
        // gpoGet returns {data, ms}; endpoint body is {ok, text}.
        const body = j.data || {};
        if (body.ok === false) throw new Error(body.message || 'unavailable');
        typeBox.value = body.text || '';
        toast(body.text ? 'PC clipboard pulled' : 'PC clipboard has no text');
      } catch (e) { toast('Clipboard read failed: ' + e.message); }
    });
    const clipSet = document.getElementById('clip-set-btn');
    if (clipSet && typeBox) clipSet.addEventListener('click', async () => {
      try {
        const j = await gpoPost('/api/clipboard/set', { text: typeBox.value || '' });
        toast('PC clipboard set (' + j.chars + ' chars)');
      } catch (e) { toast('Clipboard write failed: ' + e.message); }
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

  function paint(d, ms) {
    lastStatus = d;
    paintStreamOverlay(!!d.stream_live);
    if (ms != null) gpoSetText('stream-rtt', ms + 'ms');
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
      const { data, ms } = await gpoGet('/api/status');
      paint(data, ms);
    } catch (e) { /* offline: keep last frame */ }
  }

  document.addEventListener('DOMContentLoaded', () => {
    hideBrokenStream();
    reloadStream();
    paintStreamLabels();
    gpoSetText('stream-event-text', 'Awaiting spawn data…');
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
