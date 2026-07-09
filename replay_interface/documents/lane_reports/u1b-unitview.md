# LANE U1 — Selected-unit view panel (unitview.gui) layout + binding

**Branch:** `lane/u1b-unitview` · **Date:** 2026-07-09
**Scope:** REPLAY_PRODUCTION_PLAN §6a / UI_COVERAGE_MATRIX §2 "Selected-unit
panel" — LAY the layout for `interface/unitview.gui`'s selected-unit detail
panels AND BIND `replay_core`'s read-only unit views onto their fields.

> Note: this file previously held a stale, git-conflict-marked report from an
> unrelated lane (`lane/f4-anomaly`); replaced with this lane's report per the
> mission deliverable path.

## Deliverables

| Artifact | Path |
|---|---|
| New module (layout gate + binding) | `replay_interface/native/game_ui/src/unitview.rs` |
| lib.rs export | `replay_interface/native/game_ui/src/lib.rs` (`pub mod unitview;`) |
| New test file | `replay_interface/native/game_ui/tests/unitview_binding.rs` |
| Ledger rows | `FIDELITY_LEDGER.md` §1 (UI chrome counts) + §2.16 (4 rows) |
| Matrix row | `UI_COVERAGE_MATRIX.md` §2 row → POPULATED (U1); §8 counts |

No `extractor/` / `replay_core` / `map_viewer` / fixture edits. No existing
module renamed/moved. No git commit.

## Gate results (all green)

- `cargo build -p game_ui` — OK (exit 0).
- `cargo test -p game_ui` — OK: lib unit tests **60 passed** (incl. 3 new in
  `unitview::tests`); `unitview_binding.rs` integration **5 passed**; every
  pre-existing gate holds (topbar_binding 3, outliner_binding 3, topbar_quads 1,
  diplomacy_binding 4, widget_layout 3, markup_law 2, ui_completion_binding 4,
  flag_precedence 1).
- `cargo test -p replay_core` — OK (all pass; unchanged).
- `cargo fmt -p game_ui -- --check` — my two files are clean. Remaining
  diffs are **pre-existing** in files this lane did NOT touch (binding.rs,
  markup.rs, markup_law.rs, topbar_quads.rs: installed-rustfmt vs repo-rustfmt
  discrepancy, empty `git diff` on them) — left untouched per the 18-lane rule.

## Layout gates (FILE — interface/unitview.gui)

Parsed via the shared `game_ui` `.gui` parser (`LayoutSet::parse_file`), gated
by `inspect_unitview_layout`. HOI4-root-gated (SKIPs when the vanilla root is
absent, same convention as `widget_layout.rs`). Three selected-unit detail
windows, each with its FILE field slots verified present + runtime grids
classified as documented skips:

| Panel window | domain | FILE field slots gated present | runtime grids (skips) |
|---|---|---|---|
| `army_division_stats_view` | land division | `unit_title`, `experience`, `combat_width_text`, `support_header_label` | ≥5 (`stats_grid`×3, `support_grid`, `regiments_grid`, `regiment_support_grid`, `medals_grid`, `history_entry_grid`, …) |
| `ship_stats_window` | ship / task force | `ship_title`, `ship_name`, `experience`, `current_manpower`, `dominance_value`, `support_value`, `fuel_consumption` (7) | `stats_grid`, `terrain_grid`, `equipment_grid`, `list` |
| `ship_stats_air_wing_entry` | air wing row | `name`, `count` (2) | — |

Gate L1 (`unitview_detail_panels_have_all_file_field_slots`) asserts
`field_slots_missing` is empty for all three and pins the present-slot lists.

## Binding table (replay_core view → panel field)

Fail-closed per field: `Leaf::Measured` → `Some`, else `None` (never a `0`
default). Selection is by stable UID; a missing UID → `None` (not a fabricated
empty unit). Number **formatting is UNKNOWN → values carried raw**
(`Option<f64>`/`Option<i64>`), no percent/bar/grouping invented (Constraint 3).

### Land — `LandUnitView` ← `DivisionUnitRow`

| Panel field | View source | Label | Fixture ground truth (fx_division_movement) |
|---|---|---|---|
| `uid` | `div_uid` (raw) | FILE identity | `101:1` (GER), `201:1` (SOV) |
| `owner` | `owner` | view | `GER` / `SOV` |
| `display_name` | — | **VIEW GAP** → `None` | division has no name field in the read-only view |
| `template_name` | `template_name` | view | `Fixture Infantry` |
| `template_role` | `template_role` | view | `infantry` |
| `template_id` | `template_id` | view | `1` |
| `strength` | `strength` | view (UNKNOWN fmt) | `0.95` |
| `organisation` | `organisation` | view (UNKNOWN fmt) | `42.0` |
| `location_province` | `location_province` | view | `1000`→`1001`→`2000` by tick |
| `moving` / `in_combat` | flags | view | `moving = tick<8` |
| `equipment[]` | `equipment` | view | `Measured`-but-empty → `equipment_available=true`, 0 lines |
| (`experience` slot) | — | **VIEW GAP** unbound | no experience field in `DivisionUnitRow` |

### Naval — `NavalUnitView` ← `FleetUnitRow`

| Panel field | View source | Label | Fixture ground truth (fx_naval_air_activity) |
|---|---|---|---|
| `uid` | `task_force_uid` | FILE identity | `501:1` |
| `fleet_uid` | `fleet_uid` | view | `500:1` |
| `owner` | `owner` | view | `ENG` |
| `display_name` | — | **VIEW GAP** → `None` | task force has no name field in the read-only view |
| `mission` | `mission` | view | `patrol` |
| `ship_count` | `ship_count` | view | `8 + tick` |
| `dominant_hull_class` | `dominant_hull_class` | view | `capital` |
| `location_province` | `location_province` | view | `3000` |
| `ships[]` | `ships` | view | **recording-Absent** (no inline `ships`) → `ships_available=false`, 0 lines |

### Air — `AirWingView` ← `WingUnitRow`

| Panel field | View source | Label | Fixture ground truth |
|---|---|---|---|
| `uid` | `wing_uid` | FILE identity | `800:1` |
| `owner` | `owner` | view | `ENG` |
| `display_name` | `name` | view (bound — air HAS a name) | `Fighter Wing` |
| `plane_count` | `plane_count` (`count`) | view | `100 + tick` |
| `effective_planes` | `effective_planes` | view | `90 + tick` |
| `mission_kind` | `mission_kind` | view | `air_superiority` |
| `strategic_region_id` | `strategic_region` | view | `301` |
| `air_base` | `air_base` | view | `3000:1` |
| `state_id` | `state_id` | view | `30` |
| `plane_definition` | `plane_definition` | recording-Absent → `None` | absent in fixture |
| `experience` | `experience` | recording-Absent → `None` | absent in fixture |
| `manpower` | `manpower` | recording-Absent → `None` | absent in fixture |

## View gaps (replay_core, NOT recording gaps)

The recording carries the data; the READ-ONLY view struct does not surface it
(same class as the D4 diplomacy view gaps). No new recording requirement — so
**no REPLAY_DATA_REQUIREMENTS.md / EXTRACTION_NEEDS.md §A change** is warranted:

1. **Division custom name** — `DivisionUnitRow` has no `name` (fixture emits
   `division_name`). Panel `unit_title` → `None`.
2. **Task-force custom name** — `FleetUnitRow` has no `name` (fixture task force
   emits `name: "Channel Patrol"`). Panel `ship_title`/name → `None`.
3. **Division experience** — `DivisionUnitRow` has no `experience` (fixture
   emits `experience: 0.2`). Land `experience` slot unbound.

Closure (offline, no capture): add `name`/`experience` to `DivisionUnitRow` and
`name` to `FleetUnitRow` in `replay_core::world`, then bind. Filed in
FIDELITY_LEDGER §2.16 "Selected-unit VIEW GAPS (U1)".

## Recording-Absent (view supports, fixtures don't emit)

Air `experience`/`manpower`/`plane_definition`; naval `ships` composition. These
are already schema-supported (the view reads them) and degrade to `None` — not
new requirements.

## Downstream follow-up (out of this lane's scope)

Composite render of the bound fields into the parsed layout (a compositor pass,
like `major_views::compose_view`) — this lane delivers the layout gate + typed
binding seam; drawing the bound values onto the panel is a separate step.

## Test inventory (new)

Module unit tests (`unitview::tests`, 3): `absent_land_row_binds_fail_closed_none`,
`measured_land_row_carries_raw_values`, `air_recording_absent_fields_are_none`.
Integration (`unitview_binding.rs`, 5): `unitview_detail_panels_have_all_file_field_slots`
(L1 layout), `land_division_panel_binds_fixture_ground_truth` (B1),
`naval_task_force_panel_binds_fixture_ground_truth` (B2),
`air_wing_panel_binds_fixture_ground_truth` (B3), `detail_binders_are_domain_isolated` (B4).
