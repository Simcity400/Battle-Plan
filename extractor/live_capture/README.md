# live_capture

Python development/proof harness for the native Battle Plan app.

`extractor/live_capture/` currently turns read-only memory-reader results into
schema-conformant snapshots and portable `.battleplan` recordings. Under the
in-process collector plan, it is a validation harness and temporary adapter for
collector frames. Production live extraction must be owned by the DLL/Rust path:
`hoi4.exe` -> DLL collector -> compact raw/event/snapshot stream ->
`.battleplan`. Python must keep measured zero, captured null, and absent data
distinct when validating that output, but it is not the production hot loop.

Runtime capture has one authoritative source: the live host `hoi4.exe` process.
Saves are optional development fixtures and validation artifacts only; they are
not required to record or replay a live session.

## Role

This package owns development capture assembly, durable snapshot writes,
continuity metadata, recorder UI/CLI experiments, collector-frame ingestion,
proof gates, and `.battleplan` validation/writer integration. It may attach the
DLL, run proof gates, validate manifests/replay files, and perform temporary
bounded preflight discovery. It must not walk memory hourly/sub-hourly, own the
production hot extraction loop, or be required to keep up with speed five. See
`../documents/MASTER_EXTRACTION_CONTRACT.md` for the current extractor backlog,
`../documents/archive/plans/IN_PROCESS_COLLECTOR_PLAN.md` for historical
collector planning context, and
`../../replay_interface/documents/HOI4REPLAY_CONTAINER_SPEC.md` for the portable recording format.

## Main Pieces

| Module | Role |
| --- | --- |
| `capture.py` | Current capture loop, live process attachment hooks, single-shot and interval capture. |
| `collector_attach.py` | External no-op attach helper for loading the collector DLL into a supported `hoi4.exe` and receiving one pipe frame. |
| `collector_country_bridge.py` | Save-free country-object/country-record preflight bridge with same-process cache; feeds compact control contracts while keeping heap scans out of recorder runs. |
| `collector_frames.py` | Synthetic/decoded in-process collector frame validation and snapshot adapter. |
| `collector_map_bridge.py` | State/province map-control preflight helpers and caches; state roots are fast when cached, province array relocation remains the open blocker. |
| `collector_profile_check.py` | Profile preflight: running-exe hash match, root-probe sentinel/address/domain validation, and transport diagnostics. |
| `collector_probe_record.py` | Low-level live collector probe harness used for bounded cadence proofs and transport debugging. |
| `collector_session.py` | Development/proof session wrapper that writes lifecycle frames, durable snapshots, `.battleplan` output, and cadence/transport diagnostics without requiring saves. Not production completion evidence by itself. |
| `collector_profiles/` | Build-gated collector profiles: executable hash, offsets, sentinels, roots, and enabled domains. |
| `registry.py` | Import-guarded reader plug registry. Failed/missing plugs omit domains instead of fabricating data. |
| `assembler.py` | Folds reader results into replay snapshots while preserving no-fabrication semantics. |
| `continuity.py` | Campaign/segment/capture sequence tagging and rollback/time-jump handling. |
| `durable_store.py` | Atomic snapshot/index writes and single-writer locking. |
| `recorder/` | GUI/CLI recorder experiments, cadence scheduling, manifest diagnostics, collector transport adapter, packaging spec. |

## Current Recording Reality

- The live host `hoi4.exe` collector is the only runtime-authoritative source.
  Save oracle output is validation/development data only.
- Python is not production live extraction. A segment is complete only after the
  DLL/Rust path captures it hourly-or-better, with no skipped game hours, at
  speed five or a measured equivalent.
- Canonical replay snapshots are hourly-or-better. Daily rows can be rollups,
  not replay coverage.
- `collector_session` currently wraps repeated bounded DLL workers as one
  logical development session. That bridge writes lifecycle frames, durable
  snapshots, manifests, and `.battleplan` output for proof/validation, but the
  target remains a Rust/native long-running collector recorder.
- The session writes a validated `collector_frames.jsonl` sidecar containing
  every accepted DLL frame and embeds the same byte stream into finalized
  `.battleplan` files at `payload/<game>/collector_raw/collector_frames.jsonl`.
  The replay store still keeps one canonical snapshot per unique game hour, so
  the raw stream is where duplicate 10 ms samples survive for movement,
  land-combat lifecycle events/deltas, and future event extraction.
- Country identity/scalars are now DLL-native for object-backed fields. The DLL
  reads the `CGameState+0x310` tag-db-id-indexed `CCountry*` table, validates
  each candidate by heap range, country vtable, object-owned `tag_db_id`,
  tag-table decode, stable name, and duplicate checks, then emits
  `country_scalars` without a country preflight bridge. Political power is also
  emitted on this path by validating `CCountry+0xF90 -> CPoliticalStatus` and
  reading `CPoliticalStatus+0xE0`.
- The preflight cache/control-contract path remains for fallback/bootstrap and
  for any future record-only fields: `tag_db_id:country_object_ptr` plus
  `tag_db_id:country_record_ptr`, validated by the DLL per frame. Record rows
  are additive and must not reduce object-row coverage. `country_object_scan`
  must stay false for normal and speed-five proof runs.
- The collector-frame adapter can fold `state_ownership`, `state_control`, and
  `province_control` rows when a collector frame explicitly claims those
  domains. It also folds scheduled `politics_live` rows from the DLL into
  `countries.*.politics` for ruling party, party popularities, active idea
  tokens, typed `laws`, complete `idea_groups`, active cabinet/designer
  `advisor_ideas`, broad non-law `ideas_other`, narrower `national_spirits`,
  idea count, election frequency, party count, low-confidence diagnostics, and
  verified active-ruler rows folded into `countries.*.commanders.leader`,
  and scheduled `politics_residuals` rows for
  `last_election` and `elections_allowed`. `appointed_advisors`,
  `appointed_advisor_details`, `appointed_advisor_slots`, and
  `appointed_advisor_ledgers` are folded when the session supplies a validated
  static advisor contract built from `game_defs` / `common/characters`; the
  details include `{idea_token, slot, ledger, character}` without sweeping
  `CAdvisor` heap objects in the hot path. Typed
  law / idea-group / advisor-like / non-law / national-spirit classification is
  folded when the session supplies the static `common/ideas` group contract.
  The DLL intersects those contracts with live active ideas. Missing or invalid
  contracts omit the relevant leaves, while measured-empty `[]` is real.
  Current strict classification proof
  `replay_interface/replay/snapshots/_collector_session_politics_idea_gate/20260629T211245Z_pid43412`
  passed `--require-politics-idea-classification`: it advanced 12 game hours in
  45.797 wall seconds, verified replay output, supplied 6474 vanilla
  `common/ideas` rows, emitted 12 scheduled `politics_live` frames, folded 4248
  active-idea rows, 4248 law rows, 4248 idea-group rows, 1104 advisor-idea rows,
  4248 non-law idea rows, and 4248 national-spirit rows, and kept worker-poll
  frames free of politics payloads. Active-ruler proof remains separate:
  `replay_interface/replay/snapshots/_collector_session_politics_leader_gate/20260629T171216Z_pid43412`
  folded 4095 `commanders.leader` rows into the replay store. Countries whose
  ruling-party leader vector has multiple entries are deliberately omitted until
  the active element can be disambiguated without guessing.
  Current advisor-residual proof
  `replay_interface/replay/snapshots/_collector_session_politics_advisor_gate/20260629T212028Z_pid43412`
  passed `--require-politics-advisor-residuals`: it advanced 12 game hours in
  46.047 wall seconds, verified replay output, supplied 3722 vanilla
  `common/characters` advisor tokens, emitted 13 scheduled `politics_residuals`
  frames, and observed 4602 country rows with `appointed_advisors`,
  `appointed_advisor_details`, `appointed_advisor_slots`, and
  `appointed_advisor_ledgers` fields present. The current run measured those
  appointment lists as empty, so a non-empty hired-advisor fixture is still
  required before claiming populated appointed-advisor values.
  It also folds scheduled `decisions_live` rows into
  `countries.*.decisions.status_present` and `.timed_live`; measured-empty
  `timed_live = []` is protected only for tags actually measured by the
  collector.
  It also folds scheduled `diplomacy_live` rows. `CCountry+0x598` supplies the
  opaque per-country diplomatic faction group; the adapter folds
  `countries.*.diplomacy.faction_group` and reconstructs
  `diplomacy_live.faction_members` with `source = "C"`. War relations now use a
  one-time preflight bridge from the existing live `war_relation` heap
  discovery; the hot DLL path only revalidates pinned rows and rereads
  casualties on scheduled frames. The bridge accepts both live date-vtable
  variants seen in the war pool (`CGameDate` at RVA `0x26F2398` and
  `CGregorianDate` at RVA `0x26F21A0`) and can fall back to the live tag table
  when the old country-object identity bridge is unavailable. Current faction
  no-cold proof
  `replay_interface/replay/snapshots/_collector_session_diplomacy_gate/20260629T152734Z_pid43412`
  advanced 12 game hours in 48.328 wall seconds, stored 13 verified snapshots,
  observed `diplomacy_live` on 14 scheduled frames, captured 4984 diplomacy
  country rows with up to 356 countries and 9 faction groups per frame, and
  kept diplomacy payloads off worker-poll frames. War/casualty proof
  `replay_interface/replay/snapshots/_collector_session_diplomacy_war_gate/20260629T163650Z_pid43412`
  advanced 12 game hours in 43.203 wall seconds, stored 13 verified snapshots,
  observed 14 scheduled diplomacy war frames, captured up to 1033
  `war_relations` per frame, folded 1022 country `at_war` rows and 1022
  country casualty rows, observed a max per-frame casualty sum of 56475731,
  and kept diplomacy payloads off worker-poll frames. Current-game rerun
  `replay_interface/replay/snapshots/_collector_session_diplomacy_war_gate/20260629T202751Z_pid43412`
  passed the same strict gate after advancing 12 game hours in 43.984 wall
  seconds, resolving 689 bridge rows in preflight, observing 13 scheduled war
  frames, folding 884 country `at_war` rows and 884 casualty rows, and reaching
  max per-frame casualty sum 14124532.
  It also folds scheduled `focus_live` rows into `countries.*.focus` for
  `focus_tree`, `completed`, `completed_count`, `current`,
  `current_continuous`, and `progress`; measured-empty `completed = []` is
  protected only for tags actually measured by `focus_live`, and `paused`
  remains save-only until a live byte is pinned. Live proof
  `collector_live_focus_probe` advanced 2 game hours with zero skipped hours,
  emitted 3 scheduled `focus_live` frames, emitted 1065 country focus rows, and
  kept worker-poll focus payloads at zero.
  It also folds scheduled `tech_live` rows into
  `countries.*.research.tech_count`, `.research_slots`, and
  `.in_progress_live` from the DLL `CCountry+0xF60 -> CTechnologyStatus`
  slice. The DLL reads the bounded `CTechnologyStatus+0xA0` `CResearchSlot*`
  vector and emits raw slot rows plus `progress_units = progress_raw / 1000`.
  It also follows each active slot's `CResearchSlot+0x18` active-tech pointer
  and emits moving `live_progress_raw/live_progress_units` from
  `CTechnology+0x198`. It reads the base denominator from the same active tech
  via `CTechnology+0x160 -> CTechnologyTemplate+0x3e0`, scaled through the
  engine's `TECH_BASE_COST` helper, and emits
  `base_required_points_raw/base_required_points_units`. This is base progress
  denominator data, not an effective-cost modifier or research-speed claim.
  When `--preflight-tech-defs` or an explicit `tech_def_filter` supplies a
  validated live registry map, those rows may also carry `tech_name`. Current
  no-cold proof
  `replay_interface/replay/snapshots/_collector_session_research_denominator_gate/20260629T192125Z_pid43412`
  used the tech-definition cache, advanced 36 game hours in 48.125 wall
  seconds, emitted 37 scheduled `tech_live` frames, captured 15687 named active
  research-slot rows, captured 15687 live base-denominator rows, verified
  replay output, and kept worker polls free of research payloads. The manifest
  recorded 15263 persisted live-progress rows, 15263 DLL-base progress rows,
  and 346 ETA rows after the daily progress movement. A current-game rerun
  `replay_interface/replay/snapshots/_collector_session_research_denominator_gate/20260629T204418Z_pid43412`
  used a no-cold tech-definition cache hit in 0.109s, advanced 36 game hours in
  52.344 wall seconds, emitted 38 scheduled `tech_live` frames, captured 16562
  named active research-slot rows and 16126 DLL-base progress rows, produced 708
  ETA rows after two positive progress-delta frames, verified replay output, and
  kept worker polls free of research payloads. `progress_units` remains
  source-badged to `CResearchSlot+0x38` and is not accepted as moving live
  progress; effective modifier aggregation remains unclaimed.
  Worker polls must not carry research payloads.
  It can fold scheduled `projects_live` frames into top-level
  `special_projects` with `source = "C"`. The normal session path can now
  preflight/cache special-project roots into a `special_projects_bridge`
  control payload and keeps the old `special_projects_scan` heap sweep disabled
  unless explicitly requested. The bridge carries facilities, all CProject
  identities, and scientist characters to the DLL; manifests keep only counts,
  cache status, and `address_manifest_policy = "omitted"`. Project
  progress/completion, owner attribution, and resolved project definition
  tokens remain open. Worker polls must not carry `special_projects`, and
  pointer/tag-db diagnostics are rejected before folding.
  Session/probe manifests summarize
  country-object identity status, scalar-frame counts, focus-domain sample
  counts, tech-domain sample counts, project-domain sample counts, politics-
  domain sample counts, residual politics-domain sample counts, and raw-frame
  country/state/province row counts.
- Normal map-control recording no longer depends on the old state/province
  preflight caches: the DLL can read the static `CProvince*` array from
  `CGameState+0x2B0`, derive state owner/controller rows through
  `CProvince+0xC0`, and publish province controller TAGs or raw tag-db ids
  through `CProvince+0x188`. The old bridge caches remain useful as
  corroboration/fallback. Current no-cold proof on pid 43412 captured 1081
  state rows and 10272 province rows in each of 12 scheduled map-control frames
  with zero skipped hours and no map preflight bridges supplied.
- Scheduled `buildings_live` is now DLL-native on the same static map spine.
  The DLL reads `STATE+0x158` and `CProvince+0x1C8` `vector<CBuilding*>`
  payloads, validates each `CBuilding` by vtable, type id, and sane level, and
  emits measured building maps on scheduled hourly frames only. The adapter
  folds those rows into `states.*.buildings` and `provinces.*.buildings` while
  preserving measured-empty `{}` maps. Current no-cold proof
  `replay_interface/replay/snapshots/_collector_session_buildings_gate/20260629T150353Z_pid43412`
  advanced 12 game hours, stored 13 verified snapshots, observed
  `buildings_live` on all 13 scheduled frames, captured 14053 state building
  rows and 133536 province building rows, observed non-empty building maps in
  both state and province rows, and kept building payloads off worker-poll
  frames.
- Division movement uses a cache-first `division_bodies.json` bridge. Normal
  recording validates cached body pins without scanning; use
  `--allow-division-cold-scan` only as an explicit bootstrap step because cold
  body discovery can take tens of seconds.
- Army order capture uses a cache-first `army_bodies.json` bridge. Normal
  recording salvages still-live cached army bodies row-by-row; use
  `--allow-army-cold-scan` only as an explicit bootstrap step because the army
  vtable sweep is outside the hot-loop budget.

## Proof Harness Smoke / Tests

These commands are development and validation commands. Passing them can prove a
DLL stream shape or catch regressions, but it does not by itself make a segment
production-complete. Production completion requires the DLL/Rust recorder path.

```powershell
python extractor/live_capture/recorder/recorder_main.py --smoke
python -m live_capture.collector_attach --pid <hoi4-pid> --dll extractor/native/target/debug/collector.dll --pipe
python -m live_capture.collector_profile_check --pid <hoi4-pid> --profile extractor/live_capture/collector_profiles/vanilla_latest_2026_06_28.json --dll extractor/native/target/debug/collector.dll
python -m live_capture.collector_session --pid <hoi4-pid> --dll extractor/native/target/debug/collector.dll --out-root replay_interface/replay/snapshots/_collector_session --game-id collector_live_speed5_session --duration-s 8 --sample-ms 10 --dwell-s 0.25 --worker-bursts 3 --burst-interval-s 0.25 --until-game-hours 12 --max-wall-s 30
python -m live_capture.collector_session --pid <hoi4-pid> --dll extractor/native/target/debug/collector.dll --out-root replay_interface/replay/snapshots/_collector_session_economy_gate --game-id production_fuel_misc_live --duration-s 180 --sample-ms 10 --dwell-s 0.25 --worker-bursts 3 --burst-interval-s 0.25 --until-game-hours 12 --max-wall-s 300
python -m live_capture.collector_proof_gate replay_interface/replay/snapshots/_collector_session_economy_gate/<run-dir>/session_manifest.json --min-game-hours 12 --require-scheduled-domain production_live --require-domain fuel_live --require-domain country_misc --require-domain country_scalars
python -m live_capture.collector_session --pid <hoi4-pid> --dll extractor/native/target/debug/collector.dll --out-root replay_interface/replay/snapshots/_collector_session_construction_gate --game-id construction_queue_live --duration-s 180 --sample-ms 10 --until-game-hours 12 --max-wall-s 300
python -m live_capture.collector_proof_gate replay_interface/replay/snapshots/_collector_session_construction_gate/<run-dir>/session_manifest.json --min-game-hours 12 --require-scheduled-domain production_live --require-production-construction
python -m live_capture.collector_session --pid <hoi4-pid> --dll extractor/native/target/debug/collector.dll --out-root replay_interface/replay/snapshots/_collector_session_research_denominator_gate --game-id research_denominator_cache_bootstrap --duration-s 8 --sample-ms 10 --until-game-hours 1 --max-wall-s 240 --preflight-tech-defs --allow-tech-def-cold-scan
python -m live_capture.collector_session --pid <hoi4-pid> --dll extractor/native/target/debug/collector.dll --out-root replay_interface/replay/snapshots/_collector_session_research_denominator_gate --game-id research_denominator_dll_base_live --duration-s 240 --sample-ms 10 --until-game-hours 36 --max-wall-s 300 --preflight-tech-defs
python -m live_capture.collector_proof_gate replay_interface/replay/snapshots/_collector_session_research_denominator_gate/<run-dir>/session_manifest.json --min-game-hours 36 --require-scheduled-domain tech_live --require-tech-live-names --require-tech-live-progress --require-tech-live-dll-base-progress --require-tech-live-eta
python -m live_capture.collector_session --pid <hoi4-pid> --dll extractor/native/target/debug/collector.dll --out-root replay_interface/replay/snapshots/_collector_session_buildings_gate --game-id buildings_live --duration-s 180 --sample-ms 10 --until-game-hours 12 --max-wall-s 300
python -m live_capture.collector_proof_gate replay_interface/replay/snapshots/_collector_session_buildings_gate/<run-dir>/session_manifest.json --min-game-hours 12 --require-scheduled-domain buildings_live
python -m live_capture.collector_session --pid <hoi4-pid> --dll extractor/native/target/debug/collector.dll --out-root replay_interface/replay/snapshots/_collector_session_state_resources_gate --game-id state_resources_unknown_ids_live --duration-s 180 --sample-ms 10 --until-game-hours 12 --max-wall-s 300
python -m live_capture.collector_proof_gate replay_interface/replay/snapshots/_collector_session_state_resources_gate/<run-dir>/session_manifest.json --min-game-hours 12 --require-scheduled-domain state_detail --require-state-resources
python -m live_capture.collector_session --pid <hoi4-pid> --dll extractor/native/target/debug/collector.dll --out-root replay_interface/replay/snapshots/_collector_session_diplomacy_gate --game-id diplomacy_live --duration-s 180 --sample-ms 10 --until-game-hours 12 --max-wall-s 300
python -m live_capture.collector_proof_gate replay_interface/replay/snapshots/_collector_session_diplomacy_gate/<run-dir>/session_manifest.json --min-game-hours 12 --require-scheduled-domain diplomacy_live
python -m live_capture.collector_session --pid <hoi4-pid> --dll extractor/native/target/debug/collector.dll --out-root replay_interface/replay/snapshots/_collector_session_diplomacy_war_gate --game-id diplomacy_war_relations_live --duration-s 180 --sample-ms 10 --until-game-hours 12 --max-wall-s 300 --preflight-war-relations
python -m live_capture.collector_proof_gate replay_interface/replay/snapshots/_collector_session_diplomacy_war_gate/<run-dir>/session_manifest.json --min-game-hours 12 --require-scheduled-domain diplomacy_live --require-diplomacy-war-relations
python -m live_capture.collector_session --pid <hoi4-pid> --dll extractor/native/target/debug/collector.dll --out-root replay_interface/replay/snapshots/_collector_session_politics_idea_gate --game-id politics_idea_classification_live --duration-s 180 --sample-ms 10 --until-game-hours 12 --max-wall-s 300
python -m live_capture.collector_proof_gate replay_interface/replay/snapshots/_collector_session_politics_idea_gate/<run-dir>/session_manifest.json --min-game-hours 12 --require-scheduled-domain politics_live --require-politics-idea-classification
python -m live_capture.collector_session --pid <hoi4-pid> --dll extractor/native/target/debug/collector.dll --out-root replay_interface/replay/snapshots/_collector_session_politics_advisor_gate --game-id politics_advisor_residuals_live --duration-s 180 --sample-ms 10 --until-game-hours 12 --max-wall-s 300
python -m live_capture.collector_proof_gate replay_interface/replay/snapshots/_collector_session_politics_advisor_gate/<run-dir>/session_manifest.json --min-game-hours 12 --require-scheduled-domain politics_residuals --require-politics-advisor-residuals
python -m live_capture.collector_session --pid <hoi4-pid> --dll extractor/native/target/debug/collector.dll --out-root replay_interface/replay/snapshots/_collector_session_project_roots_bootstrap --game-id collector_live_focus_decision_project_roots --duration-s 180 --sample-ms 10 --dwell-s 0.25 --worker-bursts 3 --burst-interval-s 0.25 --until-game-hours 12 --max-wall-s 300 --preflight-special-projects
python -m live_capture.collector_proof_gate replay_interface/replay/snapshots/_collector_session_project_roots_bootstrap/<run-dir>/session_manifest.json --min-game-hours 12 --require-scheduled-domain focus_live --require-scheduled-domain decisions_live --require-scheduled-domain projects_live --require-projects-live-payload
python -m live_capture.collector_proof_gate replay_interface/replay/snapshots/_collector_session/<run-dir>/session_manifest.json --min-game-hours 12 --require-raw-domain land_combats --require-raw-event-type land_combat_started --require-raw-event-type land_combat_updated --require-raw-event-type land_combat_ended
python -m live_capture.collector_session --pid <hoi4-pid> --dll extractor/native/target/debug/collector.dll --out-root replay_interface/replay/snapshots/_collector_session --game-id collector_live_division_cache_bootstrap --duration-s 5 --sample-ms 20 --dwell-s 0.25 --preflight-division-movement --allow-division-cold-scan
python -m live_capture.collector_session --pid <hoi4-pid> --dll extractor/native/target/debug/collector.dll --out-root replay_interface/replay/snapshots/_collector_session_order_org_gate --game-id order_org_strength_cache_bootstrap --duration-s 8 --sample-ms 10 --until-game-hours 1 --max-wall-s 240 --preflight-division-movement --allow-division-cold-scan --preflight-army-orders --allow-army-cold-scan
python -m live_capture.collector_session --pid <hoi4-pid> --dll extractor/native/target/debug/collector.dll --out-root replay_interface/replay/snapshots/_collector_session_order_org_gate --game-id order_org_strength_live --duration-s 180 --sample-ms 10 --until-game-hours 12 --max-wall-s 300 --preflight-division-movement --preflight-army-orders
python -m live_capture.collector_proof_gate replay_interface/replay/snapshots/_collector_session_order_org_gate/<run-dir>/session_manifest.json --min-game-hours 12 --require-raw-domain divisions --require-raw-domain army_orders --require-raw-event-type division_org_strength_delta
python -m live_capture.collector_session --pid <hoi4-pid> --dll extractor/native/target/debug/collector.dll --out-root replay_interface/replay/snapshots/_collector_session_air_ship_oob_gate --game-id ship_oob_cache_bootstrap --duration-s 8 --sample-ms 10 --until-game-hours 1 --max-wall-s 360 --preflight-ships --allow-ship-cold-scan --preflight-oob-command --allow-oob-command-cold-scan
python -m live_capture.collector_session --pid <hoi4-pid> --dll extractor/native/target/debug/collector.dll --out-root replay_interface/replay/snapshots/_collector_session_air_ship_oob_gate --game-id ship_oob_live --duration-s 300 --sample-ms 10 --until-game-hours 12 --max-wall-s 420 --preflight-ships --preflight-oob-command
python -m live_capture.collector_proof_gate replay_interface/replay/snapshots/_collector_session_air_ship_oob_gate/<run-dir>/session_manifest.json --min-game-hours 12 --require-raw-domain ships --require-ship-raw-telemetry --require-raw-domain oob_command --require-oob-command-raw-telemetry
python -m live_capture.collector_session --pid <hoi4-pid> --dll extractor/native/target/debug/collector.dll --out-root replay_interface/replay/snapshots/_collector_session_oob_leader_gate --game-id oob_leader_cache_bootstrap --duration-s 8 --sample-ms 10 --until-game-hours 1 --max-wall-s 360 --preflight-oob-command --allow-oob-command-cold-scan
python -m live_capture.collector_session --pid <hoi4-pid> --dll extractor/native/target/debug/collector.dll --out-root replay_interface/replay/snapshots/_collector_session_oob_leader_gate --game-id oob_leader_detail_warm_live --duration-s 180 --sample-ms 10 --until-game-hours 12 --max-wall-s 300 --preflight-oob-command
python -m live_capture.collector_proof_gate replay_interface/replay/snapshots/_collector_session_oob_leader_gate/<run-dir>/session_manifest.json --min-game-hours 12 --require-raw-domain oob_command --require-oob-command-raw-telemetry --require-oob-command-leader-detail
python -m live_capture.collector_session --pid <hoi4-pid> --dll extractor/native/target/debug/collector.dll --out-root replay_interface/replay/snapshots/_collector_session_naval_combat_gate --game-id naval_combat_live --duration-s 180 --sample-ms 10 --until-game-hours 12 --max-wall-s 300 --preflight-naval-combat
python -m live_capture.collector_proof_gate replay_interface/replay/snapshots/_collector_session_naval_combat_gate/<run-dir>/session_manifest.json --min-game-hours 12 --require-raw-domain naval_air_combats --require-raw-domain naval_combat_members --require-raw-event-type naval_combat_started --require-raw-event-type naval_combat_updated
python -m live_capture.collector_session --pid <hoi4-pid> --dll extractor/native/target/debug/collector.dll --out-root replay_interface/replay/snapshots/_collector_session_air_wing_gate --game-id air_wing_live --duration-s 180 --sample-ms 10 --until-game-hours 12 --max-wall-s 300 --preflight-air-wings
python -m live_capture.collector_proof_gate replay_interface/replay/snapshots/_collector_session_air_wing_gate/<run-dir>/session_manifest.json --min-game-hours 12 --require-raw-domain air_wings --require-air-wing-raw-telemetry
python -m live_capture.collector_probe_record --pid <hoi4-pid> --dll extractor/native/target/debug/collector.dll --out-root replay_interface/replay/snapshots/_collector_probe --store-root replay_interface/replay/snapshots --game-id collector_live_speed5_worker --dwell-s 0.25 --worker-duration-s 8 --worker-sample-ms 10 --until-game-hours 12 --max-wall-s 15 --unique-store-hours-only
python -m unittest live_capture.tests.test_collector_session live_capture.tests.test_collector_proof_gate live_capture.tests.test_recorder_cadence live_capture.tests.test_recorder_worker_cadence live_capture.tests.test_recorder_manifest_diagnostics live_capture.tests.test_replay_writer
python -m pytest extractor/live_capture
```

Run `collector_profile_check` as a separate development preflight immediately
before worker proof runs. Do not combine it into the same proof process;
back-to-back hook loads are more reliable as separate commands.

Use `collector_session` for proof-harness capture work. It wraps the bounded
collector worker with explicit `session_start`/`session_stop` lifecycle frames,
feeds decoded collector frames through the durable store, finalizes a
`.battleplan`, packages the raw collector stream, and records cadence plus
transport diagnostics in the session manifest. This remains a Python proof path,
not the production hot loop. Use `collector_probe_record` directly only when
proving collector timing, reproducing transport behavior, or collecting raw probe
fixtures.

For focus/decision/project root-contract acceptance, the unit tests above are
only executable scaffolding. The segment has Python-harness live evidence only
after `collector_session` runs against the real running HOI4 process with
`hourly_requirement.met = true`, `replay_verified = true`, and
`collector_proof_gate --require-scheduled-domain focus_live --require-scheduled-domain decisions_live --require-scheduled-domain projects_live --require-projects-live-payload`
passes on that run's `session_manifest.json`. It is not production-complete until
the same segment is captured by the DLL/Rust recording path. If the
special-project cache is
missing, first run a cache-populating diagnostic with
`--preflight-special-projects --allow-special-projects-cold-scan`, then rerun
the acceptance proof without the cold-scan flag using the same `out-root` so the
scoped `_preflight_cache` is reused. Keep `--max-wall-s` larger than
`--duration-s` so the bounded worker can start before the wall-budget guard
closes the run. The 2026-06-29 pid 43412 proof used a
no-cold `special_projects_cache` bridge hit in 0.391s, advanced 12 game hours,
stored 13 snapshots, verified replay output, emitted 12 scheduled frames each
for `focus_live`, `decisions_live`, and `projects_live`, and kept heavy project
payloads off worker-poll frames. A current-game rerun
`replay_interface/replay/snapshots/_collector_session_project_roots_bootstrap/20260629T203922Z_pid43412`
used a no-cold cache hit in 0.375s, advanced 12 game hours in 38.969 wall
seconds, verified replay output, supplied 21548 bridge rows, emitted 14
scheduled frames each for `focus_live`, `decisions_live`, and `projects_live`,
folded 4956 focus country rows, 4956 decision country rows, 392 facility rows,
392 worked-project rows, and 0 scientist rows, and kept worker polls free of
heavy/project payloads. The latest strict-payload rerun
`replay_interface/replay/snapshots/_collector_session_project_roots_bootstrap/20260629T224333Z_pid43412`
used the same no-cold cache path with 21496 bridge rows, advanced 12 game hours
in 40.781 wall seconds, verified replay output, emitted 12 scheduled frames
each for `focus_live`, `decisions_live`, and `projects_live`, folded 4236 focus
country rows, 4236 decision country rows, 288 facility rows, 348 worked-project
rows, and 0 scientist rows, kept project payloads off lightweight frames, and
passed `--require-projects-live-payload`. This verifies the project root slice
at hourly cadence for cache-backed no-cold recorder runs.

For land-combat lifecycle acceptance, require raw `land_combats` plus
`land_combat_started`, `land_combat_updated`, and `land_combat_ended` events on
a no-cold 12-game-hour run. The 2026-06-29 pid 43412 speed-five proof
`replay_interface/replay/snapshots/_collector_session_diplomacy_gate/20260629T152734Z_pid43412`
advanced 12 game hours in 48.328 wall seconds, verified replay output, kept
heavy payloads off lightweight frames, and explicitly passed
`collector_proof_gate` with raw `land_combats` plus 56 `land_combat_started`,
6695 `land_combat_updated`, and 11 `land_combat_ended` events. The stricter raw
military gate
`replay_interface/replay/snapshots/_collector_session_raw_military_gate/20260629T194359Z_pid43412`
also passed `--require-land-combat-raw-lifecycle`, advancing 12 game hours in
62.688 wall seconds, verifying replay output, observing 160 raw land-combat
deltas with 8 starts, 151 updates, 1 end, and reporting zero raw gap domains.

For diplomacy war/casualty acceptance, require scheduled `diplomacy_live` plus
`--require-diplomacy-war-relations` on a 12-game-hour run using
`--preflight-war-relations`. The preflight is allowed to do one heap discovery
before recording; the DLL hot path must only validate pinned bridge rows during
scheduled frames. The 2026-06-29 pid 43412 proof
`replay_interface/replay/snapshots/_collector_session_diplomacy_war_gate/20260629T163650Z_pid43412`
advanced 12 game hours in 43.203 wall seconds, verified replay output, supplied
1033 bridge rows from the live tag-table identity path, observed 14 scheduled
diplomacy war frames, folded 1022 country `at_war` rows and 1022 country
casualty rows, observed max per-frame `war_relations = 1033` and casualty sum
56475731, and reported zero diplomacy payloads on lightweight worker frames. A
current-game rerun
`replay_interface/replay/snapshots/_collector_session_diplomacy_war_gate/20260629T202751Z_pid43412`
also passed: it advanced 12 game hours in 43.984 wall seconds, verified replay
output, supplied 689 bridge rows after a 90.422s preflight, observed 13
scheduled war frames, folded 884 country `at_war` rows and 884 casualty rows,
and observed max per-frame casualty sum 14124532.

For economy acceptance, require scheduled `production_live` plus snapshot
domains `fuel_live`, `country_misc`, and `country_scalars`. The 2026-06-29 pid
43412 proof advanced 12 game hours, stored 13 verified snapshots, emitted 12
production frames and 12 fuel frames, and observed up to 20 military lines, 12
naval lines, and 146 stockpile entries per production frame. Construction queue
acceptance is a separate scheduled-production gate:
`replay_interface/replay/snapshots/_collector_session_construction_gate/20260629T172526Z_pid43412`
advanced 12 game hours in 40.219 wall seconds, observed 14 scheduled
`production_live` frames, folded 3661 construction queue rows with up to 262
rows per scheduled frame and 29 rows per country, and reported zero heavy
payloads on lightweight worker frames. The queue rows include live line id,
template id when readable, assigned factories, cost, produced progress, and
priority; target state/location remains unavailable until the target handle is
proven.

For buildings acceptance, require scheduled `buildings_live` on a no-cold
12-game-hour run. The 2026-06-29 pid 43412 proof
`replay_interface/replay/snapshots/_collector_session_buildings_gate/20260629T150353Z_pid43412`
advanced 12 game hours in 45.734 wall seconds, stored 13 verified snapshots,
observed 13 scheduled `buildings_live` frames, captured 1081 state rows and
10272 province rows per scheduled frame, captured non-empty building maps in
13689 state rows and 50765 province rows across the run, and reported zero
building payloads on lightweight worker frames.

For state resource-deposit acceptance, require scheduled `state_detail` plus
`--require-state-resources` on a 12-game-hour run. This proves live raw state
resource deposits from `CState` definition tables, not country
produced/exported/imported economy totals. The 2026-06-29 pid 43412 proof
`replay_interface/replay/snapshots/_collector_session_state_resources_gate/20260629T223450Z_pid43412`
advanced 12 game hours in 44.579 wall seconds, stored 13 verified snapshots,
observed 13 scheduled `state_detail` frames, folded 14053 state resource rows
with 9334 resource entries, reached full 1081/1081 resource-bearing state rows
in a single frame, and reported zero heavy payloads on lightweight worker
frames. Unknown resource db ids are preserved as `tid_0x...`; the reader also
accepts the verified `size=6/cap=5` trailing `NO_ID` sentinel shape used by
Piedmont and Liaotung without emitting guessed rows.
The proof gate requires the resource-bearing row count to cover every scheduled
`state_detail` frame and rejects partial 1079/1081-style coverage.

For division org/strength plus army-order raw-stream acceptance, first populate
same-process caches with the cold bootstrap command if needed, then rerun
without any cold-scan flags. The 2026-06-29 pid 43412 proof advanced 12 game
hours, verified replay output, observed raw `divisions` and `army_orders`, and
captured `division_org_strength_delta`. `army_order_updated` is a separate live
fixture: it requires an already-sampled offensive order to be edited/redrawn
during the capture window. The stricter raw military gate
`replay_interface/replay/snapshots/_collector_session_raw_military_gate/20260629T194359Z_pid43412`
passed `--require-division-raw-telemetry` and
`--require-army-order-raw-telemetry`, advanced 12 game hours in 62.688 wall
seconds, verified replay output, emitted 12002 division deltas, 547 army-order
deltas, 6189 `division_org_strength_delta`, 2109
`division_movement_started`, 38 `division_movement_stopped`, 19
`division_combat_ref_changed`, and 547 `army_order_added` events with zero raw
gap domains. The same run failed `--require-army-order-raw-updates`, correctly,
because no existing order changed. A later cache-backed fixture run
`replay_interface/replay/snapshots/_collector_session_raw_military_gate/20260629T200009Z_pid43412`
passed the explicit `--require-army-order-raw-removals` gate: it advanced 36
game hours in 45.266 wall seconds, verified replay output, resolved 580 army
bodies from cache in 0.078s, emitted 519 army-order events, observed one
`army_order_removed`, and reported zero raw gap domains. That run still did not
observe `army_order_updated`; parsing the raw stream showed the removal had no
same-army/same-slot replacement nearby, so in-place order edits remain a live
fixture gap rather than a hidden replacement case in that evidence. A follow-up
speed-five cache-backed run
`replay_interface/replay/snapshots/_collector_session_raw_military_gate/20260629T201212Z_pid43412`
advanced 48 game hours in 55.031 wall seconds, verified replay output, resolved
496 army bodies from cache in 0.078s, emitted 476 army-order events, observed
463 `army_order_added` and 13 `army_order_removed` events, and reported zero raw
gap domains. It still failed `--require-army-order-raw-updates`; this segment
must not be promoted until an already-sampled type-2 line order is
redrawn/resized during a live capture and the strict update gate passes. A
second fixture window
`replay_interface/replay/snapshots/_collector_session_raw_military_gate/20260629T201839Z_pid43412`
advanced 49 game hours in 54.188 wall seconds, verified replay output, resolved
425 army bodies from cache in 0.078s, emitted 391 army-order events, observed
390 adds and one removal, and also failed the update gate with zero raw gap
domains.

For ship/OOB raw-stream acceptance, first populate same-process `ship_bodies`
and `oob_command_roots` caches if needed, then rerun without cold-scan flags.
The 2026-06-29 pid 43412 ship-specific proof
`replay_interface/replay/snapshots/_collector_session_air_ship_oob_gate/20260629T210118Z_pid43412`
passed `--require-ship-raw-telemetry`: it advanced 12 game hours in 57.297
wall seconds, verified replay output, hit `ship_body_cache` in 0.125s, supplied
1309 bridge rows, emitted 22249 raw `ships` deltas / 22248 object rows, and
reported zero raw gap domains. This slice has high raw-frame volume, so a
full-game recorder should add coalescing or lower-frequency sampling policy
before long soaks. The strict raw military gate also passed
`--require-oob-command-raw-telemetry`, capturing 17705 `oob_command` deltas
over 12 game hours with zero raw gap domains after an intentionally allowed
135.703s cold preflight. That cold discovery time is not acceptable for normal
startup and should be treated as cache/root-refresh work, not hot-loop
recording cost.

OOB army leader-detail acceptance has its own stricter gate. It is not enough
to observe generic `oob_command` rows: `--require-oob-command-leader-detail`
requires exact army rows where `leader_id`, `leader_status`, `leader_skill`,
`leader_traits`, and `leader_trait_status` are co-present. The 2026-06-29 pid
43412 proof first populated `oob_command_roots` with an intentionally cold
139.688s bootstrap, then the no-cold run
`replay_interface/replay/snapshots/_collector_session_oob_leader_gate/20260629T215306Z_pid43412`
advanced 12 game hours in 54.110 wall seconds, verified replay output, emitted
19842 raw `oob_command` deltas, reported zero raw OOB gap domains, and captured
7485 exact army leader-detail rows with 15911 trait tokens. This raw stream
currently proves army leader skill and assigned trait tokens only; portraits,
names, temporary/captured traits, naval leaders, global commander rosters, and
save-shaped OOB nesting remain separate segments.

For naval-combat acceptance, `--preflight-naval-combat` must find at least one
active battle at attach time; this bridge does not yet discover battles born
after preflight. The 2026-06-29 pid 43412 proof advanced 12 game hours, verified
replay output, observed raw `naval_air_combats` and `naval_combat_members`,
captured `naval_combat_started` / `naval_combat_updated`, and reported zero raw
gap frames. `naval_combat_ended` requires a battle-ending fixture.

For air-wing acceptance, first populate the same-process air-wing cache only
when the current game actually has active wings; the acceptance run must then
rerun without `--allow-air-wing-cold-scan`. Cold preflight now live-discovers
the current heap arena before `NavalAirReader.enumerate_air_wings`; this avoids
the stale fixed-arena zero-row failure documented by the air-combat research.
The 2026-06-29 pid 43412 bootstrap discovered a 9803792384-byte arena and
stored 653 air-wing body rows. The no-cold acceptance run
`replay_interface/replay/snapshots/_collector_session_air_wing_gate/20260629T164933Z_pid43412`
then hit the cache in 0.094s, supplied 652 bridge rows, advanced 12 game hours
in 52.891 wall seconds, verified replay output, observed raw `air_wings`,
captured 652 `air_wing_observed` and 9128 `air_wing_updated` events, reported
zero raw gaps, and kept heavy scheduled payloads off lightweight frames. A
current-game no-cold rerun
`replay_interface/replay/snapshots/_collector_session_air_wing_gate/20260629T205254Z_pid43412`
passed the stricter `--require-air-wing-raw-telemetry` gate: it advanced 12 game
hours in 55.875 wall seconds, verified replay output, hit the air-wing cache in
0.094s, supplied 585 bridge rows, emitted 11115 raw `air_wings` deltas, observed
585 `air_wing_observed` and 10530 `air_wing_updated` events, and reported zero
raw gaps.

Repeated bounded workers are one logical recorder session. The raw probe
manifest keeps every verified observation, but the replay writer processes one
selected frame per unique game hour so duplicate 10 ms samples do not dominate
post-processing. Worker sequence ids restart per bounded worker; diagnostics
therefore report both global reset evidence and burst-local monotonicity. If a
later worker burst fails after earlier verified frames, the session writes the
prefix replay and manifest, then exits fail-closed with the attach error.

Country identity rows include the live `tag_db_id` from the collector tag table
even when country scalar capture is unavailable. Treat that id as the safe join
key for future state/province/division/country-object work; do not enable
heap-wide country-object scans in normal recorder runs.
The worker control file and probe manifest explicitly record
`country_object_scan: false` for this reason. A current successful scalar run
should show `country_object_identity_statuses` containing
`gamestate_country_table_verified` and nonzero `country_scalars_frames`; if a
run only has `country_identity`, it captured identity but not live country
scalars.

When `collector_probe_record` is asked for `--until-game-hours`, hourly cadence
is a required proof: the run must advance at least the requested number of
in-game hours, keep game time monotonic, and skip zero scheduled hours. The
manifest records `hourly_requirement`; the CLI exits nonzero on proof failure.
Use `--allow-hourly-gaps` only for diagnostic runs where a failed proof should
still exit 0.

For speed-five acceptance, use `--worker-sample-ms 10` until a measured
less-aggressive interval is documented, and keep `--max-wall-s` set. A run that
catches every hour but exceeds `--max-wall-s` fails with
`max_wall_s_exceeded`; that means cadence worked but speed-five throughput was
not proven. The control file intentionally does not enable
`country_object_scan`; heap-wide discovery is not allowed in proof runs. Worker
proof runs use a per-run named-pipe token and keep sidecar file logging disabled
so stale workers and local file I/O cannot contaminate the timing signal.

## Capture Semantics

- A domain is captured only if its plug runs to completion.
- `0` and `0.0` are measured zero. `null` is captured-but-not-measured or
  unavailable. An omitted domain is not captured.
- Failed plugs do not leak partial state into snapshots.
- Unknown or untrusted dates are kept out of the canonical replay index.
- Canonical snapshot filenames are keyed by trusted in-game time and segment.
- The single-writer lock prevents concurrent capture loops from corrupting a
  store.
- Publication writes a same-directory temporary artifact, verifies and flushes/fsyncs it, then
  atomically replaces the destination. Any verification, sync, or replace failure propagates and
  preserves the previous last-good artifact; a failed publication never becomes success-shaped.

## Store Layout

```text
<root>/<game_id>/s<segment>.<hour>.snapshot.json
<root>/<game_id>/index.json
<root>/<game_id>/_noncanonical/
<root>/_continuity/<game_id>.state.json
<root>/_continuity/<game_id>.lock
```

Portable recordings are finalized as `.battleplan` containers and are consumed
through the same snapshot-store interface by whatever replay surface replaces
the reset app.

## Safety

- Live access is read-only: legacy external reads stay under `memory_reader/`,
  and collector-frame ingestion consumes already-decoded DLL output.
- Unknown HOI4 builds fail closed.
- Save-derived data is development validation/discovery data only; do not use it
  as runtime fallback and do not present it as a fresh live read.
- Collector transport gaps are manifest data, not silently repaired by saves.
- Local capture stores, partial recordings, and finalized recordings are
  gitignored.
- Do not commit personal paths, raw dumps, or scratch probe outputs.
