// WCAG 2.1 AA contrast audit for the Japandi scoreboard palette.
// Walks every fg/bg pair the stylesheets can produce (incl. alpha blending
// and the html.dark overrides) and checks 4.5:1 normal / 3:1 large text
// (>=18.66px bold or >=24px) and 3:1 UI components.
const hex = (h) => [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16) / 255);
const lin = (c) => (c <= 0.04045 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4));
const lum = (rgb) => 0.2126 * lin(rgb[0]) + 0.7152 * lin(rgb[1]) + 0.0722 * lin(rgb[2]);
const blend = (fg, bg, a) => fg.map((c, i) => c * a + bg[i] * (1 - a));
const ratio = (fg, bg) => {
  const [a, b] = [lum(fg), lum(bg)].sort((x, y) => y - x);
  return (a + 0.05) / (b + 0.05);
};
const rgb = (h) => hex(h);

// tokens (japandi.css) — light: warm washi paper, dark: warm espresso
const T = {
  light: { bg: '#f1ece2', surface: '#e8e1d3', panel: '#faf7ef', ink: '#2e2921', mut: '#665e51',
    up: '#54684a', flag: '#7d6128', mumble: '#96542a', down: '#9c3e2a', info: '#47617b',
    hov: '#2f4a66' },
  dark: { bg: '#171512', surface: '#1d1a16', panel: '#26221c', ink: '#eae5db', mut: '#9a9184',
    up: '#8aa07c', flag: '#c9a86a', mumble: '#c67f53', down: '#d07a5e', info: '#8ca0b3',
    hov: '#b3c2d1' },
};

// [name, fg token, bg token, alpha, size: 'normal'|'large'|'ui']
const CASES = [
  ['body text ink/bg', 'ink', 'bg', 1, 'normal'],
  ['muted small/.small', 'mut', 'bg', 1, 'normal'],
  ['muted on surface (hover rows)', 'mut', 'surface', 1, 'normal'],
  ['muted on panel (popover content)', 'mut', 'panel', 1, 'normal'],
  ['dropdown item ink/panel', 'ink', 'panel', 1, 'normal'],
  ['link info/bg', 'info', 'bg', 1, 'normal'],
  ['link hover', 'hov', 'bg', 1, 'normal'],
  ['header title ink/bg', 'ink', 'bg', 1, 'normal'],
  ['header small mut/bg', 'mut', 'bg', 1, 'normal'],
  ['backlink mut/bg', 'mut', 'bg', 1, 'normal'],
  ['tick time span ink/bg', 'ink', 'bg', 1, 'normal'],
  ['tag up (label-success)', 'up', 'bg', 0.9, 'normal'],
  ['tag flag (label-warning)', 'flag', 'bg', 0.9, 'normal'],
  ['tag mumble', 'mumble', 'bg', 0.9, 'normal'],
  ['tag down (label-danger)', 'down', 'bg', 0.9, 'normal'],
  ['tag info (label-info/frozen)', 'info', 'bg', 0.9, 'normal'],
  ['tag mut (label-default)', 'mut', 'bg', 0.9, 'normal'],
  ['firstblood flag 12px', 'flag', 'bg', 1, 'normal'],
  ['rank h4 mut 16px serif', 'mut', 'bg', 1, 'normal'],
  ['rank1 flag (first row)', 'flag', 'bg', 1, 'normal'],
  ['team-total ink', 'ink', 'bg', 1, 'normal'],
  ['delta small mut', 'mut', 'bg', 1, 'normal'],
  ['chart ticks mut/bg', 'mut', 'bg', 1, 'ui'],
  ['graphs btn mut/bg', 'mut', 'bg', 1, 'ui'],
  ['settings gear mut/bg', 'mut', 'bg', 1, 'ui'],
  // rank badges: fixed gold/silver/bronze with near-black text in both themes
  ['rank badge text on gold', '#171512', '#d4af37', 1, 'normal'],
  ['rank badge text on silver', '#171512', '#c0c0c0', 1, 'normal'],
  ['rank badge text on bronze', '#171512', '#c79b56', 1, 'normal'],
  // overlay veil rgba(10,9,7,.75) composited over the page bg, text pinned
  // to cream / warm grey in both themes
  ['overlay ink on veil', '#eae5db', 'bg', -0.75, 'large'],
  ['overlay service/team on veil', '#b8afa0', 'bg', -0.75, 'large'],
  ['overlay bigicon down (graphic)', '#d07a5e', 'bg', -0.75, 'ui'],
  ['thead ink/bg', 'ink', 'bg', 1, 'normal'],
  ['btn-link headercell mut', 'mut', 'bg', 1, 'normal'],
  ['blocked team down strikethrough', 'down', 'bg', 1, 'normal'],
  ['skip-link ink/panel', 'ink', 'panel', 1, 'normal'],
  ['popover title mumble/panel', 'mumble', 'panel', 1, 'normal'],
];

const need = (size) => (size === 'ui' ? 3 : size === 'large' ? 3 : 4.5);
const out = [];
for (const theme of ['light', 'dark']) {
  for (const [name, fgK, bgK, alpha, size] of CASES) {
    const t = T[theme];
    // negative alpha = an alpha bg (veil) composited over the page bg
    const veilBg = alpha < 0
      ? blend(rgb('#0a0907'), rgb(t[bgK] ?? bgK), -alpha)
      : rgb(t[bgK] ?? bgK);
    const fg = blend(rgb(t[fgK] ?? fgK), veilBg, 1);
    const r = ratio(fg, veilBg);
    out.push({ theme, name, r: +r.toFixed(2), need: need(size), pass: r >= need(size) });
  }
}
const fails = out.filter((o) => !o.pass);
console.log(`checked ${out.length} pairs, ${fails.length} fail\n`);
for (const f of fails) console.log(`FAIL [${f.theme}] ${f.name}: ${f.r} < ${f.need}`);
if (!fails.length) console.log('all pass');
