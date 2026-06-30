# Compact `.battleplan` Packaging Checkpoint

This note records the first Rust-native compact-stream packaging checkpoint after
the compact extraction gate.

## Current Boundary

- `collector_record --stream-format compact --extraction-perf-gate` writes the
  DLL byte stream to `collector_stream.bin` and validates H4CR headers, sequence
  ordering, per-hour domain coverage, frame timing, status flags, and skipped
  hours.
- The compact stream still includes `DOMAIN_PERF` plus `FRAME_END` records for
  extraction proof, and now also includes the first typed replay data payload:
  `fuel_live` rows keyed by `tag_db_id`.
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

- Build a small compact stream with `fuel_live` perf/data and `frame` records.
- Pack it with a minimal `index.json` into `.battleplan` using the snapshot
  packer.
- Reopen through `snapshot::Container`, `verify_all`, read the compact entry,
  and assert byte identity.
- Run `summarize_compact_stream` over compact bytes and assert decodable,
  ordered, zero skipped hours, no worker panic, expected domain coverage, and
  one fuel data record per fuel frame.

These tests should not use Python, `collector_frames.jsonl`, or full JSON root
frames as the production path.

## First Typed Fuel Record Contract

- Fuel rows use `COMPACT_RECORD_TYPE_DOMAIN_DATA`; `DOMAIN_PERF` remains for
  extraction metrics and per-frame coverage.
- The host treats unknown record types as parse errors.
- The payload has a fixed 24-byte header and fixed 40-byte row stride:
  elapsed wall ms, row count, changed-row count, status flags, fuel scale
  (`32768`), then rows.
- Fuel fields are raw signed fixed-point integers with explicit scale metadata,
  not f64 payloads. Measured zero remains distinct from an omitted row.
- Include stable country identity in every row, at minimum `tag_db_id`; validate
  duplicate rows and invalid lengths fail closed.
- Treat `gain` and `cost` as signed. Do not assert `current <= max`; current fuel
  can drift at tick boundaries and the collector-side validation already avoids
  that assumption.
- Preserve one sequence authority and make `FRAME_END` the last record for a
  game hour. Parallel read lanes may produce data concurrently, but packaged
  records need deterministic order.

## Still Open

- Run a fresh live speed-five proof with the current fuel data record and
  `.battleplan` packing path. Unit tests and release build are green; the live
  proof is blocked when no the game process is running.
- Add typed data records for production, focus, tech, politics, decisions,
  diplomacy, combat, movement, and orders.
- Fold typed records into the future hourly snapshot spine once the raw compact
  stream is fully covered.
