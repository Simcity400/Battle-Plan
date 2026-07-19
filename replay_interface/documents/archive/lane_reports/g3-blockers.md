# Lane report — G3 canonical blocker list machinery

**Branch:** `lane/g3-blockers`
**Plan item:** G3 (REPLAY_PRODUCTION_PLAN.md §0 Constraint 3, §7 Definition-of-production item 2)
**Date:** 2026-07-09

## What landed

`replay_app/src/blockers.rs` — a static, single-source registry of every
fail-closed product block in `replay_app`. Every `main.rs` gate now routes its
fail-closed message through the registry (pure indirection, no behavior change),
and `--list-blockers` emits the whole list deterministically for G3 audits and
the definition-of-done.

- **29 blocker surfaces** = **23 blocked CLI switches** + **6 rejected env
  overrides**.
- **10** of those surfaces carry a `*_product_path_blocked` gate test (the count
  called out in §0 Constraint 3). The remaining 19 are legacy opt-outs, the
  removed `--no-trees-second-subdraw` flag, and debug-knob env rejections.
- `BlockerId` enum + `Surface` (CLI switch / env override) + `BlockerKind`
  (law-residual / legacy-opt-out / debug-knob) + `Status` (BLOCKED) +
  `reason` (the exact fail-closed message) + `waits_for` (closure ref) +
  `product_path_test` (covering gate).

### Gates added (all green)

| Gate | Asserts |
|---|---|
| `registry_covers_every_product_path_gate` (main.rs) | greps the crate for every `fn *_product_path_blocked` (10) and asserts the set equals the registry's `product_path_test` references — **no block can be added without a registry row**, and no phantom reference is allowed |
| `every_blocked_cli_switch_arm_routes_through_registry` (main.rs) | each of the 23 blocked CLI switches is still rejected by `parse_cli`, returning the registry reason verbatim (proves indirection is wired, no gate weakened) |
| `list_blockers_cli_runs_without_a_bundle` (main.rs) | `--list-blockers` is a standalone entry needing no map bundle |
| `render_list_is_deterministic_and_snapshot_stable` (blockers.rs) | snapshot of the emission: header `replay_app product blockers: 29`, exact golden rows, one line per registry row |
| `counts_are_exact` (blockers.rs) | 29 total / 23 CLI / 6 env, pinned |
| `registry_has_one_row_per_id_and_unique_surfaces` (blockers.rs) | one row per `BlockerId`; unique surfaces + slugs |
| `every_reason_message_starts_with_its_surface_token` (blockers.rs) | each reason begins `<surface> is blocked/removed` — ties registry text to the `main.rs` `.contains(...)` assertions |
| `env_reason_lookup_resolves_all_env_rows` (blockers.rs) | env-name → reason lookup covers all 6 env rows |

### Numbers

- **Build:** `cargo build -p replay_app` green (pre-existing warnings only, no new ones).
- **Tests:** `cargo test -p replay_app` **178 passed / 0 failed** (baseline 170 + 8 new; all 10 pre-existing `*_product_path_blocked` tests pass unchanged).

## Canonical blocker list

### Blocked CLI switches (23)

| # | Id / slug | Surface | Kind | Waits-for (closure ref) | Gate test |
|---|---|---|---|---|---|
| 1 | `demo_scrub` | `--demo-scrub` | law-residual | Plan A5 renderer keying swap + E1 transport (real ReplaySource-driven timeline) | `cli_demo_scrub_product_path_blocked` |
| 2 | `gb_cpu_flag` | `--gb-cpu` | law-residual | Plan A5 + POLITICAL_COLOR_LAW.md / GB field law (exact GradientBorder generation); ledger §2.8 | — |
| 3 | `synthesize_terrain_chunks` | `--synthesize-terrain-chunks` | law-residual | Plan B7 terrain vSnap/render-view frustum (CAP1); ledger §2.9 | `cli_synthesize_terrain_chunks_product_path_blocked` |
| 4 | `chunks_from_files` | `--chunks-from-files` | law-residual | Plan B7 terrain vSnap/LOD level-assignment (CAP1); ledger §2.9 (LODSNAP/LODRING/LODCULL2) | `cli_chunks_from_files_product_path_blocked` |
| 5 | `no_water_full` | `--no-water-full` | legacy-opt-out | Design block (WATER_LAW.md; `--water-full` default-on, fail-closed) | — |
| 6 | `borders_from_files` | `--borders-from-files` | law-residual | Plan B1 water-strip residuals + island loops + fade interior (BORDER_LAW.md §2.5, BORD8/9/10) | `cli_borders_from_files_product_path_blocked` |
| 7 | `no_borders_from_files` | `--no-borders-from-files` | legacy-opt-out | Design block (borders already blocked; no-op opt-out) | `cli_borders_from_files_product_path_blocked` |
| 8 | `no_capture_water` | `--no-capture-water` | legacy-opt-out | Design block (captured engine water required) | — |
| 9 | `no_capture_rivers` | `--no-capture-rivers` | legacy-opt-out | Design block (captured engine rivers required) | — |
| 10 | `no_capture_trees` | `--no-capture-trees` | legacy-opt-out | Design block (captured engine trees required) | — |
| 11 | `no_postfx` | `--no-postfx` | legacy-opt-out | Design block (GRADING_LAW.md; `--postfx` default-on, raw RTs pre-tonemap) | — |
| 12 | `no_postfx_restore_full` | `--no-postfx-restore-full` | legacy-opt-out | Design block (GRADING_LAW.md GRAD1; default-on, fail-closed) | — |
| 13 | `tree_seasons_by_date` | `--tree-seasons-by-date` | law-residual | Plan B4 tree season CB1 from files (CAP9); ledger §2.11 | `cli_tree_season_date_patch_product_path_blocked` |
| 14 | `trees_from_files` | `--trees-from-files` | law-residual | Plan B4 tree scatter anchor/rotation/model selection (CAP9); ledger §2.11 | `cli_trees_from_files_product_path_blocked` |
| 15 | `no_trees_second_subdraw` | `--no-trees-second-subdraw` | legacy-opt-out | Design block (RE-refuted; alleged 2nd draw is the LOW_END_GFX shader) | — |
| 16 | `buildings_from_files` | `--buildings-from-files` | law-residual | Plan B3 building `.mesh` importer / map_objects_pass (CAP9) | `cli_building_preview_product_path_blocked` |
| 17 | `buildings_preview_all` | `--buildings-preview-all` | law-residual | Plan B3 building `.mesh` importer / map_objects_pass (CAP9) | `cli_building_preview_product_path_blocked` |
| 18 | `freeze_clock` | `--freeze-clock` | law-residual | Plan D6 replay-native clock + ledger §2.9 scene-CB0 runtime time (vGlobalTime) | `cli_freeze_clock_product_path_blocked` |
| 19 | `map_labels` | `--map-labels` | law-residual | Plan B10 label owner-vs-controller keying + per-label Transp_OffsetX.x (CAP5); MAP_LABEL_LAW.md §2.6 | `cli_map_labels_product_path_blocked` |
| 20 | `no_map_labels` | `--no-map-labels` | legacy-opt-out | Design block (labels already blocked; no-op opt-out) | `cli_map_labels_product_path_blocked` |
| 21 | `map_labels_fallback` | `--map-labels-fallback` | legacy-opt-out | Design block (legacy label reconstruction, not engine-derived) | `cli_map_labels_product_path_blocked` |
| 22 | `map_furniture` | `--map-furniture` | law-residual | Ledger §2 furniture unblock assessment (highlight ring + 0.40 map-text scale origin; RINT1/FURN-REL) | `cli_map_furniture_product_path_blocked` |
| 23 | `no_map_furniture` | `--no-map-furniture` | legacy-opt-out | Design block (furniture already blocked; no-op opt-out) | `cli_map_furniture_product_path_blocked` |

### Rejected env overrides (6)

| # | Id / slug | Surface | Kind | Waits-for (closure ref) |
|---|---|---|---|---|
| 24 | `env_gb_cpu` | `REPLAY_APP_GB_CPU` | law-residual | Plan A5 + POLITICAL_COLOR_LAW.md / GB field law (env twin of `--gb-cpu`); ledger §2.8 |
| 25 | `env_debug_dump_underwater` | `REPLAY_DEBUG_DUMP_UNDERWATER` | debug-knob | Design block (debug/validation knob barred from product render path) |
| 26 | `env_debug_clear_before_water` | `REPLAY_DEBUG_CLEAR_BEFORE_WATER` | debug-knob | Design block (debug/validation knob barred from product render path) |
| 27 | `env_debug_underwater_inputs` | `REPLAY_DEBUG_UNDERWATER_INPUTS` | debug-knob | Design block (debug/validation knob barred from product render path) |
| 28 | `env_debug_constant_refraction` | `REPLAY_DEBUG_CONSTANT_REFRACTION` | debug-knob | Design block (debug/validation knob barred from product render path) |
| 29 | `env_dump_labels` | `REPLAY_DUMP_LABELS` | debug-knob | Design block (debug/validation knob barred from product render path) |

## Notes

- **10 distinct `*_product_path_blocked` gates** are referenced by **15** CLI
  rows: four gates cover a family of switches — `map_labels` covers
  `--map-labels` + `--no-map-labels` + `--map-labels-fallback`, `map_furniture`
  covers `--map-furniture` + `--no-map-furniture`, `borders_from_files` covers
  `--borders-from-files` + `--no-borders-from-files`, and the building gate
  covers `--buildings-from-files` + `--buildings-preview-all`. The other six
  gates are 1:1 with their switch.
- **No blocking behavior was changed.** Each gate returns the identical
  message it did before, now sourced from the registry. The env functions
  (`reject_gb_cpu_env`, `reject_product_debug_env_value`) keep their signatures
  (unit tests call them directly) and pull their message from the registry.
- **`--list-blockers`** is intercepted before `parse_cli` (like
  `--validate-recording`), so it runs with no map bundle. Emission is
  tab-separated: `slug<TAB>kind<TAB>surface-kind<TAB>surface<TAB>status<TAB>waits-for`.
- **Adding a new block** now requires: a `BlockerId` variant + a `REGISTRY`
  row; if it is a CLI/env gate, route the `main.rs` arm through
  `blockers::reason(...)`. The coverage gate fails the build's test step if a
  new `*_product_path_blocked` gate has no registry row.
