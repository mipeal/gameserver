// Shared launch / console-capture / screenshot / API helpers.
// Plain CommonJS; `playwright` is supplied by the caller's NODE_PATH.
const { chromium } = require('playwright');
const fs = require('fs');
const path = require('path');

const BASE = process.env.BASE_URL || 'http://localhost:8080';
const OUT = path.join(__dirname, 'out');
fs.mkdirSync(OUT, { recursive: true });

function launch() {
  return chromium.launch();
}

// Collect everything a route must not emit.
function track(page) {
  const e = { console: [], page: [], failed: [], bad: [] };
  page.on('console', m => { if (m.type() === 'error') e.console.push(m.text()); });
  page.on('pageerror', x => e.page.push(String(x)));
  page.on('requestfailed', r => e.failed.push(r.url() + ' :: ' + ((r.failure() || {}).errorText || '')));
  page.on('response', r => { if (r.status() >= 400) e.bad.push(r.status() + ' ' + r.url()); });
  return e;
}

async function open(browser, vp, colorScheme = 'dark') {
  const ctx = await browser.newContext({
    viewport: { width: vp.width, height: vp.height },
    colorScheme,
  });
  const page = await ctx.newPage();
  return { ctx, page, errors: track(page) };
}

async function settle(page, route, ms = 2200) {
  await page.goto(BASE + route, { waitUntil: 'networkidle', timeout: 30000 });
  await page.waitForTimeout(ms);
}

async function shot(page, name) {
  const p = path.join(OUT, name + '.png');
  await page.screenshot({ path: p, fullPage: true });
  return p;
}

// Ground truth for every value assertion in this suite.
async function api(route) {
  const r = await fetch(BASE + route, { cache: 'no-store' });
  if (!r.ok) throw new Error(`GET ${route} -> ${r.status}`);
  return r.json();
}

function ck(checks, name, ok, detail) {
  checks.push({ name, ok: !!ok, detail });
}

function near(a, b, tol = 0.1) {
  return Number.isFinite(a) && Number.isFinite(b) && Math.abs(a - b) <= tol;
}

const errCounts = e => ({
  console: e.console.length,
  pageerror: e.page.length,
  requestfailed: e.failed.length,
  http4xx5xx: e.bad.length,
});

const errorsClean = e => errCounts(e).console + errCounts(e).pageerror + errCounts(e).requestfailed + errCounts(e).http4xx5xx === 0;

function writeJson(name, data) {
  fs.writeFileSync(path.join(OUT, name + '.json'), JSON.stringify(data, null, 2));
}

module.exports = { BASE, OUT, launch, track, open, settle, shot, api, ck, near, errCounts, errorsClean, writeJson };
