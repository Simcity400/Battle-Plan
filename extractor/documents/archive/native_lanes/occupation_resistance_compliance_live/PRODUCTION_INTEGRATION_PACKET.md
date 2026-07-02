# Occupation Resistance Compliance Live Lane Reference

Lane: `occupation_resistance_compliance_live`

Status: lane-local collector/compact/decoder/replay-verifier mini stack with
blocker evidence, not shared-integrated. Shared collector/recorder entrypoints
are untouched until the orchestrator grants the integration slot.

## Registry Assignment

Use compact domain id `52`, per `extractor/native/DOMAIN_REGISTRY.md` and
`extractor/native/domain_registry.json`.

Current shared constants already consume:

- `COMPACT_DOMAIN_SUPPLY_LOGISTICS_ATTRITION_LIVE = 44`
- `COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE = 45`

Orchestrator-reserved lower ids for other lanes:

- `46`: air
- `47`: characters
- `48`: construction
- `49`: naval combat
- `50`: naval combat member/detail
- `51`: scripted-state

The older lane-local proof used ids `44` and `46`; lane-local code has been
advanced to `52` so the eventual shared patch does not collide with supply,
focus/research/decisions, air, characters, construction, naval combat, or
scripted-state.

Current shared-file availability proof, checked for this lane update:

- `extractor/native/collector/src/lib.rs` has no
  `occupation_resistance_compliance_live` scheduled/raw token and no compact
  emission hook for domain `52`.
- `extractor/native/recorder_host/src/compact_domain_registry.rs` reserves domain
  `52`, but it is `CompactDomainStatus::Reserved`, `schedulable = false`, and
  has no decoder module.
- `extractor/native/recorder_host/src/main.rs` has no domain-data dispatch arm for
  `52`; an unregistered domain-52 stream would hit the unsupported-domain
  parse-error path.
- Numeric `52` occurrences in current shared files are payload offsets,
  `DIVISION_TEMPLATE_TYPE`, or test values, not compact domain assignments.

Current-HEAD lane-local proof:

- `extractor/native/lanes/occupation_resistance_compliance_live/proof_current_head_20260701_bc0797784`
  refreshed the lane against HEAD `bc0797784` and live build
  `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.
- `live_probe.py`: domain `52`, `1081` state rows, `319` occupied rows,
  `279` garrison-template rows, `0` invalid state rows, and `69404` compact
  stream bytes.
- `collector_module.rs` tests: `9 passed`; `domain52_compact.rs` tests:
  `5 passed`; `replay_verify` tests: `2 passed`.
- `domain52_gate.rs --stream live_probe/collector_stream.bin` decoded
  `records_seen=3`, `domain_data_records=1`, `rows=1081`, with no parse errors.
- `replay_verify --run-dir live_probe --game-id orc52` wrote
  `native_session.battleplan`, verified all entries through
  `snapshot::Container`, and byte-compared `69404` compact stream bytes
  unchanged.
- `remaining_fields_probe.py` now validates resistance speed `0x280`
  (`85/85` delta rows) and compliance speed `0x2B0` (`224/224` delta rows).
  `CStateGarrisonData+0x48` remains rejected as country identity.
- `garrison_scalar_probe.py` keeps pinned garrison scalar candidates rejected
  or fixture-blocked; v23 additionally rejects direct/root scalar candidates,
  manpower-pool value, equipment-need values, and reinforcement request count
  for the current fixture.
- Collaboration/GIE detail remains fixture-blocked because the save fixture has
  empty country collaboration payloads and the targeted v23 live scalar/tag hits
  do not satisfy the expected host plus target country or unique-legitimacy
  contract.

Earlier lane-local proof:

- `extractor/native/runs/occupation_resistance_compliance_live_870ebf_20260701_v17_replay_verify/collector_stream.bin`
  contains compact records with domain `52` and frame domain `1000`.
- `domain52_gate.exe --stream collector_stream.bin` decoded `1081` rows and
  `1` domain-data record with `expected_domain_id=52`.
- `replay_verify` packed that stream into `native_session.battleplan`; the
  shared container reader verified all entries and byte-compared `69404`
  compact stream bytes unchanged.

## Production Roots

Build-gated current executable:
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`

State roots:

- `hoi4.exe + 0x3316E90 -> CGameState*`
- `CGameState + 0x2B0 -> CProvince**`
- `CGameState + 0x2BC -> province_count`
- `CProvince + 0xC0 -> CState*`
- `CState + 0x58 -> state_id`
- `CState + 0xC8 -> owner tag_db_id`
- `CState + 0xCC -> controller tag_db_id`
- `CState + 0x278 -> resistance_raw`, scale `100000`
- `CState + 0x280 -> resistance_speed_raw`, scale `100000`
- `CState + 0x2A8 -> compliance_raw`, scale `100000`
- `CState + 0x2B0 -> compliance_speed_raw`, scale `100000`

Occupation roots:

- `CGameState + 0x310 -> CCountry**`
- `CGameState + 0x31C -> country_count`
- `CCountry + 0xFD0 -> CCountryOccupationStatus*`
- `CCountryOccupationStatus` vtables `module+0x296DD50/+0x296DD88`
- `CCountryOccupationStatus + 0x118 -> COccupationLaw*`
- `COccupationLaw` vtable `module+0x2928218`; id string at `+0x10`
- `CCountry + 0xFD8 -> CCountryCollaborationStatus*`
- `CCountryCollaborationStatus` vtable `module+0x29A5D58`
- `CCountryOccupationStatus + 0x48` occupation-data container
- `CCountryOccupationData` vtable `module+0x296DD00`
- `CCountryOccupationData + 0x08 -> CCountryOccupationStatus*`
- `CCountryOccupationData + 0x10 -> occupied/original tag_db_id`
- `CCountryOccupationData + 0x20` state-list container
- `CCountryOccupationData + 0xB0` law-list container
- `CCountryOccupationData + 0xD8` garrison-list container
- `CStateGarrisonData` vtable `module+0x296DCB0`
- `CStateGarrisonData + 0x50 -> CState*`
- `CStateGarrisonData + 0x58 -> CCountryOccupationData*`

Rejected/narrowed:

- `CStateGarrisonData + 0x48` is not controller-country or occupied-country
  identity.
- `CCountryOccupationStatus + 0x78 -> CDivisionTemplate*` is the garrison
  template root. Fresh autosave/live proof matched `102/102` comparable rows
  with `0` mismatches.
- `CCountryOccupationStatus + 0x80` is rejected as the garrison-template root
  for current `870ebf`; the fresh autosave/live proof had `99` mismatches.
- Garrison scalar/manpower/equipment candidates are narrowed by the v23 fixture
  proof. The current save/live pair produced `368` save garrison rows, `305`
  live rows, `296` key matches, and `295` same-law matches. Inline candidates
  for `strength_ratio`, `garrison_required`, and `army_manpower_need` were
  rejected; the same fields were rejected across `3374` direct/root pointer
  scans from `CStateGarrisonData+0x0..0x17F`; and detail candidates for
  `manpower_pool` value, `equipment_need` values by equipment token, and
  reinforcement request count had `0` threshold scopes. The fixture has `340`
  manpower-pool rows, all single-tag and all matching the already emitted
  controller tag, so it does not prove an independent garrison manpower vector.
  Do not wire these candidates. Remaining garrison detail semantics require a
  same-hour or one-variable controlled fixture before any nonzero
  scalar/manpower/equipment payload can be emitted.
- Collaboration/GIE detail is narrowed by the v23 stable autosave fixture. The
  occupation oracle reports `countries.<TAG>.collaboration` empty for every
  country. The melt contains `101` `government_in_exile_tag` rows, but the
  probe classifies all `101/101` as asset scope (`20` leader, `58` ship, `3`
  deployment, `20` air wing), so those keys are rejected as
  occupation/collaboration country detail. The same melt contains `8`
  country-scope GIE rows with `legitimacy` and `exile_army_leaders`; the
  targeted live graph over `BEL/ENG/FRA/HOL/LUX/NOR/PHI/POL/USA/YUG` has
  `10/10` valid collaboration-status roots, `1` `CCountryCollaborationData`
  hit, and `16` scalar/tag hits, but `0` contract-match scopes. The current
  scalar/tag hits are rejected for this fixture.

## Row Emission Contract

Emit one row per validated `CState` root.

Lane-owned implementation files:

- `collector_module.rs`: collector-side Rust extraction module, exact offsets,
  build gate, fail-closed row badges, compact encoder call, and baseline/delta
  tracker.
- `domain52_compact.rs`: domain-52 compact schema and decoder checks.
- `domain52_gate.rs`: lane-local payload/stream gate.
- `replay_verify/`: lane-local native `.battleplan` packing and verification
  path for the domain-52 compact stream.
- `REGISTRATION_PATCH_PLAN.md`: registration-only shared patch plan for the
  collector/coordinator and recorder host.

Measured fields:

- owner/controller tag-db ids
- occupied flag from owner/controller mismatch plus nonzero
  resistance/compliance context
- occupied-country tag-db id derived from owner when occupied
- resistance/compliance raw
- resistance/compliance speed raw
- controller default occupation-law token when `CCountryOccupationStatus`
  validates
- controller garrison-template id on occupied rows when
  `CCountryOccupationStatus + 0x78` validates as a `CDivisionTemplate*`
- per-state occupation-law token only when a same-offset pair validates:
  `CCountryOccupationData + 0xB0` has `COccupationLaw*` and
  `CCountryOccupationData + 0xD8` has `CStateGarrisonData*` at the same entry
  offset, and `CStateGarrisonData + 0x50/+0x58` join back to the row state and
  occupation-data object.

Fail closed:

- If no validated same-offset law/garrison pair exists for a state, keep
  `ROW_FLAG_OCCUPATION_LAW_UNAVAILABLE`; a controller default law token may
  still be present with `ROW_FLAG_COUNTRY_DEFAULT_LAW_TOKEN`.
- Keep `ROW_FLAG_GARRISON_SIGNALS_UNAVAILABLE` while
  `garrison_need_raw`, `garrison_equilibrium_raw`, and
  `garrison_progress_raw` are zero.
- Keep `garrison_template_id = 0` and no `ROW_FLAG_GARRISON_TEMPLATE` on
  non-occupied rows, or when `CCountryOccupationStatus + 0x78` is absent or
  does not validate as a `CDivisionTemplate*`.

The Rust gate must reject any row that sets unavailable garrison signals while
carrying nonzero garrison scalar values.

## Registration-Only Collector Patch Plan

When this lane receives the integration slot:

1. Add `RAW_OCCUPATION_RESISTANCE_COMPLIANCE_LIVE_DOMAIN =
   "occupation_resistance_compliance_live"`.
2. Add `COMPACT_DOMAIN_OCCUPATION_RESISTANCE_COMPLIANCE_LIVE: u16 = 52`.
3. Add a `CompactOccupationResistanceComplianceTracker` keyed by `state_id`.
4. Wire scheduling beside `map_control`/`state_detail` because this lane reuses
   the same `CProvince -> CState` topology.
5. Build the domain-52 payload with the exact `HEADER_LEN = 52` and
   `ROW_LEN = 64` layout in `domain52_compact.rs`.
6. Emit baseline rows, then delta rows by `state_id`, preserving sorted state
   order.
7. Do not emit guessed garrison scalar data or unvalidated template data.

## Registration-Only Recorder Patch Plan

When this lane receives the integration slot:

1. Add compact domain name/id mapping for `52`.
2. Port or include the `decode_payload` checks from `domain52_compact.rs`.
3. Add gate units:
   - `occupation_state_rows`
   - `occupation_observed_state_rows`
   - `occupation_occupied_state_rows`
   - `occupation_wall_badge_rows`
   - `occupation_law_token_rows`
   - `occupation_garrison_template_rows`
4. Require no parse errors before `.battleplan` acceptance.

## Acceptance Gate

Required before marking production complete:

- DLL-only capture path.
- Compact stream contains domain `52` data records.
- Rust recorder decodes domain `52` without parse errors.
- `.battleplan` pack verifies.
- Speed-five or current-equivalent hourly-or-lower cadence.
- Zero skipped game hours.
- No source/partial/worker/bridge failures.

## Latest Lane-Local Proof

`extractor/native/runs/occupation_resistance_compliance_live_870ebf_20260701_v17_replay_verify`:

- `collector_module.rs` tests: `9 passed`.
- `domain52_compact.rs` tests: `5 passed`.
- `replay_verify` tests: `2 passed`.
- live domain-52 payload: `1081` rows, `306` occupied rows, `269`
  garrison-template rows, `0` invalid rows.
- `domain52_gate.rs --stream collector_stream.bin`: `expected_domain_id=52`,
  `records_seen=3`, `domain_data_records=1`, `rows=1081`, no parse errors.
- `replay_verify --run-dir ... --game-id orc52`: `verified=true`,
  `replay_bytes=8841`, `collector_compact_stream_bytes=69404`,
  `index_snapshot_count=0`, stream entry
  `payload/orc52/collector_raw/collector_stream.bin`.

`extractor/native/runs/occupation_resistance_compliance_live_870ebf_20260701_v19_semantic_gaps`:

- `parse_occupation.py`: save date `1946.5.19.9`, `143` countries with
  occupation data, `12` active occupiers, `60` occupation pairs, `566`
  per-state law rows, and empty country collaboration payloads.
- `garrison_scalar_probe.py`: rejected the pinned `CStateGarrisonData` scalar
  candidates listed above; remaining scalar search is fixture-blocked by only
  `18` same-law comparable rows.
- `remaining_fields_probe.py`: `1081` state roots, `432` valid occupation
  status roots, `77` valid garrison-template roots, `432` valid collaboration
  status roots, `170` valid state-garrison rows, `409` per-state law pairs
  with state join, and `1` reachable collaboration-data object. Current GIE
  assignment keys are leader scope, not occupation/collaboration payload.

`extractor/native/runs/occupation_resistance_compliance_live_870ebf_20260701_v21_stable_autosave_residuals`:

- `parse_occupation.py`: stable snapshot save date `1942.10.1.1`, `439`
  countries with occupation data, `30` active occupiers, `152` occupation pairs,
  and `399` per-state law rows.
- `live_probe.py`: build
  `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`,
  `1081` state rows, `381` occupied rows, `345` garrison-template rows,
  `1042` law-token rows, `0` invalid state rows, compact stream bytes `69404`.
- `domain52_gate.rs --stream live_probe/collector_stream.bin`:
  `expected_domain_id=52`, `records_seen=3`, `domain_data_records=1`,
  `rows=1081`, no parse errors.
- `replay_verify --run-dir live_probe --game-id orc52v21`: `verified=true`,
  byte-compared `69404` compact stream bytes unchanged inside
  `native_session.battleplan`.
- `remaining_fields_probe.py`: resistance speed `0x280` validated `140/140`
  delta rows; compliance speed `0x2B0` validated `271/271` delta rows;
  `CStateGarrisonData+0x50/+0x58` state/occupation-data joins remain
  validated; `+0x48` remains rejected for country identity. Collaboration/GIE
  verdict:
  `fixture_has_country_gie_payload_unjoined_to_live_collaboration_status`.
  Targeted GIE live graph over `BEL/ENG/FRA/HOL/LUX/NOR/PHI/POL/YUG` found
  `9/9` valid collaboration-status roots, `1` `CCountryCollaborationData` hit,
  and `13` ambiguous scalar/tag hits; verdict
  `candidate_scalar_hits_require_controlled_fixture`. This v21 candidate status
  is superseded by the v23 rejection proof above.
- Existing shared DLL-only stream
  `extractor/native/runs/occupation_resistance_compliance_live_shared_20260701T165935`
  verified through `shared_stream_pack`: `81` domain-data frames, `80` game
  hours advanced, `0` skipped hours, no source/partial/worker/bridge flags, and
  `.battleplan` verification succeeded.

`extractor/native/runs/occupation_resistance_compliance_live_870ebf_20260701_v23_garrison_child_roots`:

- `parse_occupation.py`: fresh snapshot save date `1943.5.1.1`, `439`
  countries with occupation data, `31` active occupiers, `163` occupation pairs,
  and `443` per-state law rows.
- `remaining_fields_probe.py`: resistance speed `0x280` validated `92/92`
  delta rows; compliance speed `0x2B0` validated `277/277` delta rows;
  `CStateGarrisonData+0x50/+0x58` state/occupation-data joins remain
  validated; `+0x48` remains rejected for country identity. Collaboration/GIE
  verdict:
  `fixture_has_country_gie_payload_unjoined_to_live_collaboration_status`.
  Targeted GIE live graph over `BEL/ENG/FRA/HOL/LUX/NOR/PHI/POL/USA/YUG` found
  `10/10` valid collaboration-status roots, `1` `CCountryCollaborationData`
  hit, `16` scalar/tag hits, and `0` contract-match scopes; verdict
  `rejected_targeted_scalar_hits_no_gie_contract_current_fixture`.
- `garrison_scalar_probe.py`: verdict
  `scanned_garrison_candidates_rejected_current_fixture`; inline scalar,
  direct/root scalar, manpower-pool value, equipment-need value, and
  reinforcement-count candidates all had `0` threshold scopes.
