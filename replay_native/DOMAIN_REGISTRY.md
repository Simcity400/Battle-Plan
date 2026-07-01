# Replay Native Compact Domain Registry

This is the orchestrator-owned source of truth for compact domain IDs. Lane
subagents must not choose IDs independently. If a lane needs a new domain, the
orchestrator assigns it here first.

For scope-sensitive status, read `EXTRACTION_SCOPE_STATUS.md` alongside this
registry. The registry status is a compact label; that file lists what each
validated slice actually covers and what remains open for full replay fidelity.

Registration rule: lane implementation lives in lane-owned collector/decoder
modules. Shared `collector/src/lib.rs` and `recorder_host/src/main.rs` changes
should be registration hooks only unless the orchestrator explicitly creates or
migrates the module interface.

Status vocabulary:

- `validated slice` means the named scope passed the DLL/compact/Rust replay
  gate at hourly-or-better cadence. It does not mean the whole gameplay concept
  is captured.
- `concept complete` is reserved for full replay coverage of the concept: every
  replay-relevant extractable field is captured, or each remaining field has an
  exact blocker or conclusive rejection.
- Open walls stay in the status label. Do not compress them back to
  `production-complete`.

## Active ID Map

| ID | Domain | Status | Owner lane |
| --- | --- | --- | --- |
| 10 | `fuel_live` | existing production compact domain | country economy |
| 11 | `production_live` | existing production compact domain | country economy |
| 12 | `focus_live` | existing base compact domain | focus base |
| 13 | `tech_live` | existing base compact domain | research base |
| 14 | `politics_live` | existing base compact domain | politics base |
| 15 | `politics_residuals` | existing base compact domain | politics base |
| 16 | `decisions_live` | existing base compact domain | decisions base |
| 17 | `diplomacy_live` | existing base compact domain | diplomacy base |
| 20 | `divisions_live` | existing compact domain | divisions |
| 21 | `land_combats_live` | existing compact domain | land combat |
| 22 | `map_control_live` | existing compact domain | map control |
| 23 | `map_detail_live` | existing compact domain | map detail |
| 24 | `weather_live` | validated slice: weather visual drivers plus sandstorm flag; enum/particle parity and internals open | weather visual drivers |
| 26 | `division_equipment_live` | existing compact domain | divisions |
| 27 | `army_orders_live` | existing compact domain | army orders |
| 28 | `oob_command_live` | existing compact domain | army OOB |
| 29 | `ships_live` | existing compact domain | naval OOB |
| 30 | `air_wings_live` | existing compact domain | air wings |
| 31 | `air_region_activity_live` | existing compact domain | air activity |
| 32 | `air_wing_loadouts_live` | existing compact domain | air wings |
| 33 | `air_variant_defs` | existing compact domain | air variants |
| 34 | `air_base_status_live` | existing compact domain | air bases |
| 35 | `air_variant_stats` | existing compact domain | air variants |
| 36 | `air_variant_reserves` | existing compact domain | air reserves |
| 37 | `stockpile_variant_defs` | existing compact domain | stockpile |
| 38 | `air_superiority_inputs` | existing compact domain | air superiority |
| 39 | `air_variant_upgrades` | existing compact domain | air variants |
| 40 | `stockpile_variant_upgrades` | existing compact domain | stockpile |
| 41 | `projects_live` | partial validated slice: facilities/scientists/progress/completed bit; start/reward fixture blocked | projects |
| 42 | `air_combat_losses_live` | validated slice: loss/reinforcement delta; direct event root rejected; not full air combat | air combat losses |
| 43 | `market_trade_live` | validated slice: purchase contracts; dynamic D-tag buyer resolved; omitted candidates/full trade economy open | market/trade |
| 44 | `supply_logistics_attrition_live` | validated slice: supply/logistics subset; `CSupplySystem` rows structurally integrated via anchored root but speed-five proof open; exact province supply wall open | supply/logistics |
| 45 | `focus_research_decisions_depth_live` | validated slice: transition depth; decision body candidates rejected; unresolved fields open | focus/research/decisions depth |
| 46 | `air_variant_archetypes_live` | validated slice: variant archetypes; token `20378` resolved; shared token hook/full air lifecycle open | air wings/variants lifecycle |
| 47 | `characters_leaders_cabinet_live` | lane-ready; shared adapter/registration blocked; remaining matrix written | characters/leaders/cabinet |
| 48 | `construction_buildings_state_damage_live` | target-state observer ready; external same-frame save/UI action blocked | construction/buildings/state damage |
| 49 | `naval_combat_lifecycle_live` | fixture-blocked: no active battle in current process; current CShip route validated | naval combat |
| 50 | `naval_combat_members_live` | fixture-blocked: no active battle in current process; current CShip route validated | naval combat |
| 51 | `scripted_state_flags_variables_live` | validated lane slice: shared registered; country/global/state variables and token/date/value flags; open opaque roots | scripted state/flags/variables |
| 52 | `occupation_resistance_compliance_live` | validated slice: shared registered occupation state rows; garrison scalar/collaboration detail open | occupation/resistance/compliance |
| 53 | `diplomacy_intel_operatives_live` | lane-ready; stale operation RVAs rejected; operation/intel-map/incoming-action fixtures blocked | diplomacy/intel/operatives |
| 54 | `army_battleplans_depth_live` | redraw/toggle observer ready; external fixture action blocked | army OOB/orders/battleplans depth |
| 1000 | `frame` | compact stream framing/perf records | recorder infrastructure |

## Lane Acceptance Under This Registry

- A lane may not report a new compact domain ID as free unless this registry
  assigns it.
- A lane-local proof must use its assigned final ID.
- A lane-local packet that still needs major shared collector/recorder
  implementation is not merge-ready.
- Final integration should be registration plus combined proof. If a lane cannot
  reach that shape, it must return an exact blocker or conclusive rejection.
