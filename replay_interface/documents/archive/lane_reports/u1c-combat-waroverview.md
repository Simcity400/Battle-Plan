# LANE REPORT — lane/u1c-combat-waroverview (U1 wave: combat + war-overview panels)

Plan: `REPLAY_PRODUCTION_PLAN.md` §6a (U1 wave) + `UI_COVERAGE_MATRIX.md` §3
(popups). Two panels derived + populated at full fidelity the recording allows:
the **land-combat detail** window (`interface/landcombat.gui` `landcombatview`)
and the **war-overview / war-summary** window (`interface/waroverview.gui`
`waroverview_window`). Worktree `C:\Users\Example\hoi4_lanes\u1c-combat-waroverview`,
branch `lane/u1c-combat-waroverview`. READ-ONLY on `replay_core`; no
`extractor/`, `map_viewer`, or fixture edits; no existing modules renamed.

## Gate numbers (all green)

| Gate | Result |
|---|---|
| `cargo build -p game_ui` | **Finished, 0 errors**, no new warnings |
| `cargo test -p game_ui` | **62 lib unit** (+5 new src unit tests) + **combat_binding.rs 3** + **waroverview_binding.rs 3** + all pre-existing test files → **0 failed** |
| `cargo test -p replay_core` | **23 + 7 + 3 + 3 passed / 0 failed** (untouched, READ-ONLY) |
| Layout parse | `landcombat.gui` and `waroverview.gui` parse via the shared `game_ui` `LayoutSet` with **0 recoveries** each |

Vanilla root `<local-path>` (no bare joins — layout gates read via the
game root, skip cleanly if absent). Fixtures driven through the real
`replay_core` `ReplaySource`/`WorldTimeline`.

## Files touched

- New src modules: `game_ui/src/combat_binding.rs`, `game_ui/src/waroverview_binding.rs`
  (+ registered in `game_ui/src/lib.rs`).
- New test files: `game_ui/tests/combat_binding.rs`, `game_ui/tests/waroverview_binding.rs`.
- Docs (same change): `UI_COVERAGE_MATRIX.md`, `FIDELITY_LEDGER.md`,
  `REPLAY_DATA_REQUIREMENTS.md`, `EXTRACTION_NEEDS.md`, this report.

## Layout gates (FILE)

Both panels parse through the shared parser (no forked parser) and resolve named
elements through the LAW-solid `resolve_element_rect` (engine anchor/position law).

- **landcombat.gui** — `landcombatview` resolves to `(448, 284, 567, 617)` on a
  1920×1080 screen (declared `567x617`, `orientation center`, position
  `{-512,-256}` → center-anchor `(960,540)` + position). Direct children
  `window_title (55,2)`, `terrain_picture (77,37)`, `combat_progress (185,116)`,
  `land_battle_progress_frame (183,114)` resolve to window-relative pixel offsets;
  the `attacker_window`/`defender_window` division/reserve sub-lists are present.
- **waroverview.gui** — `waroverview_window` declared width `1167`; the wars
  gridbox `waroverview_wars_box` + both participant columns present. The
  participant-row template `war_ally_entry` (`530x41`) resolves its named
  textboxes `country_name (56,4)`, `warscore (269,11)`, `divisions (317,11)`,
  `ic (393,11)`, `casualties (466,11)`.

## Binding tables

### Combat detail (`combat_binding.rs` — from `replay_core::CombatRow`)

| Panel field | Source | Class |
|---|---|---|
| combat identity (`combat_id`/`kind`/`location_province`) | `CombatRow.{combat_id,kind,location_province}` | POPULATED |
| attacker/defender division counts (`*_committed_label`) | `CombatRow.{attackers,defenders}` list len | POPULATED |
| `combat_progress` / `land_battle_progress_frame` / `combat_heading` fill | — | **A16 fail-closed** (recording-Absent) |
| `combat_width` value | — | **A16 fail-closed** |
| shore-bombardment / air-superiority modifier | — | **A16 fail-closed** |
| attacker/defender leaders, tactics | — | fail-closed (not in `CombatRow`) |

Gate: `fx_division_movement` — one combat `{9001, land, loc 1001→2000,
att [101], def [201]}` appears ticks 6–9 exactly per generator, absent otherwise;
all A16 display fields `None`.

### War overview (`waroverview_binding.rs` — from `replay_core::DiplomacyView`)

| Panel field | Source | Class |
|---|---|---|
| belligerent-pair casualties (`casualties`) | `DiplomacyView.war_relations` (`WarRelationRow`) | POPULATED |
| belligerent set (participant columns) | `DiplomacyView.pairs` (`at_war==true`) ∪ war_relations | POPULATED |
| POV belligerents (highlight) | `pov → other` `at_war==Some(true)` | POPULATED |
| war-object grouping (attacker/defender sides, front, wargoal) | — | **fail-closed** (view gap) |
| `warscore` / `divisions` / `ic` / `surrender_progressbar` | — | **fail-closed** (NEW recording dep) |

Gate: `fx_war_faction_matrix` — casualties `[{GER,ENG,1200,900},{ITA,ENG,200,300}]`
+ belligerents `[ENG,GER,ITA]` materialize at tick 7 exactly; belligerent set is
POV-independent, POV-belligerents per-POV; absent POV sees the global picture.

All fail-closed per Constraint 3 (absent leaf → `None`, never a fabricated `0`);
no recording bytes parsed in `game_ui` (Constraint 2).

## Gaps filed (no fabrication)

1. **A16 (OPEN, already tracked)** — combat-display detail (land `progress` /
   `bombardment_active` / `air_superiority_state`; naval combat block). The
   combat panel fails closed on all of these. Additionally noted: even the
   fixture-scaffolded `combat_width`/`terrain`/`progress` are not surfaced by
   `replay_core::CombatRow` (scoped to id/kind/location/participant-ids) — a
   READ-ONLY `replay_core` view gap. `EXTRACTION_NEEDS.md` A16 / `COMBAT_INDICATOR_LAW.md`.
2. **A18 (NEW)** — war-participant aggregate stats
   `{warscore, division_count, industrial_capacity, capitulation_progress,
   wargoals[]}`. Added to `EXTRACTION_NEEDS.md §A` + `REPLAY_DATA_REQUIREMENTS.md §8b`.
3. **`replay_core` view gap (READ-ONLY, not a recording change)** — war-object
   grouping (`diplomacy_world.wars` attacker/defender sides + front) is in the
   recording but not surfaced by `DiplomacyView` (same D4 known-view-gap class).

## Matrix rows updated (`UI_COVERAGE_MATRIX.md`)

- §3 Land combat `landcombat.gui` → **POPULATED (U1)**.
- §3 War overview `waroverview.gui` → **POPULATED (U1)**.
- §8 popup category + grand-total POPULATED count 3 → **5 panels**.
