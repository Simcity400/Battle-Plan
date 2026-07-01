# Occupation Resistance Compliance Live Registration Patch Plan

Lane: `occupation_resistance_compliance_live`

Compact domain: `52`

This is a registration-only plan for the shared production path. The lane-owned
implementation already lives under
`extractor/native/lanes/occupation_resistance_compliance_live/`; shared entrypoints
should only name and call it once the orchestrator grants the integration slot.

Current-HEAD refresh: `bc0797784`, 2026-07-01.

## Current Shared State

- `extractor/native/DOMAIN_REGISTRY.md` and
  `extractor/native/domain_registry.json` assign id `52` to
  `occupation_resistance_compliance_live` with status `reserved`.
- `extractor/native/recorder_host/src/compact_domain_registry.rs` includes id `52`
  as `CompactDomainStatus::Reserved`, `schedulable = false`,
  `requires_data_record = true`, and no data decoder.
- `extractor/native/collector/src/lib.rs` has no scheduled/raw domain token or
  compact emission hook for `occupation_resistance_compliance_live`. Numeric
  `52` hits there are payload offsets or `DIVISION_TEMPLATE_TYPE`, not a
  compact domain assignment.

Current shared registration blocker:

- Domain id `52` is assigned/reserved, but not schedulable.
- The collector has no `occupation_resistance_compliance_live` scheduled token,
  compact domain id constant, tracker, or hourly emit hook.
- The recorder host has no domain-52 data decoder/dispatch arm, so a shared
  domain-52 compact stream would still hit the unsupported-domain parse-error
  path.
- Therefore no lane-local collector or decoder code should be copied into shared
  entrypoints until the orchestrator opens the registration slot.

## Current-HEAD Lane Proof

Proof directory:
`extractor/native/lanes/occupation_resistance_compliance_live/proof_current_head_20260701_bc0797784/`

Fresh live proof command:

```powershell
python extractor/native/lanes/occupation_resistance_compliance_live/live_probe.py --out-dir extractor/native/lanes/occupation_resistance_compliance_live/proof_current_head_20260701_bc0797784/live_probe
```

Result:

- Build SHA-256:
  `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.
- Domain id `52`; compact stream `69404` bytes; record types
  `domain_perf`, `domain_data`, `frame_end`.
- State roots `1081`; province rows `13413`; invalid province/state rows `0`.
- Occupied rows `319`; wall-badged rows `319`; law-token rows `1030`;
  garrison-template rows `279`; invalid state rows `0`.

Fresh compact gate command:

```powershell
rustc --edition=2021 extractor/native/lanes/occupation_resistance_compliance_live/domain52_gate.rs -o extractor/native/lanes/occupation_resistance_compliance_live/proof_current_head_20260701_bc0797784/domain52_gate.exe
& .\extractor\native\lanes\occupation_resistance_compliance_live\proof_current_head_20260701_bc0797784\domain52_gate.exe --stream extractor/native/lanes/occupation_resistance_compliance_live/proof_current_head_20260701_bc0797784/live_probe/collector_stream.bin
```

Result:

- `expected_domain_id=52`
- `records_seen=3`
- `domain_data_records=1`
- `rows=1081`
- `occupation_observed_state_rows=1081`
- `occupation_occupied_state_rows=319`
- `occupation_wall_badge_rows=319`
- `occupation_law_token_rows=1030`
- `occupation_garrison_template_rows=279`
- no parse errors.

Fresh replay verification command:

```powershell
cargo test --manifest-path extractor/native/lanes/occupation_resistance_compliance_live/replay_verify/Cargo.toml
cargo run --manifest-path extractor/native/lanes/occupation_resistance_compliance_live/replay_verify/Cargo.toml -- --run-dir extractor/native/lanes/occupation_resistance_compliance_live/proof_current_head_20260701_bc0797784/live_probe --game-id orc52
```

Result:

- `replay_verify` tests: `2 passed`.
- `verified=true`.
- `.battleplan`:
  `extractor/native/lanes/occupation_resistance_compliance_live/proof_current_head_20260701_bc0797784/live_probe/native_session.battleplan`.
- Replay bytes `9248`.
- Compact stream entry:
  `payload/orc52/collector_raw/collector_stream.bin`.
- Byte-compared compact stream bytes: `69404`.

Fresh bounded remaining-field probes:

- `garrison_scalar_probe.py`: v23 current live/save pair has `368` save
  garrison rows, `305` live garrison rows, `296` key matches, `295` same-law
  matches, and verdict `scanned_garrison_candidates_rejected_current_fixture`.
  Inline candidates for `strength_ratio`, `garrison_required`, and
  `army_manpower_need` are rejected; direct/root pointer candidates from
  `CStateGarrisonData+0x0..0x17F` are rejected; and manpower/equipment detail
  candidates for `manpower_pool` value, `equipment_need` values, and
  reinforcement request count all have `0` threshold scopes. The fixture has
  `340` manpower-pool rows, all single-tag and all matching the emitted
  controller tag. Keep garrison scalar/manpower/equipment detail badged
  unavailable.
- `remaining_fields_probe.py`: v23 resistance speed `0x280` validated `92/92`
  delta rows; compliance speed `0x2B0` validated `277/277` delta rows. The
  occupation object graph remains validated for state/tag/law joins:
  `CStateGarrisonData+0x50` is the `CState` join and `+0x58` is the
  `CCountryOccupationData` join; `+0x48` again had `0` controller-country and
  `0` occupied-country matches. Collaboration/GIE detail remains unwired but
  current candidates are rejected: `countries.<TAG>.collaboration` is empty,
  all `101/101` `government_in_exile_tag` rows are asset-scope rows, and the
  targeted country-scope GIE graph over
  `BEL/ENG/FRA/HOL/LUX/NOR/PHI/POL/USA/YUG` has `10/10` valid
  collaboration-status roots, `1` `CCountryCollaborationData` hit, `16`
  scalar/tag hits, and `0` contract-match scopes. Verdict:
  `rejected_targeted_scalar_hits_no_gie_contract_current_fixture`.

## Collector Hook

When the lane receives the shared integration slot:

1. Include `collector_module.rs` as the lane implementation module. Do not copy
   offsets or row logic into the monolithic collector.
2. Add the scheduled/raw token
   `occupation_resistance_compliance_live`.
3. Register compact domain id `52` for that scheduled token.
4. Add one `CompactOccupationResistanceComplianceTracker` to the collector's
   compact tracker state.
5. At the scheduled hourly compact boundary, call:
   - `validate_build(EXPECTED_SHA256)`;
   - `resolve_state_roots` or the coordinator-provided `CProvince -> CState`
     topology;
   - `extract_rows_from_state_roots`;
   - `CompactOccupationResistanceComplianceTracker::encode_frame`.
6. Emit the resulting domain-52 payload as a normal compact domain data record,
   followed by existing frame/perf records.
7. Preserve fail-closed badges for unresolved garrison scalar, collaboration,
   and government-in-exile detail fields.

## Recorder Hook

When the lane receives the shared integration slot:

1. Promote registry id `52` from `Reserved` to the active integration/production
   status chosen by the orchestrator and set `schedulable = true`.
2. Include or port the checks in `domain52_compact.rs` into the recorder-host
   compact decoder.
3. Add a domain-data dispatch arm for id `52` in `summarize_compact_stream`.
4. Add semantic gate units:
   - `occupation_state_rows`
   - `occupation_observed_state_rows`
   - `occupation_occupied_state_rows`
   - `occupation_wall_badge_rows`
   - `occupation_law_token_rows`
   - `occupation_garrison_template_rows`
5. Keep the fail-closed rejection that forbids nonzero garrison scalar fields
   while `ROW_FLAG_GARRISON_SIGNALS_UNAVAILABLE` is set.
6. Require domain `52` in the expected compact domain set when
   `occupation_resistance_compliance_live` is scheduled.
7. Reuse the recorder's existing native `.battleplan` packing path. The
   lane-local verifier proves the domain-52 compact stream already round-trips
   through that container shape.

## Acceptance Proof To Run After Registration

Run the shared DLL/recorder path with scheduled domains including
`map_control`, `state_detail`, and `occupation_resistance_compliance_live`.

Acceptance requires:

- DLL-only capture from `hoi4.exe`, not Python/save hot-loop capture.
- Compact stream contains domain `52` data records.
- Rust recorder decodes domain `52` with no parse errors.
- Native `.battleplan` opens, verifies all entries, and contains the compact
  stream unchanged.
- Speed-five or current-equivalent cadence is hourly-or-lower.
- Zero skipped game hours.
- No source unavailable, partial data, worker panic, or bridge unavailable
  flags.
