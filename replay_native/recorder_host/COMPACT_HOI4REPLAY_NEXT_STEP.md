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
  production-shaped country-state lanes plus `divisions` and `land_combats`.
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

## Still Open

- Move `divisions` root discovery out of temporary Python preflight into the
  DLL/Rust host before marking that segment DLL-complete.
- Add typed data records for army orders, OOB command, naval/air combat,
  air wings, ships, province/state/building deltas, logistics/supply proxies,
  weather, and remaining replay-critical event layers.
- Fold typed records into the future hourly snapshot spine once the raw compact
  stream is fully covered.

## Live Proof

`compact_country_divisions_land_combats_speed5_v2_retry` passed the DLL/Rust
compact path at speed five/current live speed with 10 scheduled domains:
`fuel_live`, `production_live`, `focus_live`, `tech_live`, `politics_live`,
`politics_residuals`, `decisions_live`, `diplomacy_live`, `divisions`, and
`land_combats`. It advanced 187 game hours with 188 frame-end records, 0 skipped
hours, full per-hour coverage, 0 parse/order errors, no worker-panic,
bridge-unavailable, source-unavailable, or partial-data flags, avg frame
60.247 ms, p95 90.664 ms, max 117.845 ms, 30,022,402 raw stream bytes, and an
independently verified 9,456,740-byte `native_session.battleplan`. The
`divisions` lane in that mixed proof still depends on a bounded one-time bridge
preflight and remains temporary debt.
