# Extraction Scope Status

This file prevents proof-gated slices from being mistaken for whole-concept
replay completion.

`MASTER_EXTRACTION_CONTRACT.md` is the final source of truth for handoff and
completion. This file is a scoped summary only.

`validated slice` means the named production slice passed the live
DLL/compact/Rust/.battleplan gate at hourly-or-better cadence with zero skipped
game hours and no partial/source/worker/bridge failures. It does not mean every
replay-relevant field in the gameplay concept is captured.

`concept complete` is reserved for a much higher bar: every replay-relevant
extractable field in the concept is recorded through the production path, or
each remaining field has a concrete blocker or conclusive rejection with proof
evidence. Missed extractable data is backlog.

## Current Scoped Status

| Domain | Validated production slice | Still open for full 1:1 replay |
| --- | --- | --- |
| `weather_live` / 24 | Weather visual drivers: per-region weather including the named sandstorm flag, plus per-province displayed temperature, temperature-change, water, snow, mud measured/active, map recolor, and dynamic LUT drivers. Sandstorm is production-proven in `weather_live_sandstorm_shared_20260701T185714`. | Non-visual meteorological internals, raw enum-name parity for unobserved weather values, close-zoom particle/entity parity, and causal state are not concept-complete unless a lane proves they are replay-relevant and extracts them, or conclusively rejects them as non-extractable/non-replay-relevant. |
| `supply_logistics_attrition_live` / 44 | Province logistics/building-derived rows, supply hubs, naval hubs, railways, infrastructure, division current/max/gain supply, division deficit, and division fuel proxy. `CSupplySystem` province-input rows are shared-stream integrated through the anchored `CGameState+0x3D8` root and structurally decoded in `supply_csupply_directroot_shared_20260701T195744`; read time dropped to `76065us`, but this is not a validated production slice yet because the run did not advance game hours. | Exact per-province supply tooltip / remaining supply amount, attrition flag/reason, hub motorization, convoy/logistics pressure, and speed-five/current-equivalent hourly proof for the anchored `CSupplySystem` rows remain open. |
| `focus_research_decisions_depth_live` / 45 | Focus transitions, completed-focus count changes, current/continuous focus changes, research ETA/speed from live deltas, active timed decision transitions, and counts. | Focus paused state, direct completed-focus token closure in all cases, decision target/cooldown/body fields, and direct event-hook capture remain open or rejected only for specific candidates. |
| `air_variant_archetypes_live` / 46 | Active air variant archetype classification through compact rows, including fail-closed handling for token `20378`. | This is not full air lifecycle. Ace identity/wing assignment, richer base-airframe/role strings where fallbacks remain, variant stat closure, and create/delete/rebase semantics remain open unless covered by another domain. |
| `air_combat_losses_live` / 42 | Air strength delta rows: losses, reinforcements, deleted rows, mission region, old/new count, and equipment amount deltas. | Sortie lifecycle, combat pairing, kill attribution, cause/source of loss, tactical engagement state, ace participation, and rich reinforcement reason remain open. Recent residual probes rejected several helper/source candidates and narrowed ace-management command `0x2FB5`; see the master contract for exact offsets. |
| `market_trade_live` / 43 | International-market `CPurchaseContract` compact rows for validated seller/buyer, contract id/type, equipment variant, and amount/delivery fields. Dynamic D-tag buyer rows are production-proven in `market_trade_live_dtag_shared_20260701T174028`; price/speed/completed/days/delivery-country/convoys have lane-local `MTD1` proof only. | Full trade economy is not complete: resource import/export, lend lease semantic closure, licensing, market access, diplomatic trade context, delivery efficiency/progress, market stockpile/automation, and omitted-candidate closure remain open. |
| `projects_live` / 41 | Facilities, currently worked projects, scientist foreign keys/rows, progress fields, and completed-bit slice where validated. | Project start/completion semantics, reward history/objects, breakthrough state, completion/reward lifecycle events, and exact state-machine semantics remain open or fixture-blocked. |
| `scientists_live` under 41 | Country roster route `CCountry+0xFF0 -> roster+0xA0 -> CCharacter -> CScientist` and scientist skill/profile rows. | This is not full character/cabinet extraction. Leaders, advisors, cabinet roles, portraits, country-leader semantics, advisor availability/hired state, and broader character relationships remain under domain 47. |

## Active Residual Owner Lanes

The orchestrator has assigned the following residual lanes back to production
owner agents. A lane can reduce this backlog only by shipping production
extraction, removing a blocker with exact roots/offsets/functions, or
conclusively rejecting a candidate with proof.

| Residual lane | Domain | Required outcome |
| --- | --- | --- |
| `weather_full_replay_residuals` | 24 | Capture, block, or reject non-visual weather internals and any missing visual drivers. |
| `supply_logistics_attrition_full_residuals` | 44 | Capture, block, or reject exact province supply, attrition reasons, motorization, convoy/logistics pressure, and speed-five/current-equivalent hourly proof for anchored `CSupplySystem` rows. |
| `focus_research_decisions_full_residuals` | 45 | Capture, block, or reject paused focus state, direct completed tokens, decision body/target/cooldown fields, and event hooks. |
| `projects_progress_rewards_full_residuals` | 41 | Capture, block, or reject start/completion/reward/breakthrough state and lifecycle events. |
| `air_combat_full_events_residuals` | 42 | Capture, block, or reject sortie/combat/kill/cause/ace/reinforcement event state. |
| `air_variant_ace_lifecycle_residuals` | 46 | Capture, block, or reject variant stat closure, ace linkage, rich token closure, and wing lifecycle gaps. |
| `trade_economy_full_residuals` | 43 | Capture, block, or reject resource trade, lend lease, licensing, market access, diplomatic context, and omitted candidates beyond purchase contracts. |
| `characters_leaders_cabinet_full_residuals` | 47 | Capture, block, or reject full character, leader, advisor, cabinet, portrait, availability, hired-state, and relationship coverage. |
| `diplomacy_intel_operatives_full_residuals` | 53 | Integrate relation/subject/agency/operative/cryptology/state-event slices and capture, block, or reject operations, spy-network/intel-map, incoming-action queues, and war grouping/name/date closure. |
| `land_combat_detail_events_full_residuals` | 55 | Resolve collector sentinel mismatch, integrate phase/participant event rows, and capture, block, or reject tactics, reserves, width, casualty, equipment-loss, retreat, overrun, terrain, and modifier fields. |
| `naval_missions_status_full_residuals` | 56 | Capture, block, or reject naval mission, home-base, repair/retreat, convoy route/context, and controlled fixture semantics beyond validated task-force/ship roots. |
| `session_multiplayer_metadata_full_residuals` | 57 | Validate live host/session roots and hooks for player-country, hotjoin/tag-switch, speed/pause, chat/system messages, and notifications. |
| `country_economy_counters_full_residuals` | 58 | No accepted lane output yet; capture, block, or reject factory/resource/consumer-goods UI counter closure. |
| `templates_equipment_catalog_full_residuals` | 59 | Provide/locate vanilla game root, rerun strict static catalog proof, integrate domain 59, and join template/equipment catalog rows to live rows. |
