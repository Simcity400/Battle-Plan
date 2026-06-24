# Battle Plan — local web dashboard

Companion to the **Battle Plan** HOI4 mod. The mod writes one `HOI4MPSTAT;…` line
per country per month into `…\Hearts of Iron IV\logs\game.log`; this tool serves
a live dashboard on **localhost** that parses those lines. Stdlib-only Python 3 —
no `pip install`.

## Run it — just double-click
**`Open Battle Plan Dashboard.bat`** — that's it. It starts the local server and
opens `http://127.0.0.1:8077` in your browser. Keep the little black window open
while you use it; close it to stop. No typing, no setup.

- It auto-finds your HOI4 `game.log`, so you can open the dashboard **before** you
  even launch the game — it shows "waiting…" and fills in live once a month ticks.
- The page **auto-refreshes every 15 s**, so leave it open while a game runs and it
  updates live. Works post-game too.
- The launcher finds your real Python by full path, so the Microsoft Store
  "python not found" alias can't hijack it.
- Runs **100% offline** — no internet needed at any point.

### Command line (optional)
```
python battle_plan.py ["path\to\game.log"] [--port 9000] [--no-open]
```

The dashboard: a line chart (any metric × any countries), a per-country stacked
**production mix** (units/mo by type), and a sortable latest-tick table.

## Try the demo now (no game needed)
```
python battle_plan.py sample_game.log
```
`sample_game.log` is synthetic (GER/USA, 3 ticks) so you can see the UI.

## Offline / single file
```
python battle_plan.py <game.log> --static dashboard.html
```
Writes a self-contained HTML file (data embedded) instead of serving.

## Notes
- **Fully offline** — Chart.js is vendored (`chart.umd.min.js`) and inlined into
  the page, so nothing is fetched from the internet. Keep that file in this folder.
- Bound to `127.0.0.1` only (not exposed to the network).
- **In multiplayer** every client's `game.log` gets identical lines (lockstep), so
  the host (or anyone) can run this on their own log.
- **No `HOI4MPSTAT` lines / empty dashboard?** Launch HOI4 with `-debug` — the `log`
  effect can require it.
- Per-type log values are *cumulative produced* counters; this tool derives the
  monthly output and IC-weighting itself, so retune weights in `TYPES`
  (`battle_plan.py`) without touching the mod.
