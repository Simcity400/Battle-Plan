# recorder_host

Native proof host for the production recording path.

This crate is the first Rust-side replacement for the Python hot loop. It does
not walk HOI4 memory. It writes a bounded `collector_control.json`, installs the
collector DLL through the existing read-only Windows hook attach path, keeps the
hook alive for the worker duration, reads the DLL named-pipe stream, writes
either debug `collector_frames.jsonl` or compact `collector_stream.bin`, and
emits `native_session_manifest.json` with cadence and skipped-hour diagnostics.

Current scope:

- Production-shaped transport: `hoi4.exe` -> DLL collector -> compact binary
  stream -> native host manifest/recorder.
- No Python in the live capture loop.
- No save input.
- Country-state, `divisions`, `land_combats`, and `map_control` compact gates
  require no Python preflight bridge. `divisions` is intentionally not
  scan-backed in the compact production path; it uses the anchored
  `CGameState+0x310 -> CCountry+0x290 -> wrapper+0x10 CDivision` path and fails
  closed with `SOURCE_UNAVAILABLE` if that root does not validate for a build.
  The current compact division row is 88 bytes and carries division id,
  location/source/target/progress, organisation/strength buckets,
  combat-ref/in-combat state, owner tag-db id, parent army id, template id,
  experience, max/current supply, supply gain, and fuel. `divisions` emits a
  full baseline and then compact delta/tombstone rows; perf records still
  report the full observed division root count each frame. Large division root
  sets are sampled by bounded internal DLL workers before ordered record
  emission.
  `map_control` emits a full baseline and then compact delta records; perf
  records still report the full observed state/province row counts each frame.
  `map_detail` covers `state_detail`/`buildings_live` as compact state-resource
  rows plus state/province building-level rows from the same native CProvince /
  CState topology; it emits a full baseline followed by deltas and uses bounded
  province workers with batched building-vector metadata reads.
- Optional DLL-side project scanning through `--special-projects-scan`; current
  live evidence shows this is not yet bounded enough for production capture.
- Compact country-frame recording through `--compact-country-frame`; filtered
  sessions emit root snapshots only at scheduled game-hour boundaries and keep
  worker polls to explicitly requested raw domains. Filtered country sessions no
  longer emit a separate raw clock delta unless `clock` is requested; scheduled
  root snapshots still carry the canonical clock.
- Compact binary performance gates through
  `--stream-format compact --extraction-perf-gate`. In this mode the DLL builds
  typed in-memory domain rows/counters and emits packed records, not full JSON
  root frames. Record headers include sequence id, `game_hour_raw`, domain id,
  record type, and payload length. The Rust host verifies decodability,
  monotonic ordering, byte volume, frame timings, every scheduled domain on
  every frame hour, one typed data record per required domain per frame,
  worker-panic/bridge/source/partial-data status, and skipped game hours.
- For speed-five proof runs, use the release DLL/host. `--sample-ms 1` is the
  normal high-cadence gate; `--sample-ms 0` enables DLL worker true no-sleep
  mode for stress gates where the game advances faster than a 1 ms sleep loop
  can prove. The default remains 10 ms.
- Rust-native `.battleplan` packing for the raw stream: the host writes
  `native_session.battleplan`, includes a minimal index/timeline, stores
  `collector_raw/collector_stream.bin` with role `collector_compact_stream` and
  `recorder/native_session_manifest.json` with role `recorder_manifest`, opens
  the container, verifies every entry, and byte-compares the stored stream
  against the DLL pipe bytes. Python may validate the raw output, but a segment
  is not production-complete until this native path passes hourly-or-better
  speed-five proof.

Example:

```powershell
cargo run --manifest-path replay_native/Cargo.toml -p recorder_host --bin collector_record -- `
  --pid <hoi4-pid> `
  --dll replay_native\target\debug\collector.dll `
  --out-dir replay\snapshots\_native_collector_session\clock_lightweight `
  --duration-s 180 `
  --sample-ms 10 `
  --min-game-hours 12 `
  --scheduled-domain clock `
  --lightweight-root-frame
```

Project probe example:

```powershell
cargo run --manifest-path replay_native/Cargo.toml -p recorder_host --bin collector_record -- `
  --pid <hoi4-pid> `
  --dll replay_native\target\debug\collector.dll `
  --out-dir replay\snapshots\_native_collector_session\projects_lightweight `
  --duration-s 15 `
  --sample-ms 10 `
  --min-game-hours 120 `
  --scheduled-domain projects_live `
  --lightweight-root-frame `
  --special-projects-scan
```

The command exits nonzero when the manifest fails the requested hourly gate.

Compact release country-domain proof example:

```powershell
cargo build --release --manifest-path replay_native/Cargo.toml -p collector -p recorder_host

replay_native\target\release\collector_record.exe `
  --pid <hoi4-pid> `
  --dll replay_native\target\release\collector.dll `
  --out-dir replay\snapshots\_native_collector_session\politics_live_compact_release `
  --duration-s 15 `
  --sample-ms 0 `
  --dwell-s 0.25 `
  --min-game-hours 120 `
  --scheduled-domain politics_live `
  --compact-country-frame `
  --idea-group-filter-file replay\snapshots\_native_collector_session\static_contracts\idea_group_filter.txt `
  --advisor-token-filter-file replay\snapshots\_native_collector_session\static_contracts\advisor_token_filter.txt
```

When `--scheduled-domain` is supplied, heavy raw domains such as `map_control`,
`land_combats`, `divisions`, `army_orders`, `naval_air_combats`,
`naval_combat_members`, `air_wings`, `ships`, and `oob_command` are captured
only if explicitly requested. This keeps independent country-domain proof gates
from being slowed by unrelated late-game combat traffic. Raw clock deltas follow
the same rule for filtered sessions: request `--scheduled-domain clock` when a
raw clock sidecar is the thing under test. With no domain filter, the DLL keeps
the legacy default of enabling all domains.

Compact extraction gate example:

```powershell
replay_native\target\release\collector_record.exe `
  --pid <hoi4-pid> `
  --dll replay_native\target\release\collector.dll `
  --out-dir replay_native\runs\compact_fuel_production_speed5_v1 `
  --duration-s 20 `
  --dwell-s 0.25 `
  --sample-ms 0 `
  --min-game-hours 24 `
  --stream-format compact `
  --extraction-perf-gate `
  --compact-country-frame `
  --lightweight-root-frame `
  --scheduled-domain fuel_live `
  --scheduled-domain production_live
```

Current proof status: `--sample-ms 0` is a measurement tool, not a pass by
itself, and JSON throughput is not a production target. The compact DLL/Rust
gate has these live speed-five results:

- `compact_fuel_production_speed5_v1`: passed, 284 game hours advanced, 0
  skipped, avg frame 23.973 ms, p95 39.567 ms, 59,280 bytes.
- `compact_country_state_nonprod_speed5_v1`: passed, 247 game hours advanced,
  0 skipped, avg frame 42.434 ms, p95 57.007 ms, 123,008 bytes.
- `compact_all_country_parallel_coverage_speed5_v2`: passed, 332 game hours
  advanced, 333 frame-end records, 0 skipped, full per-hour domain coverage, no
  worker panic, avg frame 38.151 ms, p95 48.743 ms, 213,120 bytes.
- `compact_fuel_replay_container_speed5_v1`: passed, 291 game hours advanced
  in 30 seconds, 292 frame-end records, 0 skipped, full per-hour domain
  coverage, no worker panic, 292 fuel data records for 292 fuel frames, 11,611
  typed fuel rows, avg frame 44.869 ms, p95 57.064 ms, max 116.131 ms, 667,672
  raw stream bytes, and a verified 86,012-byte `native_session.battleplan`.
- `compact_politics_only_speed5_v5_sharded`: passed after politics popularity
  deltas and row-sharded politics reads, 386 game hours advanced, 387
  frame-end records, 0 skipped, ordered/decodable compact stream, no worker
  panic, avg frame 6.969 ms, p95 11.313 ms, max 32.044 ms, 247,675 raw stream
  bytes.
- `compact_politics_residuals_speed5_v3`: passed after residual-only cached
  reads, 448 game hours advanced, 449 frame-end records, 0 skipped,
  ordered/decodable compact stream, no worker panic, avg frame 14.894 ms, p95
  17.472 ms, max 33.503 ms, 96,567 raw stream bytes.
- `compact_verified_country_state_speed5_v2_lanes`: passed with lane-parallel
  compact reads for `fuel_live`, `focus_live`, `tech_live`, `politics_live`,
  `politics_residuals`, `decisions_live`, and `diplomacy_live`; 287 game hours
  advanced, 288 frame-end records, 0 skipped, full per-hour domain coverage, no
  worker panic, avg frame 20.576 ms, p95 32.615 ms, max 52.020 ms, 1,122,564
  raw stream bytes, a 220,682-byte `native_session.battleplan`, and an
  independent `pack_replay verify` pass.
- `compact_production_live_speed5_v1`: passed after typed production data
  record emission, 318 game hours advanced, 319 frame-end records, 0 skipped,
  ordered/decodable compact stream, no worker panic, avg frame 16.966 ms, p95
  27.467 ms, max 36.684 ms, 3,723,392 raw stream bytes, and an 880,339-byte
  `native_session.battleplan`.
- `compact_full_country_state_speed5_v2_host_gate`: passed with lane-parallel
  compact reads for `fuel_live`, `production_live`, `focus_live`, `tech_live`,
  `politics_live`, `politics_residuals`, `decisions_live`, and
  `diplomacy_live` under the tightened host rule requiring per-frame production
  data records; 180 game hours advanced, 181 frame-end records, 0 skipped, full
  per-hour domain coverage, no worker panic, avg frame 28.809 ms, p95
  48.311 ms, max 79.622 ms, 3,094,957 raw stream bytes, a 799,688-byte
  `native_session.battleplan`, and an independent `pack_replay verify` pass.
- `compact_divisions_speed5_wip_v1`: passed the DLL/Rust compact hot-loop after
  a bounded one-time division bridge preflight, 240 game hours advanced, 241
  frame-end records, 0 skipped, 2,298 bridge rows sampled per hour, 548,496
  typed division rows, ordered/decodable stream, no bridge-unavailable flag, avg
  frame 15.807 ms, p95 21.671 ms, max 28.866 ms, 30,763,976 raw stream bytes,
  a 10,291,262-byte `native_session.battleplan`, and an independent
  `pack_replay verify` pass. This proves throughput only; native division root
  discovery remains temporary debt.
- `compact_land_combats_speed5_v1`: passed with the native `CGameState+0x260`
  land-combat manager root, 237 game hours advanced, 238 frame-end records, 0
  skipped, 5,882 active combat rows, 67,458 participant unit rows, no
  source-unavailable or partial-data flags, avg frame 1.779 ms, p95 2.400 ms,
  max 3.487 ms, 1,901,872 raw stream bytes, an 86,841-byte
  `native_session.battleplan`, and an independent `pack_replay verify` pass.
- `compact_map_control_speed5_v2`: passed with native `CGameState+0x2B0`
  CProvince/CState roots, 145 game hours advanced, 146 frame-end records, 0
  skipped, 1,081 state ownership/control rows and 13,413 province
  controller/state rows per frame, no source-unavailable or partial-data flags,
  avg frame 45.687 ms, p95 58.764 ms, max 73.840 ms, 33,887,184 raw stream
  bytes, and an independent `pack_replay verify` pass.
- `compact_country_divisions_land_combats_speed5_v2_retry`: passed after a
  short native land-combat manager retry for transient manager churn, with 10
  scheduled domains (`fuel_live`, `production_live`, `focus_live`, `tech_live`,
  `politics_live`, `politics_residuals`, `decisions_live`, `diplomacy_live`,
  `divisions`, `land_combats`), 187 game hours advanced, 188 frame-end records,
  0 skipped, full per-hour coverage, 0 parse/order errors, no worker-panic,
  bridge-unavailable, source-unavailable, or partial-data flags, avg frame
  60.247 ms, p95 90.664 ms, max 117.845 ms, 30,022,402 raw stream bytes, a
  9,456,740-byte `native_session.battleplan`, and an independent
  `pack_replay verify` pass. This is a mixed stress gate; the `divisions` lane
  still carries bridge preflight debt.
- `compact_country_divisions_land_combats_map_speed5_v1`: failed with the same
  11-domain set after adding `map_control` because serial high-level reads
  skipped 19 game hours; the stream itself decoded, had full per-domain
  coverage, and had no source/partial/bridge failures. This is the regression
  proof for keeping internal per-domain workers.
- `compact_country_divisions_land_combats_map_speed5_v2_workers`: passed after
  parallelizing the high-level DLL read lanes while preserving one ordered
  sequence authority. It covered 11 scheduled domains (`fuel_live`,
  `production_live`, `focus_live`, `tech_live`, `politics_live`,
  `politics_residuals`, `decisions_live`, `diplomacy_live`, `divisions`,
  `land_combats`, `map_control`), advanced 141 game hours with 142 frame-end
  records, 0 skipped, full per-hour coverage, 0 parse/order errors, no
  worker-panic, bridge-unavailable, source-unavailable, or partial-data flags,
  avg frame 63.127 ms, p95 101.411 ms, max 145.158 ms, 55,244,380 raw stream
  bytes, and an independent `pack_replay verify` pass. The `divisions` lane
  still carries bridge preflight debt.
- `compact_divisions_dll_native_speed5_20260630`: first no-bridge DLL-native
  division-root attempt exposed a collector lifetime bug: scan-based discovery
  could outlive the host hook hold, unload `collector.dll`, and crash HOI4 in
  `collector.dll_unloaded`. The collector worker now pins its module for the
  worker lifetime. Unit tests pass, but this did not solve division discovery.
- `compact_divisions_dll_native_speed5_20260630_followup`: after restarting,
  the cheap compact attach sanity gate passed with 14 game hours advanced in 2
  seconds, 0 skipped, ordered/decodable stream, and no source/worker failures.
  The no-bridge `divisions` gate still failed before emitting records: even with
  reusable hook-triggered worker startup, bounded pipe-connect/read timeouts,
  aligned qword scanning, private-memory filtering, and a narrowed oracle-seeded
  division arena, scan-based native discovery did not emit within a 180 second
  prewarm budget. The later 20 second compact no-bridge run advanced 101 game
  hours with 0 skipped, but emitted 0 division rows and set
  `division_root_source = "unresolved"` / `SOURCE_UNAVAILABLE`. Treat these
  as negative evidence: division completion needs a native anchored
  root/container, not a blind vtable heap scan.
- `compact_divisions_anchored_refresh_speed5_20260630_v1`: passed with the
  native anchored division path refreshed every compact frame:
  `CGameState+0x310 -> CCountry+0x290 unit wrappers -> wrapper+0x10 CDivision`.
  The no-bridge compact DLL/Rust gate advanced 35 game hours with 36 frame-end
  records, 0 skipped, 497 typed division rows, no bridge/source/partial
  failures, `division_root_source = "dll_native"`, avg frame 6.286 ms, p95
  17.590 ms, max 18.589 ms, division read avg 4.115 ms, p95 8.374 ms, 35,032
  raw stream bytes, and an independent `pack_replay verify` pass.
- Current vanilla sessions can expose country-wrapper `CDivision` bodies whose
  handle type is `51` rather than the older save-correlated `4713`. The compact
  division root and sampler now accept `{4713, 51}` only after the body vtable
  matches `module+0x2933F00`, then still serialize the canonical replay
  division type `4713`. This fixed the failed
  `compact_country_divisions_land_combats_map_detail_weather_region_speed5_20260630_v2`
  run, where the anchored path was present but all division roots were rejected
  as `SOURCE_UNAVAILABLE`.
- `compact_country_divisions_land_combats_map_anchored_refresh_speed5_20260630_v1`:
  passed the 11-domain compact stress gate with the same per-frame native
  anchored division root and no division bridge. It covered `fuel_live`,
  `production_live`, `focus_live`, `tech_live`, `politics_live`,
  `politics_residuals`, `decisions_live`, `diplomacy_live`, `divisions`,
  `land_combats`, and `map_control`; advanced 34 game hours with 35 frame-end
  records, 0 skipped, full per-hour coverage, 0 parse/order errors, no
  worker-panic, bridge-unavailable, source-unavailable, or partial-data flags,
  avg frame 57.143 ms, p95 69.584 ms, max 81.650 ms, division read avg 7.889
  ms, p95 23.575 ms, 8,578,373 raw stream bytes, and an independent
  `pack_replay verify` pass.
- `compact_divisions_anchored_telemetry_validroot_speed5_20260630_v1`: passed
  the widened no-bridge compact division row contract through the same DLL
  native anchor. It advanced 665 game hours with 666 frame-end records, 0
  skipped hours, 91,642 typed division rows, no bridge/source/partial failures,
  `division_root_source = "dll_native"`, ordered/decodable compact stream,
  division read avg 7.505 ms, p95 8.002 ms, max 15.187 ms, avg frame 8.135 ms,
  p95 8.768 ms, max 15.776 ms, 8,197,696 raw stream bytes, and an independent
  `pack_replay verify` pass. This is the current acceptance proof for the
  division row fields listed in the scope section above.
- `compact_country_divisions_land_combats_map_telemetry_validroot_speed5_20260630_v1`:
  verified that the widened division rows decode and pack inside the 11-domain
  compact bundle, but it did not meet the hourly gate at the current live speed.
  It advanced 448 game hours with 381 frame-end records and skipped 68 hours.
  The stream was ordered/decodable, had no bridge/source/partial failures, and
  `pack_replay verify` passed; the bottleneck is extraction volume under the
  combined load. Per-domain p95 read times were 43.350 ms for `map_control`,
  20.128 ms for `focus_live`/`tech_live`, 18.002 ms for `politics_residuals`,
  17.680 ms for `fuel_live`/`production_live`, 17.281 ms for `politics_live`,
  14.381 ms for widened `divisions`, 9.224 ms for `decisions_live`, 2.319 ms
  for `land_combats`, and 1.066 ms for `diplomacy_live`. Treat this as the next
  scheduler/domain-volume problem, not as a completed all-domain proof.
- `compact_country_divisions_land_combats_map_mapdelta_speed5_20260630_v1`:
  passed the widened 11-domain compact gate after `map_control` moved to cached
  topology plus baseline/delta data records. It covered `fuel_live`,
  `production_live`, `focus_live`, `tech_live`, `politics_live`,
  `politics_residuals`, `decisions_live`, `diplomacy_live`, `divisions`,
  `land_combats`, and `map_control`; advanced 349 game hours with 350 frame-end
  records, 0 skipped, full per-domain coverage, no parse/order errors, no
  worker-panic, bridge-unavailable, source-unavailable, or partial-data flags,
  avg frame 31.373 ms, p95 37.631 ms, max 51.696 ms, 34,334,975 raw stream
  bytes, and an independent `pack_replay verify` pass. `map_control` observed
  5,072,900 full rows but emitted only 15,418 changed data rows; its read p95
  fell to 12.583 ms. The heaviest remaining lanes in this run were widened
  `divisions` at p95 33.470 ms and `fuel_live`/`production_live` at p95
  34.625 ms.
- `compact_country_divisions_land_combats_map_divdelta_parallel_speed5_20260630_v1`:
  passed the same widened 11-domain compact gate after `divisions` moved to
  baseline-plus-delta data records with explicit zero-filled delete/tombstone
  rows and bounded internal sampling workers. It covered `fuel_live`,
  `production_live`, `focus_live`, `tech_live`, `politics_live`,
  `politics_residuals`, `decisions_live`, `diplomacy_live`, `divisions`,
  `land_combats`, and `map_control`; advanced 345 game hours with 346 frame-end
  records, 0 skipped, full per-domain coverage, no parse/order errors, no
  worker-panic, bridge-unavailable, source-unavailable, or partial-data flags,
  avg frame 34.743 ms, p95 52.766 ms, max 66.712 ms, 62,786,985 raw stream
  bytes, and an independent `pack_replay verify` pass. `divisions` observed
  772,920 full rows but emitted 585,450 changed/tombstone rows; division read
  p95 was 37.112 ms. The remaining heavy lanes in this run were
  `fuel_live`/`production_live` at p95 51.208 ms and widened `divisions` at p95
  37.112 ms.
- `compact_map_detail_buildings_cached_speed5_20260630_v1`: passed the
  isolated compact `map_detail` gate for `state_detail` plus `buildings_live`
  after province building reads moved to bounded workers and batched vector
  metadata reads. It advanced 259 game hours with 260 frame-end records, 0
  skipped, full per-domain coverage, no parse/order errors, no worker-panic,
  source-unavailable, or partial-data flags, avg frame 20.292 ms, p95 24.443
  ms, max 59.611 ms, 473,672 raw stream bytes, and an independent
  `pack_replay verify` pass.
- `compact_country_divisions_land_combats_map_detail_speed5_20260630_v1`:
  passed the 12-domain compact bundle with `state_detail` and `buildings_live`
  added to the previous country/division/land-combat/map set. It covered
  `fuel_live`, `production_live`, `focus_live`, `tech_live`, `politics_live`,
  `politics_residuals`, `decisions_live`, `diplomacy_live`, `divisions`,
  `land_combats`, `map_control`, and `map_detail`; advanced 289 game hours with
  290 frame-end records, 0 skipped, full per-domain coverage, no parse/order
  errors, no worker-panic, bridge-unavailable, source-unavailable, or
  partial-data flags, avg frame 54.922 ms, p95 89.906 ms, max 99.708 ms,
  56,044,696 raw stream bytes, and an independent `pack_replay verify` pass.
  `map_detail` observed 4,203,260 full rows and emitted 14,528 changed rows;
  its read p95 was 36.421 ms.
- `compact_country_divisions_land_combats_map_detail_split_economy_speed5_20260630_v1`:
  passed the same 12-domain compact bundle after the former shared economy
  worker was split into independent `fuel_live` and `production_live` workers.
  It advanced 262 game hours with 263 frame-end records, 0 skipped, full
  per-domain coverage, no parse/order errors, no worker-panic,
  bridge-unavailable, source-unavailable, or partial-data flags, avg frame
  61.468 ms, p95 90.135 ms, max 117.487 ms, 62,020,850 raw stream bytes, and
  an independent `pack_replay verify` pass. The split showed `fuel_live` read
  p95 at 3.218 ms while `production_live` remained the main country-domain
  limiter at p95 86.877 ms.
- `compact_weather_region_decode_smoke_20260630_v1`: passed a weather-only
  compact decode/replay smoke gate for the new `weather_live` region stream.
  It emitted 305 region rows, decoded without parse/order/source/partial/worker
  failures, read weather in 54 us, wrote 9,976 compact bytes, and passed
  independent `pack_replay verify`. This is not a speed-five completion proof:
  the live game clock did not advance during the smoke window
  (`game_hours_advanced = 0`). A full weather completion gate still needs an
  advancing speed-five run with zero skipped game hours.
- `compact_country_divisions_land_combats_map_detail_weather_region_speed5_20260630_v3`:
  passed the full compact bundle with `weather_live` added after widening the
  native division body-type validator. It covered `fuel_live`,
  `production_live`, `focus_live`, `tech_live`, `politics_live`,
  `politics_residuals`, `decisions_live`, `diplomacy_live`, `divisions`,
  `land_combats`, `map_control`, `map_detail`, and `weather_live`; advanced 57
  game hours with 58 frame-end records, 0 skipped, full per-domain coverage, no
  parse/order errors, no worker-panic, bridge-unavailable, source-unavailable,
  or partial-data flags, avg frame 22.567 ms, p95 24.484 ms, max 79.348 ms,
  2,983,908 raw stream bytes, `division_root_source = "dll_native"`, and an
  independent `pack_replay verify` pass. In that run `divisions` observed
  71,012 roots and emitted 19,243 changed rows with read p95 22.148 ms;
  `weather_live` observed 17,690 region rows, emitted 394 changed rows, and
  read at p95 123 us.
- `compact_country_divisions_land_combats_map_detail_weather_region_anchor_speed5_20260630_v2`:
  passed the same 13-domain compact bundle after replacing weather singleton
  heap discovery with the anchored `CGameState+0x688 -> CWeatherManager` path
  and aligning the `map_detail` schedule alias with full state/building detail
  capture. It advanced 56 game hours with 57 frame-end records, skipped 0 game
  hours, had full per-domain coverage, no parse/order errors, no worker-panic,
  bridge-unavailable, source-unavailable, or partial-data flags, avg frame
  22.319 ms, p95 26.714 ms, max 68.701 ms, 3,084,775 raw stream bytes,
  `division_root_source = "dll_native"`, and an independent `pack_replay
  verify` pass. `weather_live` observed 17,385 region rows, emitted 396
  changed rows, and read at p95 87 us; `map_detail` observed 826,158 rows and
  read at p95 21.948 ms.
- `compact_country_divisions_land_combats_map_detail_weather_region_anchor_soak_speed5_20260630_v1`:
  passed a longer 120-second anchored 13-domain soak. It advanced 222 game
  hours with 223 frame-end records, skipped 0 game hours, had full domain
  coverage, no parse/order errors, no worker-panic, bridge-unavailable,
  source-unavailable, or partial-data flags, avg frame 22.113 ms, p95 24.463
  ms, max 69.586 ms, 9,180,014 raw stream bytes, and an independent
  `pack_replay verify` pass. `divisions` observed 298,132 roots and emitted
  69,187 changed rows with read p95 22.296 ms.
- `compact_country_divisions_template_id_speed5_20260630_v1`: passed the full
  13-domain compact bundle after widening `divisions` to read
  `CDivision+0x3A8 -> CDivisionTemplate+0x0C` as `template_id`, guarded by the
  template vtable `module+0x29291D0` and type tag `52`. It advanced 56 game
  hours with 57 frame-end records, skipped 0 game hours, had full coverage, no
  parse/order errors, no worker-panic, bridge-unavailable, source-unavailable,
  or partial-data flags, avg frame 22.930 ms, p95 25.829 ms, max 72.687 ms,
  3,025,600 raw stream bytes, and an independent `pack_replay verify` pass.
  A byte-level stream audit confirmed 19,222 compact division rows carried the
  template flag with nonzero template ids, with 0 flag/value mismatches.

The extraction-only compact performance gate is green for the current
production-shaped country-state domains and the native-root `divisions`,
`land_combats`, and `map_control` lanes above. The recorder packs/verifies
`native_session.battleplan`, and the combined country/division/land-combat/map
stress replay passed independent `pack_replay verify`. `production_live` is now
DLL/Rust compact-path complete at hourly cadence for the current vanilla build,
including stockpile, military production lines, naval production lines, and
construction queue rows. `land_combats` is DLL/Rust compact-path complete for
active combat lifecycle snapshots and participant vectors at hourly cadence on
the current vanilla build. `map_control` is DLL/Rust compact-path complete for
state ownership/control and province controller/state rows at hourly cadence on
the current vanilla build. `map_detail` is DLL/Rust compact-path complete for
state resource deposits and state/province building levels at hourly cadence on
the current vanilla build. `divisions` is DLL/Rust compact-path complete for
the current widened compact division row contract (division id, location,
movement progress, movement source/target, org, strength, combat reference
count, in-combat flag, owner tag-db id, parent army id, template id, experience,
supply, and fuel) on the current vanilla build, including compact baseline/delta
rows and explicit delete/tombstone records. It now uses the anchored country
unit-wrapper path and no longer needs Python preflight, saves, or heap/vtable
scans for compact division body discovery. Wider division replay coverage still
needs typed records for the template catalog, per-division held equipment,
exact manpower once pinned, assignment, and order geometry/details.
`weather_live` now has a DLL/Rust compact region stream for raw phenomenon,
boolean weather flags, intensity, active-modifier counts, next-change raw value,
and region temperature inputs. It resolves `CWeatherManager` through
`u64[module+0x33048C0]` (`CGameState`) then `u64[CGameState+0x688]`, validates
the manager vtable and `CWeatherManager+0x40` region vector, and passed the
advancing speed-five hourly gate in the 13-domain anchored bundle above. The
old heap/vtable singleton scan is reverse-engineering fallback only, not the
production compact capture path. The full 13-domain widened-row bundle is now
speed-five complete for the currently implemented compact domains on the active
vanilla process, with longer soaks and the next replay-critical domains still
open.

`compact_wide_semantic_counters_speed5_20260630_v1` reran the widened compact
path with recorder-side semantic counters for all 14 currently implemented
typed domains: the 8 country-state domains, `divisions`, `division_equipment`,
`land_combats`, `map_control`, `map_detail`, and `weather_live`. It advanced
105 game hours in 60 seconds at speed five with 0 skipped, full per-hour domain
and data-record coverage, ordered/decodable records, no
source-unavailable/partial/bridge/worker failures, avg frame 44.608 ms, p95
64.820 ms, max 103.623 ms, 4,695,510 compact bytes, and a verified
1,315,765-byte `.battleplan`. The manifest now emits `data_units_total` per
domain so proof runs can report real decoded unit counts, not just frame bytes:
production stockpiles/lines/construction, active research slots, timed
decisions, division movement/combat/org-strength rows, held equipment entries,
land-combat participants, map resource/building rows, and active weather
regions.

`compact_army_orders_wide_speed5_20260630_v2` adds compact `army_orders` as the
15th typed domain. The DLL derives active CArmy roots from anchored live
`CDivision+0xB0` parents, validates the CArmy and order vtables/handles, and
emits compact rows for order identity, generation, slot, type, delete rows, and
validated type-5 target-state slices. The 60-second speed-five proof advanced
105 game hours with 0 skipped, full per-hour domain/data-record coverage,
ordered/decodable records, no source-unavailable/partial/bridge/worker flags,
avg frame 47.094 ms, p95 60.541 ms, max 121.066 ms, 6,386,939 compact bytes,
and a verified 2,031,491-byte `.battleplan`. Recorder semantic counters report
106 `army_orders` data records, 43,290 observed army rows, 42,226 observed live
orders, 436 changed order rows, 19 delete rows, 176 exact-target parent rows,
233 unsupported-target parent rows, 8 validation-failed target rows, and 966
target-state entries. This replaces the old raw-only/bridge-backed army-order
path for active armies reachable through live divisions. Do not mark full OOB
complete yet: empty armies, army-group/fleet/task-force roots, and deeper
non-type-5 order geometry still need native compact coverage.

Implementation note: see `COMPACT_HOI4REPLAY_NEXT_STEP.md` for the bounded
Rust-native packaging path and the test contract for putting `collector_stream.bin`
inside a manifest-covered `.battleplan` without using Python or full JSON root
frames as the production path.
