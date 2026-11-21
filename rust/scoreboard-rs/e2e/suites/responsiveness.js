// RESPONSIVENESS: viewport matrix x route, no console/page/request errors,
// no horizontal document overflow, team-1 SCORE cell within viewport width,
// screenshot per cell, header controls reachable on phones, dark default +
// one light-mode pass at 375x667.
const { open, settle, shot, ck, errCounts, errorsClean, writeJson } = require('../helpers');

const VIEWPORTS = [
  { width: 1920, height: 1080, name: '1920x1080' },
  { width: 1440, height: 900, name: '1440x900' },
  { width: 1280, height: 800, name: '1280x800' },
  { width: 1024, height: 768, name: '1024x768' },
  { width: 768, height: 1024, name: '768x1024' }, // tablet portrait
  { width: 414, height: 896, name: '414x896' },   // phone
  { width: 375, height: 667, name: '375x667' },   // small phone
];

const ROUTES = [
  { path: '/', name: 'scoretable', row: true },
  { path: '/team/1', name: 'team1' },
  { path: '/graphs', name: 'graphs' },
];

// All layout facts in one evaluate so a 30s tick re-render cannot split them.
function measure(page, withRow) {
  return page.evaluate(withRow => {
    const de = document.documentElement;
    const innerWidth = window.innerWidth;
    const table = document.querySelector('table');
    const out = {
      scrollWidth: de.scrollWidth,
      bodyScrollWidth: document.body.scrollWidth,
      innerWidth,
      overflowPx: de.scrollWidth - innerWidth,
      tableWidth: table ? Math.round(table.getBoundingClientRect().width) : null,
      scrollContainers: [],
    };
    let n = table;
    while (n && n !== document.documentElement) {
      const cs = getComputedStyle(n);
      if (/auto|scroll/.test(cs.overflowX) && n.scrollWidth > n.clientWidth + 1) {
        out.scrollContainers.push({
          tag: n.tagName.toLowerCase(),
          cls: String(n.className),
          scrollWidth: n.scrollWidth,
          clientWidth: n.clientWidth,
        });
      }
      n = n.parentElement;
    }
    if (withRow) {
      const a = document.querySelector('tbody tr a[href="/team/1"]');
      const tr = a && a.closest('tr');
      if (tr) {
        const rr = tr.getBoundingClientRect();
        const score = tr.querySelector('td.num.score');
        const sr = score && score.getBoundingClientRect();
        out.team1Row = {
          found: true,
          row: { left: Math.round(rr.left), right: Math.round(rr.right), top: Math.round(rr.top), bottom: Math.round(rr.bottom), height: Math.round(rr.height) },
          score: sr && { left: Math.round(sr.left), right: Math.round(sr.right), width: Math.round(sr.width) },
          scoreText: score && score.innerText.replace(/\s+/g, ' ').trim(),
          valueCount: score ? score.querySelectorAll('.score-line .v').length : 0,
          inViewportWidth: !!(sr && sr.right <= innerWidth + 1 && sr.left >= -1),
          verticallyVisible: rr.bottom > 0 && rr.top < window.innerHeight,
        };
      } else {
        out.team1Row = { found: false };
      }
    }
    return out;
  }, withRow);
}

// Header is not sticky, so measure control reachability at the top of the page
// before scrolling down to the team-1 row.
function measureControls(page) {
  return page.evaluate(() => ({
    scrollY: window.scrollY,
    controls: [...document.querySelectorAll('.chip, #settings-button')].map(c => {
      const r = c.getBoundingClientRect();
      return {
        label: (c.textContent || '').trim() || c.id,
        left: Math.round(r.left),
        right: Math.round(r.right),
        top: Math.round(r.top),
        bottom: Math.round(r.bottom),
        width: Math.round(r.width),
        inViewport: r.left >= 0 && r.right <= window.innerWidth && r.top >= 0 && r.bottom <= window.innerHeight && r.width > 0,
      };
    }),
  }));
}

async function scrollTeam1IntoView(page) {
  await page.evaluate(() => {
    const a = document.querySelector('tbody tr a[href="/team/1"]');
    if (a) a.closest('tr').scrollIntoView({ block: 'center', inline: 'nearest', behavior: 'instant' });
  });
  await page.waitForTimeout(150);
}

async function runCell(browser, checks, vp, route, opts = {}) {
  const { ctx, page, errors } = await open(browser, vp, opts.colorScheme || 'dark');
  await settle(page, route.path);
  if (opts.light) {
    await page.click('button.chip:has-text("dark")');
    await page.waitForTimeout(800);
  }
  const lightBg = opts.light
    ? await page.evaluate(() => getComputedStyle(document.body).backgroundColor)
    : null;
  const controlsAtTop = vp.width <= 414 && route.row ? await measureControls(page) : null;
  if (route.row) await scrollTeam1IntoView(page);
  const m = await measure(page, !!route.row);
  if (controlsAtTop) {
    m.controls = controlsAtTop.controls;
    m.controlsScrollY = controlsAtTop.scrollY;
  }
  const name = `vp-${vp.name}-${route.name}${opts.light ? '-light' : ''}`;
  const shotPath = await shot(page, name);
  const tag = `R-${vp.name}-${route.name}${opts.light ? '-light' : ''}`;

  if (opts.light) {
    ck(checks, `${tag}-light-mode-active`, lightBg === 'rgb(255, 255, 255)', { bodyBg: lightBg });
  }

  ck(checks, `${tag}-no-errors`, errorsClean(errors), {
    ...errCounts(errors),
    console: errors.console.slice(0, 5),
    pageerror: errors.page.slice(0, 5),
    requestfailed: errors.failed.slice(0, 5),
    http4xx5xx: errors.bad.slice(0, 5),
  });
  ck(checks, `${tag}-no-horizontal-overflow`, m.scrollWidth <= m.innerWidth + 1, {
    scrollWidth: m.scrollWidth,
    innerWidth: m.innerWidth,
    overflowPx: m.overflowPx,
  });

  if (route.row) {
    ck(checks, `${tag}-team1-row-found`, !!(m.team1Row && m.team1Row.found), m.team1Row);
    ck(checks, `${tag}-team1-score-in-viewport-width`, !!(m.team1Row && m.team1Row.found && m.team1Row.inViewportWidth && m.team1Row.valueCount === 3), m.team1Row);
    ck(checks, `${tag}-team1-row-visible`, !!(m.team1Row && m.team1Row.found && m.team1Row.verticallyVisible && m.team1Row.row.height > 0), { row: m.team1Row && m.team1Row.row, scoreText: m.team1Row && m.team1Row.scoreText });
    m.scrollMode = m.scrollContainers.length
      ? 'scrollable-container'
      : (m.scrollWidth > m.innerWidth + 1 ? 'page-overflow' : 'none');
    ck(checks, `${tag}-horizontal-behavior-recorded`, ['scrollable-container', 'page-overflow', 'none'].includes(m.scrollMode), {
      mode: m.scrollMode,
      containers: m.scrollContainers,
      tableWidth: m.tableWidth,
      innerWidth: m.innerWidth,
    });
    if (vp.width <= 414) {
      const unreachable = m.controls.filter(c => !c.inViewport);
      ck(checks, `${tag}-header-controls-reachable`, m.controls.length >= 4 && unreachable.length === 0, {
        controls: m.controls,
        unreachable,
      });
    }
  }

  await ctx.close();
  return { viewport: vp.name, route: route.path, light: !!opts.light, screenshot: shotPath, ...m };
}

async function run(browser, checks) {
  const matrix = [];
  for (const vp of VIEWPORTS) {
    for (const route of ROUTES) {
      matrix.push(await runCell(browser, checks, vp, route));
    }
  }

  // Dashboard: once, at 1440 only.
  {
    const vp = { width: 1440, height: 900, name: '1440x900' };
    const { ctx, page, errors } = await open(browser, vp);
    await settle(page, '/dashboard');
    const m = await measure(page, false);
    const p = await shot(page, 'vp-1440x900-dashboard');
    ck(checks, 'R-1440x900-dashboard-no-errors', errorsClean(errors), errCounts(errors));
    ck(checks, 'R-1440x900-dashboard-no-horizontal-overflow', m.scrollWidth <= m.innerWidth + 1, { scrollWidth: m.scrollWidth, innerWidth: m.innerWidth });
    matrix.push({ viewport: '1440x900', route: '/dashboard', screenshot: p, ...m });
    await ctx.close();
  }

  // Light mode: repeat the small-phone scoretable pass.
  matrix.push(await runCell(browser, checks, { width: 375, height: 667, name: '375x667' }, ROUTES[0], { light: true }));

  writeJson('responsiveness-matrix', { generatedAt: new Date().toISOString(), cells: matrix });
}

module.exports = { name: 'responsiveness', run };
