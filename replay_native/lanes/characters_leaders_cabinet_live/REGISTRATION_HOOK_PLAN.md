# characters_leaders_cabinet_live domain-47 registration hook plan

Domain registry assignment:

- Final domain: `47 = characters_leaders_cabinet_live`.
- `replay_native/domain_registry.json` currently assigns id `47` to this lane
  with status `lane_ready_shared_adapter_registration_blocked_remaining_matrix`.
- `replay_native/DOMAIN_REGISTRY.md` currently marks id `46` as
  `air_variant_archetypes_live` production-complete and id `47` as
  lane-ready with shared registration blocked.
- `DOMAIN47_REGISTRATION_ONLY.patch` is an apply-checkable patch against the
  current worktree for registry activation, Cargo dependencies, and
  recorder-side domain-47 decode. `git apply --check --verbose` passes. It
  deliberately does not claim merge-ready collector extraction, because the
  shared collector still lacks the root cache and scheduled call that translate
  live `CollectorState`/country-politics/character roots into the lane-local
  adapter input.
- `DOMAIN47_SHARED_ADAPTER_HOOK_PLAN.md` contains the smallest remaining
  shared collector hook: populate `scheduled::NativeRootCache`, build
  per-frame `scheduled::NativeFrameInput` from shared roots, call
  `scheduled::read_native_scheduled_frame`, and append the domain-47 payload
  through the shared coordinator's sequence-owned `append_compact_record`
  helper. It now includes exact helper skeletons for `CollectorState` root-cache
  refresh and `CountryPoliticsRow` to `NativeFrameInput` population, so the
  orchestrator hook should only adapt names/placement to the shared file shape.
- The latest lane-local live proof writes and verifies a real current-process
  domain-47 compact record, but the Python root walk took `507.542` wall seconds
  while the game advanced `745` hours. That rejects another Python full-root
  scan as the production cadence path and narrows the remaining work to the
  native root cache plus scheduled adapter call.
- The release native scheduled benchmark runs the lane-owned hot loop for `120`
  frames at live-ish row scale in `49304us` average / `102401us` max per frame
  and verifies its last compact record through `.battleplan`; the cadence
  budget proof measures that as `0.0839` average / `0.1742` max of the observed
  live hour. The remaining shared work is cache/input population and scheduling,
  not lane-owned encoding/gating.

Lane-owned module now exists:

- Rust crate: `replay_native/lanes/characters_leaders_cabinet_live/Cargo.toml`.
- DLL-facing collector wrapper: `src/dll_collector.rs`.
- Extractor/root contract: `src/extractor.rs`.
- CollectorState-to-LaneFrame adapter: `src/adapter.rs`.
- Compact schema/encoder/decoder gate: `domain47_compact.rs`, exported as
  `domain47_characters_leaders_cabinet_live`.
- Rust fixture gate: `src/bin/appointed_fixture_gate.rs`.
- Adapter fixture gate: `src/bin/adapter_fixture_gate.rs`.
- LaneFrame adapter contract and shared-input map:
  `replay_native/lanes/characters_leaders_cabinet_live/LANEFRAME_ADAPTER_PLAN.md`.
- Smallest shared adapter hook plan:
  `replay_native/lanes/characters_leaders_cabinet_live/DOMAIN47_SHARED_ADAPTER_HOOK_PLAN.md`.

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
   glue plus the shared input-translation hook:

   - raw scheduled token:
     `RAW_CHARACTERS_LEADERS_CABINET_LIVE_DOMAIN =
     "characters_leaders_cabinet_live"`;
   - tracker/root-cache field holding validated addresses and static metadata
     needed to build `characters_leaders_cabinet_live_lane::scheduled::NativeRootCache`;
   - scheduled-hour branch gated by the registry/scheduled-domain token;
   - construct `characters_leaders_cabinet_live_lane::scheduled::NativeFrameInput`
     from existing collector roots and existing politics idea data;
   - call
     `characters_leaders_cabinet_live_lane::scheduled::read_native_scheduled_frame(...)`;
   - append the returned payload to the compact stream through shared
     `append_compact_record`.

4. Do not inline the domain-47 schema or decoder into `collector/src/lib.rs`.
   The shared collector should only pass roots/rows into the lane module.

Smallest shared collector hook:

- The exact hook points are:
  `build_compact_extraction_gate_records` scheduled enabled flags,
  read-handle locals, `thread::scope` spawn/join, domain perf/data append
  sequencing after `air_variant_archetypes`, and a payload append helper near
  `append_compact_air_variant_archetypes_data_record`.
- The lane crate exposes `scheduled::read_native_scheduled_frame(...)`, which
  owns adapter execution, compact payload encoding, gate counters, and status
  propagation. The shared collector cannot honestly call it until the root cache
  provides real `NativeRootCache` values from `CollectorState`, country bridge
  records, politics active ideas, and validated character/leader/advisor roots.
  A placeholder empty/source-unavailable frame would not satisfy merge-ready
  acceptance.
- `LANEFRAME_ADAPTER_PLAN.md` now specifies the exact required shared inputs,
  row population flow, failure flags, and proof gate for that scheduled path.

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
  - `character_portrait_rows`

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
