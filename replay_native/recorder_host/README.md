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
- Country-state and `land_combats` compact gates require no Python preflight
  bridge. `divisions` currently accepts an explicit bounded bridge input for
  proof runs; that is temporary root-discovery debt and is not considered
  DLL-complete until native/Rust discovery replaces it.
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

When `--scheduled-domain` is supplied, heavy raw domains such as
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

The extraction-only compact performance gate is green for the current
production-shaped country-state domains and the native-root `land_combats` lane
above. The recorder packs/verifies `native_session.battleplan`, and the combined
country/division/land-combat stress replay passed independent `pack_replay
verify`. `production_live` is now DLL/Rust compact-path complete at hourly
cadence for the current vanilla build, including stockpile, military production
lines, naval production lines, and construction queue rows. `land_combats` is
DLL/Rust compact-path complete for active combat lifecycle snapshots and
participant vectors at hourly cadence on the current vanilla build. `divisions`
is not DLL-complete until its root discovery is moved out of Python preflight.

Implementation note: see `COMPACT_HOI4REPLAY_NEXT_STEP.md` for the bounded
Rust-native packaging path and the test contract for putting `collector_stream.bin`
inside a manifest-covered `.battleplan` without using Python or full JSON root
frames as the production path.
