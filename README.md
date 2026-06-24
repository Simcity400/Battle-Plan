# Battle Plan

A Hearts of Iron IV multiplayer statistics suite: an in-game mod that records
per-country metrics over time, plus a fully-offline local web dashboard for
deep analysis.

## What's here
```
battle-plan/
  mod/         the HOI4 mod (loads in-game; draws the line graphs, logs the data)
  dashboard/   a local-web dashboard that reads the game log (Python, stdlib only)
```

## The mod (`mod/`)
- **Auto-binds** its graph curves to the human players in the lobby (not hardcoded
  tags), so it works without per-game setup.
- Tracks, per month: **military IC output** (gross — measured from
  `total_equipment_produced_*`, so production efficiency is captured and it's
  immune to losses / lend-lease / capture), **construction power**, **army
  manpower**, and **casualties**.
- Auto-scaling graphs, survives a major being annexed (ticks off an always-alive
  scope), and writes a `HOI4MPSTAT;` line per country per month to `game.log` for
  the dashboard.

Install: point a `.mod` descriptor at `mod/` (an example launcher descriptor lives
in the HOI4 `mod/` folder), enable it in the launcher, and play. Currently tuned
for the AMSB multiplayer mod (it ticks off that mod's always-alive `WST` scope).

## The dashboard (`dashboard/`)
Double-click **`Open Battle Plan Dashboard.bat`** — it serves a live dashboard
at `http://127.0.0.1:8077`, fully offline (Chart.js is vendored). It charts any
metric for any countries, shows each country's per-type production mix, and a
sortable table. See `dashboard/README.md` for details.

## Lineage
The in-game graphing engine began as a fork of **Aigle2's Stats Generator**
(Steam Workshop `2659156021`); internal script identifiers keep its `ASG_` prefix.
The production metric, auto-binding, auto-scaling, AMSB-safety, the log export, and
the entire dashboard are new.
