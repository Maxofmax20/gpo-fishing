'use strict';
/* CRAFT STATION wiring: real backend tiers, target-based auto-stop, live log. */
(function () {
  const TIERS = {
    common: { title: 'Common Fish Bait', synth: 'TIER 1 SYNTH', materials: 'Common bait • uses caught fish + Peli, crafts until materials run out' },
    rare: { title: 'Rare Fish Bait', synth: 'TIER 3 SYNTH', materials: 'Rare bait • uses caught fish + Peli, crafts until materials run out' },
    legendary: { title: 'Legendary Fish Bait', synth: 'TIER 5 SYNTH', materials: 'Legendary bait • uses stock fish + Peli, crafts until materials run out' },
    all: { title: 'All Tiers', synth: 'FULL SWEEP', materials: 'Legendary first, then rare • crafts until materials run out' },
  };
  let tier = 'rare';
  let target = 25;
  let craftStartMs = 0;
  let logEntries = [];

  function toast(m) {
    const box = document.getElementById('audio-toast');
    const msg = document.getElementById('toast-msg');
    if (box && msg) {
      msg.textContent = m;
      box.classList.remove('opacity-0');
      clearTimeout(window._craftToast);
      window._craftToast = setTimeout(() => box.classList.add('opacity-0'), 1800);
    } else {
      gpoToast(m);
    }
  }

  function fmtElapsed(ms) {
    const s = Math.floor(ms / 1000);
    return Math.floor(s / 60) + 'm ' + String(s % 60).padStart(2, '0') + 's';
  }

  function nowTime() {
    const d = new Date();
    let h = d.getHours(), m = String(d.getMinutes()).padStart(2, '0');
    const ap = h >= 12 ? 'PM' : 'AM';
    h = h % 12 || 12;
    return h + ':' + m + ' ' + ap;
  }

  function pushLog(title, sub) {
    logEntries.unshift({ title, sub, at: nowTime() });
    logEntries = logEntries.slice(0, 8);
    renderLog();
  }

  function renderLog() {
    const box = document.getElementById('craft-logs');
    if (!box) return;
    box.innerHTML = '';
    if (!logEntries.length) {
      box.innerHTML = '<div class="text-center font-label-sm text-label-sm text-on-surface-variant uppercase py-3">No craft runs yet this session.</div>';
      return;
    }
    logEntries.forEach((e) => {
      const row = document.createElement('div');
      row.className = 'flex items-center justify-between p-space-sm rounded-lg bg-surface-container-low hover:bg-surface-bright/40 cursor-pointer active:scale-[0.99] transition-all log-entry';
      row.innerHTML =
        '<div class="flex items-center gap-space-sm min-w-0">' +
          '<div class="w-2 h-2 rounded-full bg-tertiary shrink-0 shadow-[0_0_8px_#4edea3]"></div>' +
          '<div class="flex flex-col min-w-0">' +
            '<span class="font-label-md text-body-sm font-semibold text-on-surface truncate">' + gpoEsc(e.title) + '</span>' +
            '<span class="font-body-sm text-[11px] text-on-surface-variant">' + gpoEsc(e.sub) + '</span>' +
          '</div></div>' +
        '<span class="font-label-sm text-[10px] text-on-surface-variant shrink-0 font-medium tracking-wider pl-2">' + gpoEsc(e.at) + '</span>';
      box.appendChild(row);
    });
  }

  function setQty(n) {
    target = Math.max(1, Math.min(99, n));
    gpoSetText('batch-display', target);
    const slider = document.getElementById('batch-slider');
    if (slider) slider.value = target;
    paintTargets();
  }

  function paintTargets() {
    gpoSetText('req-fish', target + 'x target');
    gpoSetText('req-peli', TIERS[tier].title);
    gpoSetText('estimated-time', 'Est. Run: —');
  }

  function selectTier(t) {
    if (!TIERS[t]) return;
    tier = t;
    gpoSetText('selected-recipe-title', TIERS[t].title);
    const mt = document.getElementById('recipe-materials-text');
    if (mt) mt.textContent = TIERS[t].materials;
    document.querySelectorAll('span').forEach((s) => {
      if (s.textContent.trim() === 'Tier 3 Synth' || s.textContent.trim() === 'TIER 3 SYNTH' || /TIER \d SYNTH|FULL SWEEP/.test(s.textContent.trim())) {
        s.textContent = TIERS[t].synth;
      }
    });
    document.querySelectorAll('#recipe-menu [data-tier]').forEach((b) => {
      const on = b.getAttribute('data-tier') === t;
      b.classList.toggle('bg-surface-container', !on);
      b.classList.toggle('bg-surface-container-high', on);
    });
    const menu = document.getElementById('recipe-menu');
    if (menu) { menu.classList.add('hidden'); menu.classList.remove('flex'); }
    paintTargets();
  }

  function paintCta(crafting) {
    gpoSetText('cta-label', crafting ? 'STOP CRAFT' : 'START AUTO CRAFT');
    const icon = document.getElementById('cta-icon');
    if (icon) icon.textContent = crafting ? 'stop' : 'construction';
    const card = document.getElementById('craft-progress-card');
    if (card) card.classList.toggle('hidden', !crafting);
  }

  async function toggleCraft() {
    try {
      const { data } = await gpoGet('/api/status');
      const crafting = data.crafting && data.crafting.is_crafting;
      if (crafting) {
        await gpoPost('/api/craft', { action: 'stop' });
        paintCta(false);
        pushLog('Craft stopped', (data.crafting.crafted_count || 0) + ' batches • ' + TIERS[tier].title);
        toast('Auto-craft stop requested');
      } else {
        const j = await gpoPost('/api/craft', { action: 'start', tier });
        craftStartMs = Date.now();
        paintCta(true);
        pushLog('Started: ' + TIERS[tier].title, 'target ' + target + 'x • ' + j.message);
        toast('Auto-craft started: ' + TIERS[tier].title);
      }
      await refresh();
    } catch (e) { toast('Craft failed: ' + e.message); }
  }

  async function refresh() {
    try {
      const { data } = await gpoGet('/api/status');
      const c = data.crafting || { is_crafting: false, crafted_count: 0, message: '' };
      paintCta(c.is_crafting);
      gpoSetText('progress-counter', (c.crafted_count || 0) + ' / ' + target);
      const fill = document.getElementById('progress-fill');
      if (fill) fill.style.width = Math.min(100, Math.round(((c.crafted_count || 0) / target) * 100)) + '%';
      if (c.is_crafting) {
        gpoSetText('req-fish', (c.crafted_count || 0) + ' / ' + target + ' crafted');
        gpoSetText('req-peli', c.tier ? String(c.tier).toUpperCase() : TIERS[tier].title.toUpperCase());
        gpoSetText('estimated-time', craftStartMs ? ('Elapsed: ' + fmtElapsed(Date.now() - craftStartMs)) : 'Running…');
        if (target && (c.crafted_count || 0) >= target) {
          await gpoPost('/api/craft', { action: 'stop' }).catch(() => {});
          paintCta(false);
          pushLog('Target reached: ' + TIERS[tier].title, target + ' batches • auto-stopped');
          toast('Craft target reached — auto-stopped');
        }
      } else if (craftStartMs && (c.message || '').toLowerCase().indexOf('finish') >= 0) {
        pushLog('Finished: ' + TIERS[tier].title, c.message);
        craftStartMs = 0;
      }
      gpoPaintPwr(document.getElementById('pwr-btn'), data.is_running && !data.paused);
    } catch (e) { /* offline */ }
  }

  function bind() {
    const trig = document.getElementById('recipe-trigger');
    const menu = document.getElementById('recipe-menu');
    if (trig && menu) trig.addEventListener('click', () => {
      menu.classList.toggle('hidden');
      menu.classList.toggle('flex');
    });
    document.querySelectorAll('#recipe-menu [data-tier]').forEach((b) => {
      b.addEventListener('click', () => selectTier(b.getAttribute('data-tier')));
    });
    const q = (id, d) => {
      const el = document.getElementById(id);
      if (el) el.addEventListener('click', () => setQty(target + d));
    };
    q('qty-minus-10', -10); q('qty-minus', -1); q('qty-plus', 1); q('qty-plus-10', 10);
    const slider = document.getElementById('batch-slider');
    if (slider) slider.addEventListener('input', () => setQty(parseInt(slider.value, 10) || 1));
    const start = gpoRebind(document.getElementById('start-craft-btn'));
    if (start) start.addEventListener('click', toggleCraft);
    const clear = document.getElementById('clear-logs-btn');
    if (clear) clear.addEventListener('click', () => { logEntries = []; renderLog(); });
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

  document.addEventListener('DOMContentLoaded', () => {
    bind();
    selectTier('rare');
    renderLog();
    gpoPaintHostline();
    refresh();
    setInterval(() => { if (!document.hidden) refresh(); }, 2500);
  });
})();
