# Occupation Resistance Compliance Live Registration Patch Plan

Lane: `occupation_resistance_compliance_live`

Compact domain: `52`

This is a registration-only plan for the shared production path. The lane-owned
implementation already lives under
`replay_native/lanes/occupation_resistance_compliance_live/`; shared entrypoints
should only name and call it once the orchestrator grants the integration slot.

## Current Shared State

- `replay_native/DOMAIN_REGISTRY.md` and
  `replay_native/domain_registry.json` assign id `52` to
  `occupation_resistance_compliance_live` with status `reserved`.
- `replay_native/recorder_host/src/compact_domain_registry.rs` includes id `52`
  as `CompactDomainStatus::Reserved`, `schedulable = false`,
  `requires_data_record = true`, and no data decoder.
- `replay_native/collector/src/lib.rs` has no scheduled/raw domain token or
  compact emission hook for `occupation_resistance_compliance_live`. Numeric
  `52` hits there are payload offsets or `DIVISION_TEMPLATE_TYPE`, not a
  compact domain assignment.

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
