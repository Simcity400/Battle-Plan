# LANE REPORT — UI + map-mode inventory (docs-only)

**Branch: lane/ui-inventory. Authored 2026-07-09.** Deliverables: two new
coverage-matrix docs + Phase D/E3/wave-plan edits to the production plan. No
code, no build, no git commit (orchestrator commits).

## Deliverables produced
- `replay_interface/documents/UI_COVERAGE_MATRIX.md` — one row per shipped `.gui`
  panel, grouped by category, with status + data dependency + capture/RE need.
- `replay_interface/documents/MAPMODE_COVERAGE_MATRIX.md` — one row per selectable
  map mode + display toggle, with fill-law source + data dependency + status.
- `REPLAY_PRODUCTION_PLAN.md` — Phase D intro rewritten to the "every panel"
  scope; E3 row rewritten to the "all map modes" scope; both matrices pointered
  as the residual ledgers; new **§6a UI + map-mode derivation wave plan**.

## Counts (exhaustive, file-cited)
- **Panel files = 133** (130 base `interface/*.gui` + 3 DLC music). True
  sub-panel surface = **2,009 `containerWindowType`** window definitions.
- Per category: major-views **31** · HUD **24** · popups **27** · country
  scripted-GUI **23** · frontend/meta/dev **25** · DLC music **3**.
- **In-scope for replay = 105** (excludes 25 frontend/meta + 3 music).
  POPULATED today = **3** (topbar D1/D2, outliner D3, diplomacy D4); the other
  **102 need binding** beyond the (solid, file-cited) layout law.
- Event/decision CONTENT is data-driven (128 event files, 88 decision files =
  hundreds of instances) but renders through ~3 chrome panels — counted as
  panels, not instances.
- **Map modes = 16 selectable** (12 toolbar clicksound-identified + 4 engine
  hard-coded: states/diplomacy/players/ideology) + **6 display toggles**.
  Mode status: 1 done (political) · 8 law-known · 7 not-started. Vanilla ships
  0 scripted modes (mod path = generic scripted-layer composer).

## Top data-dependency gaps (fold into REPLAY_DATA_REQUIREMENTS.md)
14 new country-scoped, hourly/on-change streams (all degrade to unavailable
badges); full list in UI_COVERAGE_MATRIX §9 + MAPMODE_COVERAGE_MATRIX §4:
1. Research state (techs/doctrine). 2. National-focus state. 3. Politics detail
(ideas/spirits/popularities/advisors/laws). 4. Production queue + equipment
stockpile. 5. Division/ship/plane templates + equipment variants. 6. Intel/
operatives state (also map mode 4). 7. Trade state. 8. Occupation resistance/
compliance/laws/garrison (also map modes 7/8). 9. Character/leader roster +
traits + aces. 10. Power-balance state. 11. Decision state. 12. Naval/air combat
detail + losses. 13. Fired-event stream (+ chosen option). 14. Scripted-GUI
variable state (generic, mod-extensible — one map, not 23 fields). Plus small:
active command abilities (item 3/9b), radar coverage (map toggle), raid state
(map mode 12). Modes air/supply/infra ride existing items 8/11/10.

## Honesty flags
- The per-strategic-button sub-mode CYCLING is engine-code runtime, NOT fully
  file-enumerable; the matrix uses the file-derivable clicksound identity + the
  6 engine hard-coded layer primitives. Stated as a caveat in the mode matrix.
- "POPULATED" is claimed ONLY for topbar/outliner/diplomacy (per orchestrator);
  the plan's D1–D4 rows still say OPEN — I marked layout LAW-solid and binding
  in-progress, not "done".
- Nothing claimed done that isn't: political map mode is the only `done` mode;
  naval/FoW/factions etc. are `law-known` (law exists, binding/gate remains).

## Recommended FIRST derivation wave (Wave U1 + M1, parallel)
**U1 — war-legibility HUD + popups:** `mapicons` (VP + unit counters, ties to
Phase C1), `unitview`, `divisions_summary`, `landcombat`, `navalcombat`,
`navalcombatresultswindow`, `combatlogview`, `waroverview`, `alerts`,
`worldtensionwindowpopup`. Reuses A3 unit/combat/relation views already
skeletoned; makes a war replay legible with the least new work.
**In parallel, M1 — the 8 law-known map modes** (naval/terrain/resource/factions/
states/diplomacy/players/ideology): one binding+gate lane per shared engine layer
primitive, since their fill law is already file-derivable or ported.
**Then U2** (event/decision chrome — 3 panels, hundreds of instances, best
coverage-per-effort). Critical dependency: all U/M waves need Phase A3 typed
views wired to the A1/A2 fixtures (the plan's current A3 gap).
