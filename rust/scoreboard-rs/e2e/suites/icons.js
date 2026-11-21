// ICONS: score-cell line icons + sr-only labels, first-blood drop, history
// status badges and status pills. Status/colour truth comes from the round
// file the UI is displaying (dc[] for history, services[].c for the pill).
const { open, settle, api, ck, writeJson } = require('../helpers');

const ICON_FOR = { att: 'fa-crosshairs', def: 'fa-shield-alt', sla: 'fa-tachometer-alt' };

const STATUS = {
  SUCCESS: { text: 'up', cls: 'label-success', tone: 'success', tip: 'Service online' },
  FLAGMISSING: { text: 'flag', cls: 'label-warning', tone: 'warning', tip: 'No flag found' },
  MUMBLE: { text: 'mumble', cls: 'label-warning', tone: 'warning', tip: 'Mumble' },
  OFFLINE: { text: 'down', cls: 'label-danger', tone: 'danger', tip: 'Service unreachable' },
  TIMEOUT: { text: 'down', cls: 'label-danger', tone: 'danger', tip: 'Service unreachable' },
  RECOVERING: { text: 'rec', cls: 'label-info', tone: 'info', tip: 'Service online, but flags from previous ticks not found' },
  CRASH: { text: '-', cls: 'label-danger', tone: 'danger', tip: 'Checker crashed' },
  REVOKED: { text: '-', cls: 'label-default', tone: 'default', tip: 'Not checked' },
  PENDING: { text: '-', cls: 'label-default', tone: 'default', tip: 'Not checked' },
};

// css/ops.css: --ok/--warn/--err/--acc/--dim for dark and light.
const TONE_BG = {
  dark: {
    success: 'rgb(63, 185, 80)',
    warning: 'rgb(210, 153, 34)',
    danger: 'rgb(248, 81, 73)',
    info: 'rgb(88, 166, 255)',
    default: 'rgb(139, 148, 158)',
  },
  light: {
    success: 'rgb(26, 127, 55)',
    warning: 'rgb(154, 103, 0)',
    danger: 'rgb(207, 34, 46)',
    info: 'rgb(9, 105, 218)',
    default: 'rgb(87, 96, 106)',
  },
};
const REQUIRED_3 = {
  dark: ['rgb(63, 185, 80)', 'rgb(210, 153, 34)', 'rgb(248, 81, 73)'],
  light: ['rgb(26, 127, 55)', 'rgb(154, 103, 0)', 'rgb(207, 34, 46)'],
};
const STATUS_WORDS = ['up', 'flag', 'mumble', 'down', 'rec'];

const stat = s => STATUS[s] || STATUS.PENDING;

function read(page) {
  return page.evaluate(() => {
    const txt = el => (el ? el.textContent.trim() : null);
    const tickText = (document.querySelector('thead th') || {}).innerText || '';
    const tm = /Tick\s+(-?\d+)/.exec(tickText);
    const rows = [...document.querySelectorAll('tbody tr')].map(tr => {
      const a = tr.querySelector('.teamcell a');
      const fb = tr.querySelector('.fb-count');
      const scoreLines = [...tr.querySelectorAll('td.num.score .score-line')].map(l => {
        const sr = l.querySelector('.sr-only');
        const v = l.querySelector('.v');
        const r = sr ? sr.getBoundingClientRect() : null;
        return {
          key: (l.className.match(/score-line\s+(\w+)/) || [])[1] || null,
          iconClass: l.querySelector('i.fas') ? l.querySelector('i.fas').className : null,
          srText: txt(sr),
          srWidth: r ? +r.width.toFixed(2) : null,
          srHeight: r ? +r.height.toFixed(2) : null,
          value: txt(v),
          rest: txt(l) && txt(sr) != null ? txt(l).slice(txt(sr).length) : null,
          hasParen: /\(/.test(l.textContent),
        };
      });
      const cells = [...tr.querySelectorAll('td')]
        .filter(td => td.querySelector('.sla-col'))
        .map(td => ({
          badges: [...td.querySelectorAll('[class*="history-"]')].map(x => {
            const cs = getComputedStyle(x);
            return { cls: x.className, text: x.textContent.trim(), title: x.getAttribute('title'), bg: cs.backgroundColor, radius: cs.borderRadius };
          }),
          pill: td.querySelector('.status.label')
            ? {
                text: td.querySelector('.status.label').textContent.trim(),
                title: td.querySelector('.status.label').getAttribute('title'),
                cls: td.querySelector('.status.label').className,
              }
            : null,
        }));
      return {
        id: a ? +a.getAttribute('href').split('/').pop() : null,
        name: a ? a.textContent.trim() : null,
        scoreLines,
        fb: fb ? { n: +fb.textContent.trim(), aria: fb.getAttribute('aria-label'), svg: !!fb.querySelector('svg.drop') } : null,
        cells,
      };
    });
    const icon = document.querySelector('td.num.score i.fas');
    const ir = icon ? icon.getBoundingClientRect() : null;
    return {
      displayedTick: tm ? +tm[1] : null,
      tickText: tickText.replace(/\s+/g, ' ').trim(),
      bodyBg: getComputedStyle(document.body).backgroundColor,
      rows,
      icons: {
        fontFamily: icon ? getComputedStyle(icon).fontFamily : null,
        width: ir ? +ir.width.toFixed(2) : null,
        height: ir ? +ir.height.toFixed(2) : null,
        woff2: performance.getEntriesByType('resource').map(r => r.name).filter(n => /fa-solid[^/]*\.woff2/.test(n)),
        fa5: document.fonts.check('900 12px "Font Awesome 5 Free"'),
      },
    };
  });
}

async function run(browser, checks) {
  const { ctx, page } = await open(browser, { width: 1440, height: 900 });
  await settle(page, '/', 2500);
  const dark = await read(page);
  const round = await api(`/api/scoreboard_round_${dark.displayedTick}.json`);
  const byId = Object.fromEntries(round.scoreboard.map(t => [t.team_id, t]));
  // Confirmed first bloods per team name = top-level services[].first_blood.
  const fbTruth = {};
  for (const svc of round.services || [])
    for (const fb of svc.first_blood || [])
      if (fb.confirmed) fbTruth[fb.name] = (fbTruth[fb.name] || 0) + 1;

  // --- score-cell lines -------------------------------------------------
  const T = dark.displayedTick;
  const lineProblems = [];
  const labelProblems = [];
  for (const row of dark.rows) {
    if (row.scoreLines.length !== 3) {
      lineProblems.push({ team: row.id, why: `expected 3 score lines, got ${row.scoreLines.length}` });
      continue;
    }
    row.scoreLines.forEach((l, i) => {
      const key = ['att', 'def', 'sla'][i];
      if (l.key !== key) lineProblems.push({ team: row.id, index: i, key: l.key, why: 'unexpected line order/key' });
      if (!l.iconClass || !l.iconClass.split(/\s+/).includes('fas') || !l.iconClass.includes(ICON_FOR[key]))
        lineProblems.push({ team: row.id, key, iconClass: l.iconClass, why: `expected fas ${ICON_FOR[key]}` });
      if (!l.srText || l.srWidth == null || l.srWidth > 1.5 || l.srHeight > 1.5)
        labelProblems.push({ team: row.id, key, srText: l.srText, srWidth: l.srWidth, srHeight: l.srHeight, why: 'missing/visible sr-only label' });
      if (l.hasParen) labelProblems.push({ team: row.id, key, text: l.rest, why: 'parenthesis in score line' });
      if (l.rest == null || !/^-?\d+(\.\d+)?$/.test(l.rest) || l.rest !== l.value)
        labelProblems.push({ team: row.id, key, rest: l.rest, value: l.value, why: 'visible text is not just the numeric value' });
    });
  }
  ck(checks, 'I-01-score-lines-icons', lineProblems.length === 0, {
    teams: dark.rows.length,
    expected: ICON_FOR,
    problems: lineProblems.slice(0, 10),
  });
  ck(checks, 'I-02-score-lines-sr-only-labels', labelProblems.length === 0, {
    problems: labelProblems.slice(0, 10),
  });

  // --- first blood ------------------------------------------------------
  const fbProblems = [];
  let fbShown = 0;
  for (const row of dark.rows) {
    const expect = fbTruth[row.name] || 0;
    const got = row.fb ? row.fb.n : 0;
    if (expect !== got) fbProblems.push({ team: row.id, name: row.name, ui: got, api: expect });
    if (row.fb) {
      fbShown++;
      if (!row.fb.svg) fbProblems.push({ team: row.id, name: row.name, why: 'no svg.drop' });
      if (row.fb.aria !== `${row.fb.n} confirmed first bloods`) fbProblems.push({ team: row.id, name: row.name, aria: row.fb.aria, n: row.fb.n });
      if (!(row.fb.n > 0)) fbProblems.push({ team: row.id, name: row.name, why: 'fb-count present with non-positive count' });
    }
  }
  ck(checks, 'I-03-first-blood-drop-and-count', fbProblems.length === 0 && fbShown > 0, {
    tick: T,
    teamsWithBloods: Object.entries(fbTruth),
    rowsWithDrop: fbShown,
    problems: fbProblems,
  });

  // --- history status badges -------------------------------------------
  const structProblems = [];
  const hueProblems = [];
  const truthProblems = [];
  const notChecked = [];
  let badgeCount = 0;
  for (const row of dark.rows) {
    const team = byId[row.id];
    if (!team) { truthProblems.push({ team: row.id, why: 'team missing from round file' }); continue; }
    if (row.cells.length !== team.services.length) {
      truthProblems.push({ team: row.id, cells: row.cells.length, apiServices: team.services.length });
      continue;
    }
    row.cells.forEach((cell, si) => {
      const dc = team.services[si].dc || [];
      const expectedKs = [1, 2, 3].filter(k => dc[k - 1] != null);
      const gotKs = cell.badges.map(b => {
        const m = /history-(\d)/.exec(b.cls);
        return m ? +m[1] : null;
      });
      badgeCount += cell.badges.length;
      const ord = a => [...a].sort().join(',');
      if (gotKs.some(k => k == null) || ord(gotKs) !== ord(expectedKs))
        truthProblems.push({ team: row.id, service: si, gotKs, expectedKs, dc });
      cell.badges.forEach(b => {
        const k = /history-(\d)/.exec(b.cls) ? +/history-(\d)/.exec(b.cls)[1] : null;
        if (k == null) { structProblems.push({ team: row.id, service: si, cls: b.cls }); return; }
        if (!/^Tick \d+: /.test(b.title || '')) structProblems.push({ team: row.id, service: si, title: b.title, why: 'title must match ^Tick N: ' });
        if (b.radius !== '8px') structProblems.push({ team: row.id, service: si, radius: b.radius });

        const code = dc[k - 1];
        const exp = stat(code);
        if (b.text !== exp.text) truthProblems.push({ team: row.id, service: si, k, field: 'text', ui: b.text, api: exp.text, code });
        if (!b.cls.includes(exp.cls)) truthProblems.push({ team: row.id, service: si, k, field: 'class', ui: b.cls, api: exp.cls, code });
        if (b.title !== `Tick ${T - k}: ${exp.tip}`) truthProblems.push({ team: row.id, service: si, k, field: 'title', ui: b.title, api: `Tick ${T - k}: ${exp.tip}`, code });
        if (b.bg !== TONE_BG.dark[exp.tone]) truthProblems.push({ team: row.id, service: si, k, field: 'bg', ui: b.bg, api: TONE_BG.dark[exp.tone], code });

        // Literal dispatch criterion, scoped to badges that carry a checker
        // result. "Not checked" (REVOKED/PENDING) is a distinct state and is
        // asserted separately in I-05.
        if (!/Not checked/.test(b.title || '')) {
          if (!STATUS_WORDS.includes(b.text)) hueProblems.push({ team: row.id, service: si, k, text: b.text, title: b.title, why: 'not a status word' });
          if (!REQUIRED_3.dark.includes(b.bg)) hueProblems.push({ team: row.id, service: si, k, bg: b.bg, why: 'bg outside success/warning/danger set' });
        } else {
          notChecked.push({ text: b.text, bg: b.bg, title: b.title });
        }
      });
    });
  }
  ck(checks, 'I-04-history-badge-structure', structProblems.length === 0, { badges: badgeCount, problems: structProblems.slice(0, 10) });
  ck(checks, 'I-05-history-status-hues', hueProblems.length === 0, {
    criterion: 'history badges with a checker result use a status word and one of rgb(63,185,80)/rgb(210,153,34)/rgb(248,81,73)',
    problems: hueProblems.slice(0, 10),
  });
  ck(checks, 'I-06-history-badge-api-truth', truthProblems.length === 0, {
    tick: T,
    badges: badgeCount,
    problems: truthProblems.slice(0, 10),
  });
  ck(checks, 'I-07-not-checked-state', notChecked.length === 0 || notChecked.every(x => x.text === '-' && x.bg === TONE_BG.dark.default), {
    note: 'REVOKED/PENDING services legitimately render "-"/gray; recorded, not treated as a status badge',
    count: notChecked.length,
    sample: notChecked.slice(0, 3),
  });

  // --- status pill per service cell ------------------------------------
  const pillProblems = [];
  let pillCells = 0;
  let upPills = 0;
  for (const row of dark.rows) {
    const team = byId[row.id];
    if (!team) continue;
    row.cells.forEach((cell, si) => {
      pillCells++;
      const code = team.services[si].c;
      const exp = stat(code);
      if (!cell.pill) { pillProblems.push({ team: row.id, service: si, why: 'no .status.label pill' }); return; }
      if (cell.pill.text !== exp.text) pillProblems.push({ team: row.id, service: si, field: 'text', ui: cell.pill.text, api: exp.text, code });
      if (!cell.pill.cls.includes('status') || !cell.pill.cls.includes(exp.cls)) pillProblems.push({ team: row.id, service: si, field: 'class', ui: cell.pill.cls, api: exp.cls, code });
      if (cell.pill.title !== exp.tip) pillProblems.push({ team: row.id, service: si, field: 'title', ui: cell.pill.title, api: exp.tip, code });
      if (cell.pill.text === 'up') upPills++;
    });
  }
  ck(checks, 'I-08-status-pill-per-service-cell', pillProblems.length === 0 && pillCells > 0 && upPills > 0, {
    serviceCells: pillCells,
    upPills,
    note: '"up" holds for SUCCESS cells; REVOKED cells are checked against the API and render "-"/Not checked',
    problems: pillProblems.slice(0, 10),
  });

  // --- FontAwesome actually renders ------------------------------------
  ck(checks, 'I-09-fontawesome-rendered', dark.icons.width > 0 && dark.icons.height > 0 &&
    /Font Awesome/.test(dark.icons.fontFamily || '') && dark.icons.woff2.length > 0 && dark.icons.fa5 === true, {
    iconBox: { width: dark.icons.width, height: dark.icons.height },
    fontFamily: dark.icons.fontFamily,
    woff2Loaded: dark.icons.woff2,
    fontsCheckFa5: dark.icons.fa5,
    note: 'document.fonts.check also returns true for unavailable families, so the woff2 resource entry is the reliable signal',
  });

  // --- light mode badge hues -------------------------------------------
  await page.click('button.chip:has-text("dark")');
  await page.waitForTimeout(800);
  const light = await read(page);
  // The tick may have advanced while toggling; re-ground if it did.
  let byIdLight = byId;
  if (light.displayedTick !== T) {
    const r2 = await api(`/api/scoreboard_round_${light.displayedTick}.json`);
    byIdLight = Object.fromEntries(r2.scoreboard.map(t => [t.team_id, t]));
  }
  const lProblems = [];
  let lBadges = 0;
  for (const row of light.rows) {
    const team = byIdLight[row.id];
    if (!team) continue;
    row.cells.forEach((cell, si) => {
      const dc = team.services[si].dc || [];
      cell.badges.forEach(b => {
        lBadges++;
        const m = /history-(\d)/.exec(b.cls);
        const k = m ? +m[1] : null;
        const exp = stat(k != null ? dc[k - 1] : 'PENDING');
        if (b.radius !== '8px') lProblems.push({ team: row.id, service: si, radius: b.radius });
        if (b.bg !== TONE_BG.light[exp.tone]) lProblems.push({ team: row.id, service: si, k, bg: b.bg, api: TONE_BG.light[exp.tone], code: k != null ? dc[k - 1] : null });
        if (!/Not checked/.test(b.title || '')) {
          if (!STATUS_WORDS.includes(b.text)) lProblems.push({ team: row.id, service: si, k, text: b.text, why: 'not a status word' });
          if (!REQUIRED_3.light.includes(b.bg)) lProblems.push({ team: row.id, service: si, k, bg: b.bg, why: 'bg outside light success/warning/danger set' });
        }
      });
    });
  }
  ck(checks, 'I-10-light-mode-badge-hues', light.bodyBg === 'rgb(255, 255, 255)' && lBadges > 0 && lProblems.length === 0, {
    bodyBg: light.bodyBg,
    badges: lBadges,
    allowedLight: REQUIRED_3.light,
    problems: lProblems.slice(0, 10),
  });

  await ctx.close();
  writeJson('icons-detail', {
    generatedAt: new Date().toISOString(),
    displayedTick: T,
    iconFont: dark.icons,
    firstBloodsApi: fbTruth,
    rows: dark.rows.map(r => ({ id: r.id, name: r.name, fb: r.fb, badges: r.cells.flatMap(c => c.badges), pills: r.cells.map(c => c.pill) })),
  });
}

module.exports = { name: 'icons', run };
