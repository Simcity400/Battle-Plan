# Chunk 3 Fold Packet

Scope: draft-only fold specs and standalone Rust handoff modules for domains 24, 26, 41, 45, and 51. These files are intended to be moved mechanically into `recorder_host/src/world_fold.rs` by a serial worker after re-verifying the current recorder contracts.

Current fold conventions applied here:
- Fail closed on compact status failures. Unknown current-hour data is represented by a null/unknown marker plus provenance notes, not fabricated empty containers.
- Delta domains require a full baseline before any delta frame is applied. A delta-before-baseline frame records a domain problem and leaves the last known state untouched.
- Country references are kept as compact `tag_db_id` until fold time, then re-keyed through the TAG resolver.
- Decoded-and-unfolded domains need contract evidence wording that explicitly says the domain is now folded.
- Fields absent from decoded rows stay absent/null. Extra shapes are named with `_r2_...` keys.

## Domain 41 - projects

Shape source: `extractor/native/recorder_host/src/main.rs`, `decode_domain41_projects`.

Decoded header:
- `elapsed_wall_ms: u64`
- `row_count: u32`
- `changed_rows: u32`
- `status_flags: u32`
- `facility_count: u32`
- `project_count: u32`
- `scientist_count: u32`
- `observed_facility_rows: u32`
- `observed_project_rows: u32`
- `observed_scientist_rows: u32`
- `bridge_rows_supplied: u32`
- `source_kind: u32` (`1 = scan`, `2 = bridge`)
- `header_extension: u32`
- `scan_ms: u64`

Optional root counters when `header_extension >= 1`:
- `invalid_program_status_countries: u32`
- `invalid_facility_countries: u32`
- `invalid_project_pool_countries: u32`
- `neutral_program_status_countries: u32`
- `neutral_facility_countries: u32`
- `neutral_project_pool_countries: u32`

Optional reason counters when `header_extension >= 2`:
- `invalid_facility_root_read_fail: u32`
- `invalid_facility_slot_read_fail: u32`
- `invalid_facility_candidate_vtable_read_fail: u32`
- `invalid_facility_candidate_sentinel_after_pin: u32`
- `invalid_facility_candidate_fields_invalid: u32`
- `invalid_project_pool_root_read_fail: u32`
- `invalid_project_pool_vtable_read_fail: u32`
- `invalid_project_pool_first_project_read_fail: u32`
- `invalid_project_candidate_vtable_read_fail: u32`
- `invalid_project_candidate_sentinel_after_pin: u32`
- `invalid_project_candidate_fields_invalid: u32`
- `invalid_project_short_array: u32`

Optional progress counters when `header_extension >= 3`:
- `project_state_count: u32`
- `breakthrough_count: u32`
- `observed_project_state_rows: u32`
- `observed_breakthrough_rows: u32`

Facility row:
- `id: u32`
- `province: u32`
- `flags: u32` (`active_project = 1`, `scientist = 2`, `building = 4`)
- `active_project_handle: u32`
- `scientist_char_id: u32`
- `unread_rewards: u32`
- `reserved0: u32`
- `reserved1: u32`
- `building: Option<String>` from trailing compact string when the building flag is set

Project row:
- `id: u32`
- `static_def_handle: u32`
- `iterations: u32`
- `worked_at_facility: u32`
- `flags: u32` (`worked_at_facility_present = 1`)
- `reserved0: u32`

Scientist row:
- `char_id: u32`
- `flags: u32` (`name_token = 1`, `display_name = 2`, `trait_count = 4`, `skills = 8`)
- `trait_count: u32`
- `skill_values: [i32; 4]`
- `reserved0: u32`
- `name_token: Option<String>` from trailing compact string
- `display_name: Option<String>` from trailing compact string

Project state row:
- `id: u32`
- `static_def_handle: u32`
- `current_state_offset: u32`
- `current_state_enum: u32`
- `prototype_state_enum: u32`
- `simple_state_1_enum: u32`
- `simple_state_2_enum: u32`
- `simple_state_3_enum: u32`
- `prototype_iterations: u32`
- `phase_progress_raw: u32`
- `phase_target_raw: u32`
- `state_flags_or_reserved: u32` (`completed = 1` only for extension v2+ progress rows)

Breakthrough row:
- `tag_db_id: u32`
- `specialization_handle: u32`
- `points_raw: u32`
- `reserved0: u32`

Schema home: REPLAY_SCHEMA section 13b, top-level `special_projects`.

Fold semantics: full replace. The collector emits a complete project slice rather than `COMPACT_STATUS_DELTA_DATA`. A successful frame replaces `special_projects` with decoded facilities, projects, scientists, and source/count metadata. Breakthrough rows use `resolve_country_key(tag_db_id)` before they are exposed.

Failure handling: if `compact_status_is_failure(status_flags)` is true, the fold marks `special_projects` unknown for the current hour, records provenance, and leaves the previous accumulator untouched. The snapshot writer should emit `special_projects: null` for that hour, matching the current unknown-hour convention.

R2 deltas:
- `special_projects._r2_project_states[]` for progress/state extension rows, because section 13b does not define the state-machine/progress raw handles.
- `special_projects._r2_breakthroughs[]` for country specialization handles and raw points, because decoded rows do not carry specialization names.
- Scientist skill names/specializations are not decoded; preserve the four raw skill integers as `_r2_skill_values`.

Contract row evidence wording:
- `projects_live / 41`: `PRODUCTION_SLICE_DONE - folded into top-level special_projects; R2 project-state/progress and breakthrough handle sidecars preserve decoded rows while lifecycle/reward/name joins remain open.`

## Domain 45 - focus_research_decisions_depth

Shape source: `extractor/native/recorder_host/src/main.rs`, `decode_domain45_focus_research_decisions_depth`.

Decoded header in the active recorder host:
- `elapsed_wall_ms: u64`
- `depth_status_flags: u32`
- `focus_count: u32`
- `research_count: u32`
- `decision_status_count: u32`
- `decision_event_count: u32`
- `observed_focus_rows: u32`
- `observed_research_slot_rows: u32`
- `observed_decision_rows: u32`
- `observed_timed_decision_rows: u32`
- `badge_count: u32`

Focus row:
- `tag_db_id: u32`
- `kind: u32` (`1 completed_count_increased`, `2 current_started`, `3 stopped`, `4 changed`, `5 progress_regressed`, `6 invalid_both_current_types`)
- `focus_kind: u32` (`2` means continuous focus in the lane)
- `flags: u32` (`completed_token_inferred = 1`, `completed_token_exact = 2`)
- `previous_completed: u32`
- `current_completed: u32`
- `previous_progress: i32`
- `current_progress: i32`
- `completed_delta: u32`
- `from_token: String`
- `to_token: String`
- `completed_token: String`

Research row:
- `tag_db_id: u32`
- `slot_index: u32`
- `flags: u32` (`same_tech = 1`, `positive_speed = 2`, `eta_present = 4`, `rollover = 8`, `negative_delta = 16`, `base_present = 32`)
- `hour_delta: u32`
- `previous_handle: u32`
- `current_handle: u32`
- `previous_def: u32`
- `current_def: u32`
- `previous_progress: i64`
- `current_progress: i64`
- `delta: i64`
- `speed: i64`
- `base: i64`
- `eta: i64`

Decision status row:
- `tag_db_id: u32`
- `active_timed_count: u32`
- `targeted_count: u32`
- `targeted_timed_count: u32`
- `decisions_taken_bucket_count: u32`
- `re_enable_count: u32`
- `timed_live_count: u32`

Decision event row:
- `tag_db_id: u32`
- `kind: u32` (`1 started`, `2 stopped`, `3 day_delta`, `4 state_changed`, `5 status_counts_changed`)
- `flags: u32`
- `previous_days: i32`
- `current_days: i32`
- `previous_state: i32`
- `current_state: i32`
- `decision: String`

Badge row:
- `kind: u32` (`1 focus_pause_fail_closed`, `2 decision_target_fields_fail_closed`, `3 direct_hooks_unavailable`)
- `flags: u32` (`fail_closed = 1`, `derived_deterministic = 2`, `current_build_probed = 4`)
- `source_domain_id: u32`
- `affected_count: u32`
- `evidence_code: u32` (`1 no_paused_true`, `2 timed_definition_layout_rejected`, `3 compact_frame_delta_derived`)
- `detail: String`

Schema home: REPLAY_SCHEMA sections 3.4 `countries.*.research`, 3.5 `countries.*.focus`, and 3.7 `countries.*.decisions`.

Fold semantics: current-hour depth annotation, not a full baseline and not a compact delta stream. Research rows append/replace `research.in_progress_live` entries for the current hour. Focus rows update the measured focus current/completed count when tokens or counts are decoded, and also preserve raw transition events. Decision status rows update measured decision counters; decision event rows are transition evidence and remain R2 annotations rather than a fabricated full live decision list.

Failure handling: active `main.rs` decodes `depth_status_flags`, but the row stream is not guarded by `COMPACT_STATUS_DELTA_DATA` and is not a full-replace domain. Badges are evidence, not a reason to fabricate nulls. Malformed payloads should mark the domain as a problem and leave existing country state untouched.

R2 deltas:
- `countries.TAG.focus._r2_depth_events[]` keeps focus transition kinds, raw progress values, and inferred/exact completed-token evidence.
- `countries.TAG.research.in_progress_live[]` can carry raw handles/defs because decoded rows do not carry technology names.
- `countries.TAG.decisions._r2_depth_events[]` keeps decision start/stop/day/state transition rows.
- Top-level or provenance `_r2_depth_badges[]` keeps fail-closed badge rows.

Risk to re-verify: the lane library has a `DecisionTargetRow` extension in a separate decoder path, but the active recorder host decoder and its current test payload use the first 8 header bytes as `elapsed_wall_ms` and do not parse target rows. Do not merge target rows into the fold until `recorder_host/src/main.rs` exposes them in this active decoded shape.

Contract row evidence wording:
- `focus_research_decisions_depth_live / 45`: `PRODUCTION_SLICE_DONE - folded into country focus/research/decisions depth annotations and measured counters; active main.rs target-row extension remains a re-verify item.`

## Domain 51 - scripted_state_flags_variables

Shape source: `extractor/native/recorder_host/src/main.rs` delegates to `lanes/scripted_state_flags_variables_live/scripted_state_compact.rs`.

Decoded header:
- `elapsed_wall_ms: u64`
- `row_count: u32`
- `key_bytes_len: u32`
- `changed_rows: u32`
- `status_flags: u32`
- `observed_scopes: u32`
- `measured_scopes: u32`
- `rejected_scopes: u32`
- `variable_rows: u32`
- `flag_rows: u32`
- `token_only_rows: u32`
- `fixed_scale: u32`
- `schema_version: u32`
- `reserved0: u32`
- `reserved1: u32`

Row:
- `scope_kind: u32` (`1 global`, `2 country`, `3 state`)
- `owner_id: u32` (`tag_db_id` for country, state id for state, zero for global)
- `value_kind: u32` (`1 variable_fixed_i64`, `2 flag_named`, `3 flag_token_only`, `4 delayed_event_opaque`, `5 scripted_gui_opaque`)
- `flags: u32` (`key_present = 1`, `token_present = 2`, `value_present = 4`, `date_present = 8`, `deleted = 16`)
- `key_offset: u32`
- `key_len: u32`
- `token_id: u32`
- `date_hour_raw: i32`
- `value_raw: i64`
- `reserved0: u32`
- `reserved1: u32`

Schema home: section 18 `world.flags` / `world.variables`, section 3.12 `countries.*.extra.script_flags`, and R2 sidecars for country variables and state-scoped rows.

Fold semantics: full replace in the current collector, with delta support drafted because the compact row format has `COMPACT_STATUS_DELTA_DATA` and deletion flags. A non-delta successful frame clears previously folded scripted globals, country scripted extras, and state sidecars. A delta frame applies key deletes/upserts only after a baseline was seen.

Failure handling: `STATUS_PARTIAL_DATA` and `STATUS_SOURCE_UNAVAILABLE` are hard failures for folding. The fold marks scripted state unknown for the current hour and preserves previous state. `STATUS_FLAG_NAMES_PARTIAL`, `STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED`, and `STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED` are not row-stream failures; they become provenance/R2 badges while decoded rows are folded.

R2 deltas:
- `countries.TAG.extra._r2_script_variables` for country variables, because section 3.12 only defines `script_flags`.
- `states.STATE_ID._r2_scripted_state.flags` and `.variables` for state-scoped flags/variables; section 4 has no first-class scripted-state home.
- `world._r2_token_only_flags`, `countries.TAG.extra._r2_token_only_flags`, and state equivalents for token-only flags with unresolved names.
- `world._r2_scripted_state_badges[]` for opaque delayed-event/scripted-GUI roots.

Contract row evidence wording:
- `scripted_state_flags_variables_live / 51`: `PRODUCTION_SLICE_DONE - folded into world flags/variables, country extra script_flags plus R2 variables/token sidecars, and state R2 scripted-state sidecars; opaque roots remain fail-closed provenance.`

## Domain 26 - division_equipment

Shape source: `extractor/native/recorder_host/src/main.rs`, `decode_domain26_division_equipment`.

Decoded header:
- `elapsed_wall_ms: u64`
- `row_count: u32`
- `changed_rows: u32`
- `status_flags: u32`
- `scale_raw: u32`
- `root_rows: u32`
- `observed_division_rows: u32`
- `equipment_entry_count: u32`
- `observed_equipment_rows: u32`

Division equipment root row:
- `owner_tag_db_id: u32`
- `div_id: u32`
- `flags: u32` (`template_present = 1`, `deleted = 2`)
- `template_id: u32`
- `entry_first: u32`
- `entry_count: u32`

Equipment entry row:
- `variant_id: u32`
- `reserved: u32`
- `amount_raw: i64`

Schema home: section 6 `divisions.*.equipment[]`.

Fold semantics: baseline plus delta. The collector emits the first successful frame as a full baseline and later frames with `COMPACT_STATUS_DELTA_DATA`. A non-delta frame replaces the domain-26 equipment accumulator. A delta frame updates or deletes only keyed `(owner_tag_db_id, div_id)` equipment slices.

Identity keying: decoded rows do not carry `div_type`, while section 6 division identity is `div_uid = "id:type"`. The draft only attaches equipment to a live folded division when `(owner_tag_db_id, div_id)` resolves to exactly one division. Otherwise it preserves the equipment in an R2 side accumulator and emits a note. The serial worker should re-key through the existing domain-20 division map and TAG resolver.

Failure handling: compact status failure marks division equipment unknown for the current hour, records provenance, and leaves prior accumulator data untouched. A delta-before-baseline frame records `delta_before_baseline`, marks unknown, and is not applied.

R2 deltas:
- `divisions.DIV_UID.equipment[].equip_id = variant_id`.
- `archetype` and `equip_type` remain absent/null because decoded rows do not carry archetype names or equipment type.
- `_r2_division_equipment_unjoined` is needed for equipment rows that cannot be uniquely attached to a current `div_uid`.

Contract row evidence wording:
- `division_equipment_live / 26`: `PRODUCTION_SLICE_DONE - folded into division.equipment when a unique owner/division join exists, with unjoined variant rows preserved under R2; equipment type/archetype joins remain open.`

## Domain 24 - weather

Shape source: `extractor/native/recorder_host/src/main.rs`, `decode_domain24_weather`.

Decoded header:
- `elapsed_wall_ms: u64`
- `region_count: u32`
- `changed_rows: u32`
- `status_flags: u32`
- `province_count: u32`
- `observed_region_rows: u32`
- `observed_province_rows: u32`
- `scale_raw: u32`
- `active_region_count: u32`
- `reserved0: u32`
- `reserved1: u32`

Region row:
- `region_id: u32`
- `flags: u32` (`rain_light = 1`, `rain_heavy = 2`, `snow = 4`, `blizzard = 8`, `arctic_water = 16`, `active_signal = 32`, `phenomenon_active = 64`, `sandstorm = 128`)
- `temperature_raw: i32`
- `phenomenon: u32`
- `next_weather_change_raw: u32`
- `active_modifier_count: u32`
- `intensity: u32`
- `reserved: u32`

Province row:
- `province_id: u32`
- `flags: u32` (`temperature_present = 1`, `temperature_change_present = 2`, `water_present = 4`, `snow_present = 8`, `mud_present = 16`, `mud_active = 32`, `custom_modifier_count_present = 64`, `struct_validated = 128`, `map_recolor = 256`, `dynamic_lut = 512`)
- `temperature_raw: i32`
- `temperature_change_raw: i32`
- `water_raw: i32`
- `snow_raw: i32`
- `custom_modifier_count: u32`
- `reserved: u32`

Schema home: no clear first-class home in REPLAY_SCHEMA. Rejection would drop visual weather data that the collector intentionally emits. Proposed R2 home is `map._r2_weather_live`, because the data is province/region map state and not a country/state/division object.

Fold semantics: baseline plus delta. The collector emits a first full frame and later `COMPACT_STATUS_DELTA_DATA` frames. A non-delta frame replaces the weather accumulator. A delta frame upserts keyed region/province rows.

Failure handling: compact status failure marks weather unknown for the current hour, records provenance, and leaves previous accumulator data untouched. A delta-before-baseline frame records `delta_before_baseline`, marks unknown, and is not applied.

R2 deltas:
- `map._r2_weather_live.regions.REGION_ID` for region flags, raw phenomenon id, scaled temperature, intensity, active modifier count, and next-change raw hour/tick.
- `map._r2_weather_live.provinces.PROVINCE_ID` for province temperature/water/snow/mud values and validation/recolor/LUT flags.
- The schema should keep raw values beside scaled values until the UI validates exact units.

Contract row evidence wording:
- `weather_live / 24`: `PRODUCTION_SLICE_DONE - folded under map._r2_weather_live as baseline/delta visual weather region and province state; first-class schema placement remains an R2 decision.`

## Serial Worker Re-verify List

- Domain 45 target rows: reconcile the lane `DecisionTargetRow` extension with the active `recorder_host/src/main.rs` decoder before treating the first 8 header bytes as target metadata or accepting separate target rows.
- Domain 26 join safety: only attach equipment when `(owner_tag_db_id, div_id)` maps to a unique current division; reuse of division ids without `div_type` must not overwrite the wrong `div_uid`.
- Domain 24 schema placement: no first-class weather home exists. The packet recommends `map._r2_weather_live`; reject only if the product decision is to keep weather out of snapshots.
- Domain 41 naming joins: decoded project/scientist/breakthrough rows carry raw handles and skill integers, not localization or specialization names.
- Domain 51 partial statuses: keep `SOURCE_UNAVAILABLE` and `PARTIAL_DATA` as fold failures, but do not treat unresolved opaque roots or flag-name partials as row-stream failures.
