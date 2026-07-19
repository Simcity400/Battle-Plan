# Lane report — D3 (outliner population) + D4 (country/diplomacy panels)

Branch `lane/d-panels`. Scope: REPLAY_PRODUCTION_PLAN items **D3** and **D4** —
populate the outliner (army/navy/air lists) and the read-only country/diplomacy
panels from the `replay_core` typed timeline views, following the D1/D2 binding
seam (`game_ui/src/binding.rs`). Bindings only; no layout law, `replay_core`,
`extractor/`, or fixture edits.

## What landed

All population code is additive in `game_ui/src/binding.rs`; two new
fixture-driven gate suites in `game_ui/tests/`. No recording bytes are parsed in
`game_ui` — every value flows through a `replay_core` typed view (Constraint 2),
and every unavailable value renders the fail-closed absent form (Constraint 3).

| Item | New symbols (`game_ui::binding`) | Source view (`replay_core`) |
|---|---|---|
| D3 outliner | `outliner_unit_lists_from_frame`, `outliner_binding_from_lists`, `outliner_binding_from_frame`, `OutlinerUnitLists`, `OutlinerUnitRow`, `OUTLINER_{ARMY,NAVY,AIR}_ICON` | `division_units()` / `fleets()` / `wings()` (+ their `*_status()`) |
| D4 diplomacy | `diplomacy_panel_from_frame`, `DiplomacyPanelBinding`, `FactionRow`, `RelationRow` | `diplomacy()` → `DiplomacyView` (`countries`, `pairs`, `status`) |

`OutlinerBinding`/`OutlinerRowBinding` (the existing outliner layout consumer in
`hud.rs`) are reused: `outliner_binding_from_frame` flattens the typed
army→navy→air lists into that struct, so the population wires into the existing,
already-gated outliner compositor rather than a parallel path.

## D3 binding coverage

Rows are owner-filtered to the POV: a unit whose `owner` leaf is not `Measured`
is attributable to no POV and omitted from every list. Per-branch `ViewStatus`
is carried so absent/unavailable domains are distinguishable from present-empty.

| Field | Value | Label |
|---|---|---|
| army row key | `div_uid` (`"101:1"`) from `division_units` | data (view) |
| navy row key | `task_force_uid` (`"501:1"`) from `fleets` | data (view) |
| air row key | `wing_uid` (`"800:1"`) from `wings` | data (view) |
| row icon (army/navy/air) | `GFX_theatre_army_shield` / `_task_force_shield` / `_air_force_shield` | **FILE** (`interface/theatreselector.gfx`; same sprites the `hud.rs` outliner layout gate resolves) |
| row text label | raw unit UID | **UNKNOWN** — the game shows a formatted unit *name*+status; no shipped-file formatter citation in this decoupled context, so the raw stable identity is rendered per Constraint 3 |
| row progress | `None` | fail-closed — no cited law maps strength/org onto the outliner progress bar |

## D4 binding coverage

| Field | Value | Label |
|---|---|---|
| `factions[]` | global membership matrix (name, leader, sorted members) grouped from each country's `politics.faction` | data (view) |
| `relations[]` | one directed `pov → other` row per recorded country; each flag `Some(bool)` only when the pair leaf is `Measured`, else `None` | data (view), fail-closed |
| `wars[]` | POV belligerents = relation rows with `at_war == Some(true)` | data (view) |
| `available` | `true` only when `DiplomacyView.status == Present` | data (view) |

Relation flags come exclusively from measured leaves; an unrecorded relation is
`None`, never a fabricated `false`.

## View gaps found (recorded per the READ-ONLY-on-replay_core constraint)

These are data the fixture/recording **carries** but the read-only `DiplomacyView`
does **not** surface, so the D4 binding cannot bind them. Closing any of them is a
`replay_core` change (a parallel lane owns that crate); this lane binds
fail-closed around them. **No new recording-side dependency** is implied — the
data already exists in the schema, so nothing was added to
REPLAY_DATA_REQUIREMENTS.md / EXTRACTION_NEEDS.md.

1. **`diplomacy_world.wars` war objects** (attackers/defenders/front/start_hour):
   unread. The view exposes only per-pair `at_war` booleans, so a war's
   belligerent grouping is reconstructed from directional pairs, not read as a
   first-class war object. Consequence: `wars` is a per-POV belligerent list, not
   a global war-object matrix, and a war recorded on only one side's `wars` list
   is directional (e.g. `fx_war_faction_matrix` sets `ENG.wars`→ITA but no
   `ITA.wars`, so `ITA → ENG` is absent — reproduced faithfully by the binding).
2. **`diplomacy_live.war_relations` casualties**: unread → not bindable.
3. **`diplomacy.relation_flags` object / `diplomacy_live.faction_members`**:
   unread; the view derives the same facts from the per-country `wars`/faction
   lists, so these are redundant (not a functional gap for D4 today).
4. **Fleet / task-force / air-wing display names**: `FleetUnitRow` carries no
   `name`; outliner navy rows fall back to the UID (contributes to the UNKNOWN
   row-text label above). `WingUnitRow.name` exists but is not needed for the
   row key.

## Gate results

### 2026-07-17 D53 proof-stream regression reconciliation

The positive production-fold gate briefly regressed after `domain53_proof`
began emitting a complete three-record H4CR stream (SESSION_META, D53 DATA,
FRAME_END). The `game_ui` harness still treated that artifact as one record: it
relocated only the first header and appended a second frame end. Consequently
the D53 data remained at the proof CLI's historical smoke hour while the tested
1936 hour correctly folded `faction_roster_status = unavailable` and no country
objects. The production recorder fold was not at fault.

`tests/diplomacy_binding.rs` now walks the framed stream, validates every record
boundary, and relocates every header to the same canonical 1936 hour without
changing payload bytes. The isolated real encoder -> recorder fold -> typed view
-> UI binding gate again proves the measured roster `1 -> [1, 3, 22]`, the
leader decoration, and the absence of a fabricated faction-name leaf.

| Gate | Result |
|---|---|
| 1. Outliner: army/navy/air rows == fixture ground truth (counts + row key strings), ≥2 POVs | **PASS** — `tests/outliner_binding.rs` 3/3. Army across GER/SOV (`fx_division_movement`); navy+air on ENG vs empty on GER (`fx_naval_air_activity`), all 12 hours |
| 2. Diplomacy: war rows / faction memberships before/after a matrix change; change hour flips exactly the expected rows | **PASS** — `tests/diplomacy_binding.rs` 4/4. Faction flip at tick 3, war flip at tick 7, per-POV (GER gains ENG; ENG gains GER+ITA); fail-closed absent-POV panel |
| 3. Pre-existing `game_ui` gates held | **HELD** by this lane. One unrelated red is **pre-existing** (see below) |
| 4. Builds/tests green | see counts below |

### Gate 4 counts

- `cargo build -p game_ui` — **green**.
- `cargo build -p replay_app` — **green** (6 pre-existing warnings, no errors).
- `cargo test -p game_ui` — **64 passed / 1 failed** (lib 52 + integration). The
  single failure is **pre-existing and unrelated** (verified: fails identically
  with this lane's changes stashed, at base commit 808db1953). New D3/D4 gates:
  **7/7 pass** (outliner 3 + diplomacy 4). Harness=false `acceptance` test exits 0.
- `cargo test -p replay_core` — **30 passed / 0 failed** (untouched).

### Pre-existing unrelated failure (NOT this lane, NOT fixed here)

`tests/topbar_binding.rs::topbar_scalar_sweep_renders_generator_values` asserts
`binding.supply == None`, but `replay_core::build_topbar_scalars` now reads
`countries.*.scalars.supply_value`, which `fx_topbar_scalars_sweep` sets to
`12.0 + tick`, so the binding correctly serves `Some("12")`. This is a **D1
(topbar) gate** with a stale expectation left behind by a `replay_core` view
enhancement; touching it is out of this lane's D3/D4 scope and would require
editing another lane's gate. Suggested reconciliation for the D1/orchestrator
owner: update the `supply` expectation in `topbar_binding.rs` to the generator
value now served (`scalars.supply_value = 12 + tick`). (The `world_tension`
assertions on the preceding lines still pass — only `supply_value` became
readable, so the fix is scoped to the `supply` field.)
