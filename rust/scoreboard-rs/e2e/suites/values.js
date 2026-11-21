// VALUES / DATA HONESTY: displayed numbers vs the live API, in the same run.
// Note: api/scoreboard_current.json carries tick metadata only (no values);
// team values live in api/scoreboard_round_<tick>.json. We always fetch the
// round file named by the tick the UI itself displays.
const { open, settle, api, ck, near } = require('../helpers');

const TOL = 0.1;
const fmt = v => (typeof v === 'number' ? +v.toFixed(2) : v);

async function scoretable(browser, checks) {
  const { ctx, page } = await open(browser, { width: 1440, height: 900 });
  await settle(page, '/', 2500);
  const snap = await page.evaluate(() => {
    const tickText = (document.querySelector('thead th') || {}).innerText || '';
    const tm = /Tick\s+(-?\d+)/.exec(tickText);
    return {
      displayedTick: tm ? +tm[1] : null,
      tickText: tickText.replace(/\s+/g, ' ').trim(),
      rows: [...document.querySelectorAll('tbody tr')].map(tr => {
        const a = tr.querySelector('.teamcell a');
        return {
          id: a ? +a.getAttribute('href').split('/').pop() : null,
          name: a ? a.textContent.trim() : null,
          vals: [...tr.querySelectorAll('td.num.score .score-line .v')]
            .map(v => parseFloat(v.textContent.replace(/[^\d.\-]/g, ''))),
        };
      }),
    };
  });
  await ctx.close();

  ck(checks, 'V-01-scoretable-tick-header', snap.displayedTick != null, { tickText: snap.tickText, displayedTick: snap.displayedTick });
  if (snap.displayedTick == null) return;

  const round = await api(`/api/scoreboard_round_${snap.displayedTick}.json`);
  const cur = await api('/api/scoreboard_current.json');
  const byId = Object.fromEntries(round.scoreboard.map(t => [t.team_id, t]));

  ck(checks, 'V-02-scoretable-tick-matches-round-file', round.tick === snap.displayedTick, { displayedTick: snap.displayedTick, roundTick: round.tick });
  ck(checks, 'V-03-scoretable-tick-fresh', snap.displayedTick >= cur.scoreboard_tick - 1, { displayedTick: snap.displayedTick, scoreboardTick: cur.scoreboard_tick });
  ck(checks, 'V-04-scoretable-all-rows-have-3-values', snap.rows.length > 0 && snap.rows.every(r => r.vals.length === 3), { rows: snap.rows.length, valueCounts: snap.rows.map(r => r.vals.length) });

  const mismatches = [];
  for (const row of snap.rows) {
    const t = byId[row.id];
    if (!t) { mismatches.push({ id: row.id, name: row.name, why: 'team missing from round file' }); continue; }
    [['attack', t.o], ['defense', t.d], ['sla', t.s]].forEach(([field, exp], i) => {
      if (!near(row.vals[i], exp, TOL)) mismatches.push({ id: row.id, name: row.name, field, ui: row.vals[i], api: fmt(exp) });
    });
  }
  ck(checks, 'V-05-scoretable-values-match-api', mismatches.length === 0 && snap.rows.length === round.scoreboard.length, {
    rows: snap.rows.length,
    apiTeams: round.scoreboard.length,
    tolerance: TOL,
    displayedTick: snap.displayedTick,
    mismatches: mismatches.slice(0, 20),
  });
}

async function team3(browser, checks) {
  const { ctx, page } = await open(browser, { width: 1440, height: 900 });
  await settle(page, '/team/3', 2500);
  const snap = await page.evaluate(() => {
    const tiles = {};
    document.querySelectorAll('.tile').forEach(t => {
      const k = t.querySelector('.k');
      const v = t.querySelector('.v');
      if (k && v) tiles[k.textContent.trim().toLowerCase()] = parseFloat(v.textContent);
    });
    const rows = [...document.querySelectorAll('table.table-stickyhead tbody tr')].map(tr => {
      const label = (tr.querySelector('.teamcell h4') || {}).textContent || '';
      const m = /Tick\s+(-?\d+)/.exec(label);
      const lines = [...tr.querySelectorAll('td.num.score .score-line')].map(l => {
        const v = l.querySelector('.v, b');
        const d = l.querySelector('.delta small');
        return {
          key: (l.className.match(/score-line\s+(\w+)/) || [])[1] || null,
          value: v ? parseFloat(v.textContent.replace(/[^\d.\-]/g, '')) : NaN,
          delta: d ? parseFloat(d.textContent.replace(/[()]/g, '')) : NaN,
          deltaText: d ? d.textContent.trim() : null,
        };
      });
      return { tick: m ? +m[1] : null, lines };
    });
    return { tiles, rows };
  });
  await ctx.close();

  const ticks = [...new Set(snap.rows.map(r => r.tick).filter(t => t != null))].sort((a, b) => b - a);
  const newest = ticks[0] != null ? ticks[0] : null;
  const cur = await api('/api/scoreboard_current.json');
  ck(checks, 'V-06-teampage-fresh', newest != null && newest >= cur.scoreboard_tick - 1, { newestDisplayedTick: newest, scoreboardTick: cur.scoreboard_tick });

  // Ground truth for every displayed per-tick row.
  const rounds = {};
  for (const t of ticks) {
    try { rounds[t] = await api(`/api/scoreboard_round_${t}.json`); }
    catch (e) { rounds[t] = { error: String(e) }; }
  }
  const valBad = [];
  const deltaBad = [];
  const structural = [];
  let nonzeroAttack = 0;
  let nonzeroAttackWrong = 0;

  for (const row of snap.rows) {
    const rr = rounds[row.tick];
    const t = rr && rr.scoreboard && rr.scoreboard.find(x => x.team_id === 3);
    if (!t) { structural.push({ tick: row.tick, why: 'no round file/team' }); continue; }
    const exp = {
      att: { v: t.o, d: t.do },
      def: { v: t.d, d: t.dd },
      sla: { v: t.s, d: t.ds },
      total: { v: t.points, d: t.do + t.dd + t.ds },
    };
    if (row.lines.map(l => l.key).join(',') !== 'att,def,sla,total' || row.lines.length !== 4) {
      structural.push({ tick: row.tick, keys: row.lines.map(l => l.key), n: row.lines.length });
      continue;
    }
    for (const l of row.lines) {
      const e = exp[l.key];
      if (!near(l.value, e.v, TOL)) valBad.push({ tick: row.tick, key: l.key, ui: l.value, api: fmt(e.v) });
      if (!near(l.delta, e.d, TOL)) deltaBad.push({ tick: row.tick, key: l.key, ui: l.delta, deltaText: l.deltaText, api: fmt(e.d) });
    }
    if (Math.abs(t.do) > TOL) {
      nonzeroAttack++;
      const att = row.lines.find(l => l.key === 'att');
      if (!att || !near(att.delta, t.do, TOL)) nonzeroAttackWrong++;
    }
  }

  ck(checks, 'V-07-teampage-row-structure', structural.length === 0 && snap.rows.length >= 3, { rows: snap.rows.length, problems: structural });
  ck(checks, 'V-08-teampage-values-match-api', valBad.length === 0, { tolerance: TOL, rowsCompared: snap.rows.length, mismatches: valBad.slice(0, 20) });
  ck(checks, 'V-09-teampage-deltas-match-api', deltaBad.length === 0, { tolerance: TOL, mismatches: deltaBad.slice(0, 20) });
  ck(checks, 'V-10-teampage-attack-delta-not-serde-zero', nonzeroAttack > 0 && nonzeroAttackWrong === 0, {
    ticksWithNonzeroAttackDeltaInApi: nonzeroAttack,
    displayedAttackDeltaWrong: nonzeroAttackWrong,
    note: 'API do==0 renders +0.0 legitimately; only nonzero API deltas must render nonzero',
  });

  // Fact tiles = newest displayed tick's row values.
  const ft = newest != null && rounds[newest] && rounds[newest].scoreboard
    ? rounds[newest].scoreboard.find(x => x.team_id === 3)
    : null;
  if (ft) {
    const map = { attack: ft.o, defense: ft.d, sla: ft.s, total: ft.points };
    const bad = Object.entries(map).filter(([k, v]) => !near(snap.tiles[k], v, TOL));
    ck(checks, 'V-11-teampage-fact-tiles-match-api', bad.length === 0, {
      tick: newest,
      tiles: snap.tiles,
      api: Object.fromEntries(Object.entries(map).map(([k, v]) => [k, fmt(v)])),
      mismatches: bad,
    });
  } else {
    ck(checks, 'V-11-teampage-fact-tiles-match-api', false, { why: 'no round file for newest displayed tick', newest });
  }
}

async function graphs(browser, checks) {
  const { ctx, page } = await open(browser, { width: 1440, height: 900 });
  await settle(page, '/graphs', 3000);
  const g = await page.evaluate(() => {
    const canvases = [...document.querySelectorAll('canvas')];
    const painted = canvases.map(c => {
      try {
        const d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data;
        let n = 0;
        for (let i = 3; i < d.length; i += 4) if (d[i] > 0) n++;
        return n;
      } catch (e) { return -1; }
    });
    const insts = window.Chart && window.Chart.instances ? Object.values(window.Chart.instances) : [];
    return {
      canvasCount: canvases.length,
      painted,
      sizes: canvases.map(c => { const r = c.getBoundingClientRect(); return { w: Math.round(r.width), h: Math.round(r.height) }; }),
      chartGlobal: typeof window.Chart,
      instances: insts.length,
      configs: insts.map(c => ({
        type: c.config && c.config.type,
        datasets: ((c.config && c.config.data && c.config.data.datasets) || []).length,
        labels: ((c.config && c.config.data && c.config.data.labels) || []).length,
        dsLabels: ((c.config && c.config.data && c.config.data.datasets) || []).map(d => d.label),
        dataLens: ((c.config && c.config.data && c.config.data.datasets) || []).map(d => (d.data || []).length),
        legend: !!(c.options && c.options.plugins && c.options.plugins.legend && c.options.plugins.legend.display),
      })),
      h2: [...document.querySelectorAll('h2')].map(h => h.textContent.trim()),
    };
  });
  await ctx.close();

  ck(checks, 'V-12-graphs-six-canvases', g.canvasCount === 6, { canvases: g.canvasCount, sizes: g.sizes });
  ck(checks, 'V-13-graphs-canvases-painted', g.painted.length === 6 && g.painted.every(n => n > 0), {
    paintedPixels: g.painted,
    note: 'alpha>0 pixel count in each canvas',
  });
  ck(checks, 'V-14-graphs-chart-instances-labelled', g.chartGlobal === 'function' && g.instances === 6 &&
    g.configs.every(c => c.datasets > 0 && c.labels > 0 && c.dsLabels.every(Boolean) && c.dataLens.every(n => n > 0) && c.legend), {
    chartGlobal: g.chartGlobal,
    instances: g.instances,
    configs: g.configs.map(c => ({ type: c.type, datasets: c.datasets, labels: c.labels, dsLabels: c.dsLabels, dataLens: c.dataLens, legend: c.legend })),
  });
  ck(checks, 'V-15-graphs-section-titles', g.h2.length === 6, { h2: g.h2 });
}

async function cadence(browser, checks) {
  const { ctx, page } = await open(browser, { width: 1440, height: 900 });
  await settle(page, '/', 2500);
  await page.waitForTimeout(3000); // skip the initial load burst
  const t0 = Date.now();
  const ev = [];
  page.on('request', r => {
    if (/\/api\//.test(r.url())) ev.push({ t: Date.now(), u: new URL(r.url()).pathname });
  });
  await page.waitForTimeout(35000);
  await ctx.close();

  const reqs = ev.map(e => ({ sec: (e.t - t0) / 1000, u: e.u }));
  const endpoints = [...new Set(reqs.map(r => r.u))];
  // store.rs polls current.json once per second while scoreboard_tick is behind
  // current_tick (waiting for the new round file). That is a bounded transition
  // burst, not steady cadence; a round fetch marks it. Steady state is still
  // held to <= 2 per endpoint per 10s, and the burst itself is bounded.
  const roundTimes = reqs.filter(r => /scoreboard_round_/.test(r.u)).map(r => r.sec);
  const inTransition = s => roundTimes.some(r => s >= r - 6 && s <= r + 1);
  const steady = reqs.filter(r => !inTransition(r.sec));

  const windows = [];
  for (let s = 0; s + 10 <= 35; s += 10) windows.push([s, s + 10]);
  const steadyCounts = {};
  let maxSteady = 0;
  for (const u of endpoints) {
    steadyCounts[u] = windows.map(([a, b]) => steady.filter(r => r.u === u && r.sec >= a && r.sec < b).length);
    maxSteady = Math.max(maxSteady, ...steadyCounts[u]);
  }
  const burstCounts = {};
  let maxBurst = 0;
  for (const r of roundTimes) {
    for (const u of endpoints) {
      const n = reqs.filter(x => x.u === u && x.sec >= r - 6 && x.sec <= r + 1).length;
      burstCounts[u] = Math.max(burstCounts[u] || 0, n);
      maxBurst = Math.max(maxBurst, n);
    }
  }
  const at = reqs.map(r => +r.sec.toFixed(1));

  ck(checks, 'V-16-api-cadence-steady', endpoints.length > 0 && maxSteady <= 2, {
    windowSec: 35,
    settleSkippedSec: 3,
    criterion: '<= 2 requests per endpoint per 10s window outside tick-transition windows',
    totalRequests: reqs.length,
    steadyRequests: steady.length,
    endpoints,
    maxSteadyPerEndpointPer10s: maxSteady,
    steadyCountsPer10sWindow: steadyCounts,
    requestTimesSec: at,
    roundFetchTimesSec: roundTimes.map(r => +r.toFixed(1)),
  });
  ck(checks, 'V-17-api-cadence-transition-bounded', endpoints.length > 0 && maxBurst <= 5 && reqs.length <= 20, {
    criterion: 'transition burst (<= 6s wait for the new round file) stays <= 5 requests per endpoint per 10s and <= 20 total',
    roundFetchTimesSec: roundTimes.map(r => +r.toFixed(1)),
    maxBurstPerEndpointPer10s: maxBurst,
    burstCounts,
    totalRequests: reqs.length,
  });
}

async function run(browser, checks) {
  await scoretable(browser, checks);
  await team3(browser, checks);
  await graphs(browser, checks);
  await cadence(browser, checks);
}

module.exports = { name: 'values', run };
