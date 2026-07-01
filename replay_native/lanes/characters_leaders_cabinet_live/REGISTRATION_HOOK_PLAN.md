# characters_leaders_cabinet_live domain-47 registration hook plan

Domain registry assignment:

- Final domain: `47 = characters_leaders_cabinet_live`.
- `replay_native/domain_registry.json` currently assigns id `47` to this lane with
  status `reserved`.
- `replay_native/DOMAIN_REGISTRY.md` currently marks id `46` as
  `air_variant_archetypes_live` production-complete and id `47` as this lane's
  reserved id.
- `DOMAIN47_REGISTRATION_ONLY.patch` is the exact low-conflict shared patch for
  registry activation, Cargo dependencies, and recorder-side domain-47 decode.
  It is intentionally not applied while shared collector/recorder files are
  actively dirty from other lanes.

Lane-owned module now exists:

- Rust crate: `replay_native/lanes/characters_leaders_cabinet_live/Cargo.toml`.
- DLL-facing collector wrapper: `src/dll_collector.rs`.
- Extractor/root contract: `src/extractor.rs`.
- Compact schema/encoder/decoder gate: `domain47_compact.rs`, exported as
  `domain47_characters_leaders_cabinet_live`.
- Rust fixture gate: `src/bin/appointed_fixture_gate.rs`.

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
   glue:

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

Current collector conflict:

- `replay_native/collector/src/lib.rs` is already modified by other active lanes.
- The domain-47 hook points overlap the same hot-loop blocks currently touched by
  those edits: scheduled enabled flags around the air archetype/supply/market
  section, read-handle setup around the air archetype read block, and compact
  append sequencing around the domain data append block.
- This is why `DOMAIN47_REGISTRATION_ONLY.patch` excludes `collector/src/lib.rs`.

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
adapter, mirroring the existing domain-46 lane-summary merge pattern.

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
