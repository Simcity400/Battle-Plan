# LANE fx-streams — 12 UI-inventory data streams wired end-to-end

**Branch:** `lane/fx-streams` · **Date:** 2026-07-09 · **No git commit** (per lane rules).

## Mission

`UI_COVERAGE_MATRIX.md` §9 flagged **14 new recording-data streams** the deep UI
panels bind against. This lane added, for **12 of the 14** (the `u2a-event-chrome`
and `u2b-decision-chrome` lanes own the fired-event stream + decision state), a
**synthetic fixture + typed `replay_core` view + round-trip gate**, so the next UI
wave gates against real fixture data instead of fail-closing every panel. All
changes are **additive** (zero public removals); the existing 12 positive + 4
negative + 5 anomaly fixtures and their fields are untouched.

Scope owned & edited: `replay_fixture_gen`, `replay_core`,
`REPLAY_DATA_REQUIREMENTS.md`, `EXTRACTION_NEEDS.md`. `extractor/` read-only
(field names mirrored from `REPLAY_SCHEMA.md`).

## Per-stream deliverables

Each row: the `REPLAY_SCHEMA.md` shape used, the generator fixture that emits it,
and the typed view + round-trip gate that certifies it. Every view preserves the
existing no-coercion trichotomy (`Absent` / `Unobtained` / `Measured`); a
wrong-typed value becomes `Unobtained` + a coverage warning, never a guess.

| # | Stream | Schema shape | Generator fixture | replay_core view | Gate |
|---|--------|--------------|-------------------|------------------|------|
| 1 | Research state | `countries.<tag>.research.{tech_count,research_slots,in_progress[{name,progress_pct}]}` §3.4 | `fx_research_focus` | `ResearchView` (`research()`) | `rt_research_focus` |
| 2 | National focus | `countries.<tag>.focus.{focus_tree,current,completed[],completed_count,progress}` §3.5 | `fx_research_focus` | `FocusView` (`focus()`) | `rt_research_focus` |
| 3 | Politics detail + command abilities | `politics.{party_popularities,ideas[],national_spirits[],laws.*,appointed_advisors[]}` §3.3 + `abilities[].ability` | `fx_country_management` | `PoliticsDetailView` (`politics_detail()`) | `rt_country_management` |
| 4 | Production queue + stockpile | `production.{stockpile_total,stockpile,construction_queue[]}` §3.6 + `military.equipment_stockpile[]` §3.2 | `fx_country_management` | `ProductionView` (`production()`) | `rt_country_management` |
| 5 | Templates + equipment variants | `designs.division_templates[]` + `designs.equipment_variants{"id:type":…}` §11 | `fx_designs_catalog` | `DesignsView` (`designs()`) | `rt_designs_catalog` |
| 6 | Intel / operatives | `intel.{agency,operatives[],operations.running[]}` §3.9 | `fx_country_management` | `IntelView` (`intel()`) | `rt_country_management` |
| 7 | Trade state | `economy.resources.{produced,exported,to_export}` §3.1 + `diplomacy.lend_lease[]` §3.8 | `fx_country_management` | `TradeView` (`trade()`) | `rt_country_management` |
| 8 | Occupation resistance/compliance | `states.<id>.occupation{resistance,resistance_speed,compliance,compliance_speed,occupied_country_tag,operational_status}` + `occupation_law` §4 | `fx_occupation_sweep` | `OccupationRow` (`occupation()`, keyed by state) | `rt_occupation_sweep` |
| 9 | Leader roster + traits | `commanders.{leader,field_marshals[],generals[],admirals[],counts}` §3.10 | `fx_leader_roster` | `LeaderRosterView` (`leaders()`) | `rt_leader_roster` |
| 10 | Power-balance state | `world.power_balances[{template,value,left_side,right_side,trending_side,countries[]}]` §18 | `fx_power_balance_combat` | `PowerBalanceView` (`power_balance()`) | `rt_power_balance_combat` |
| 11 | Naval/air combat detail + losses | `naval_combats[]` (new top-level) + `world_events.sunk_convoys[]` §17; air = existing `air_superiority` | `fx_power_balance_combat` | `NavalAirCombatView` (`naval_air_combat()`) | `rt_power_balance_combat` |
| 12 | Scripted-GUI variable state | `countries.<tag>.script_variables{}` + `world.variables{}` + `world.flags{}` §3/§18 | `fx_scripted_gui_vars` | `ScriptedGuiView` (`scripted_gui()`) | `rt_scripted_gui_vars` |
| + | Strategic raid state (small companion) | `raids.{counters.next_target,countries[{tag,priority}]}` §16 | `fx_power_balance_combat` | `RaidStateView` (`raids()`) | `rt_power_balance_combat` |

Radar coverage (the remaining "small" item) has **no `REPLAY_SCHEMA.md` field**;
filed UNKNOWN in `EXTRACTION_NEEDS.md §A30`, no view built until a shape exists.

## Files changed

- `replay_interface/native/replay_fixture_gen/src/main.rs` — 7 new deterministic
  fixtures (`fx_research_focus`, `fx_country_management`, `fx_designs_catalog`,
  `fx_occupation_sweep`, `fx_leader_roster`, `fx_power_balance_combat`,
  `fx_scripted_gui_vars`), each with a manifest probe. Existing 12 fixtures +
  helpers untouched. Positive fixture count **12 → 19**.
- `replay_interface/native/replay_core/src/world.rs` — 13 new `pub` view
  structs + row types, 13 `WorldFrame` fields + accessors, 13 builder passes
  (shared `country_map` for the 6 country-scoped views; bespoke passes for
  occupation/designs/power-balance/naval-combat/scripted-gui/raids), and 4 new
  shared helpers (`leaf_f64_map`, `object_keys`, `leaf_rows`,
  `leaf_obj_field_list`). Additive only — no existing symbol changed/removed.
- `replay_interface/native/replay_core/src/lib.rs` — re-export the new types.
- `replay_interface/native/replay_core/tests/roundtrip_gates.rs` — 8 new gates
  (7 stream gates + `rt_fx_streams_domain_absence` trichotomy gate);
  `POSITIVE_FIXTURES` 12 → 19; `assert_state_equal` extended with the new views
  (so A4 `seek==play` and reopen-determinism also certify them).
- `replay_interface/native/replay_core/tests/fixture_conformance.rs` — positive
  count assertion 12 → 19.
- `replay_interface/native/replay_app/src/validate.rs` — positive count
  assertion 12 → 19 (only count literal; no logic change).
- `replay_interface/documents/REPLAY_DATA_REQUIREMENTS.md` — new **item 14**
  (12 stream shapes + raids) + coverage-status table row.
- `replay_interface/documents/EXTRACTION_NEEDS.md` — new §A rows **A18–A29**
  (per stream, R-* extractor tags) + **A30** radar UNKNOWN; dedup note vs u2a/u2b.

## Proofs (all FILE/gate-verified, no eyeballing)

- **Regeneration determinism** — `replay_fixture_gen --out A` and `--out B` at the
  default seed produce **byte-identical** trees (`diff -r` clean); `--seed 999`
  differs (variance sanity). Same-seed reopen equality also gated in
  `snapshot_path_is_deterministic`.
- **`cargo test -p replay_core`** — roundtrip_gates **31 passed / 0 failed**
  (8 new + 23 existing), fixture_conformance **3 passed / 0 failed** (positive
  count now 19, negative still 4), world_timeline **7 passed**. Every new gate's
  oracle is the packed snapshot's raw JSON path == the typed view (fixture is the
  oracle). `well_formed_fixtures_emit_no_coverage_warnings` passes across all 19
  fixtures ⇒ no builder coerces or warns on well-typed data.
- **Downstream no-break** — `cargo build --workspace` green;
  `cargo test -p map_viewer -p game_ui -p replay_app` = **982 passed / 0 failed**
  (incl. `validate::all_positive_fixtures_open_through_shared_validator` with the
  new count, and the game_ui/replay_app suites that consume the existing 12
  fixtures unchanged).

## Exact counts

- Positive fixtures: **12 → 19** (+7). Negative: **4** (unchanged). Anomaly: **5**
  (unchanged). Snapshots per fixture: **12** (unchanged).
- New `pub` types in `replay_core`: **25** (12 view structs + 13 row/leaf types).
- New `WorldFrame` accessor methods: **34**; new builder functions: **13**;
  new shared helpers: **4**.
- New round-trip gates: **8**. Total `replay_core` tests: 31 roundtrip + 3
  conformance + 7 timeline = **41**, all green.
- Streams covered this lane: **12 of 14** (+ raids companion); 2 (fired-event,
  decision-state) left to u2a/u2b; 1 (radar) filed UNKNOWN.

## Notes / handoff

- The 12 shapes are extractor-OPEN (A18–A29 in `EXTRACTION_NEEDS.md`): the replay
  side is proven on synthetic fixtures; the extractor must fold each into item 1's
  hourly snapshots. Field names mirror `REPLAY_SCHEMA.md` so the fold is a rename-
  free ingest.
- Occupation view is DomainAbsent (no rows) when no state carries an occupation
  object — it never floods the map with unobtained rows; only an active occupation
  produces a row. Also feeds the resistance/compliance map modes.
- `naval_combats` is a **new** top-level domain key (not yet in `REPLAY_SCHEMA.md`
  §7, which is land-combat-only); flagged in A28 to be added schema-side.
- At merge, dedup A18–A29 against any stream rows the u2a/u2b or other u2 lanes
  filed concurrently (this worktree branched from local master). The stale
  `f4-anomaly` LANE_REPORT that previously occupied this path (with unresolved
  merge markers) has been replaced by this report.
