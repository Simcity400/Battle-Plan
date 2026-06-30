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
- No Python preflight bridge.
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
  every frame hour, one typed `fuel_live` data record per fuel frame,
  worker-panic status, and skipped game hours.
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

The extraction-only compact performance gate is now green for the current
country domains. The first replay-payload step is implemented: `fuel_live` now
emits a typed binary data record with raw fixed-point `i64` fuel values and
explicit `32768` scale metadata, and the recorder packs/verifies
`native_session.battleplan`. Unit tests and release build pass for this path.
It still needs a fresh live speed-five proof because the current HOI4 process
was not running during this checkpoint.

Implementation note: see `COMPACT_HOI4REPLAY_NEXT_STEP.md` for the bounded
Rust-native packaging path and the test contract for putting `collector_stream.bin`
inside a manifest-covered `.battleplan` without using Python or full JSON root
frames as the production path.
