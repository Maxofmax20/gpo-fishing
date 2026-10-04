'use strict';
/* MACRO STUDIO wiring: record, macro library, loop/speed playback, inspector. */
(function () {
  let macros = [];
  let selectedName = '';
  let loopSel = null;   // null = endless
  let speedSel = 1;
  let lastRecorder = null;

  function toast(m) { gpoToast(m); }

  // "Calibrate Timers" has no backend calibrator: report the live loop
  // round-trip honestly instead of faking a calibration run.
  async function calibrateReport() {
    try {
      const { ms } = await gpoGet('/api/status');
      toast('Timers auto-tracked • API ' + ms + 'ms • no manual calibration needed');
    } catch (e) { toast('Backend unreachable: ' + e.message); }
  }

  // ---- globals required by the page's inline onclick handlers ----
  window.selectLoop = function (el, label) {
    document.querySelectorAll('.loop-btn').forEach((b) => {
      b.classList.remove('active-loop', 'bg-secondary-container', 'text-on-secondary-container', 'shadow-sm', 'font-bold');
      b.classList.add('text-on-surface-variant');
    });
    el.classList.add('active-loop', 'bg-secondary-container', 'text-on-secondary-container', 'shadow-sm', 'font-bold');
    el.classList.remove('text-on-surface-variant');
    loopSel = (label === '∞ Endless') ? null : parseInt(label, 10);
    gpoSetText('active-loop-label', label === '∞ Endless' ? '∞ Endless' : (label + ' loops'));
  };

  const SPEEDS = [1, 2, 5, 10, 25, 100];
  const SPEED_LABELS = ['1x (Native)', '2x Overclock', '5x Accelerated', '10x Turbo', '25x Hyper', '100x Insane⚡'];
  window.selectSpeed = function (el, _label, idx) {
    document.querySelectorAll('.speed-btn').forEach((b) => {
      b.classList.remove('active-speed', 'bg-primary', 'text-on-primary', 'font-bold');
      b.classList.add('text-on-surface-variant');
    });
    el.classList.add('active-speed', 'bg-primary', 'text-on-primary', 'font-bold');
    el.classList.remove('text-on-surface-variant');
    speedSel = SPEEDS[idx] || 1;
    gpoSetText('active-speed-label', SPEED_LABELS[idx] || '1x (Native)');
  };

  window.triggerPlayOnce = function () { playSelected(false); };
  window.triggerLoopPlay = function () { playSelected(true); };
  window.removeStep = function () {
    toast('Steps are fixed once recorded — delete the macro to re-record.');
  };

  async function playSelected(loop) {
    if (!selectedName) { toast('Pick a macro first'); return; }
    try {
      if (loop && lastRecorder && lastRecorder.is_looping && lastRecorder.playing_macro_name === selectedName) {
        await gpoPost('/api/macro/play', { action: 'stop' });
        toast('Loop stopped');
      } else if (loop) {
        const body = { action: 'loop', name: selectedName, speed: speedSel };
        if (loopSel) body.max_loops = loopSel;
        await gpoPost('/api/macro/play', body);
        toast('Looping ' + selectedName + (loopSel ? ' ×' + loopSel : ' ∞') + ' @ ' + speedSel + 'x');
      } else {
        await gpoPost('/api/macro/play', { action: 'play', name: selectedName, speed: speedSel });
        toast('Playing ' + selectedName);
      }
      await refreshMacros();
    } catch (e) { toast('Playback failed: ' + e.message); }
  }

  function stepDesc(s, i) {
    const t = s.type || '?';
    const pct = (v) => Math.round((v || 0) * 100) + '%';
    if (t === 'Click') return { icon: 'touch_app', cls: 'text-primary', title: 'LEFT CLICK (' + pct(s.rx) + ', ' + pct(s.ry) + ')', sub: '+' + (s.delay_ms || 0) + 'ms', ms: s.delay_ms || 0 };
    if (t === 'Drag') return { icon: 'swipe', cls: 'text-secondary', title: 'DRAG (' + pct(s.start_rx) + ', ' + pct(s.start_ry) + ' → ' + pct(s.end_rx) + ', ' + pct(s.end_ry) + ')', sub: 'Hold ' + (s.duration_ms || 0) + 'ms', ms: (s.duration_ms || 0) + (s.delay_ms || 0) };
    if (t === 'KeyTap') return { icon: 'keyboard', cls: 'text-primary', title: 'KEY ' + String(s.key || '').toUpperCase(), sub: '+' + (s.delay_ms || 0) + 'ms', ms: s.delay_ms || 0 };
    if (t === 'KeyHold') return { icon: 'keyboard', cls: 'text-primary', title: 'HOLD ' + String(s.key || '').toUpperCase() + ' ' + (s.duration_ms || 0) + 'ms', sub: '+' + (s.delay_ms || 0) + 'ms', ms: (s.duration_ms || 0) + (s.delay_ms || 0) };
    if (t === 'MouseMove') return { icon: 'mouse', cls: 'text-primary', title: 'MOVE (' + pct(s.rx) + ', ' + pct(s.ry) + ')', sub: '+' + (s.delay_ms || 0) + 'ms', ms: s.delay_ms || 0 };
    if (t === 'Sleep') return { icon: 'timer', cls: 'text-on-surface-variant', title: 'WAIT ' + (s.ms || 0) + 'ms', sub: '', ms: s.ms || 0 };
    return { icon: 'vpn_key', cls: 'text-secondary', title: 'VPN ' + t, sub: (s.timeout_s || 0) + 's timeout', ms: 0 };
  }

  function renderInspector() {
    const body = document.getElementById('inspector-body');
    const m = macros.find((x) => x.name === selectedName);
    const steps = (m && m.steps) || [];
    gpoSetText('step-count-badge', steps.length + ' STEPS');
    if (!body) return;
    // Keep the toolbar row out (removed at build); render rows only.
    body.innerHTML = '';
    let total = 0;
    steps.forEach((s, i) => {
      const d = stepDesc(s, i);
      total += d.ms;
      const row = document.createElement('div');
      row.className = 'step-row flex items-center justify-between p-2 rounded-lg bg-surface-container-low hover:bg-surface-container-high transition-all border border-transparent';
      row.innerHTML =
        '<div class="flex items-center gap-2 min-w-0">' +
          '<span class="font-label-sm text-label-sm text-on-surface-variant font-mono w-4">#' + (i + 1) + '</span>' +
          '<span class="material-symbols-outlined ' + d.cls + ' text-[16px] shrink-0">' + d.icon + '</span>' +
          '<div class="flex flex-col min-w-0">' +
            '<span class="font-label-sm text-label-sm ' + d.cls + ' font-bold truncate">' + gpoEsc(d.title) + '</span>' +
            (d.sub ? '<span class="font-label-sm text-label-sm text-on-surface-variant">' + gpoEsc(d.sub) + '</span>' : '') +
          '</div></div>' +
        '<div class="flex items-center gap-2 shrink-0">' +
          '<span class="font-label-sm text-label-sm text-on-surface-variant font-mono bg-surface-container-lowest px-1.5 py-0.5 rounded">+' + d.ms + 'ms</span>' +
        '</div>';
      body.appendChild(row);
    });
    if (!steps.length) {
      body.innerHTML = '<div class="text-center font-label-sm text-label-sm text-on-surface-variant uppercase py-3">No steps — record a workflow above.</div>';
    }
    // TOTAL label lives in the inspector header button.
    const toggle = document.getElementById('inspector-toggle');
    const totalEl = toggle ? toggle.querySelector('.font-mono') : null;
    if (totalEl) totalEl.textContent = 'TOTAL: ' + total + 'ms';
  }

  function renderMacroSelect() {
    const sel = document.getElementById('macro-select');
    if (!sel) return;
    sel.innerHTML = '';
    if (!macros.length) {
      const o = document.createElement('option');
      o.textContent = 'No macros recorded yet';
      sel.appendChild(o);
      gpoSetText('macro-sub', 'Record your first workflow above');
      selectedName = '';
    } else {
      macros.forEach((m) => {
        const o = document.createElement('option');
        o.value = m.name;
        o.textContent = m.name + ' (' + m.steps.length + ' steps)';
        sel.appendChild(o);
      });
      if (!macros.some((m) => m.name === selectedName)) selectedName = macros[0].name;
      sel.value = selectedName;
      const m = macros.find((x) => x.name === selectedName);
      gpoSetText('macro-sub', m ? (m.steps.length + ' steps • saved ' + (m.created_at || '')) : '');
    }
    renderInspector();
  }

  function paintRecorder(r) {
    lastRecorder = r;
    const recording = !!r.is_recording;
    gpoSetText('record-badge', recording ? ('REC • ' + r.recorded_steps_count) : 'Ready');
    gpoSetText('record-btn-text', recording ? 'Stop & Save' : 'Record');
    gpoSetText('play-status-badge', r.is_looping ? ('LOOP ×' + (r.current_loop || '∞')) : (r.is_playing ? 'PLAYING' : 'ARMED'));
    const dot = document.getElementById('engine-status-dot');
    const txt = document.getElementById('engine-status-text');
    const st = recording ? 'RECORDING' : (r.is_looping ? 'LOOPING' : (r.is_playing ? 'PLAYING' : 'ENGINE IDLE'));
    if (txt) txt.textContent = st;
    if (dot) dot.className = 'w-2 h-2 rounded-full animate-ping ' + (st === 'ENGINE IDLE' ? 'bg-tertiary' : 'bg-error');
    const loopLabel = document.getElementById('loop-btn-label');
    if (loopLabel) loopLabel.textContent = (r.is_looping && r.playing_macro_name === selectedName) ? 'Stop Loop' : 'Loop Play';
  }

  async function refreshMacros() {
    try {
      const { data } = await gpoGet('/api/macro/list');
      macros = data.macros || [];
      renderMacroSelect();
      if (data.status) paintRecorder(data.status);
    } catch (e) { /* offline */ }
  }

  function bind() {
    const sel = document.getElementById('macro-select');
    if (sel) sel.addEventListener('change', () => { selectedName = sel.value; renderInspector(); });
    const recBtn = document.getElementById('record-btn');
    if (recBtn) recBtn.addEventListener('click', async () => {
      try {
        const { data } = await gpoGet('/api/macro/list');
        if (data.status && data.status.is_recording) {
          const input = document.getElementById('macro-name-input');
          const name = (input && input.value.trim()) || ('Macro ' + new Date().toISOString().slice(5, 16).replace('T', ' '));
          const j = await gpoPost('/api/macro/record', { action: 'stop', name });
          toast(j.message);
        } else {
          await gpoPost('/api/macro/record', { action: 'start' });
          toast('Recording — switch to Remote and perform the sequence');
        }
        await refreshMacros();
      } catch (e) { toast('Record failed: ' + e.message); }
    });
    const ren = document.getElementById('macro-rename-btn');
    if (ren) ren.addEventListener('click', async () => {
      if (!selectedName) { toast('Pick a macro first'); return; }
      const nn = prompt('Rename macro:', selectedName);
      if (!nn || !nn.trim() || nn.trim() === selectedName) return;
      try {
        await gpoPost('/api/macro/rename', { name: selectedName, new_name: nn.trim() });
        selectedName = nn.trim();
        toast('Renamed');
        await refreshMacros();
      } catch (e) { toast('Rename failed: ' + e.message); }
    });
    const del = document.getElementById('macro-delete-btn');
    if (del) del.addEventListener('click', async () => {
      if (!selectedName) { toast('Pick a macro first'); return; }
      if (!confirm('Delete macro "' + selectedName + '"?')) return;
      try {
        await gpoPost('/api/macro/delete', { name: selectedName });
        selectedName = '';
        toast('Deleted');
        await refreshMacros();
      } catch (e) { toast('Delete failed: ' + e.message); }
    });
    const cal = document.getElementById('calibrate-btn');
    if (cal) cal.addEventListener('click', calibrateReport);
    const pwr = document.getElementById('pwr-btn');
    if (pwr) pwr.addEventListener('click', async () => {
      try {
        const { data } = await gpoGet('/api/status');
        const running = data.is_running && !data.paused;
        await gpoAction(running ? 'pause' : 'start');
        toast(running ? 'Macro paused' : 'Macro started');
      } catch (e) { toast('Failed: ' + e.message); }
    });
    const alerts = document.getElementById('alerts-btn');
    if (alerts) alerts.addEventListener('click', async () => {
      try { const j = await gpoAction('toggle_spawn'); toast(j.message); }
      catch (e) { toast('Failed: ' + e.message); }
    });
  }

  async function refreshMeta() {
    try {
      const { data, ms } = await gpoGet('/api/status');
      gpoSetText('sync-rtt-text', 'Stable (' + ms + 'ms)');
      gpoSetText('studio-pity-text', 'Pity: ' + (data.pity_legendary || 0) + '/100');
      gpoSetText('studio-queue-text', 'Steps: ' + (lastRecorder ? lastRecorder.recorded_steps_count : 0));
      gpoPaintPwr(document.getElementById('pwr-btn'), data.is_running && !data.paused);
    } catch (e) { /* offline */ }
  }

  document.addEventListener('DOMContentLoaded', () => {
    bind();
    // Prime the inspector neutral: static mock rows must never present,
    // even before the first macro-list fetch lands.
    const inspBody = document.getElementById('inspector-body');
    if (inspBody) inspBody.innerHTML = '';
    gpoSetText('step-count-badge', '0 STEPS');
    const inspToggle = document.getElementById('inspector-toggle');
    const inspTotal = inspToggle ? inspToggle.querySelector('.font-mono') : null;
    if (inspTotal) inspTotal.textContent = 'TOTAL: —';
    gpoPaintHostline();
    refreshMacros();
    refreshMeta();
    setInterval(() => { if (!document.hidden) { refreshMacros(); refreshMeta(); } }, 3000);
  });
})();
