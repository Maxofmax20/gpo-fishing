'use strict';
/* SYSTEM HUB wiring: thermals, RGB, sliders, timers, triggers, reconnect. */
(function () {
  let bossBase = {};   // name -> {secs, at}
  let soundAlerts = localStorage.getItem('gpo_sound_alerts') !== 'false';

  function toast(m, icon) {
    if (typeof showToast === 'function') { try { showToast(m, icon); return; } catch (e) {} }
    gpoToast(m);
  }

  function paintSlider(fillId, badgeId, container, pct, label) {
    const fill = document.getElementById(fillId);
    if (fill) fill.style.width = pct + '%';
    const badge = document.getElementById(badgeId);
    if (badge) badge.textContent = label != null ? label : (pct + '%');
    if (container) {
      const thumb = container.querySelector(':scope > div.absolute');
      if (thumb) thumb.style.left = 'calc(' + pct + '% - 12px)';
    }
  }

  function sliderContainer(idx) {
    const all = document.querySelectorAll('div.relative.w-full.flex.items-center.h-8');
    return all[idx] || null;
  }

  function bindSlider(idx, fillId, badgeId, onSet) {
    const c = sliderContainer(idx);
    if (!c) return;
    c.addEventListener('pointerdown', (e) => {
      const r = c.getBoundingClientRect();
      const pct = Math.max(0, Math.min(100, Math.round(((e.clientX - r.left) / r.width) * 100)));
      onSet(pct);
    });
  }

  function setActiveBtn(list, el) {
    list.forEach((b) => {
      const on = b === el;
      b.classList.toggle('bg-primary', on);
      b.classList.toggle('text-on-primary', on);
      b.classList.toggle('bg-surface-container-high', !on);
      b.classList.toggle('text-on-surface-variant', !on);
    });
  }

  function setActiveRgb(list, el) {
    list.forEach((b) => {
      const on = b === el;
      b.classList.toggle('bg-secondary', on);
      b.classList.toggle('text-on-secondary', on);
      b.classList.toggle('bg-surface-container-high', !on);
      b.classList.toggle('text-on-surface-variant', !on);
    });
  }

  function paintFan(fan) {
    const mode = (fan.current_mode || '').toLowerCase();
    const modes = ['quiet', 'balance', 'performance', 'turbo'];
    const labels = { quiet: 'QUIET', balance: 'BALANCE (AUTO)', performance: 'PERF', turbo: 'TURBO (MAX)' };
    gpoSetText('fan-mode-badge', labels[mode] || (fan.mode_label || '—').toUpperCase());
    const btns = Array.prototype.slice.call(document.querySelectorAll('[data-fanmode]'));
    const active = btns.find((b) => b.getAttribute('data-fanmode') === mode);
    if (active) setActiveBtn(btns, active);
    const pct = fan.fan_level_pct != null ? fan.fan_level_pct : (mode === 'quiet' ? 25 : mode === 'performance' ? 75 : mode === 'turbo' ? 100 : 50);
    paintSlider('fan-speed-fill', 'fan-speed-badge', sliderContainer(0), pct, pct + '% (Auto Managed)');
    const t = document.getElementById('turbo-toggle');
    if (t) {
      const knob = t.firstElementChild;
      const on = !!fan.auto_turbo;
      t.classList.toggle('bg-tertiary/20', on);
      t.classList.toggle('border-tertiary/50', on);
      t.classList.toggle('bg-surface-container-highest', !on);
      if (knob) knob.style.transform = on ? 'translateX(20px)' : '';
    }
    const cpu = fan.cpu_temp, gpu = fan.gpu_temp, load = fan.gpu_util, rpm = fan.est_fan_rpm;
    gpoSetText('metric-cpu', cpu != null ? Math.round(cpu) : '—');
    gpoSetText('metric-gpu', gpu != null ? gpu : '—');
    gpoSetText('metric-load', (fan.gpu_power != null ? fan.gpu_power.toFixed(1) : (load != null ? load : '—')));
    gpoSetText('metric-rpm', rpm ? String(rpm).replace(' RPM', '').replace('~', '~') : '—');
    const pctOf = (v, max) => (v == null ? 0 : Math.max(0, Math.min(100, Math.round((v / max) * 100))));
    const f1 = document.getElementById('metric-cpu-fill'); if (f1) f1.style.width = pctOf(cpu, 100) + '%';
    const f2 = document.getElementById('metric-gpu-fill'); if (f2) f2.style.width = pctOf(gpu, 100) + '%';
    const f3 = document.getElementById('metric-load-fill'); if (f3) f3.style.width = pctOf(fan.gpu_power != null ? fan.gpu_power : load, fan.gpu_power != null ? 200 : 100) + '%';
  }

  function paintLight(kb) {
    const st = (kb && kb.status ? kb.status : 'Unknown').toLowerCase();
    const label = st === 'off' ? 'PRESET: OFF' : st === 'low' ? 'PRESET: LOW' : st === 'high' ? 'PRESET: HIGH' : 'PRESET: ' + st.toUpperCase();
    gpoSetText('rgb-preset-badge', label);
    const btns = Array.prototype.slice.call(document.querySelectorAll('[data-rgb]'));
    const active = btns.find((b) => b.getAttribute('data-rgb') === (st === 'unknown' ? '' : st));
    if (active) setActiveRgb(btns, active);
  }

  function fmtTimer(s) {
    s = Math.max(0, Math.floor(s));
    if (s <= 0) return 'SPAWNED';
    const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), ss = s % 60;
    if (h > 0) return h + 'h ' + String(m).padStart(2, '0') + 'm';
    return String(m).padStart(2, '0') + ':' + String(ss).padStart(2, '0');
  }

  function tickTimers() {
    document.querySelectorAll('.timer-badge[data-seconds]').forEach((el) => {
      const base = el.getAttribute('data-base');
      const at = parseInt(el.getAttribute('data-at') || '0', 10);
      if (!base) return;
      const left = parseInt(base, 10) - Math.floor((Date.now() - at) / 1000);
      el.textContent = fmtTimer(left);
    });
  }

  function paintBosses(bosses) {
    (bosses || []).forEach((b) => {
      const item = document.querySelector('.timer-item[data-target="' + b.name + '"]');
      if (!item) return;
      const badge = item.querySelector('.timer-badge');
      if (!badge) return;
      badge.setAttribute('data-base', String(b.next_spawn_s));
      badge.setAttribute('data-at', String(Date.now()));
      badge.setAttribute('data-seconds', String(b.next_spawn_s));
      badge.textContent = fmtTimer(b.next_spawn_s);
    });
    tickTimers();
  }

  function paintToggles(d) {
    const setPill = (filter, on, onText, offText) => {
      const card = document.querySelector('.toggle-card[data-filter="' + filter + '"]');
      if (!card) return;
      const pill = card.querySelector('.filter-pill');
      if (pill) pill.textContent = on ? onText : offText;
      card.classList.toggle('opacity-70', !on);
    };
    setPill('pity', !!d.legendary_only, 'ACTIVE', 'MUTED');
    setPill('screen', !!d.send_drop_screenshot, 'ACTIVE', 'MUTED');
    gpoSetText('reconnect-state-text', d.auto_reconnect ? 'DAEMON ACTIVE' : 'DAEMON STANDBY');
    const sb = document.getElementById('sound-btn');
    if (sb) {
      const t = sb.querySelector('.btn-text');
      if (t) t.textContent = soundAlerts ? 'SOUND: ON' : 'SOUND: OFF';
    }
    const pb = document.getElementById('push-btn');
    if (pb) {
      const t = pb.querySelector('.btn-text');
      if (t) t.textContent = ('Notification' in window && Notification.permission === 'granted') ? 'PUSH: ON' : 'ENABLE PUSH';
    }
  }

  async function refresh() {
    try {
      const { data } = await gpoGet('/api/status');
      gpoNoteOnline(true);
      gpoSetText('bridge-sync-text', 'BRIDGE SYNCED');
      if (data.fan) paintFan(data.fan);
      if (data.keyboard_light) paintLight(data.keyboard_light);
      paintSlider('volume-fill', 'volume-badge', sliderContainer(1), data.volume != null ? data.volume : 0);
      paintSlider('brightness-fill', 'brightness-badge', sliderContainer(2), data.brightness != null ? data.brightness : 0);
      paintBosses(data.bosses);
      paintToggles(data);
      const code = document.getElementById('input-ps-code');
      if (code && !code.dataset.touched) code.value = data.private_server_code || '';
      const url = document.getElementById('input-vip-url');
      if (url && !url.dataset.touched) url.value = data.vip_server_url || '';
      const ms = document.getElementById('macro-select');
      if (ms && !ms.dataset.loaded && data.macros) {
        ms.dataset.loaded = '1';
        const cur = data.rejoin_macro_name || '';
        ms.innerHTML = '';
        const def = document.createElement('option');
        def.value = '';
        def.textContent = '-- Auto-Click Reconnect & Enter Code --';
        ms.appendChild(def);
        data.macros.forEach((m) => {
          const o = document.createElement('option');
          o.value = m.name;
          o.textContent = m.name + ' (' + m.steps.length + ' steps)';
          ms.appendChild(o);
        });
        ms.value = cur;
      }
      gpoPaintPwr(document.getElementById('pwr-btn'), data.is_running && !data.paused);
    } catch (e) {
      gpoNoteOnline(false);
      gpoSetText('bridge-sync-text', 'BRIDGE OFFLINE');
    }
  }

  function bind() {
    document.querySelectorAll('[data-fanmode]').forEach((b) => {
      b.addEventListener('click', async () => {
        try {
          await gpoPost('/api/fan/set', { mode: b.getAttribute('data-fanmode') });
          toast('Fan: ' + b.getAttribute('data-fanmode'));
          await refresh();
        } catch (e) { toast('Fan failed: ' + e.message); }
      });
    });
    bindSlider(0, 'fan-speed-fill', 'fan-speed-badge', async (pct) => {
      try { await gpoPost('/api/fan/set', { fan_percentage: pct }); await refresh(); }
      catch (e) { toast('Fan failed: ' + e.message); }
    });
    const turbo = document.getElementById('turbo-toggle');
    if (turbo) turbo.addEventListener('click', async () => {
      try {
        const { data } = await gpoGet('/api/status');
        const j = await gpoPost('/api/fan/set', { auto_turbo: !(data.fan && data.fan.auto_turbo) });
        toast('Auto-Turbo ' + ((j && j.auto_turbo) ? 'ENABLED' : 'DISABLED'));
        await refresh();
      } catch (e) { toast('Turbo failed: ' + e.message); }
    });
    document.querySelectorAll('[data-rgb]').forEach((b) => {
      b.addEventListener('click', async () => {
        try {
          const j = await gpoPost('/api/keyboard/light', { action: b.getAttribute('data-rgb') });
          toast('Keyboard light: ' + (j.status || j.message || b.getAttribute('data-rgb')).toString().toUpperCase());
          await refresh();
        } catch (e) { toast('Light failed: ' + e.message); }
      });
    });
    bindSlider(1, 'volume-fill', 'volume-badge', async (pct) => {
      try { await gpoAction('set_volume', { value: pct }); await refresh(); }
      catch (e) { toast('Volume failed: ' + e.message); }
    });
    bindSlider(2, 'brightness-fill', 'brightness-badge', async (pct) => {
      try { await gpoAction('set_brightness', { value: pct }); await refresh(); }
      catch (e) { toast('Brightness failed: ' + e.message); }
    });
    const snd = document.getElementById('sound-btn');
    if (snd) snd.addEventListener('click', () => {
      soundAlerts = !soundAlerts;
      localStorage.setItem('gpo_sound_alerts', soundAlerts);
      toast(soundAlerts ? 'Browser sound alerts ON' : 'Browser sound alerts OFF');
      refresh();
    });
    const push = document.getElementById('push-btn');
    if (push) push.addEventListener('click', async () => {
      try {
        if (!('Notification' in window)) { toast('Push not supported here'); return; }
        const p = await Notification.requestPermission();
        toast(p === 'granted' ? 'Push alerts enabled' : 'Push permission: ' + p);
        await refresh();
      } catch (e) { toast('Push failed: ' + e.message); }
    });
    const pityCard = document.querySelector('.toggle-card[data-filter="pity"]');
    if (pityCard) pityCard.addEventListener('click', async () => {
      try { const j = await gpoAction('toggle_legendary_only'); toast(j.message); await refresh(); }
      catch (e) { toast('Failed: ' + e.message); }
    });
    const screenCard = document.querySelector('.toggle-card[data-filter="screen"]');
    if (screenCard) screenCard.addEventListener('click', async () => {
      try { const j = await gpoAction('toggle_drop_screenshot'); toast(j.message); await refresh(); }
      catch (e) { toast('Failed: ' + e.message); }
    });
    const rec = document.getElementById('reconnect-toggle');
    if (rec) rec.addEventListener('click', async () => {
      try { const j = await gpoAction('toggle_auto_reconnect'); toast(j.message); await refresh(); }
      catch (e) { toast('Failed: ' + e.message); }
    });
    const markTouched = (id) => {
      const el = document.getElementById(id);
      if (el) el.addEventListener('input', () => { el.dataset.touched = '1'; });
    };
    markTouched('input-ps-code'); markTouched('input-vip-url');
    const saveCode = document.getElementById('save-code-btn');
    if (saveCode) saveCode.addEventListener('click', async () => {
      const el = document.getElementById('input-ps-code');
      try { const j = await gpoAction('set_private_server_code', { value: el ? el.value : '' }); toast(j.message); }
      catch (e) { toast('Save failed: ' + e.message); }
    });
    const saveUrl = document.getElementById('save-url-btn');
    if (saveUrl) saveUrl.addEventListener('click', async () => {
      const el = document.getElementById('input-vip-url');
      try { const j = await gpoAction('set_vip_url', { value: el ? el.value : '' }); toast(j.message); }
      catch (e) { toast('Save failed: ' + e.message); }
    });
    const ms = document.getElementById('macro-select');
    if (ms) ms.addEventListener('change', async () => {
      try { const j = await gpoAction('set_rejoin_macro', { value: ms.value }); toast(j.message); }
      catch (e) { toast('Save failed: ' + e.message); }
    });
    const studio = document.getElementById('studio-btn');
    if (studio) studio.addEventListener('click', () => { location.href = '/studio?token=' + encodeURIComponent(GPO_TOKEN); });
    const pwr = document.getElementById('pwr-btn');
    if (pwr) pwr.addEventListener('click', async () => {
      try {
        const { data } = await gpoGet('/api/status');
        const running = data.is_running && !data.paused;
        await gpoAction(running ? 'pause' : 'start');
        toast(running ? 'Macro paused' : 'Macro started');
        await refresh();
      } catch (e) { toast('Failed: ' + e.message); }
    });
    const alerts = document.getElementById('alerts-header-btn');
    if (alerts) alerts.addEventListener('click', async () => {
      try { const j = await gpoAction('toggle_spawn'); toast(j.message); await refresh(); }
      catch (e) { toast('Failed: ' + e.message); }
    });
  }

  // MODEL & DATASET module: fetched on load + manual refresh only — the
  // full validation pass is expensive on large datasets, never polled.
  async function refreshMl() {
    const box = document.getElementById('ml-body');
    if (!box) return;
    box.innerHTML = '<div class="font-body-sm text-body-sm text-on-surface-variant">Reading dataset state…</div>';
    try {
      const { data } = await gpoGet('/api/ml/status');
      const d = data.dataset || {};
      const r = data.readiness || {};
      const m = data.model || {};
      const cov = d.state_coverage || {};
      const w = cov.waiting_for_bite || {};
      const b = cov.bite || {};
      const cr = cov.catch_result || {};
      const pct = Math.min(100, Math.round(((r.entity_linked_result || 0) / Math.max(1, 1000)) * 100));
      const modelLine = m.trained
        ? 'Model ' + gpoEsc(m.name || '?') + ' v' + gpoEsc(m.version || '?') + ' loaded (' + gpoEsc(m.runtime || '?') + ').'
        : 'MODEL: NOT TRAINED' + (m.reason ? ' — ' + gpoEsc(m.reason) : '') + '. OCR + heuristics active.';
      box.innerHTML =
        '<div class="flex items-center justify-between">' +
          '<span class="font-label-md text-label-md uppercase text-on-surface">Vision Model</span>' +
          '<span class="font-label-sm text-label-sm uppercase font-bold ' + (m.trained ? 'text-tertiary' : 'text-[#f59e0b]') + '">' + (m.trained ? 'LOADED' : 'NOT TRAINED') + '</span></div>' +
        '<div class="font-body-sm text-body-sm text-on-surface-variant">' + modelLine + '</div>' +
        '<div class="flex items-center justify-between mt-1">' +
          '<span class="font-label-md text-label-md uppercase text-on-surface">Dataset gpo-vision/v1</span>' +
          '<span class="font-label-sm text-label-sm uppercase font-bold ' + (r.ready ? 'text-tertiary' : 'text-[#f59e0b]') + '">' + gpoEsc(r.status || (r.ready ? 'READY' : 'BLOCKED')) + '</span></div>' +
        '<div class="w-full h-2 rounded-full bg-surface-container-highest overflow-hidden">' +
          '<div class="h-full bg-gradient-to-r from-primary-container to-primary rounded-full" style="width:' + pct + '%"></div></div>' +
        '<div class="font-mono text-[11px] text-on-surface-variant">Entity-linked RESULTs: ' + (r.entity_linked_result || 0) + ' / 1000' +
          ' · WAITING eligible ' + (w.eligible || 0) + ' · BITE eligible ' + (b.eligible || 0) + ' · RESULT eligible ' + (cr.eligible || 0) + '</div>' +
        '<div class="font-mono text-[11px] text-on-surface-variant">Sessions ' + (d.sessions || 0) + ' · Entities ' + (d.entities || 0) + ' · Hard ' + (d.hard || 0) +
          ' · exact-dup ' + ((d.leakage && d.leakage.exact_duplicate_files ? d.leakage.exact_duplicate_files.length : 0)) +
          ' · near-sim ' + ((d.leakage && d.leakage.near_similarity_groups != null) ? d.leakage.near_similarity_groups : '—') + '</div>' +
        '<div class="font-mono text-[11px] text-[#f59e0b]">' + gpoEsc(r.ready ? 'All gates pass.' : ('Blocking: ' + (r.blocking_requirement || 'unknown'))) + '</div>';
    } catch (e) {
      box.innerHTML = '<div class="font-body-sm text-body-sm text-error">Model state unavailable: ' + gpoEsc(e.message) + '</div>';
    }
  }

  document.addEventListener('DOMContentLoaded', () => {
    bind();
    document.querySelectorAll('.timer-badge').forEach((el) => { el.textContent = '—'; });
    gpoPaintHostline();
    refresh();
    refreshMl();
    const mrb = document.getElementById('ml-refresh-btn');
    if (mrb) mrb.addEventListener('click', refreshMl);
    setInterval(() => { if (!document.hidden) refresh(); }, 3000);
    setInterval(tickTimers, 1000);
  });
})();
