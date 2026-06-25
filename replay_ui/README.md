# replay_ui — the offline Battle Plan REPLAY UI

A self-contained, offline, host-side web app that presents the five **replay
screens** (DESIGN §7) over the canonical snapshot store. No game, no network — it
reads the JSON the offline stack already produces (`replay/snapshots/<game_id>/`)
and serves it on `127.0.0.1`.

```
double-click  "Open Replay UI.bat"   (or:  python replay_ui/serve.py)
        ↓
http://127.0.0.1:8078/   →   the 5-screen app
```

## The five screens

1. **Map / time-lapse** — embeds the existing `replay_render` map scrubber
   (owner/control + front lines + division markers, play/scrub) in an iframe. The
   scrubber is **not modified**; this screen just frames it. If a game has no
   rendered scrubber yet, the screen degrades to a one-line "generate it with
   `build_replay.py`" note and the other four screens still work.
2. **Country detail** — pick a TAG; its economy / military / politics / research /
   focus / decisions / diplomacy across the snapshot series. Scalar metrics are
   drawn as charts over time; the deep panels (laws, ideas, stockpile, focus tree,
   research, diplomacy) read the full snapshot at the frame the scrubber is on.
3. **Military / divisions** — division counts by country, template/role mix,
   organisation & strength histograms, and a sortable per-division table; scope to
   one country or all.
4. **Combat log** — active combats at the current snapshot instant (the `combats`
   domain), plus the battle lifecycle reconstructed from `combat_*` timeline events.
5. **Event timeline** — `timeline.json` rendered chronologically, filterable by
   type and country (and optionally clipped to the current frame), every row
   carrying a **source** (M / M* / S / D / marker) and **confidence** (exact /
   interval / inferred) badge.

A single **global scrubber** at the top drives every screen; a game selector
switches between archived replays and the synthetic demo.

## How it reads the schema (`replay/SCHEMA.md`)

- The store layout is consumed exactly as written: `index.json` →
  `snapshots[].file` (filenames are **read from the index**, never reconstructed
  from `hour_index`), each `<hour>.snapshot.json`, and `timeline.json`.
- `countries` / `states` / `provinces` / `diplomacy_world.wars` /
  `…factions` are treated as **maps keyed by string** (TAG / id), `divisions` /
  `combats` / `events` as **lists**. Numeric ids are object-key strings.
- `combat.attackers` / `defenders` are `[div_id, div_type]` tuples (the UI shows
  unit *counts*, not faked TAGs); `combat_id` is opaque (`"5457:62"`);
  `combat.progress` is the raw oracle float (not normalised −1..1).
- Per-field **source tags** come from `FIELD_SOURCES` in `app.js`, the JS mirror
  of `schema.FIELD_SOURCES` — concrete keys are normalised to `*` the same way
  `schema.source_of()` does. The badges you see are those canonical sources.

### No fabrication (the hard rule)

A field that is schema-present but **null** — or an empty object that is supposed
to hold scalars (e.g. `military.casualties == {}`) — renders as **"not captured"**,
never `0`, never blank, never invented. A measured `0` is shown as `0`. Charts
**skip** null members and print a coverage count ("Stability: 1/3"); aggregates
(avg org/strength) report `captured/total`. M\* domains (the `combats` block, a
division's `moving` / `in_combat` / `experience`, memory-only fuel, focus
progress-from-memory) are shown with an inferred/`M*` badge and fall back to "not
captured" when null. When `provenance.memory` is null for a frame, the frame's
scrubber label says `channel: save` so the viewer knows memory-tagged values came
from the save (or are absent), not from a live read.

### Single- vs multi-snapshot

Built to degrade gracefully both ways:
- **One snapshot** (e.g. the bundled `replay` game): charts collapse to a single
  point (larger markers, coverage note "single snapshot"); the play button is
  disabled; the timeline screen says "no adjacent pair to diff — add more saves".
- **Many snapshots** (e.g. the `demo` game): charts plot over time, the scrubber
  plays, ownership/division/event changes animate across frames.

## Pieces

| file | role |
|------|------|
| `app.html` / `app.css` / `app.js` | the front-end (reuses `dashboard/chart.umd.min.js` + the dark theme) |
| `serve.py` | stdlib loopback server + launcher. Rooted at the **repo** so the embedded scrubber's `/map_pipeline/assets/…` paths resolve. Serves `/api/games` and `/api/series/<root>/<game_id>`. Loopback-only, path-traversal-guarded, auto-falls-back to a free port. |
| `series.py` | builds the compact per-game **series** (a few KB–MB) so charts never fetch the 5 MB snapshots. READ-ONLY on the store. |
| `demo.py` | generates a synthetic **3-snapshot** demo store under `replay_ui/demo_snapshots/demo/` (regenerated on launch). Deliberately includes the no-fabrication edge cases (a memory-backed frame vs save-only frames; `casualties` present on some / `{}` on others; null `party_popularities` / `gross_ic_by_equipment`; M\* combats; null division movement) so the "not captured" handling is visible without a real save. Its events are the **real** diff-engine output. |
| `Open Replay UI.bat` | double-click launcher (finds a real Python, starts `serve.py`, opens the browser). |

## Run

```
# easiest:
double-click  replay_ui/Open Replay UI.bat

# or from a shell (repo root or replay_ui/):
python replay_ui/serve.py            # builds the demo, opens http://127.0.0.1:8078
python replay_ui/serve.py --port 9000 --no-open --no-build-demo
```

To add a real multi-snapshot replay: drop more saves through the existing stack
(`python build_replay.py --game-id <id>` per save, same `game_id`), then pick that
game in the selector. To get screen 1's map for a game, run `build_replay.py`
(which writes `replay_render/output/<game_id>_scrubber.html`).

## Boundaries / notes

- Everything here is **read-only** w.r.t. the rest of the repo. It only writes
  under `replay_ui/` (`output/`, `demo_snapshots/`), both gitignored.
- Offline only and bound to `127.0.0.1`. Opening `app.html` directly via
  `file://` will **not** work — the browser blocks `fetch()` of local files; use
  the server (that is what the `.bat` is for).

## Later: wiring into the dashboard

The two front-ends are intentionally separable today (`dashboard/` owns the live
mod-log view; `replay_ui/` owns the post-game replay). They already share the
vendored Chart.js and the dark theme. To merge them later: the dashboard's
`battle_plan.py` server can mount `replay_ui`'s two JSON endpoints
(`/api/games`, `/api/series/...`) and serve `app.html` under a `/replay` route, so
the live "casting" view and the archived-replay browser live behind one launcher —
the data model is identical (snapshot store + derived timeline), only the cadence
differs (DESIGN §7: "a single global timeline scrubber drives every view").
