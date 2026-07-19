# LANE REPORT — lane/u1a-counters-hud (Wave U1: on-map unit-counter CONTENT)

Plan item: `REPLAY_PRODUCTION_PLAN.md` §6a wave **U1** + row **C1** (unit-counter
CONTENT half) / `UI_COVERAGE_MATRIX.md` §2 "Map icons (VP/counters furniture)"
/ `EXTRACTION_NEEDS.md` **A15** (division counter badge state).
Worktree `C:\Users\Example\hoi4_lanes\u1a-counters-hud`, branch
`lane/u1a-counters-hud`.

## Scope

Bind the CONTENT a placed on-map unit counter displays — name, strength/org bar
fractions, NATO symbol key, owner colour key, badge states — for every land
division / naval task force / air wing the `replay_core` unit views serve at one
hour, across POV countries. **Not in scope** (kept out deliberately):

- Counter **screen-space geometry / stacking / selection / aggregation-by-zoom**
  → CAP6-gated (`map_viewer::unit_placement` owns world-space placement; the
  visual law is capture-gated).
- Asset decode (sprites/atlas/fonts) → already done in
  `map_viewer::unit_counter_assets.rs` (READ-ONLY reference this lane cites).

## Deliverable (own files only)

- `replay_interface/native/game_ui/src/binding_counters.rs` — NEW module.
- `replay_interface/native/game_ui/tests/counter_binding.rs` — NEW gate test.
- `game_ui/src/lib.rs` — one line: `pub mod binding_counters;`.
- Doc rows: `FIDELITY_LEDGER.md` §2.18a (new subsection), `UI_COVERAGE_MATRIX.md`
  §2 counters row, `REPLAY_PRODUCTION_PLAN.md` §6a U1 row, `REPLAY_DATA_REQUIREMENTS.md`
  item 3 append-note, `EXTRACTION_NEEDS.md` A15 append-note.

`replay_core`, `map_viewer`, `extractor/`, `binding.rs` untouched (read-only).

## Gate numbers (all green)

| Gate | Result |
|---|---|
| `cargo test -p game_ui --test counter_binding` | **3 passed / 0 failed** (fixture-driven, 2 fixtures × 12 hours × ≥2 POVs) |
| `cargo test -p game_ui --lib binding_counters` | **6 passed / 0 failed** (pure-fn: naming law, bar-fraction range, badge honesty, resolution metric) |
| `cargo test -p game_ui` (whole crate) | **see summary line** — new tests added, prior suite intact |
| `cargo build -p game_ui` / `cargo build -p replay_app` | **Finished, 0 errors** |
| Counter rows == generator ground truth | **exact** at every probed hour of `fx_division_movement` (GER 101:1, SOV 201:1) + `fx_naval_air_activity` (ENG 501:1/800:1, GER 101:1) |
| NATO-symbol key resolution | **1/1** land counters resolved per hour (GER infantry → `GFX_unit_infantry_icon_medium_white`); naval/air **0/0 keyed** (separate symbol systems, excluded by design) |
| Absence honesty | every A15 gap badge = `ServedBadge::NotSurfaced`; org-bar fraction always `None`; naval name `None`; never a fabricated default |

## What binds (served by the views), per domain

| Field | Land (`DivisionUnitRow`) | Naval (`FleetUnitRow`) | Air (`WingUnitRow`) | Class |
|---|---|---|---|---|
| uid | `div_uid` | `task_force_uid` | `wing_uid` | FILE |
| owner / country-colour key | `owner` tag | `owner` tag | `owner` tag | FILE (tag; tag→RGB later) |
| name | `template_name` | **`None` (gap)** | `name` | FILE / gap |
| NATO symbol key | `GFX_unit_<role>_icon_medium_white` from `template_role` | — (hull hint only) | — (mission hint only) | FILE naming law |
| strength (raw) | `strength` (0..1 ratio) | not served | not served | FILE |
| str bar fraction | `strength` when ∈[0,1] | `None` | `None` | FILE-derived |
| organisation (raw) | `organisation` (points) | not served | not served | FILE |
| **org bar fraction** | **`None` (gap)** | `None` | `None` | UNKNOWN / gap |
| count | `None` (stack agg = CAP6) | `ship_count` | `plane_count` | FILE |
| orders badge | `moving` | not served | not served | FILE (partial: move-vs-not only) |
| combat badge | `in_combat` | not served | not served | FILE |
| low-supply / dig-in / training / attrition / active-ability | **NotSurfaced** | NotSurfaced | NotSurfaced | GAP (A15) |

## Gaps filed (recording-side; degrade to absence, never invented)

1. **A15 badges NotSurfaced** — `DivisionUnitRow` serves only `moving`
   (orders) and `in_combat`; the A15 slots low-supply (`supply_status`),
   dig-in (`dig_in`), training (`experience`), attrition, and active-ability
   are absent from the view. Already listed in `REPLAY_DATA_REQUIREMENTS.md`
   item 3 (2026-07-09 counter-badge extension); this binding **confirms** the
   gap and fail-closes each to `NotSurfaced`.
2. **Org bar fraction — NEW gap.** The on-map `GFX_mapicon_unit_bar_org` fill
   needs `organisation / max_organisation`; `DivisionUnitRow` carries raw
   `organisation` but no max. Org bar fraction is always `None`. Filed as an
   addition to item 3 / A15 (needs a max-organisation datum, or a normalised
   org ratio like the strength ratio).
3. **Naval task-force name — gap.** `FleetUnitRow` exposes no name (the fixture
   carries `"name":"Channel Patrol"` but the view drops it); counter name is
   `None`, identity is the `task_force_uid`. Filed under item 8.
4. **Fleet/air strength+org bars** — not served at the task-force/wing level
   (`FleetUnitRow.ships` carries per-ship org/str but the aggregation law is
   UNKNOWN); left `None`.

## Fidelity labels (module header carries the full rationale)

- **FILE** — owner tag; NATO land key naming law (`GFX_unit_<role>_icon_medium_white`,
  cited from `subuniticons.gfx` via `unit_counter_assets`); `template_name`;
  wing `name`; `ship_count`/`plane_count`; strength ratio; `moving`/`in_combat`.
- **FILE-derived** — str bar fraction (= strength ratio when ∈[0,1]).
- **UNKNOWN / gap** — org bar fraction (no max); naval/air symbol keys (separate
  symbol systems, not the subuniticons division mapping); A15 non-served badges.

## Coexistence with `binding.rs` (orchestrator reconciliation note)

The committed `game_ui::binding` already carries a THINNER U1 counter binding
(`CounterHudBindings` / `division_counter_binding` / `fleet_counter_binding` /
`wing_counter_binding`, landed by a prior/sibling pass). It binds raw bar VALUES,
raw `template_role`, and a default `CounterBadgeSlots`. This lane's
`binding_counters` is the RICHER content pass mandated for u1a: derived NATO
symbol KEY (+ resolution metric), str/org bar FRACTIONS (range-gated),
`ServedBadge` tri-state badge honesty (Measured / AbsentThisHour / NotSurfaced),
POV filtering, and the org-max gap analysis. No compile collision (separate
module; distinct type names; whole crate + 67 lib tests green). If the
orchestrator wants a single seam, `binding_counters` supersedes the
`binding.rs` counter fns; I left `binding.rs` untouched per the lane rule.

## Notes for adjacent lanes

- Screen-space size/anchor/stacking and the str/org bar RENDER remain CAP6 —
  this lane produces the numeric fractions the bar quads will consume.
- The `template_role → map_icon_category` refinement (per `common/units/*`
  `sub_unit` files) would raise land-symbol fidelity beyond the direct-role key;
  not threaded here (out of the U1 content scope). Direct-role keys resolve for
  the served roles.
