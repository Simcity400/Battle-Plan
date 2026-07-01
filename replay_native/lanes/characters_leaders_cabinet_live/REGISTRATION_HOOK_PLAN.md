# characters_leaders_cabinet_live domain-47 registration hook plan

Domain registry assignment:

- Final domain: `47 = characters_leaders_cabinet_live`.
- `replay_native/domain_registry.json` currently assigns id `47` to this lane
  with status `lane_ready_shared_registration_blocked_remaining_matrix`.
- `replay_native/DOMAIN_REGISTRY.md` currently marks id `46` as
  `air_variant_archetypes_live` production-complete and id `47` as
  lane-ready with shared registration blocked.
- `DOMAIN47_REGISTRATION_ONLY.patch` is an apply-checkable HEAD
  `bc0797784` patch for registry activation, Cargo dependencies, and
  recorder-side domain-47 decode. It deliberately does not claim merge-ready
  collector extraction, because the shared collector still lacks the
  `CollectorState`/country-politics/character-root to lane `LaneFrame` adapter.

Lane-owned module now exists:

- Rust crate: `replay_native/lanes/characters_leaders_cabinet_live/Cargo.toml`.
- DLL-facing collector wrapper: `src/dll_collector.rs`.
- Extractor/root contract: `src/extractor.rs`.
- Compact schema/encoder/decoder gate: `domain47_compact.rs`, exported as
  `domain47_characters_leaders_cabinet_live`.
- Rust fixture gate: `src/bin/appointed_fixture_gate.rs`.
- LaneFrame adapter contract:
  `replay_native/lanes/characters_leaders_cabinet_live/LANEFRAME_ADAPTER_PLAN.md`.

Registration-only collector patch when this lane receives the shared slot:

1. Add a collector dependency:

   ```toml
   characters_leaders_cabinet_live_lane = { path = "../lanes/characters_leaders_cabinet_live" }
   ```

2. In `replay_native/collector/src/domain_registry.rs`, flip the existing id-47
   descriptor from `Reserved, schedulable=false` to active/schedulable:

   ```rust
   domain(
       47,
       "characters_leaders_cabinet_live",
       "characters_leaders_cabinet_live",
       CompactDomainStatus::ActiveIntegration,
       true,
       true,
       &[],
   )
   ```

3. In `replay_native/collector/src/lib.rs`, add only registration/scheduling
   glue after the lane-local adapter exists:

   - raw scheduled token:
     `RAW_CHARACTERS_LEADERS_CABINET_LIVE_DOMAIN =
     "characters_leaders_cabinet_live"`;
   - tracker field holding `characters_leaders_cabinet_live_lane::dll_collector::LaneCollectorModule`;
   - scheduled-hour branch gated by the registry/scheduled-domain token;
   - construct `characters_leaders_cabinet_live_lane::extractor::LaneFrame`
     from existing collector roots and existing politics idea data;
   - append `LaneCollectorModule::collect_frame(...).wrapped_record` to the
     compact stream.

4. Do not inline the domain-47 schema or decoder into `collector/src/lib.rs`.
   The shared collector should only pass roots/rows into the lane module.

Current collector blocker:

- `replay_native/collector/src/lib.rs` has the hook points, but no domain-47
  collector adapter exists yet.
- The exact hook points are:
  `build_compact_extraction_gate_records` scheduled enabled flags,
  read-handle locals, `thread::scope` spawn/join, domain perf/data append
  sequencing after `air_variant_archetypes`, and a payload append helper near
  `append_compact_air_variant_archetypes_data_record`.
- The lane crate exposes `LaneCollectorModule::collect_frame(game_hour_raw,
  &LaneFrame)`, but the shared collector cannot honestly call it until a real
  live `LaneFrame` is populated from `CollectorState`, country bridge records,
  politics active ideas, and validated character/leader/advisor roots. A
  placeholder empty/source-unavailable frame would not satisfy merge-ready
  acceptance.
- `LANEFRAME_ADAPTER_PLAN.md` now specifies the exact required shared inputs,
  row population flow, failure flags, and proof gate for that adapter.

Required collector inputs:

- current-build RTTI-derived type roots for `CCharacter`, `CArmyLeader`,
  `CNavyLeader`, `CCountryLeader`, `CAdvisor`, and `CUnitLeaderTrait`;
- country bridge records supplying `CCountry*` and tag database ids;
- politics active ideas from `CPoliticalStatus`;
- static character/advisor token closure for names and cabinet slots.

Registration-only recorder patch when this lane receives the shared slot:

1. Add a recorder dependency:

   ```toml
   characters_leaders_cabinet_live_lane = { path = "../lanes/characters_leaders_cabinet_live" }
   ```

2. In `replay_native/recorder_host/src/compact_domain_registry.rs`, flip the
   existing id-47 descriptor to the same active/schedulable shape as the
   collector.

3. Register a domain-47 decoder adapter that calls:

   ```rust
   characters_leaders_cabinet_live_lane::decode_payload(offset, payload)
   ```

   and merges the lane gate summary into the shared compact summary units:

   - `character_rows`
   - `unit_leader_rows`
   - `unit_leader_trait_rows`
   - `country_leader_role_rows`
   - `active_ruler_rows`
   - `advisor_roster_rows`
   - `appointed_advisor_rows`

4. If the shared decoder dispatcher is still match-based when this lane gets
   the slot, the only permitted temporary shared code is one domain-47 match arm
   that calls the lane adapter. Do not inline row parsing there.

`DOMAIN47_REGISTRATION_ONLY.patch` includes the exact match-based recorder
adapter, mirroring the existing domain-46 lane-summary merge pattern. It is
enough to make domain-47 recorder decode/gate and scheduled-domain coverage
available once collector data records exist; it is not enough to create those
records.

Live proof route once hooks are wired:

```powershell
cargo build --release --manifest-path replay_native\Cargo.toml -p collector -p recorder_host

replay_native\recorder_host\target\release\recorder_host.exe `
  --stream-format compact `
  --extraction-perf-gate `
  --scheduled-domain characters_leaders_cabinet_live `
  --duration-s 60 `
  --out-dir replay_native\runs\characters_leaders_cabinet_live_live_domain47_v1
```

Acceptance gates for that live run:

- domain `47` data record present every scheduled game hour;
- Rust decoder has zero parse errors;
- no `STATUS_WORKER_PANIC`, `STATUS_BRIDGE_UNAVAILABLE`,
  `STATUS_SOURCE_UNAVAILABLE`, or `STATUS_PARTIAL_DATA`;
- speed-five/current-equivalent hourly-or-lower cadence;
- zero skipped game hours;
- `.battleplan` verifies and its manifest includes
  `characters_leaders_cabinet_live`.
