# Lane rc-reconcile — replay_core view-path reconciliation

Branch `lane/rc-reconcile` (worktree `C:\Users\Example\hoi4_lanes\rc-reconcile`),
based on local `master` (ab881292f, contains the merged a5-keying lane).
Scope: replay_core VIEW-PATH mismatches surfaced by parallel lanes — fields that
fail-closed to `Absent` because the typed view reads a path the fixture
generator (the schema oracle, `replay_fixture_gen/src/main.rs`) does not write.

**Method:** every mismatch verified against BOTH the view code
(`replay_core/src/world.rs`) AND the generator SOURCE. Fixes are additive-only
(zero public-API removals). Round-trip gates derive expectations from the
generator SOURCE, never from the code under test. `codex` is banned per session
directive — reasoning is my own.

---

## Per-mismatch verdicts

### 1. world_tension — DOCUMENTED GAP (genuinely-missing recording field)
- **View reads:** `world.world_tension` / `world.tension` / `world_events.world_tension` (`topbar_world_tension`, world.rs).
- **Generator writes:** `world.world_tension_inputs.{average_major_ic, tension_scaling_base_country}` (base) + `world_tension_inputs.fixture_topbar_world_tension` (sweep only).
- **Verdict:** NOT wired. `fixture_topbar_world_tension` is explicitly a **fixture-only key** (fixture scaffolding, not schema — coupling production code to it would violate the everything-engine-derived mandate). Engine evidence (`extractor/.../world_extract_offsets.draft.json`, `CRACKED_LEDGER.md`, `D3_session_tension/EVIDENCE.md`): world_tension is **COMPUTED-not-stored** — the clamped-0..100 decayed sum of the ~160-record NDiplomacy threat ledger via the `TENSION_*` defines. The melt/recording carries no `world_tension` scalar and no threat ledger, and `average_major_ic`+`base_country` alone are **insufficient to recompute** it losslessly. So there is no rollup to compute.
- **Outcome:** view correctly stays `Absent` (fail-closed, no fabrication). Gap filed: **EXTRACTION_NEEDS.md A16** (new) + **REPLAY_DATA_REQUIREMENTS.md §9b caveat**. Existing gate `rt_topbar_scalars_sweep` already asserts `Absent`; left intact.

### 2. naval supremacy magnitude unit — DOCUMENTED GAP (extractor must state unit)
- **View reads:** `naval_air.region_control.<r>[].supremacy` (also `.value`) → f64. Read path is CORRECT; the open question is the UNIT.
- **Generator writes:** `supremacy: 0.55 + tick*0.02` — normalised `0..1`.
- **Verdict:** the engine accumulates naval-region supremacy in raw points and only normalises for the tint vs `NAVAL_REGION_TRANSPARENCY_RANGE` (=2000). Whether a real recording carries the normalised fraction or the raw point total is NOT pinned by the schema (D56, c6 lane). The view now surfaces the number **verbatim, never rescaling**, with the unit ambiguity documented in the `NavalSupremacyRow::supremacy` doc-comment. Gap already tracked in **EXTRACTION_NEEDS.md A6** ("the extractor must state D56's unit"); no new row needed. Label: **UNKNOWN**.

### 3. diplomacy surfaces — CLOSED (additive typed views)
- **d-panels found Absent:** `diplomacy_live.war_relations`, `diplomacy_live.faction_members`, `countries.*.diplomacy.relation_flags`.
- **Generator writes** (fx_war_faction_matrix): `diplomacy_live.war_relations` = `[{first, second, first_casualties, second_casualties}]` (tick≥7); `diplomacy_live.faction_members` = `{faction_id: [TAG,…]}` (tick≥3); `countries.GER.diplomacy.relation_flags` = `{other_tag: [flag,…]}` (tick≥7). All park explicit-null in the base before then.
- **Fix (additive to `DiplomacyView`):** new fields `war_relations: Leaf<Vec<WarRelationRow>>`, `faction_members: Leaf<BTreeMap<String, Vec<String>>>`, `relation_flags: BTreeMap<String, Leaf<BTreeMap<String, Vec<String>>>>`; new struct `WarRelationRow`; readers `read_war_relations` / `read_faction_members` / `read_relation_flags`.
- **CRITICAL design decision:** `relation_flags` is surfaced **verbatim, NOT folded into the boolean `pairs` matrix.** game_ui GATE 2c (`diplomacy_binding::relation_row_flags_are_fail_closed`) asserts `GER→ITA military_access == None` / `embargo == None` at tick 7 — but the fixture records exactly those flags. Folding them into `pairs` would break that certified downstream gate. Surfacing them as a separate typed field flips them Absent→typed data while leaving the certified directional matrix (and every downstream consumer) untouched.
- **Gate:** `rt_diplomacy_live_surfaces` (trichotomy across all 12 hours; also asserts `pairs` is unperturbed).

### 4. air_superiority — CLOSED (new additive typed view)
- **State:** the view had **NO** air-superiority field at all; `naval_supremacy_by_region` only carried naval region control.
- **Generator writes** (fx_naval_air_activity): `naval_air.air_superiority.<region>` = `{controller, mission_mask, wing_count, plane_count}` every tick; base parks it null.
- **Fix:** new `AirSuperiorityRow` struct, `build_air_superiority` builder, `WorldFrame` fields + accessors `air_superiority_by_region()` / `air_superiority(region)` / `air_superiority_status()`.
- **Gate:** `rt_air_superiority` (verbatim round-trip) + `rt_air_superiority_unobtained_when_null` (trichotomy: base-null → DomainUnavailable, empty map).

### 5. alerts / ratio bars — CLOSED (new additive typed views)
- **State:** `countries.<tag>.scalars.{industrial_ratio, convoy_ratio, fuel_ratio, xp_ratio}` and `scalars.alerts.{initiative, decisions_available, intel, market_contracts}` had NO typed field.
- **Fix:** new `TopbarRatios` + `TopbarAlerts` structs (kept SEPARATE from `TopbarScalars`, which a game_ui `#[cfg(test)]` helper constructs with a full struct literal — adding fields there would break `cargo test -p game_ui`). New `build_topbar_extras` builder, `WorldFrame` fields + accessors `topbar_ratios()` / `topbar_alerts()` / `topbar_extras_status()`.
- **Gate:** `rt_topbar_ratios_alerts` (verbatim across 3 POVs × 12 hours) + `rt_topbar_ratios_alerts_absent_without_scalars` (trichotomy: empty `scalars {}` → every leaf Absent, never fabricated 0).

---

## Public-API additions (all additive; zero removals)
`world.rs` / re-exported from `lib.rs`:
- structs `WarRelationRow`, `AirSuperiorityRow`, `TopbarRatios`, `TopbarAlerts`
- `DiplomacyView` fields `war_relations`, `faction_members`, `relation_flags`
- `NavalSupremacyRow::supremacy` doc now states the UNKNOWN unit (no behavior change)
- `WorldFrame` accessors: `air_superiority_by_region/air_superiority/air_superiority_status`, `topbar_ratios_by_country/topbar_ratios`, `topbar_alerts_by_country/topbar_alerts`, `topbar_extras_status`

## Tests
- **replay_core: 31 → 36** (roundtrip_gates 18 → 23; +5 new gates). All green, 0 failures.
  - Breakdown: lib 0, make_demo_recording 3, replay_dump 0, fixture_conformance 3, roundtrip_gates 23, world_timeline 7, doc-tests 0.
- A4 `assert_state_equal` extended (additive) to also compare air_superiority + topbar ratios/alerts; the new `DiplomacyView` fields are already covered via whole-view `PartialEq`.

## Downstream build/test proof (no breakage)
- `cargo build --workspace` — Finished (pre-existing unrelated warnings in `replay_app/overlay.rs`).
- `cargo build -p map_viewer -p game_ui -p replay_app` — Finished.
- `cargo test -p game_ui` — passed (exit 0); `diplomacy_binding` 4/4 incl. GATE 2c `relation_row_flags_are_fail_closed` — confirms relation_flags surfacing did NOT perturb the boolean pairs matrix.

## Docs updated (this lane; no extractor/ or fixture edits)
- `EXTRACTION_NEEDS.md` — new row **A16** (world_tension computed-not-stored / threat-ledger gap).
- `REPLAY_DATA_REQUIREMENTS.md` — §9b world_tension caveat + note that ratios/alerts are now surfaced.
