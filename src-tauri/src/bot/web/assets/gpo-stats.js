'use strict';
/* TELEMETRY STATS wiring: live counters, pity gauge, restock, loot feed. */
(function () {
  function toast(m) { gpoToast(m); }

  function renderLoot(list) {
    const box = document.getElementById('loot-telemetry-list');
    if (!box) return;
    box.innerHTML = '';
    if (!list || !list.length) {
      box.innerHTML = '<div class="text-center font-label-sm text-label-sm text-on-surface-variant uppercase py-4">No catches logged yet — start fishing.</div>';
      return;
    }
    list.forEach((c) => {
      const isFruit = (c.type || '').toLowerCase() === 'fruit';
      const icon = isFruit ? 'psychiatry' : 'set_meal';
      const accent = isFruit ? 'secondary' : 'primary';
      const row = document.createElement('div');
      row.className = 'luminous-card flex items-center justify-between p-space-sm rounded-lg bg-surface-container-low hover:bg-surface-container transition-all cursor-pointer active:scale-[0.98]';
      row.innerHTML =
        '<div class="flex items-center gap-space-sm min-w-0">' +
          '<div class="w-8 h-8 rounded bg-' + accent + '/20 flex items-center justify-center shrink-0 border border-' + accent + '/30">' +
            '<span class="material-symbols-outlined text-[18px] text-' + accent + '">' + icon + '</span></div>' +
          '<div class="flex flex-col truncate">' +
            '<span class="font-label-md text-label-md text-on-surface font-bold uppercase truncate">' + gpoEsc(c.name || (isFruit ? 'Devil Fruit' : 'Fish')) + '</span>' +
            '<span class="font-label-sm text-label-sm text-on-surface-variant uppercase truncate">' + gpoEsc(c.raw || '') + '</span>' +
          '</div></div>' +
        '<div class="flex flex-col items-end shrink-0 pl-space-xs">' +
          '<span class="font-label-sm text-label-sm px-1.5 py-0.5 rounded bg-' + accent + '/20 text-' + accent + ' font-bold uppercase border border-' + accent + '/30">' + (isFruit ? 'FRUIT' : 'FISH') + '</span>' +
          '<span class="font-label-sm text-label-sm text-on-surface-variant mt-0.5">' + gpoAgo(c.timestamp) + '</span>' +
        '</div>';
      box.appendChild(row);
    });
  }

  function paint(d, ms) {
    gpoSetText('fish-counter', d.fish);
    gpoSetText('catch-rate-text', d.success_rate + '% Catch Rate');
    const trend = document.getElementById('trend-pill');
    if (trend) trend.innerHTML = '<span class="material-symbols-outlined text-[12px]">trending_up</span>▲' + d.success_rate + '%';
    gpoSetText('fruit-counter', d.fruits);
    gpoSetText('fruit-pity-text', 'Pity • ' + d.pity_fruit + ' Sparks');
    const pity = Math.max(0, Math.min(100, d.pity_legendary || 0));
    gpoSetText('pity-val', pity);
    const fill = document.getElementById('pity-fill-bar');
    if (fill) fill.style.width = pity + '%';
    gpoSetText('pity-remaining-text', (100 - pity) + ' REMAINING');
    gpoSetText('session-stopwatch', gpoFmtRuntime(d.runtime_s));
    gpoSetText('auto-orders-count', d.bait_purchased);
    const left = Math.max(0, (d.every_n_catches || 0) - (d.since_purchase || 0));
    gpoSetText('last-buy-text', d.auto_purchase ? ('Next buy: ' + left + ' fish') : 'Auto-buy disabled');
    gpoSetText('restock-done', d.since_purchase || 0);
    gpoSetText('restock-total', '/ ' + (d.every_n_catches || 0));
    const rf = document.getElementById('restock-fill');
    if (rf && d.every_n_catches) rf.style.width = Math.min(100, Math.round((d.since_purchase / d.every_n_catches) * 100)) + '%';
    gpoSetText('active-tier-val', (d.bait_tier || '—').toUpperCase());
    gpoSetText('vault-units', d.legendary_reserve != null ? d.legendary_reserve : '—');
    const running = d.is_running && !d.paused;
    gpoSetText('session-chip-text', 'SESSION • ' + (running ? d.state.toUpperCase() : (d.paused ? 'PAUSED' : 'IDLE')));
    gpoSetText('session-state-pill', running ? 'OPTIMAL' : (d.paused ? 'PAUSED' : 'IDLE'));
    gpoSetText('loop-rtt-text', ms + 'ms LOOP');
    renderLoot(d.recent_catches);
    gpoPaintPwr(document.getElementById('pwr-btn'), running);
  }

  async function refresh() {
    try {
      const { data, ms } = await gpoGet('/api/status');
      paint(data, ms);
    } catch (e) { /* offline */ }
  }

  function bind() {
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
    const alerts = document.getElementById('alerts-btn');
    if (alerts) alerts.addEventListener('click', async () => {
      try { const j = await gpoAction('toggle_spawn'); toast(j.message); await refresh(); }
      catch (e) { toast('Failed: ' + e.message); }
    });
    const sync = document.getElementById('reset-stats-btn');
    if (sync) sync.addEventListener('click', async () => {
      const icon = document.getElementById('sync-icon');
      if (icon) icon.style.transform = 'rotate(360deg)';
      setTimeout(() => { if (icon) icon.style.transform = ''; }, 400);
      await refresh();
      toast('Telemetry synced');
    });
    const loop = document.getElementById('loop-stat-chip');
    if (loop) loop.addEventListener('click', refresh);
    const profile = document.getElementById('profile-btn');
    if (profile) profile.addEventListener('click', () => { location.href = '/system?token=' + encodeURIComponent(GPO_TOKEN); });
  }

  // Prime every live-bound value to neutral BEFORE the first poll so no
  // mock number is ever presented, even for a single frame.
  function primePlaceholders() {
    ['fish-counter', 'fruit-counter', 'pity-val', 'session-stopwatch',
     'auto-orders-count', 'restock-done', 'active-tier-val', 'vault-units',
     'last-buy-text', 'catch-rate-text', 'fruit-pity-text',
     'session-chip-text', 'session-state-pill', 'loop-rtt-text',
     'pity-remaining-text'].forEach((id) => gpoSetText(id, '—'));
    gpoSetText('restock-total', '/ —');
    const loot = document.getElementById('loot-telemetry-list');
    if (loot) loot.innerHTML = '';
  }

  document.addEventListener('DOMContentLoaded', () => {
    bind();
    primePlaceholders();
    gpoPaintHostline();
    refresh();
    setInterval(() => { if (!document.hidden) refresh(); }, 2500);
  });
})();
