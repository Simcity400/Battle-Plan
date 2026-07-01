# Master Replay Extraction Contract

This is the top-level checklist for deciding whether the recording layer is
actually done for a full 1:1 Battle Plan.

It is intentionally stricter than `DOMAIN_REGISTRY.md` and
`EXTRACTION_SCOPE_STATUS.md`.

- `DOMAIN_REGISTRY.md` assigns compact domain IDs.
- `EXTRACTION_SCOPE_STATUS.md` explains current scoped slice status.
- This file is the master contract: if every row here is closed, the extractor
  should have no known missed replay-relevant data.

## Completion Rule

A gameplay concept is not complete just because one compact domain passes.

For a row in this document to be considered closed, every replay-relevant field
in that row must be one of:

1. Captured through the production path:
   `hoi4.exe -> one injected DLL collector/coordinator -> typed compact records
   -> Rust recorder -> .battleplan`.
2. Rejected with proof:
   exact candidates/offsets/functions were tested and should not be retried
   without new evidence.
3. Blocked with proof:
   an exact fixture/tooling/root wall is documented and no unblocked production
   work remains for that row.

Python, saves, JSON root frames, heap scans, and bridge rows may support
research and validation. They do not close a production row unless the runtime
recording path itself is DLL-only and compact.

## Status Values

| Status | Meaning |
| --- | --- |
| `CONCEPT_COMPLETE` | The whole row is captured, rejected, or blocked with exact proof. This is the only status that means "done" for 1:1 replay coverage. |
| `PRODUCTION_COMPLETE` | The named runtime infrastructure/domain contract is accepted in the shared DLL/Rust path, but parent gameplay concepts may still have other rows open. |
| `PRODUCTION_SLICE_DONE` | A scoped production slice passed the DLL/compact/Rust/.battleplan gate at hourly-or-better speed-five/current-equivalent cadence, with zero skipped hours and no partial/source/worker/bridge failures. |
| `SHARED_INTEGRATION_PENDING` | Lane-local implementation/proof exists, but the shared one-DLL collector and Rust recorder have not accepted it. |
| `LANE_LOCAL_PROVED` | A lane-local mini stack or proof exists, but it is not yet a shared production slice. |
| `FIXTURE_BLOCKED` | A specific live/UI/save fixture is required before the row can be captured, rejected, or semantically named. |
| `ROOT_DISCOVERY_OPEN` | The row matters for replay, but a production-safe root/path/hook is not pinned. |
| `REJECTED_NO_RETRY` | A candidate was conclusively rejected. This status closes only that candidate, not the whole row unless all candidates/fields are covered. |
| `NOT_STARTED_OR_UNAUDITED` | The row has not been fully inventoried against current live memory, static definitions, UI-visible state, and save-visible state. |

## Production Gate Required For Captured Rows

Captured rows must prove all of the following:

- DLL-only capture in the hot recording path.
- Compact binary records, not full JSON frames.
- Rust recorder decodes the records.
- `.battleplan` verifies.
- Hourly-or-better cadence at speed five/current equivalent.
- Zero skipped game hours.
- No source-unavailable, partial-data, worker-panic, bridge-unavailable, or
  equivalent failure flags.
- Fail-closed behavior for invalid layouts.

## Current Production-Proven Domains

These domains currently have production-proven or existing compact coverage, but
they do not automatically make their whole gameplay category complete.

| Domain | Status | Scope |
| --- | --- | --- |
| `fuel_live` / 10 | `PRODUCTION_SLICE_DONE` | Country fuel rows. |
| `production_live` / 11 | `PRODUCTION_SLICE_DONE` | Stockpile, production lines, naval lines, construction queues as currently encoded. |
| `focus_live` / 12 | `PRODUCTION_SLICE_DONE` | Base focus tree/current/completed/progress fields as currently encoded. |
| `tech_live` / 13 | `PRODUCTION_SLICE_DONE` | Base research/completed-tech/active-slot fields as currently encoded. |
| `politics_live` / 14 | `PRODUCTION_SLICE_DONE` | Base politics, active ideas/laws, ruling-party/leader fields as currently encoded. |
| `politics_residuals` / 15 | `PRODUCTION_SLICE_DONE` | Last election, elections allowed, appointed-advisor residual fields where contracts are supplied. |
| `decisions_live` / 16 | `PRODUCTION_SLICE_DONE` | Base decision status and timed-decision rows as currently encoded. |
| `diplomacy_live` / 17 | `PRODUCTION_SLICE_DONE` | Base country diplomacy rows as currently encoded. |
| `divisions_live` / 20 | `PRODUCTION_SLICE_DONE` | Division identity, location, movement, org/strength, combat references, owner, parent army, template id, XP, supply/fuel, deletes. |
| `land_combats_live` / 21 | `PRODUCTION_SLICE_DONE` | Active land-combat lifecycle snapshots and participant vectors. |
| `map_control_live` / 22 | `PRODUCTION_SLICE_DONE` | State ownership/control and province controller/state rows. |
| `map_detail_live` / 23 | `PRODUCTION_SLICE_DONE` | State resources and state/province building levels. |
| `weather_live` / 24 | `PRODUCTION_SLICE_DONE` | Visual weather drivers including sandstorm slice; non-visual internals remain open. |
| `division_equipment_live` / 26 | `PRODUCTION_SLICE_DONE` | Division held equipment entries as currently encoded. |
| `army_orders_live` / 27 | `PRODUCTION_SLICE_DONE` | Active army orders reachable from live divisions, type-5 target-state slices, deletes. |
| `oob_command_live` / 28 | `PRODUCTION_SLICE_DONE` | Active land OOB plus widened naval hierarchy rows where accepted. |
| `ships_live` / 29 | `PRODUCTION_SLICE_DONE` | Ship telemetry, owner attribution, task-force linkage, definition/category, strength/org/XP, deletes. |
| `air_wings_live` / 30 | `PRODUCTION_SLICE_DONE` | Air-wing rows as currently encoded. |
| `air_region_activity_live` / 31 | `PRODUCTION_SLICE_DONE` | Air-region activity rows as currently encoded. |
| `air_wing_loadouts_live` / 32 | `PRODUCTION_SLICE_DONE` | Air-wing loadout rows as currently encoded. |
| `air_variant_defs` / 33 | `PRODUCTION_SLICE_DONE` | Air variant definition rows as currently encoded. |
| `air_base_status_live` / 34 | `PRODUCTION_SLICE_DONE` | Air-base status rows as currently encoded. |
| `air_variant_stats` / 35 | `PRODUCTION_SLICE_DONE` | Air variant stat rows as currently encoded. |
| `air_variant_reserves` / 36 | `PRODUCTION_SLICE_DONE` | Air reserve rows as currently encoded. |
| `stockpile_variant_defs` / 37 | `PRODUCTION_SLICE_DONE` | Stockpile variant definition rows as currently encoded. |
| `air_superiority_inputs` / 38 | `PRODUCTION_SLICE_DONE` | Air superiority input rows as currently encoded. |
| `air_variant_upgrades` / 39 | `PRODUCTION_SLICE_DONE` | Air variant upgrade rows as currently encoded. |
| `stockpile_variant_upgrades` / 40 | `PRODUCTION_SLICE_DONE` | Stockpile variant upgrade rows as currently encoded. |
| `projects_live` / 41 | `PRODUCTION_SLICE_DONE` | Facilities, scientists, progress/completed-bit slice; project lifecycle/rewards still open. |
| `air_combat_losses_live` / 42 | `PRODUCTION_SLICE_DONE` | Air loss/reinforcement delta slice; full air combat remains open. |
| `market_trade_live` / 43 | `PRODUCTION_SLICE_DONE` | Purchase-contract slice plus dynamic D-tag buyer handling; full trade economy remains open. |
| `supply_logistics_attrition_live` / 44 | `PRODUCTION_SLICE_DONE` | Supply/logistics subset; `CSupplySystem` rows are shared integrated through the anchored `CGameState+0x3D8` root, but not production-accepted until an advancing speed-five/current-equivalent hourly proof passes. |
| `focus_research_decisions_depth_live` / 45 | `PRODUCTION_SLICE_DONE` | Transition-depth slice; paused focus, decision body fields, and event hooks remain open/blocked. |
| `air_variant_archetypes_live` / 46 | `PRODUCTION_SLICE_DONE` | Active air variant archetype slice; full air lifecycle/ace linkage remains open. |
| `scripted_state_flags_variables_live` / 51 | `PRODUCTION_SLICE_DONE` | Shared registered country/global/state variables and token/date/value flags; opaque roots remain open. |
| `occupation_resistance_compliance_live` / 52 | `PRODUCTION_SLICE_DONE` | Occupation/resistance/compliance state rows; deeper garrison/collaboration detail remains open. |

## Shared Or Lane-Ready Domains Not Accepted As Production Complete

| Domain | Status | Why it is not closed |
| --- | --- | --- |
| `characters_leaders_cabinet_live` / 47 | `SHARED_INTEGRATION_PENDING` | Lane-ready adapter/root work exists, but shared registration and full speed-five proof remain. Broader leader/advisor/cabinet fields remain open. |
| `construction_buildings_state_damage_live` / 48 | `SHARED_INTEGRATION_PENDING` | Lane-local target-state/building/damage/resource proof exists, but shared registration and production proof remain. |
| `naval_combat_lifecycle_live` / 49 | `FIXTURE_BLOCKED` | No active naval battle fixture in current process. Ship route validated, but lifecycle events need a live combat fixture. |
| `naval_combat_members_live` / 50 | `FIXTURE_BLOCKED` | No active naval battle fixture in current process. Member join/leave/damage/sinking needs live combat fixture. |
| `diplomacy_intel_operatives_live` / 53 | `SHARED_INTEGRATION_PENDING` | Lane-local operative/agency slices exist; shared production integration and operation/intel-map/incoming-action fixtures remain. |
| `army_battleplans_depth_live` / 54 | `SHARED_INTEGRATION_PENDING` | Redraw/toggle observer, scalar candidates, and save/live `order+0x70/+0x78` path-geometry validation exist; shared proof and live fixture closure remain. |

## Master Gameplay Checklist

### 1. Session, Clock, Stream, And Replay Container

| System / field group | Required for 1:1 replay | Current status | Evidence / domain | Remaining closure condition |
| --- | --- | --- | --- | --- |
| Executable/build profile | Build hash, layout profile, checksum/mod identity, accepted offsets. | `PRODUCTION_SLICE_DONE` | Current 870ebf profile proofs. | Mark `CONCEPT_COMPLETE` only after profile migration/rejection procedure is documented for every supported build. |
| Game clock | Date/hour, monotonically ordered game-hour raw, pause/speed context where needed. | `PRODUCTION_SLICE_DONE` | Frame domain / compact perf gate. | Add explicit pause/speed-state row if replay UI requires exact playback timing beyond game-hour cadence. |
| Compact stream framing | Sequence id, game hour, domain id, record type, payload length, ordering. | `PRODUCTION_COMPLETE` | Domain 1000 / compact gate. | Keep complete as long as new domains use the same gate. |
| Rust decode and validation | Decode every domain, reject corrupt rows, expose counters/failures. | `PRODUCTION_COMPLETE` | Recorder host gates. | Every new domain must add decoder/gate coverage before acceptance. |
| `.battleplan` packing | Raw compact stream plus manifest packaged and verified. | `PRODUCTION_COMPLETE` | `pack_replay verify`. | Keep complete as long as new roles/domains verify. |
| Skipped-hour detection | Detect missing frame hours and missing scheduled domain records. | `PRODUCTION_COMPLETE` | Compact replay verification gates. | Every new scheduled domain must be covered by the same gate. |
| Debug JSON | Debug visibility only. | `CONCEPT_COMPLETE` | Architecture decision. | Must never be used as production feasibility proof. |

### 2. Static Definitions And Replay Enrichment

Static definitions are not necessarily live-memory rows, but they are required
for an authentic replay because compact records often carry token ids or handles.

| System / field group | Required for 1:1 replay | Current status | Evidence / domain | Remaining closure condition |
| --- | --- | --- | --- | --- |
| Country/tag definitions | Tags, names, colors, dynamic D-tags, cosmetic tags. | `PRODUCTION_SLICE_DONE` | Country table and dynamic D-tag handling. | Audit dynamic/cosmetic tag lifecycle and exile/civil-war variants. |
| State/province/map definitions | State ids, province ids, terrain, continents, strategic regions, adjacency, supply areas. | `PRODUCTION_SLICE_DONE` | `map_control_live`, `map_detail_live`, game defs. | Full map definition parity audit against the game files and replay renderer. |
| Localisation | Names for countries, focuses, ideas, techs, decisions, equipment, characters, states. | `NOT_STARTED_OR_UNAUDITED` | Game definition parsers exist. | Complete extraction contract for every token emitted by compact domains. |
| GFX/assets/icons | Unit icons, leader portraits, idea icons, equipment icons, focus icons, map assets. | `NOT_STARTED_OR_UNAUDITED` | UI/game asset work exists separately. | Define which assets must be bundled/referenced for replay. |
| Unit templates and equipment definitions | Template catalog, battalions/support companies, equipment archetypes, stats. | `ROOT_DISCOVERY_OPEN` | Division template id and variant domains. | Capture/static-parse complete template/equipment catalog and join to live rows. |
| Technology definitions | Tech names, categories, costs, prerequisites, effects. | `PRODUCTION_SLICE_DONE` | `tech_live`, static tech maps where supplied. | Audit effect/prerequisite parity needed by replay. |
| Focus definitions | Focus ids, tree structure, icons, prerequisites, rewards. | `PRODUCTION_SLICE_DONE` | `focus_live`. | Reward semantics and completed-token closure must join cleanly. |
| Decision definitions | Decision ids, categories, costs, targets, cooldowns, visible/available metadata. | `ROOT_DISCOVERY_OPEN` | `decisions_live`, domain 45 residuals. | Close target/body/cooldown fields and static metadata joins. |
| Idea/law/advisor definitions | Idea groups, law buckets, national spirits, advisor slots/ledgers. | `PRODUCTION_SLICE_DONE` | `politics_live`, `politics_residuals`. | Cabinet/advisor hired-state and character joins must close under domain 47. |
| Character definitions | Character ids, portraits, roles, traits, advisor/general/admiral/scientist metadata. | `ROOT_DISCOVERY_OPEN` | Scientist route and domain 47 lane. | Shared production character/cabinet route plus static portrait/trait joins. |
| Ship/air variant definitions | Hull/airframe categories, modules, upgrades, stats. | `PRODUCTION_SLICE_DONE` | `ships_live`, air variant domains. | Audit name/module/stat closure and token fallback rows. |

### 3. Country Core And Economy

| System / field group | Required for 1:1 replay | Current status | Evidence / domain | Remaining closure condition |
| --- | --- | --- | --- | --- |
| Country identity/rooting | Country object table, tag db id, tag string, dynamic tags. | `PRODUCTION_COMPLETE` | `CGameState+0x310` country table and market D-tag proof. | Complete if civil-war/exile/cosmetic variants are audited. |
| Fuel | Current fuel and scale metadata. | `PRODUCTION_SLICE_DONE` | `fuel_live` / 10. | Audit capacity/gain/use if not already in production/economy rows. |
| Stockpile | Equipment stockpile rows and variant joins. | `PRODUCTION_SLICE_DONE` | `production_live`, stockpile variant domains. | Audit captured/lost/obsolete/equipment-type semantics. |
| Military production | Lines, assigned factories, output/progress where encoded. | `PRODUCTION_SLICE_DONE` | `production_live` / 11. | Close any missing efficiency, resource shortage, conversion, and line-history fields. |
| Naval production | Ship construction lines and related counters. | `PRODUCTION_SLICE_DONE` | `production_live` / 11. | Join to ship/hull definitions and construction completion events. |
| Construction queue | Queue rows as currently encoded. | `PRODUCTION_SLICE_DONE` | `production_live` / 11. | Full construction/state-damage domain 48 must be shared accepted. |
| Factories/industry capacity | Civilian/military/naval factories, damaged/used/free, consumer goods. | `ROOT_DISCOVERY_OPEN` | Production/map detail slices. | Audit all UI-visible economy counters and capture or reject each. |
| Resources | Produced, imported, exported, used, deficit, market/trade effects. | `ROOT_DISCOVERY_OPEN` | `map_detail_live`, `market_trade_live` residuals. | Close resource import/export flows and market stockpile rows. |
| International market contracts | Seller/buyer, contract id/type, equipment variant, amount/delivery fields. | `PRODUCTION_SLICE_DONE` | `market_trade_live` / 43. | Keep as slice; full trade rows below remain open. |
| Market price/speed/completion | Price, speed, completed state, delivery route/progress. | `LANE_LOCAL_PROVED` | Domain 43 lane-local `MTD1`: price/amount plus same-tick `1943.11.9.17` proof for speed `CPurchaseContract+0xC0`, completed `+0x25C`, days `+0x1D0`, delivery-country tag id `+0x5C`, and convoys `+0x118/+0x11C`; efficiency/progress still unaccepted. | Shared DLL registration and speed-five/current-equivalent production proof for validated fields, plus capture/reject delivery efficiency and factory/equipment transfer progress before `CONCEPT_COMPLETE`. |
| Market stockpile/sellable equipment | Sellable equipment and market inventory. | `ROOT_DISCOVERY_OPEN` | `CCountry+0x1230` candidate rejected no-retry by same-tick save/live fixture. | Find an alternate production root, or prove-backed rejection for remaining market-stockpile semantics. |
| Resource trade/import/export | Non-market resource flow, imports, exports, convoys, trade law impact. | `ROOT_DISCOVERY_OPEN` | Market residual docs. | Production root or proof-backed rejection. |
| Lend lease | Directed lend-lease contracts, equipment, convoys, delivery progress. | `LANE_LOCAL_PROVED` | Market lane-local `MTLL` purchase-contract-derived outgoing/incoming mapping proof; delivery/progress/speed/completed and full semantic joins remain open. | Shared production integration for the mapping slice, then capture or reject delivery/progress, speed/completed, convoy, and diplomacy-context semantics. |
| Licensing | Active license production relationships and details. | `ROOT_DISCOVERY_OPEN` | Rejected old vectors; active-license fixture needed. | New root or proof-backed rejection. |

### 4. Politics, Leaders, Cabinet, And Internal Country State

| System / field group | Required for 1:1 replay | Current status | Evidence / domain | Remaining closure condition |
| --- | --- | --- | --- | --- |
| Political power | Current value. | `PRODUCTION_SLICE_DONE` | `politics_live` / country political status. | Audit gain/spend history if required for replay timeline. |
| Ruling party and popularity | Ruling party plus party popularity rows. | `PRODUCTION_SLICE_DONE` | `politics_live` / 14. | Complete if all ideology groups and dynamic party names are joined. |
| Laws | Economy, trade, mobilization, conscription/other law buckets. | `PRODUCTION_SLICE_DONE` | `politics_live` idea classification. | Audit every law group emitted by static definitions. |
| National spirits and ideas | Active ideas, idea groups, national spirits. | `PRODUCTION_SLICE_DONE` | `politics_live`. | Close token fallback/missing static-contract rows. |
| Stability and war support | Current values and changes. | `PRODUCTION_SLICE_DONE` | Existing politics/country rows if encoded. | Verify explicit counters in compact manifest or add rows. |
| Elections | Last election, elections allowed, frequency. | `PRODUCTION_SLICE_DONE` | `politics_residuals` / 15. | Audit election event/lifecycle rows. |
| Country leader | Active leader character rows. | `PRODUCTION_SLICE_DONE` | `politics_live` leader extraction. | Close multi-entry ruling-party leader vector semantics. |
| Advisors/cabinet | Hired advisors, slots, ledgers, availability, traits/effects. | `ROOT_DISCOVERY_OPEN` | `politics_residuals`, domain 47 lane. | Shared domain 47 capture and static joins. |
| Military high command/theorists/designers | Hired state, roles, effects, availability. | `ROOT_DISCOVERY_OPEN` | Domain 47 lane. | Shared production capture or exact blocker. |
| Characters/leaders/officers | Generals, field marshals, admirals, traits, XP, assignment, portraits. | `SHARED_INTEGRATION_PENDING` | `characters_leaders_cabinet_live` / 47. | Shared registration plus full speed-five proof and missing-role closure. |
| Scientist characters | Scientist roster, skill/profile rows. | `PRODUCTION_SLICE_DONE` | `scientists_live` under 41. | Treat as scientist slice only; broader character relationships remain domain 47. |

### 5. Focus, Research, Decisions, And Timed Country Actions

| System / field group | Required for 1:1 replay | Current status | Evidence / domain | Remaining closure condition |
| --- | --- | --- | --- | --- |
| Current focus | Focus id, progress, tree id. | `PRODUCTION_SLICE_DONE` | `focus_live` / 12. | Keep joined to focus static definitions. |
| Completed focuses | Completed count and tokens where encoded. | `PRODUCTION_SLICE_DONE` | `focus_live`, domain 45 exact-completed slice. | Ensure direct completed-token closure for all cases. |
| Continuous focus | Current continuous focus state. | `PRODUCTION_SLICE_DONE` | `focus_live`, domain 45. | Audit mutual exclusivity with normal focus. |
| Focus paused state | Whether focus progress is paused. | `FIXTURE_BLOCKED` | Domain 45 pause probes rejected candidates. | Same-build paused-true oracle or proof-backed rejection. |
| Research slots | Active slot, progress, live progress, base cost, tech handle/name. | `PRODUCTION_SLICE_DONE` | `tech_live` / 13. | Audit effective modifiers and completion events. |
| Completed technologies | Completed tech count/tokens. | `PRODUCTION_SLICE_DONE` | `tech_live`. | Join every token to tech definition. |
| Research ETA/speed | ETA from live deltas and speed fields where encoded. | `PRODUCTION_SLICE_DONE` | `focus_research_decisions_depth_live` / 45. | Close exact engine speed/modifier fields if replay needs them. |
| Active decisions | Decision status, timed decisions, counts. | `PRODUCTION_SLICE_DONE` | `decisions_live` / 16 and domain 45. | Complete only when target/cooldown/body fields are captured or blocked. |
| Decision targets/body/cooldowns | State/country targets, remaining cooldowns, body detail fields. | `ROOT_DISCOVERY_OPEN` | Domain 45 validated `CDecisionStatus+0xE8` / vtable `0x27D5038` / `record+0x20` country-target `tag_db_id` for `1061` exact save/live matched rows, and lane-local compact schema/decoder/gate coverage is proved for those validated target rows; shared production integration is not applied. Numeric state/province targets, cooldown/day/state fields, and `33` unmatched live transition rows remain open. | Ship validated country-target rows through the shared DLL compact stream and Rust recorder, then close or reject numeric targets, cooldowns, body fields, and unmatched transitions with exact proof. |
| Direct focus/research/decision events | Start/finish/cancel/complete decision/focus/research hooks. | `ROOT_DISCOVERY_OPEN` | Domain 45 hook candidates rejected/blocked. | Exact function/hook contract or proof-backed no-go. |

### 6. Special Projects, Scientists, And Facilities

| System / field group | Required for 1:1 replay | Current status | Evidence / domain | Remaining closure condition |
| --- | --- | --- | --- | --- |
| Facilities | Facility rows and currently worked projects. | `PRODUCTION_SLICE_DONE` | `projects_live` / 41. | Audit facility type/slot/location joins. |
| Scientists | Scientist char id, skill/profile rows, country roster route. | `PRODUCTION_SLICE_DONE` | `scientists_live` under 41. | Join to full character/portrait/role data under domain 47. |
| Project progress | Progress/state fields where validated. | `PRODUCTION_SLICE_DONE` | `projects_live` / 41. | Full state-machine semantic naming remains open. |
| Project completed bit | Completed-bit slice where validated. | `PRODUCTION_SLICE_DONE` | Project progress lane. | Completion lifecycle event semantics still open. |
| Project start/completion lifecycle | Started, completed, cancelled, repeatable state, history. | `FIXTURE_BLOCKED` | Project fixture docs. | Controlled before/after project fixture and shared compact rows. |
| Rewards/reward history | Reward object, selected reward, fired reward history. | `FIXTURE_BLOCKED` | Project reward fixture docs. | Before/after reward-option fixture and semantic root. |
| Breakthrough state | Breakthrough compact state and deltas. | `LANE_LOCAL_PROVED` | Project progress lane. | Shared integration and semantic specialization names where possible. |

### 7. Divisions, Army, Orders, OOB, And Land Combat

| System / field group | Required for 1:1 replay | Current status | Evidence / domain | Remaining closure condition |
| --- | --- | --- | --- | --- |
| Division root discovery | Stable anchored DLL-native division discovery. | `PRODUCTION_COMPLETE` | `divisions_live` / 20. | Keep per-build rediscovery/validation docs current. |
| Division state | Id, owner, location, org, strength, XP, supply, fuel, movement, combat ref, deletes. | `PRODUCTION_SLICE_DONE` | `divisions_live` / 20. | Add exact manpower and assignment/detail rows below. |
| Division template id | Template id joined from live division. | `PRODUCTION_SLICE_DONE` | `divisions_live` widened contract. | Full template catalog required separately. |
| Division template catalog | Full template composition, support companies, battalions, equipment requirements. | `ROOT_DISCOVERY_OPEN` | Template id only currently production-proven. | Static/live catalog extraction and joins. |
| Division equipment | Held equipment entries. | `PRODUCTION_SLICE_DONE` | `division_equipment_live` / 26. | Audit exact shortages/reinforcement/upgrade history if replay needs it. |
| Division manpower | Current manpower and losses. | `ROOT_DISCOVERY_OPEN` | Division docs list exact manpower as future work. | Pin field/root and compact rows. |
| Division assignment/order linkage | Army, order, front/battleplan assignment, strategic redeploy state. | `ROOT_DISCOVERY_OPEN` | `army_orders_live`, domain 54. | Full order geometry and assignment rows. |
| Active army orders | Order identity, generation, slot, type, deletes, type-5 target-state slices. | `PRODUCTION_SLICE_DONE` | `army_orders_live` / 27. | Non-type-5 geometry and redraw/toggle fixtures remain. |
| Battleplan geometry | Frontline/offensive line/fallback/garrison geometry, redraws, activation/execution state. | `SHARED_INTEGRATION_PENDING` | `army_battleplans_depth_live` / 54; `army_battleplans_depth_live_save_geometry_870ebf_20260701_v1` validates save/live paired non-type-5 `order+0x70/+0x78` path rows and rejects the old type-3 `order+0xC0/+0xC8` path candidate. | Add domain-54 compact schema/feed rows for validated path geometry, then pass shared DLL/Rust/.battleplan proof; close or exactly reject remaining redraw lifecycle, activation/execution, stance/aggressiveness, and non-path geometry semantics. |
| Army OOB | Active armies, groups, leaders, membership, owner rows. | `PRODUCTION_SLICE_DONE` | `oob_command_live` / 28. | Empty armies, theatres, inactive commands, full manager roots. |
| Land combat lifecycle | Active battle rows, participants, sides, combat state. | `PRODUCTION_SLICE_DONE` | `land_combats_live` / 21. | Audit tactics, reserves, width, casualties, terrain, modifiers if not encoded. |
| Land combat detailed events | Reinforce/retreat/overrun/casualty/equipment loss/phase events. | `ROOT_DISCOVERY_OPEN` | Land combat slice only. | Event/state roots or proof-backed rejection. |

### 8. Navy

| System / field group | Required for 1:1 replay | Current status | Evidence / domain | Remaining closure condition |
| --- | --- | --- | --- | --- |
| Ship rows | Ship id, task force, owner, definition/category, strength/org/XP, sunk convoys, deletes. | `PRODUCTION_SLICE_DONE` | `ships_live` / 29. | Direct ship names, modules, repair state, mission/home base rows below. |
| Fleet/task-force hierarchy | Fleets, task forces, membership, task-force province ids where encoded. | `PRODUCTION_SLICE_DONE` | `oob_command_live`, `ships_live`. | Empty/inactive fleets and full command roots. |
| Ship definitions/modules | Hull, variant, modules, stats, names. | `ROOT_DISCOVERY_OPEN` | Ship definition token/category slice. | Static/live catalog closure and direct ship name extraction. |
| Naval missions | Patrol/strike/convoy/repair/training/hold, region assignment, mission state. | `ROOT_DISCOVERY_OPEN` | Naval residuals. | Production root and compact rows. |
| Home base/repair/retreat | Port, home base, repair queue, retreat state. | `FIXTURE_BLOCKED` | Naval fixture requirements. | Controlled mission/repair/base-change fixture. |
| Naval combat lifecycle | Start/update/end battle events, regions, sides. | `FIXTURE_BLOCKED` | `naval_combat_lifecycle_live` / 49. | Active two-sided naval battle fixture and shared compact proof. |
| Naval combat members | Ship/task-force members, join/leave, damage/sinking linkage. | `FIXTURE_BLOCKED` | `naval_combat_members_live` / 50. | Active naval battle with sinking/member-change fixture. |
| Convoy combat/losses | Convoy involvement, losses, route/mission context. | `ROOT_DISCOVERY_OPEN` | Ships report sunk-convoy count only. | Dedicated compact event/state rows. |

### 9. Air

| System / field group | Required for 1:1 replay | Current status | Evidence / domain | Remaining closure condition |
| --- | --- | --- | --- | --- |
| Air wings | Wing identity, owner, region/base, size, equipment, mission where encoded. | `PRODUCTION_SLICE_DONE` | `air_wings_live` / 30. | Audit create/delete/rebase and mission lifecycle. |
| Air regions/activity | Region activity counters and mission activity. | `PRODUCTION_SLICE_DONE` | `air_region_activity_live` / 31. | Audit every UI-visible activity/loss/sortie counter. |
| Air wing loadouts | Equipment/variant loadouts. | `PRODUCTION_SLICE_DONE` | `air_wing_loadouts_live` / 32. | Join to variant definition/stats. |
| Air bases | Base status/capacity/occupancy where encoded. | `PRODUCTION_SLICE_DONE` | `air_base_status_live` / 34. | Audit damage/repair/overcrowding if not captured. |
| Aircraft variants | Defs, stats, reserves, upgrades, archetypes. | `PRODUCTION_SLICE_DONE` | Domains 33, 35, 36, 39, 46. | Close rich role/base-airframe strings and fallback tokens. |
| Air superiority inputs | Superiority input rows. | `PRODUCTION_SLICE_DONE` | `air_superiority_inputs` / 38. | Audit exact UI superiority output/modifiers. |
| Air combat losses/reinforcements | Loss, reinforcement, delete deltas, mission region, equipment amount deltas. | `PRODUCTION_SLICE_DONE` | `air_combat_losses_live` / 42. | Full air combat rows below remain open. |
| Sortie/combat lifecycle | Sortie start/end, combat pairings, tactical engagement state. | `ROOT_DISCOVERY_OPEN` | Air combat hook candidates rejected/blocked. | Deeper instrumentation/root or proof-backed rejection. |
| Kill/cause attribution | Who killed what, loss reason/source, AA/accident/combat cause. | `ROOT_DISCOVERY_OPEN` | Air combat residuals. | Production rows or exact blocker. |
| Ace identity/assignment | Ace roster, wing assignment, participation, death/wounding. | `SHARED_INTEGRATION_PENDING` | Air/ace lane-local proof. | Shared registration and speed-five proof. |
| Reinforcement reason | Production reinforcement, transfer, attrition replacement, deleted-wing semantics. | `ROOT_DISCOVERY_OPEN` | Loss delta slice only. | Root or event hook. |

### 10. Map, Weather, Construction, Damage, Occupation

| System / field group | Required for 1:1 replay | Current status | Evidence / domain | Remaining closure condition |
| --- | --- | --- | --- | --- |
| State ownership/control | Owner/controller per state. | `PRODUCTION_SLICE_DONE` | `map_control_live` / 22. | Audit civil-war/exile/occupation edge cases. |
| Province controller/state | Controller and state link per province. | `PRODUCTION_SLICE_DONE` | `map_control_live` / 22. | Audit impassable/water/special provinces. |
| Buildings | State/province building levels. | `PRODUCTION_SLICE_DONE` | `map_detail_live` / 23. | Domain 48 covers damage/repair/target state next. |
| Resources/deposits | State resource deposits. | `PRODUCTION_SLICE_DONE` | `map_detail_live` / 23. | Join to trade/economy resource flows. |
| Construction/building damage | Construction target state, building damage/repair/health. | `SHARED_INTEGRATION_PENDING` | `construction_buildings_state_damage_live` / 48 lane proof. | Shared registration and speed-five proof. |
| Resistance/compliance/occupation | Occupation/resistance/compliance rows. | `PRODUCTION_SLICE_DONE` | `occupation_resistance_compliance_live` / 52. | Garrison scalar/collaboration detail remains open. |
| Garrisons | Garrison law/template/manpower/equipment/suppression details. | `ROOT_DISCOVERY_OPEN` | Occupation residuals. | Production root or exact blocker. |
| Weather visual drivers | Region weather, province displayed temp/change/water/snow/mud, recolor/LUT, sandstorm. | `PRODUCTION_SLICE_DONE` | `weather_live` / 24. | Close enum/particle parity and non-visual internals where replay-relevant. |
| Weather causal internals | Internal weather-system cause/effect, unobserved enums, close-zoom particle/entity state. | `ROOT_DISCOVERY_OPEN` | Weather residuals. | Extract, reject as non-replay-relevant, or fixture-block with proof. |
| Terrain/map rendering inputs | Terrain, water, map modes, borders, supply overlays, battle indicators. | `NOT_STARTED_OR_UNAUDITED` | UI/map agent owns presentation separately. | Extraction must expose every state needed by replay renderer. |

### 11. Supply, Logistics, And Attrition

| System / field group | Required for 1:1 replay | Current status | Evidence / domain | Remaining closure condition |
| --- | --- | --- | --- | --- |
| Province logistics/buildings | Rails, hubs, naval hubs, infrastructure-derived rows. | `PRODUCTION_SLICE_DONE` | `supply_logistics_attrition_live` / 44. | Keep joined to map/building domains. |
| Division supply/fuel proxy | Current/max/gain supply, deficit, fuel proxy. | `PRODUCTION_SLICE_DONE` | Domain 44. | Exact engine tooltip supply below remains open. |
| `CSupplySystem` province inputs | Province input array rows. | `FIXTURE_BLOCKED` | Shared integrated via anchored `CGameState+0x3D8`; `supply_csupply_directroot_shared_20260701T195744` decoded 13,396 rows with no failure flags and `read_time_us=76065`, but the game clock did not advance. | Rerun the shared compact gate with the game actually advancing at speed five/current equivalent and require zero skipped hours. |
| Exact province supply tooltip | Remaining supply amount and UI tooltip parity. | `ROOT_DISCOVERY_OPEN` | Supply residuals. | Root/probe that matches UI/save oracle or exact blocker. |
| Hub motorization | Motorization level/settings/effects per hub. | `FIXTURE_BLOCKED` | Domain 44 validated `CCountrySupplySystem+0x78 -> CCountry` and `CCountrySupplySystem+0x20` `0x30`-stride country/node/tag records; `ccountry_node_records_870ebf_20260701_v1` rejected record `+0x04` as exact level (`21` matches, `96` mismatches), and `hub_motorization_level_870ebf_20260701_v1` proved the save oracle is single-level only (`490` pairs, all level `2`; best live candidate `u8@record+0x2D` mismatched `20/117`). | Same-frame hub toggle fixture with at least two known levels, ideally one level `1` and one level `3`, then capture or reject the exact level field/function output through DLL compact rows. |
| Convoy/logistics pressure | Convoy supply, port pressure, foreign/homebase pressure. | `ROOT_DISCOVERY_OPEN` | Domain 44 `foreign_pressure_records_870ebf_20260701_v1` validates a foreign-homebase candidate at `CCountrySupplySystem+0x1E0`: `0x70`-stride records with province `+0x00`, supply `i32/100000@+0x08`, penalty `i32/100000@+0x18`, duplicate province `+0x50`; stale oracle matched `25/25` joined provinces and `20` exact raw supply+penalty pairs. | Same-frame foreign-homebase/convoy pressure fixture, plus roots for convoy/port/disrupted pressure; current save has zero disrupted-supply anchors and tag `D04` does not join live. |
| Attrition reason/flag | Attrition active state, reason/source, affected unit/province. | `ROOT_DISCOVERY_OPEN` | Supply residuals. | Capture exact reason rows or proof-backed blocker. |

### 12. Diplomacy, Wars, Factions, And Intelligence

| System / field group | Required for 1:1 replay | Current status | Evidence / domain | Remaining closure condition |
| --- | --- | --- | --- | --- |
| Base diplomacy state | Base country diplomacy rows as currently encoded. | `PRODUCTION_SLICE_DONE` | `diplomacy_live` / 17. | Audit every UI-visible relation/diplomacy field. |
| Relations/opinions | Opinion, tension, guarantees, access, non-aggression, attaches, faction relations. | `ROOT_DISCOVERY_OPEN` | Base diplomacy only. | Production rows or exact rejections. |
| Factions | Faction membership, leader, name, joins/leaves, faction diplomacy. | `ROOT_DISCOVERY_OPEN` | OOB/diplomacy slices only. | Capture or block with proof. |
| Subjects/autonomy | Puppet/subject relation, autonomy progress, overlord, subject levels. | `ROOT_DISCOVERY_OPEN` | Base diplomacy only. | Capture or block with proof. |
| War lifecycle | War ids/names, attacker/defender groups, start/end, participants, casualties. | `ROOT_DISCOVERY_OPEN` | Domain 53 lane has partial validated war/casualty row evidence only. | Shared compact rows plus war grouping/name/date closure. |
| Capitulation/peace/conferences | Capitulation state, surrender progress, peace conference state, treaty results. | `NOT_STARTED_OR_UNAUDITED` | Not accepted as production slice. | Full inventory and roots. |
| Intel agency | Agency scalars, upgrades, cryptology, capacity, recruitment. | `SHARED_INTEGRATION_PENDING` | Domain 53 lane-local agency/operative slices. | Shared integration and semantic fixture closure. |
| Operatives | Operative identity, owner, skill/XP, detected state, lifecycle flags. | `SHARED_INTEGRATION_PENDING` | Domain 53 lane-local proof. | Shared compact proof plus mission/captured/pool/nationality/portrait closure. |
| Operations | Running operation, target, stage/progress, participants, outcome. | `FIXTURE_BLOCKED` | Domain 53 fixture requirements. | Same-tick operation fixture and shared rows. |
| Spy networks/intel map | Network strength, coverage, intel percentages, target states/countries. | `FIXTURE_BLOCKED` | Domain 53 fixture requirements. | Same-tick intel-map fixture and shared rows. |
| Diplomacy action queues | Incoming/outgoing diplomatic actions and timed requests. | `FIXTURE_BLOCKED` | Domain 53 fixture requirements. | Live fixture and compact event rows. |

### 13. Scripted State, Events, Variables, And Runtime-Only State

| System / field group | Required for 1:1 replay | Current status | Evidence / domain | Remaining closure condition |
| --- | --- | --- | --- | --- |
| Country/global/state flags | Token/date/value flags where currently encoded. | `PRODUCTION_SLICE_DONE` | `scripted_state_flags_variables_live` / 51. | Opaque roots below remain open. |
| Variables | Country/global/state variables where currently encoded. | `PRODUCTION_SLICE_DONE` | Domain 51. | Audit all scopes and dynamic variable types. |
| Scripted GUI open/state | Scripted GUI runtime state, open panels, selected scopes if replay-relevant. | `FIXTURE_BLOCKED` | Scripted-state residual fixture contracts. | Controlled fixture or exact rejection as non-replay-relevant. |
| Delayed events/event queues | Scheduled delayed events, pending country/state/global events. | `FIXTURE_BLOCKED` | Scripted-state residual fixture contracts. | Controlled delayed-event fixture and compact rows. |
| On-action/effect runtime state | Runtime-only transition events that alter replay-visible state. | `ROOT_DISCOVERY_OPEN` | No broad accepted event hook domain. | Hook/root audit or proof-backed rejection. |
| Mission/decision scripted state | Runtime state behind missions/decisions not visible in base rows. | `ROOT_DISCOVERY_OPEN` | Decision residuals. | Capture or reject field by field. |
| Mod-added dynamic state | Any mod/scripted variable/flag that changes replay-visible behavior. | `NOT_STARTED_OR_UNAUDITED` | Private current vanilla focus. | Build mod-aware inventory and generic compact encoding rules. |

### 14. Player, Multiplayer, And Session Metadata

| System / field group | Required for 1:1 replay | Current status | Evidence / domain | Remaining closure condition |
| --- | --- | --- | --- | --- |
| Player-country mapping | Which human controlled which country at each time. | `NOT_STARTED_OR_UNAUDITED` | Not in compact registry. | Capture from host/session metadata, logs, or live memory if required. |
| Hotjoin/tag switches | Player join/leave/tag-switch timeline. | `NOT_STARTED_OR_UNAUDITED` | Not in compact registry. | Capture or mark out of scope with proof. |
| Game speed/pause commands | Speed changes and pauses for playback timing. | `ROOT_DISCOVERY_OPEN` | Clock capture exists. | Capture if wall-clock authentic playback is required. |
| Chat/messages | Multiplayer chat, system messages, notifications. | `NOT_STARTED_OR_UNAUDITED` | Not accepted as production slice. | Decide replay requirement; capture or reject as non-required. |
| Alerts/notifications | UI alerts, important message log, popups. | `NOT_STARTED_OR_UNAUDITED` | Not accepted as production slice. | Decide if replay should reproduce UI notifications or only state. |

## Closure Sweep Required Before Calling The Extractor Done

Before the recording layer can be called complete, run these sweeps and update
every affected row above:

1. Save-visible parity sweep:
   compare every save-visible field relevant to replay against either a compact
   production row or a proof-backed blocker/rejection.
2. UI-visible parity sweep:
   walk every major the game ledger, map mode, tooltip, combat panel, production
   screen, diplomacy screen, intel screen, navy/air/army screen, and project UI;
   every displayed replay-relevant datum must map to a row here.
3. Runtime-event sweep:
   identify state transitions that are not visible from hourly snapshots alone
   and require sub-hour event rows.
4. Static-definition sweep:
   every token/id emitted by compact records must have a deterministic
   localisation/definition/asset join or an explicit omitted-field reason.
5. Fixture backlog sweep:
   every `FIXTURE_BLOCKED` row must name the exact fixture needed and the
   command/gate that will accept or reject it.
6. Rejection ledger sweep:
   every `REJECTED_NO_RETRY` candidate must be listed with offset/function/root
   evidence so it is not rediscovered by another lane.
7. Combined speed-five proof:
   schedule every production domain that is enabled for the current build,
   require hourly-or-better coverage, zero skipped hours, no failure flags, Rust
   decode, and `.battleplan` verification.

## Done Means

The extraction app is done only when:

- every row in `Master Gameplay Checklist` is `CONCEPT_COMPLETE`, or
- the row is explicitly out of scope with proof and that out-of-scope decision
  is accepted in this document, and
- the final combined speed-five proof passes with all enabled production domains,
  zero skipped game hours, no source/partial/worker/bridge failures, Rust decode,
  and `.battleplan` verification.
