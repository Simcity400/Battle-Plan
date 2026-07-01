# Replay Native Compact Domain Registry

This is the orchestrator-owned source of truth for compact domain IDs. Lane
subagents must not choose IDs independently. If a lane needs a new domain, the
orchestrator assigns it here first.

Registration rule: lane implementation lives in lane-owned collector/decoder
modules. Shared `collector/src/lib.rs` and `recorder_host/src/main.rs` changes
should be registration hooks only unless the orchestrator explicitly creates or
migrates the module interface.

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
| 24 | `weather_live` | production-complete | weather visual drivers |
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
| 41 | `projects_live` | production-complete base/project facilities/scientists/progress/completed bit; start/reward blocked | projects |
| 42 | `air_combat_losses_live` | production-complete | air combat losses |
| 43 | `market_trade_live` | production-complete | market/trade |
| 44 | `supply_logistics_attrition_live` | production-complete | supply/logistics |
| 45 | `focus_research_decisions_depth_live` | production-complete | focus/research/decisions depth |
| 46 | `air_variant_archetypes_live` | production-complete | air wings/variants lifecycle |
| 47 | `characters_leaders_cabinet_live` | lane-ready; shared registration blocked; remaining matrix written | characters/leaders/cabinet |
| 48 | `construction_buildings_state_damage_live` | lane-ready; target-state oracle fixture blocked | construction/buildings/state damage |
| 49 | `naval_combat_lifecycle_live` | lane-ready; active naval-battle fixture blocked | naval combat |
| 50 | `naval_combat_members_live` | lane-ready; active naval-battle fixture blocked | naval combat |
| 51 | `scripted_state_flags_variables_live` | lane-ready; opaque positive fixtures blocked | scripted state/flags/variables |
| 52 | `occupation_resistance_compliance_live` | lane-ready; shared registration blocked; garrison/collab candidates rejected or fixture-blocked | occupation/resistance/compliance |
| 53 | `diplomacy_intel_operatives_live` | lane-ready; operation/intel-map/incoming-action fixtures blocked | diplomacy/intel/operatives |
| 54 | `army_battleplans_depth_live` | lane-ready; battleplan redraw fixture blocked | army OOB/orders/battleplans depth |
| 1000 | `frame` | compact stream framing/perf records | recorder infrastructure |

## Lane Acceptance Under This Registry

- A lane may not report a new compact domain ID as free unless this registry
  assigns it.
- A lane-local proof must use its assigned final ID.
- A lane-local packet that still needs major shared collector/recorder
  implementation is not merge-ready.
- Final integration should be registration plus combined proof. If a lane cannot
  reach that shape, it must return an exact blocker or conclusive rejection.
