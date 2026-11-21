# scoreboard-rs e2e checks

Plain Node scripts (no package.json, no npm install). Playwright comes from the
caller's `NODE_PATH`.

Run from this directory:

```sh
cd rust/scoreboard-rs/e2e
NODE_PATH=/Users/mmpeal/.nvm/versions/node/v22.15.1/lib/node_modules/@executeautomation/playwright-mcp-server/node_modules node run.js
```

Optional: `BASE_URL=http://localhost:8080` (default) to point at another instance.

## Layout

- `run.js` — runs all suites, prints `PASS`/`FAIL` per check and the final
  `TOTAL: <pass>/<n> checks passed`; exits 0 only if everything passed.
- `helpers.js` — browser launch, console/pageerror/requestfailed capture,
  page settle, screenshots, API fetches, assertion helper.
- `suites/responsiveness.js` — 7 viewports × (`/`, `/team/1`, `/graphs`) plus
  `/dashboard` at 1440 and a light-mode `/` pass at 375×667: zero console/page/
  request errors, no horizontal document overflow, team-1 SCORE cell inside the
  viewport width, header controls reachable on phones, screenshot per cell.
- `suites/values.js` — displayed values/deltas against the live API in the same
  run; 6 painted Chart.js graphs; ~35 s API request cadence.
- `suites/icons.js` — FontAwesome score-line icons + sr-only labels, first-blood
  drop/count, history status badges and status pills checked against the round
  file the UI is displaying, dark and light hues.

Output lands in `out/`: one JSON per suite (`responsiveness.json`, `values.json`,
`icons.json`), `summary.json`, the raw viewport matrix
(`responsiveness-matrix.json`), icon detail (`icons-detail.json`) and one
screenshot per viewport × route (`vp-<w>x<h>-<route>[-light].png`).

## Ground truth notes

- `api/scoreboard_current.json` contains only tick metadata
  (`scoreboard_tick`, `current_tick`, state) — no team values. Team values come
  from `api/scoreboard_round_<tick>.json`, and every value check fetches the
  round file named by the tick the UI itself rendered.
- Deltas come from the same round file: `do`/`dd`/`ds`; Total delta is
  `do + dd + ds`. A rendered `+0.0` is only correct when the API value is 0.
- History badges (`history-1..3`) map to `services[i].dc[k-1]`; their title is
  `Tick <T-k>: <tooltip>`. The current status pill maps to `services[i].c`.
  `REVOKED`/`PENDING` legitimately render `-` on the gray default tone.
- API cadence is measured after a 3 s settle (the initial load burst is not a
  steady-state storm). Steady state must be ≤ 2 requests per endpoint per 10 s.
  At a tick boundary the app deliberately polls `scoreboard_current.json` once
  per second while `scoreboard_tick` is behind `current_tick` (store.rs
  `update_current_state_loop`); a round-file fetch marks those transition
  windows, and the burst is checked separately (≤ 5 per endpoint per 10 s,
  ≤ 20 requests total) rather than being counted as steady cadence.
