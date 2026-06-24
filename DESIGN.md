# Battle Plan — design document

High-level design for the Battle Plan suite. Everything here is **bounded by
what the game can actually export** — the "not possible" section is as important as
the feature list. Decisions below were settled in a design interview.

---

## 1. Purpose — three lenses
The tool is designed through three lenses, in priority order:
1. **Live** — watch the game unfold on a second screen as it happens.
2. **Post-game analysis** — review finished games in depth, across a whole library.
3. **Competitive integrity** — *not* automated refereeing. Just **all the data on
   hand**, comparable and browsable, so the group can eyeball it and make its own
   calls. No rule engine, no flagging.

---

## 2. Architecture
```
In-game mod (loads in the game)
  ├─ Improved line graphs   ← the ONLY thing visible in-game (see §6)
  └─ Background logger       ← invisible; writes HOI4MPSTAT lines to game.log
                                   │
game.log  ─────────────────────────┘
                                   │
Dashboard (offline, localhost)  ◄──┘  reads the live game.log
  ├─ Auto-archives every game into a local library (games/<date>/)
  └─ Serves the web app at 127.0.0.1, fully offline
```
- **Data source: the `log=` channel** (live, weekly). Kept after consideration;
  save-file parsing was weighed and set aside (it would be near-live-only).
- **Auto-archive every game.** `game.log` is overwritten each launch, so the tool
  copies/parses each game into a dated record on import. This is what makes
  post-game and cross-game review possible. No manual save step.
- **Fully offline**, localhost only, Python stdlib + vendored Chart.js.

---

## 3. Data model — what gets captured
**Cadence**
- **Economic metrics:** sampled **weekly** (≈470 samples/game). A ~90-country
  loop — trivially cheap, deterministic (no OOS).
- **State control:** **event-driven**, free (`on_state_control_changed` fires on
  its own).
- **Province control:** **polled weekly, scoped to non-WST states** (skips dead
  wasteland). This is the one piece with a real live cost — validate in a test
  game and keep a dial-back. (No province-control event hook exists.)
- **State owner:** weekly delta (cheap state scan).

**Scope:** **all countries** logged every week (cheap), with **humans flagged**
(`is_ai = no` at start). Dashboard defaults to humans + majors, can drill into any
AI minor. Countries identified by **tag / country name** (Steam names aren't
exportable).

**Per-country, logged deeply (everything feasible):**
- Industry: civ / mil / dockyard factories, construction power
- Output: gross IC output, broken down by equipment type (efficiency-, loss- and
  lend-lease-immune via `total_equipment_produced_*`)
- **Army composition:** equipment held by type (stockpile + in field)
- Casualties → K/D
- Manpower: deployed army / navy / air split
- Fuel: current / max / ratio
- Resources: produced / consumed / imported / exported per resource
- Political: stability, political power (+growth), command power

**Territory:** state owner (state-level — the game has no province ownership) and
province-level control. From this the **dashboard computes** controlled victory
points and controlled-industry share, using each state's static VP value parsed
from `history/states`.

---

## 4. Hard feasibility boundaries (explicitly NOT possible)
Listed so they never creep into scope as promises:
- **Individual divisions / troop positions on the map** — no export channel at any
  fidelity. The map shows *territory*, never unit counters.
- **Victory points as a country read** — not exposed; **computed dashboard-side**
  instead (control map × static state VP).
- **Exact division count** — not readable; proxy with deployed manpower.
- **Naval tonnage** — not readable; proxy with ship-equipment counts by type.
- **Tech / research progress** — not readable.
- **Air superiority** — per-region, not exposed.
- **Division templates** — can't read a player's template composition (so no
  template-based rule checks).
- **Per-partner trade** — only totals are readable, not *who* you traded with.
- **Province ownership** — doesn't exist in the game; ownership is state-level only.
- **Steam player names** — no script access.

---

## 5. Dashboard — the web app
A polished, app-like SPA (dark theme, nav, transitions). A single **global timeline
scrubber drives every view**, and a **game-library browser** switches between the
live game and any archived game.

**Three core screens:**
1. **Overview / leaderboard** — at-a-glance "who's winning": sortable table of all
   players across every metric (IC output, factories, casualties/K/D, controlled
   VP, …), live-updating.
2. **Country deep-dive** — click any playable country → all its data, full history
   and timeline (scrub its stats over the game), production mix and **army
   composition** over time, live rank vs rivals.
3. **Map time-lapse** — owner view (state-level) + control view (province-level,
   non-WST); play/scrub the whole game; click a state/province for its flip
   history; computed VP / industry-share overlay.

**Live behavior:** the page re-reads the in-progress game on a short interval
(~15s) so it doubles as a live view.

---

## 6. In-game mod (unchanged scope)
In-game, the player sees **only the improved graphs** — same functionality as the
original graph mod, plus our upgrades (auto-bind curves to human players, gross IC
output metric, auto-scaling, survives a major being annexed). Nothing else is
visible in-game. The logger runs silently alongside it.

---

## 7. Deliberately cut (for now)
Considered and dropped in the interview — parked here, not deleted, in case they
come back:
- **Event-marker timeline** (DOWs, capitulations, faction joins, nukes) — "not
  useful." (Note: territory/control tracking stays — that's map data, not markers.)
- **Automated integrity detection / rule flagging** — replaced by plain data
  transparency (§1).
- **Head-to-head compare view**, **awards / recap screen** — not selected as core
  screens; easy to add later on top of the data model.
- **Player names** — not wanted, not exportable anyway.

---

## 8. Tech & constraints
- Offline, localhost; Python stdlib server + single-page app; Chart.js vendored.
- Map rendering: vanilla `provinces.bmp` (5632×2048, 13,381 provinces) preprocessed
  into a province-id raster + province→state map; canvas recolor per frame.
- Province polling is the only in-game cost — bounded by the non-WST filter; treat
  as the thing to validate first in a real game.

---

## 9. Recommended build order
1. **Polished app shell + Overview + Country deep-dive** — runs on the data we
   already log; immediate, visible payoff.
2. **Game archive / library** — auto-import each game; browse past games.
3. **Map pipeline** — preprocess the bitmap; add state owner/control logging; build
   the canvas renderer + scrubber (owner first, state-level).
4. **Province-level control + computed VP** — the high-fidelity front and VP/industry
   overlays. Validate the polling cost here.

---

## 10. Open minor defaults (decide at build time)
- Map coloring: country map-colors, with a faction-grouping toggle.
- Live refresh interval: 15s.
- Archive pruning: keep everything; manual prune if the library grows.
