// Entry point: runs every suite, prints PASS/FAIL per check, writes per-suite
// JSON detail to e2e/out/, and exits non-zero if anything failed.
const { launch, writeJson } = require('./helpers');
const suites = [
  require('./suites/responsiveness'),
  require('./suites/values'),
  require('./suites/icons'),
];

(async () => {
  const browser = await launch();
  const all = [];
  for (const suite of suites) {
    console.log(`\n== ${suite.name} ==`);
    const checks = [];
    try {
      await suite.run(browser, checks);
    } catch (e) {
      checks.push({ name: `${suite.name}-suite-crashed`, ok: false, detail: String((e && e.stack) || e) });
    }
    for (const c of checks) console.log(`${c.ok ? 'PASS' : 'FAIL'}  ${c.name}`);
    writeJson(suite.name, { suite: suite.name, pass: checks.filter(c => c.ok).length, total: checks.length, checks });
    all.push(...checks);
  }
  await browser.close();

  const pass = all.filter(c => c.ok).length;
  writeJson('summary', { generatedAt: new Date().toISOString(), pass, total: all.length, checks: all });

  const fails = all.filter(c => !c.ok);
  if (fails.length) {
    console.log('\nFAILURES:');
    for (const f of fails) console.log(`- ${f.name}: ${JSON.stringify(f.detail).slice(0, 600)}`);
  }
  console.log(`\nTOTAL: ${pass}/${all.length} checks passed`);
  process.exit(fails.length ? 1 : 0);
})();
