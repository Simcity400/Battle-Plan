# Domain 53 870ebf Revalidation Audit

Build: `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`

Evidence run: `extractor/native/runs/domain53_revalidation_870ebf_20260703/`

Summary: 71 `DOMAIN53_*` constants audited; 47 `VALIDATED`, 24
`STRUCTURAL-ONLY`, 0 `NOT-VALIDATED`.

`STRUCTURAL-ONLY` means the 2026-07-03 probe walked the structure on the
current build, derived current RTTI where applicable, and produced plausible
current-build shapes, but the lane may consume it only behind runtime gates.
Those gates are listed below the table.

## Constant Audit

| Constant | Value | Validating artifact / field | Verdict |
| --- | ---: | --- | --- |
| `DOMAIN53_CGAMESTATE_COUNTRY_TABLE` | `0x310` | `probe_870ebf_diplomacy.json::roots.country_table`, script constant `CGAMESTATE_COUNTRY_TABLE`; country table read with 435 valid current-vtable countries | `VALIDATED` |
| `DOMAIN53_CGAMESTATE_COUNTRY_COUNT` | `0x31C` | `probe_870ebf_diplomacy.json::roots.country_count=440`, script constant `CGAMESTATE_COUNTRY_COUNT` | `VALIDATED` |
| `DOMAIN53_COUNTRY_VTABLE_RVA` | `0x27D0E90` | `probe_870ebf_diplomacy.json::country_vtable_candidates.current_870ebf.status=validated`, `valid_country_count=435`, `rva=0x27d0e90` | `VALIDATED` |
| `DOMAIN53_OBJ_TAG_DB_ID` | `0x08` | `probe_870ebf_diplomacy.json::country_vtable_candidates.current_870ebf.samples`, tag echo read by script constant `OBJ_TAG_DB_ID` | `VALIDATED` |
| `DOMAIN53_COUNTRY_FACTION_GROUP` | `0x598` | `probe_870ebf_diplomacy.json::factions.status=validated_structural`, `offset=0x598` | `STRUCTURAL-ONLY` |
| `DOMAIN53_RELATION_MANAGER` | `0xF88` | `cracklist_b4_d53_base` D04 melt/live validation plus 2026-07-05 PID 19204 refresh; relation root chain matched `cached_sum` 10354/10354 non-missing rows and `border_friction` 44382/44382 non-missing rows, 0 mismatches | `VALIDATED` |
| `DOMAIN53_RELATION_TABLE` | `0x08` | `cracklist_b4_d53_base` D04 melt/live validation plus 2026-07-05 PID 19204 refresh; `mgr+0x08` table indexed by target tag-db id resolved all non-missing relation rows, 0 value mismatches | `VALIDATED` |
| `DOMAIN53_RELATION_OPINION` | `0x300` | `cracklist_b4_d53_base` D04 melt/live validation plus 2026-07-05 PID 19204 refresh; `rel+0x300` i32 matched melt `cached_sum` 10354/10354 non-missing rows, 0 mismatches | `VALIDATED` |
| `DOMAIN53_RELATION_SCAN_BYTES` | `0x340` | `probe_870ebf_diplomacy.json::relation_flags.scan_bytes_per_relation=832` | `STRUCTURAL-ONLY` |
| `DOMAIN53_REL_NON_AGGRESSION_PACT_VTABLE_RVA` | `0x2949D98` | `probe_870ebf_diplomacy.json::relation_flags.candidate_sets.current_870ebf_rtti.non_aggression_pact`, observed hits in `hits_by_candidate_set.current_870ebf_rtti.non_aggression_pact=1248` | `STRUCTURAL-ONLY` |
| `DOMAIN53_REL_SUBJECT_VTABLE_RVA` | `0x2949F18` | `probe_870ebf_diplomacy.json::relation_flags.candidate_sets.current_870ebf_rtti.subject_or_puppet`, observed hits `subject_or_puppet=74`, `subjects` decoded | `STRUCTURAL-ONLY` |
| `DOMAIN53_REL_EMBARGO_VTABLE_RVA` | `0x294A070` | `probe_870ebf_diplomacy.json::relation_flags.candidate_sets.current_870ebf_rtti.embargo`, observed hits `embargo=12` | `STRUCTURAL-ONLY` |
| `DOMAIN53_REL_MILITARY_ACCESS_VTABLE_RVA` | `0x2949918` | `probe_870ebf_diplomacy.json::relation_flags.candidate_sets.current_870ebf_rtti.military_access`, observed hits `military_access=84` | `STRUCTURAL-ONLY` |
| `DOMAIN53_REL_AIR_BASE_ACCESS_VTABLE_RVA` | `0x2A0A9D8` | `probe_870ebf_diplomacy.json::relation_flags.candidate_sets.current_870ebf_rtti.air_base_access`, observed hits `air_base_access=316` | `STRUCTURAL-ONLY` |
| `DOMAIN53_REL_DOCKING_RIGHTS_VTABLE_RVA` | `0x2A0ABC8` | `probe_870ebf_diplomacy.json::relation_flags.candidate_sets.current_870ebf_rtti.docking_rights`, observed hits `docking_rights=358` | `STRUCTURAL-ONLY` |
| `DOMAIN53_REL_NAVAL_BLOCKADE_VTABLE_RVA` | `0x2A0AD70` | `probe_870ebf_diplomacy.json::relation_flags.candidate_sets.current_870ebf_rtti.naval_blockade`, observed hits `naval_blockade=2` | `STRUCTURAL-ONLY` |
| `DOMAIN53_REL_MARKET_ACCESS_RIGHTS_VTABLE_RVA` | `0x2A0AF20` | `probe_870ebf_diplomacy.json::relation_flags.candidate_sets.current_870ebf_rtti.market_access_rights`, observed hits `market_access_rights=1258` | `STRUCTURAL-ONLY` |
| `DOMAIN53_REL_EQUIPMENT_PURCHASE_RELATION_VTABLE_RVA` | `0x2A0AE58` | `probe_870ebf_diplomacy.json::relation_flags.candidate_sets.current_870ebf_rtti.equipment_purchase_relation`, observed hits `equipment_purchase_relation=114` | `STRUCTURAL-ONLY` |
| `DOMAIN53_REL_GUARANTEE_VTABLE_RVA` | `0x2949CD8` | `probe_870ebf_diplomacy.json::relation_flags.candidate_sets.current_870ebf_rtti.guarantee`, observed hits `guarantee=10` | `STRUCTURAL-ONLY` |
| `DOMAIN53_REL_IMPROVE_RELATIONS_VTABLE_RVA` | `0x2949A98` | `probe_870ebf_diplomacy.json::relation_flags.candidate_sets.current_870ebf_rtti.improve_relations`, observed hits `improve_relations=100` | `STRUCTURAL-ONLY` |
| `DOMAIN53_REL_ATTACHE_VTABLE_RVA` | `0x2949B58` | `probe_870ebf_diplomacy.json::relation_flags.candidate_sets.current_870ebf_rtti.attache`; RTTI-derived current-build vtable, no observed live hit in this fixture | `STRUCTURAL-ONLY` |
| `DOMAIN53_REL_LICENCE_PRODUCTION_VTABLE_RVA` | `0x2949858` | `probe_870ebf_diplomacy.json::relation_flags.candidate_sets.current_870ebf_rtti.licence_production`; RTTI-derived current-build vtable, no observed live hit in this fixture | `STRUCTURAL-ONLY` |
| `DOMAIN53_SUBJECT_OVERLORD` | `0x10` | `probe_870ebf_diplomacy.json::relation_flags.subjects`, script function `read_subject_candidate` field `SUBJ_OVERLORD=0x10` | `STRUCTURAL-ONLY` |
| `DOMAIN53_SUBJECT_SUBJECT` | `0x14` | `probe_870ebf_diplomacy.json::relation_flags.subjects`, script function `read_subject_candidate` field `SUBJ_SUBJECT=0x14` | `STRUCTURAL-ONLY` |
| `DOMAIN53_SUBJECT_STATE_DB` | `0x58` | `probe_870ebf_diplomacy.json::relation_flags.subjects`, script function `read_subject_candidate` field `SUBJ_STATE_DB=0x58` | `STRUCTURAL-ONLY` |
| `DOMAIN53_SUBJECT_STATE_NAME` | `0x08` | `probe_870ebf_diplomacy.json::relation_flags.subjects`, script reads autonomy state name at `state_db+0x08` | `STRUCTURAL-ONLY` |
| `DOMAIN53_COUNTRY_AGENCY_PTR` | `0xFC0` | `probe_870ebf_intel.json::agencies.country_agency_ptr_offset=0xFC0`, `valid_agencies=112`, `nonheap_agency_ptrs=0` | `VALIDATED` |
| `DOMAIN53_AGENCY_VTABLE_RVA` | `0x296B338` | `probe_870ebf_intel.json::agencies.agency_vtable_rtti[0].rva=0x296B338`, `status=validated` | `VALIDATED` |
| `DOMAIN53_AGENCY_NAME` | `0x80` | `probe_870ebf_intel.json::agencies.samples[].name`, script constant `AG_NAME=0x80` | `VALIDATED` |
| `DOMAIN53_AGENCY_ICON` | `0xA0` | `probe_870ebf_intel.json::agencies.samples[].icon`, script constant `AG_ICON=0xA0` | `VALIDATED` |
| `DOMAIN53_AGENCY_UPGRADE_PROGRESS` | `0xC8` | `probe_870ebf_intel.json::agencies.samples[].upgrade_progress_raw`, script constant `AG_UPGRADE_PROGRESS=0xC8` | `VALIDATED` |
| `DOMAIN53_AGENCY_CAPACITY_E4` | `0xE4` | `probe_870ebf_agency_capacity.json::capacity_analysis.e4_f4_equal_rows=112`, raw capacity only; old semantic assignment rejected | `VALIDATED` |
| `DOMAIN53_AGENCY_CAPACITY_F0` | `0xF0` | `probe_870ebf_agency_capacity.json::capacity_analysis.divergent_samples[].value_0xF0`, raw capacity only; old semantic assignment rejected | `VALIDATED` |
| `DOMAIN53_AGENCY_CAPACITY_F4` | `0xF4` | `probe_870ebf_agency_capacity.json::capacity_analysis.e4_f4_equal_rows=112`, `e4_f4_mismatch_rows=0`, raw capacity only | `VALIDATED` |
| `DOMAIN53_AGENCY_CRYPTO_MGR` | `0x120` | `probe_870ebf_intel.json::cryptology.agency_crypto_manager_offset=0x120`, `status=validated` | `VALIDATED` |
| `DOMAIN53_AGENCY_DEFENSE` | `0x128` | `probe_870ebf_intel.json::agencies.samples[].defense_raw`, script constant `AG_DEFENSE=0x128` | `VALIDATED` |
| `DOMAIN53_CRYPTOLOGY_VTABLE_RVA` | `0x29A6540` | `probe_870ebf_intel.json::cryptology.cryptology_vtable_rtti[0].rva=0x29A6540`, `status=validated` | `VALIDATED` |
| `DOMAIN53_CRYPTOLOGY_RECORD_VTABLE_RVA` | `0x29A64F0` | `probe_870ebf_intel.json::cryptology.country_decryption_state_vtable_rtti[0].rva=0x29A64F0` | `VALIDATED` |
| `DOMAIN53_CRYPTOLOGY_MGR_DATA` | `0x28` | `probe_870ebf_intel.json::cryptology.manager_data_offset=0x28` | `VALIDATED` |
| `DOMAIN53_CRYPTOLOGY_MGR_SIZE` | `0x30` | `probe_870ebf_intel.json::cryptology.manager_size_offset=0x30` | `VALIDATED` |
| `DOMAIN53_CRYPTOLOGY_MGR_CAP` | `0x34` | `probe_870ebf_intel.json::cryptology.manager_cap_offset=0x34` | `VALIDATED` |
| `DOMAIN53_CRYPTOLOGY_RECORD_STRIDE` | `0x38` | `probe_870ebf_intel.json::cryptology.record_stride=0x38`, `records_scanned=24` | `VALIDATED` |
| `DOMAIN53_CRYPTOLOGY_RECORD_TARGET` | `0x08` | `probe_870ebf_intel.json::cryptology.record_target_offset=0x08`, `valid_targets` | `VALIDATED` |
| `DOMAIN53_CRYPTOLOGY_RECORD_DECRYPTING` | `0x12` | `probe_870ebf_intel.json::cryptology.record_decrypting_byte_offset=0x12` | `VALIDATED` |
| `DOMAIN53_CRYPTOLOGY_RECORD_AMOUNT` | `0x18` | `probe_870ebf_intel.json::cryptology.record_amount_offset=0x18`, `amount_rejects=0` | `VALIDATED` |
| `DOMAIN53_OPERATIVE_HANDLE_TYPE` | `4713` | `probe_870ebf_intel.json::operatives.handle_type=4713` | `VALIDATED` |
| `DOMAIN53_OPERATIVE_VTABLE_RVA` | `0x293F790` | `probe_870ebf_intel.json::operatives.active_classdesc_rvas[0]=0x293F790`, `active_operatives=24` | `VALIDATED` |
| `DOMAIN53_OPERATIVE_IDENTITY_FROM_VTABLE` | `0x08` | `probe_870ebf_operative_lifecycle_groups.json::identity_offsets.classdesc=0x-8`, equivalent leader-to-identity delta `+0x08` | `VALIDATED` |
| `DOMAIN53_OPERATIVE_NAME` | `0x18` | `probe_870ebf_operative_lifecycle_groups.json::identity_offsets.name=0x18` | `VALIDATED` |
| `DOMAIN53_OPERATIVE_NAME_DUPLICATE` | `0x38` | `probe_870ebf_operative_lifecycle_groups.json::identity_offsets.name_duplicate=0x38` | `VALIDATED` |
| `DOMAIN53_OPERATIVE_DETECTED` | `0x68` | `probe_870ebf_operative_lifecycle_groups.json::identity_offsets.detected=0x68` | `VALIDATED` |
| `DOMAIN53_OPERATIVE_LIFECYCLE_FLAG` | `0x110` | `probe_870ebf_operative_lifecycle_groups.json::identity_offsets.lifeflag=0x110`, `status=operative_lifecycle_groups_validated` | `VALIDATED` |
| `DOMAIN53_OPERATIVE_OWNER` | `0x118` | `probe_870ebf_operative_lifecycle_groups.json::identity_offsets.owner=0x118`, owner-valid lifecycle groups warm-validated | `VALIDATED` |
| `DOMAIN53_OPERATIVE_LEADER_FROM_IDENTITY_DELTA` | `-0x08` | `probe_870ebf_operative_lifecycle_groups.json::skill_xp_offsets.leader_from_identity=0x-8` | `VALIDATED` |
| `DOMAIN53_OPERATIVE_SKILL_SUBOBJECT` | `0xE60` | `probe_870ebf_operative_lifecycle_groups.json::skill_xp_offsets.skill_subobject=0xE60` | `VALIDATED` |
| `DOMAIN53_OPERATIVE_EXPERIENCE_RAW` | `0xE68` | `probe_870ebf_operative_lifecycle_groups.json::skill_xp_offsets.xp_raw=0xE68` | `VALIDATED` |
| `DOMAIN53_OPERATIVE_SKILL_LEVEL` | `0x1B8` | `probe_870ebf_operative_lifecycle_groups.json::skill_xp_offsets.skill_level_in_subobject=0x1B8` | `VALIDATED` |
| `DOMAIN53_WAR_FIRST_TAG` | `0x00` | `probe_870ebf_war_shape.json::scan.old_shape_offsets.first_tag_id=0x0`, `warm_validation.status=validated` | `VALIDATED` |
| `DOMAIN53_WAR_SECOND_TAG` | `0x04` | `probe_870ebf_war_shape.json::scan.old_shape_offsets.second_tag_id=0x4`, `warm_validation.status=validated` | `VALIDATED` |
| `DOMAIN53_WAR_DATE_VTABLE` | `0x08` | `probe_870ebf_war_shape.json::scan.old_shape_offsets.type_ptr=0x8`, current date RTTI rows warm-validated | `VALIDATED` |
| `DOMAIN53_WAR_FIRST_CASUALTIES` | `0x40` | `probe_870ebf_war_shape.json::scan.old_shape_offsets.first_casualties_i32=0x40`, current row shape accepted | `VALIDATED` |
| `DOMAIN53_WAR_SECOND_CASUALTIES` | `0x48` | `probe_870ebf_war_shape.json::scan.old_shape_offsets.second_casualties_i32=0x48`, current row shape accepted | `VALIDATED` |
| `DOMAIN53_WAR_ECHO_FIRST` | `0x68` | `probe_870ebf_war_shape.json::scan.old_shape_offsets.echo_first_second=0x68/0x6C`, echo checked by warm validation | `VALIDATED` |
| `DOMAIN53_WAR_ECHO_SECOND` | `0x6C` | `probe_870ebf_war_shape.json::scan.old_shape_offsets.echo_first_second=0x68/0x6C`, echo checked by warm validation | `VALIDATED` |
| `DOMAIN53_WAR_GREGORIAN_DATE_VTABLE_RVA` | `0x2702190` | `probe_870ebf_war_shape.json::date_rtti.classes.CGregorianDate.derived_vtables[].rva=0x2702190` | `VALIDATED` |
| `DOMAIN53_WAR_GAME_DATE_VTABLE_RVA` | `0x2702388` | `probe_870ebf_war_shape.json::date_rtti.classes.CGameDate.derived_vtables[].rva=0x2702388`; second `CGameDate` vtable is not consumed | `VALIDATED` |
| `DOMAIN53_HEAP_LO` | `0x10000000000` | Probe heap scans use this current-process lower guard (`probe_870ebf_intel.py`, `probe_870ebf_war_shape.py`); extraction uses it only as a pointer/range sanity guard | `STRUCTURAL-ONLY` |
| `DOMAIN53_HEAP_HI` | `0x7FF000000000` | Probe heap scans use this current-process upper guard (`probe_870ebf_intel.py`, `probe_870ebf_war_shape.py`); extraction uses it only as a pointer/range sanity guard | `STRUCTURAL-ONLY` |
| `DOMAIN53_MAX_COUNTRY_TABLE_SLOTS` | `4096` | `probe_870ebf_diplomacy.json::roots.country_count=440`; this is a safety cap, not a decoded field | `STRUCTURAL-ONLY` |
| `DOMAIN53_MAX_VECTOR_ROWS` | `8192` | `probe_870ebf_intel.json::cryptology.records_scanned=24`; this is a safety cap for manager sizes, not a decoded field | `STRUCTURAL-ONLY` |

## Runtime Gates By Sub-Scope

- Build identity: enabled. Per frame resolves MSVC RTTI from
  `module+0x27D0E90` and `module+0x296B338` through the complete-object-locator
  and type-descriptor chain and requires decorated names that undecorate to
  `CCountry` and `CIntelligenceAgency` before
  `SOURCE_PROOF_BUILD_SHA_870EBF` is stamped. This proves the current RTTI
  identity for the two anchor vtables; it does not hash the executable bytes.
- Country roots: enabled. Per frame requires heap roots, country table count in
  range, exact `CCountry` vtable `module+0x27D0E90`, valid tag-db id, and tag id
  uniqueness. Empty country root set is a hard fail.
- Faction group: enabled as structural. Per frame first requires valid country
  roots, then reads `country+0x598` for every validated country root. The
  sub-scope emits country diplomacy rows and faction state events only if all
  reads are bounded integers, at least 40 countries are covered, at least one
  nonzero group exists, and at least one nonzero group has multiple members. If
  the gate fails, faction group values are absent and
  `STATUS_FACTION_GROUP_UNVALIDATED` is set.
- Relations: base manager/table/opinion chain is value-validated against the
  D04 melt. Per frame requires valid country roots first, relation
  manager/table heap pointers, plausible opinion range, exact current RTTI
  relation vtables, known relation kind, and valid source and target tags. Zero
  worldwide relation rows is a hard fail. Relation flags and subject/autonomy
  rows still rely on their per-row structural gates until their individual
  serialized values are fixture-closed.
- Subjects: enabled as validated relation rows. Per frame requires exact
  `CSubjectRelation` vtable discovery through the relation scan, valid overlord
  and subject tags, non-self pair, a full-width i64 serialized freedom-level read bounded
  to `0..=100000`, and hashed autonomy state name only when readable. The compact
  carrier remains i32 only after lossless range-checked narrowing.
- Agencies and capacity: enabled. Per frame requires exact
  `CIntelligenceAgency` vtable, bounded raw scalars, readable strings when
  present, and raw capacity `+0xE4/+0xF0/+0xF4` with `+0xE4 == +0xF4`. Capacity
  semantic labels remain unresolved; raw cluster rows carry the values.
- Cryptology: enabled. Per frame requires exact `CCryptology` and
  `CCountryDecryptionState` vtables, bounded manager size/capacity, valid target
  tag, and bounded raw amount.
- Operatives and lifecycle groups: enabled. Per frame requires exact
  `COperativeLeader` vtable, identity handle type `4713`, valid operative id,
  valid owner tag, bounded lifecycle flag, duplicate-name consistency, bounded
  skill and XP, and owner membership in the validated country root set.
- War shape/date rows: enabled. Per frame requires current date vtable equality,
  valid distinct tag pair, bounded casualties, and `+0x68/+0x6C` echo tags.
  Frames with war rows must still carry
  `STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED`; war grouping/name/start-date
  semantics remain open.
- Operation detail: fail-closed. The current operation definition/index probes
  reject stale and definition-only routes. No operation rows are emitted unless
  `SOURCE_PROOF_OPERATION_DETAIL_870EBF` is present; absent rows still set
  `STATUS_OPERATIONS_DETAIL_RESISTED`.
- Intel-map percentages and spy-network graph: fail-closed. No current live
  owner/target percentage or spy-network root is accepted. No rows are emitted
  unless `SOURCE_PROOF_INTEL_MAP_PERCENTAGES_870EBF` is present; absent rows
  still set `STATUS_INTEL_MAP_RESISTED`.
- Incoming diplomatic action queue: fail-closed. No current queue root is
  accepted; frames without `SOURCE_PROOF_INCOMING_ACTION_QUEUE_870EBF` set
  `STATUS_INCOMING_ACTION_QUEUE_UNVALIDATED`.

## Rejection Evidence

- `probe_870ebf_operations.json::summary.accepted_for_operation_detail=false`
  and `status=definition_registry_current_but_operation_detail_unvalidated`.
- `probe_870ebf_operation_definition_index.json::summary.accepted_for_running_operation_detail=false`;
  current `COperation` heap hits are accounted for by the definition table, not
  running-operation state.
- `probe_870ebf_war_lifecycle_date_scalar.json` supports a candidate scalar at
  `row+0x10` but still records the blocker: same-tick UI/save evidence is
  required before naming it as war start date. No constant is enabled for that
  scalar in this patch.
