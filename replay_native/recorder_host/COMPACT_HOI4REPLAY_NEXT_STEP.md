# Compact `.battleplan` Packaging Checkpoint

This note records the first Rust-native compact-stream packaging checkpoint after
the compact extraction gate.

## Current Boundary

- `collector_record --stream-format compact --extraction-perf-gate` writes the
  DLL byte stream to `collector_stream.bin` and validates H4CR headers, sequence
  ordering, per-hour domain coverage, frame timing, status flags, and skipped
  hours.
- The compact stream includes `DOMAIN_PERF` plus `FRAME_END` records for
  extraction proof, and now includes typed replay data payloads for the
  production-shaped country-state lanes plus `divisions`, `land_combats`, and
  `map_control`.
- The snapshot crate already owns the Rust `.battleplan` container primitives:
  `snapshot::pack` byte-copies payloads into a manifest-covered ZIP, and
  `snapshot::Container` reopens entries through verified hashes.
- A normal `.battleplan` reader requires `payload/<game_id>/index.json`, so even
  a compact-stream-only proof container must include a minimal index payload.

## Implemented Packaging Path

`recorder_host` depends on the workspace `snapshot` crate and avoids a second
container writer.

1. Keep the compact stream bytes exactly as received from the pipe.
2. Build `snapshot::PackEntry` values from raw bytes:
   - `payload/<game_id>/index.json`, role `index`.
   - `payload/<game_id>/collector_raw/collector_stream.bin`, role
     `collector_compact_stream`.
   - `payload/<game_id>/recorder/native_session_manifest.json`, role
     `recorder_manifest`, for recorder diagnostics.
   - `payload/<game_id>/timeline.json`, role `timeline`, naming the compact
     stream and session-manifest entries.
3. Call `snapshot::pack(game_id, &entries, &identity, true)`.
4. Write to `<out>.battleplan.tmp`, then rename to `<out>.battleplan`.
5. Reopen with `snapshot::Container::open`, call `verify_all`, read back the
   compact stream entry, and byte-compare it with the DLL pipe bytes.

Use `collector_compact_stream` rather than the existing `collector_raw_stream`
role until native raw-stream readers are format-aware. The current scrub raw
index treats `collector_raw_stream` as UTF-8 JSONL and will correctly fail closed
on binary bytes, but that is not the desired production discovery path.

The current `PackEntry` API does not expose per-entry `format` metadata. That is
not a blocker for byte-verified packaging because the compact role is distinct,
but app ingestion may still want explicit format metadata later.

## Implemented Test Shape

`recorder_host` now has tests around synthetic H4CR bytes:

- Build small compact streams with typed domain perf/data and `frame` records.
- Pack it with a minimal `index.json` into `.battleplan` using the snapshot
  packer.
- Reopen through `snapshot::Container`, `verify_all`, read the compact entry,
  and assert byte identity.
- Run `summarize_compact_stream` over compact bytes and assert decodable,
  ordered, zero skipped hours, no worker panic, no bridge/source/partial-data
  proof-failure status, expected domain coverage, and one typed data record per
  required domain per frame.

These tests should not use Python, `collector_frames.jsonl`, or full JSON root
frames as the production path.

## Typed Record Contract

- Domain payloads use `COMPACT_RECORD_TYPE_DOMAIN_DATA`; `DOMAIN_PERF` remains
  for extraction metrics and per-frame coverage.
- The host treats unknown record types as parse errors.
- Payloads use fixed numeric headers and row strides where possible. Variable
  token sections are length-prefixed and decoded fail-closed by the host.
- Fixed-point values carry explicit scale metadata where scale is not implied by
  the source field. Measured zero remains distinct from an omitted row.
- Include stable identity in every row, such as `tag_db_id`, `division_id`, or
  `combat_id:generation`; validate duplicate rows and invalid lengths fail
  closed.
- Preserve one sequence authority and make `FRAME_END` the last record for a
  game hour. Parallel read lanes may produce data concurrently, but packaged
  records need deterministic order.
- The DLL collector now parallelizes high-level compact read lanes internally:
  country-domain lane groups, `divisions`, `land_combats`, and `map_control`
  can read concurrently, then the collector emits records in deterministic
  order from the same sequence authority. This is required for the larger
  11-domain speed-five gate; the serial version skipped hours.

## Still Open

- Add typed `divisions` records beyond the widened compact movement/org/
  strength/combat-ref/owner/army/supply/fuel row contract. Division body
  discovery itself is now DLL-native through the anchored
  `CGameState+0x310 -> CCountry+0x290 -> wrapper+0x10 CDivision` path and no
  longer needs Python preflight, saves, or heap/vtable scans for the current
  vanilla build. Remaining division details include template, equipment,
  manpower, exact order geometry/assignment, training/veterancy interpretation,
  and lifecycle identity for splits, merges, deployment, and destruction.
- Add typed data records for army orders, OOB command, naval/air combat,
  air wings, ships, map buildings/resources, logistics/supply proxies, weather,
  and remaining replay-critical event layers.
- Retune the combined compact scheduler/domain volume for the widened row set.
  The current division-only gate is green, but the latest 11-domain widened-row
  run skipped hours at current live speed.
- Fold typed records into the future hourly snapshot spine once the raw compact
  stream is fully covered.

## Live Proof

`compact_map_control_speed5_v2` passed the DLL/Rust compact path with native
`CGameState+0x2B0` CProvince/CState roots. It advanced 145 game hours with 146
frame-end records, 0 skipped hours, full per-hour coverage, 1,081 state
ownership/control rows plus 13,413 province controller/state rows per frame, no
source-unavailable or partial-data flags, avg frame 45.687 ms, p95 58.764 ms,
max 73.840 ms, 33,887,184 raw stream bytes, and an independently verified
`native_session.battleplan`.

`compact_country_divisions_land_combats_map_speed5_v1` intentionally documents
the failed serial 11-domain stress gate: the compact stream decoded, had full
per-domain coverage, and had no source/partial/bridge failures, but skipped 19
game hours with p95 frame time 154.237 ms. That failure is why the collector now
uses internal high-level workers while keeping one DLL and one sequence/order
authority.

`compact_country_divisions_land_combats_map_speed5_v2_workers` passed the
DLL/Rust compact path at speed five/current live speed with 11 scheduled domains:
`fuel_live`, `production_live`, `focus_live`, `tech_live`, `politics_live`,
`politics_residuals`, `decisions_live`, `diplomacy_live`, `divisions`,
`land_combats`, and `map_control`. It advanced 141 game hours with 142 frame-end
records, 0 skipped hours, full per-hour coverage, 0 parse/order errors, no
worker-panic, bridge-unavailable, source-unavailable, or partial-data flags, avg
frame 63.127 ms, p95 101.411 ms, max 145.158 ms, 55,244,380 raw stream bytes,
and an independently verified `native_session.battleplan`. The `divisions` lane
in that mixed proof still depends on a bounded one-time bridge preflight and
remains temporary debt.

`compact_divisions_dll_native_speed5_20260630` documents the negative scan
evidence: the first no-bridge attempt exposed a DLL lifetime issue because
scan-based discovery could outlive the host hook hold and allow `collector.dll`
to unload while the worker thread was still executing. The collector now pins
its own module for the worker lifetime. After restart, a no-bridge scan proof
advanced 101 game hours with 0 skipped but emitted 0 division rows and reported
`division_root_source = "unresolved"` / `SOURCE_UNAVAILABLE`; even a narrowed
oracle-seeded scan window was too slow. The scan code has been removed from the
compact division path.

`compact_divisions_anchored_refresh_speed5_20260630_v1` passed the replacement
native anchored path with root discovery refreshed every compact frame. The DLL
resolves
`CGameState+0x310 -> CCountry+0x290 unit wrappers -> wrapper+0x10 CDivision`,
validates country, wrapper, and division vtables plus division type/id bounds,
then emits compact division records without Python, saves, or a body bridge.
The no-bridge compact gate advanced 35 game hours with 36 frame-end records, 0
skipped, 497 typed division rows, no bridge/source/partial failures,
`division_root_source = "dll_native"`, ordered/decodable compact stream, avg
frame 6.286 ms, p95 17.590 ms, max 18.589 ms, division read avg 4.115 ms, p95
8.374 ms, and an independently verified `native_session.battleplan`.

`compact_country_divisions_land_combats_map_anchored_refresh_speed5_20260630_v1`
passed the 11-domain stress gate with the same per-frame native anchored
division root and no division bridge. It advanced 34 game hours with 35
frame-end records, 0 skipped, full per-hour coverage, no parse/order errors, no
worker-panic, bridge-unavailable, source-unavailable, or partial-data flags, avg
frame 57.143 ms, p95 69.584 ms, max 81.650 ms, division read avg 7.889 ms, p95
23.575 ms, 8,578,373 raw stream bytes, and an independently verified
`native_session.battleplan`.

`compact_divisions_anchored_telemetry_validroot_speed5_20260630_v1` passed the
widened no-bridge compact division telemetry gate. The DLL still uses the same
anchored path, but the compact row is now 88 bytes and carries division id,
location/source/target/progress, organisation/strength buckets,
combat-ref/in-combat state, owner tag-db id, parent army id, raw experience,
max/current supply, supply gain, and fuel. The gate advanced 665 game hours
with 666 frame-end records, 0 skipped, 91,642 typed rows, no
bridge/source/partial failures, `division_root_source = "dll_native"`,
ordered/decodable stream, division read avg 7.505 ms, p95 8.002 ms, max
15.187 ms, avg frame 8.135 ms, p95 8.768 ms, max 15.776 ms, 8,197,696 raw
stream bytes, and an independently verified `native_session.battleplan`.

`compact_country_divisions_land_combats_map_telemetry_validroot_speed5_20260630_v1`
is the current negative combined-load proof for the widened row set. It decoded,
ordered, packed, and verified the 11-domain stream with no bridge/source/partial
failures, but advanced 448 game hours with only 381 frame-end records and
skipped 68 hours. `map_control` remained the heaviest lane at p95 43.350 ms,
followed by country-root lanes and widened `divisions` at p95 14.381 ms. Do not
treat the full 11-domain widened-row bundle as complete until the compact path
captures the current live speed with zero skipped hours.
