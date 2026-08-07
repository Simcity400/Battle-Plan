# recorder_host

Native proof host for the production recording path.

This crate is the first Rust-side replacement for the Python hot loop. It does
not walk HOI4 memory. It writes a bounded `collector_control.json`, installs the
collector DLL through the existing read-only Windows hook attach path, keeps the
hook alive for the worker duration, reads the DLL named-pipe stream, writes
either debug `collector_frames.jsonl` or compact `collector_stream.bin`, and
emits `native_session_manifest.json` with cadence and skipped-hour diagnostics.

## Bounded-memory streaming fold/pack (Phase 1b, gate G5)

The recorder never holds the whole session in RAM. Peak memory is
`O(current world-fold state)` — independent of how long the capture runs — so a
full-game capture completes on a weaker PC and finalization takes seconds, not
minutes.

Pipeline (compact/production path):

1. **Capture compresses at the file boundary.** `capture_pipe_to_file` hashes and
   counts every collector pipe byte, then writes it through the shared
   `compact_stream::StreamWriter`. Production and legacy `--production` captures
   default to one streaming Zstandard frame; debug defaults to raw bytes. Nothing
   accumulates in memory, and the collector DLL, game thread, and H4CR wire bytes
   are unchanged.
2. **Summarize through a transparent reader.** The manifest gate auto-detects raw
   versus zstd storage and reads one complete H4CR record at a time. Coverage,
   hourly checks, and per-domain perf/status accounting use the existing record
   decoder body unchanged; peak input memory is bounded by the largest record.
3. **Streaming fold.** `stream_fold::fold_compact_stream_file` reads the stream in
   8 MiB chunks, frames each `H4CR` record against a small carry buffer, and drives
   the one production fold (`WorldFold::ingest_record`). The fold is file-backed
   (`WorldFold::new_file_backed`): each per-hour snapshot is serialized, spilled to
   disk, and *released* the moment its `FRAME_END` closes. The framing is the same
   contract `fold_check` walks, so streaming snapshots are hash-identical to the
   batch fold (gate **G1**).
4. **Streaming pack.** `snapshot::pack_streaming_zstd_to_writer` transparently
   decodes the standalone stream when it is zstd-backed, then writes the canonical
   H4CR bytes as the container entry through the existing ZIP-Zstandard encoder.
   This is bounded-memory and does not double-compress or change the logical entry
   (a trailing zip data descriptor carries the deferred crc/sizes; the true sizes
   live in the central directory, which is the reader's authority).
   Small entries (index/timeline/manifest/static) stay in RAM; snapshots are read
   one file at a time. The stream entry's manifest sha256 is the capture rolling
   hash (no re-read to hash it). Tiny entries stay STORE; large `collector_raw/*`
   and `*.snapshot.json` payloads use ZIP method 93 (Zstandard) with
   long-distance matching and Zip64 metadata when entry counts, sizes, or offsets
   cross zip32 sentinels.
5. **Streaming verify.** `snapshot::verify_replay_container_streaming` re-opens the
   finished container FROM DISK and hashes every entry against the manifest, one at
   a time, seeking to each local header and streaming its compressed bytes through
   the decompressor. The stream entry's decompressed sha256 is compared to the
   capture hash — the streaming replacement for the old whole-stream in-RAM
   byte-compare (behavior delta: evidence-equivalent hash compare rather than a
   `Vec == Vec`). This streaming path allows multi-GiB compact-stream entries
   under a separate streaming cap. The normal `snapshot::Container` reader still
   holds entries in RAM and keeps its 2 GiB per-entry guard; a container with a
   >2 GiB stream entry is valid for recorder self-verify but not for that in-RAM
   reader path.

All hourly gate checks, coverage verification, per-domain perf/status accounting,
manifest fields, and the quarantine-on-gate-fail naming
(`native_session.failed.battleplan`) are preserved unchanged.

### G5 telemetry (`native_session_manifest.json` → `host_finalize`)

The `.battleplan` embeds the telemetry known before packing: peak memory so far,
stream byte count/hash, and fold accounting. That embedded copy is marked with
`host_finalize.standalone_manifest_is_authoritative = true`; post-pack fields
(`finalize_seconds` and `verify.*`) are written only to the standalone
`native_session_manifest.json` after pack/verify completes.

Telemetry fields:

- `host_finalize.peak_private_bytes` — peak private (pagefile-committed) bytes.
  The headline bound: it excludes evictable file-backed pages, so it is the honest
  "did we allocate proportional to the stream?" number.
- `host_finalize.peak_working_set_bytes` — peak working set (includes resident
  file-backed/mapped pages, which the OS evicts under pressure).
- `host_finalize.finalize_seconds` — post-capture finalize wall time.
- `host_finalize.stream_bytes` / `stream_sha256` — decoded capture byte count + rolling hash.
- `host_finalize.stream_stored_bytes` — standalone on-disk bytes after file-boundary compression.
- `host_finalize.fold.*` — hours emitted, record counts, truncated-tail bytes.
- `host_finalize.verify.*` — entries verified and
  `stream_sha256_matches_capture` (the streaming byte-compare result).

### Offline tools

- `stream_fold <collector_stream.bin> <out_dir> [--game-id ID] [--json report.json]`
  runs the identical streaming fold offline: it auto-detects raw or zstd storage,
  reports its own peak private/working-set bytes, and prints per-hour
  `snapshot_hash`es that match `fold_check hash` on the same stream (G1, when
  `--game-id` matches; default `fold_check`).
- `fold_check hash|compare` — the G1 fold-equivalence gate (see `../fold_check`); raw and zstd stream files are transparent inputs.
- `compact_stream compress|decompress <input> <output>` — lossless standalone utility. Use `cargo run --release -p compact_stream -- decompress ...` for header-only Python tools that expect byte zero to be `H4CR`; those tools do not automatically understand a production `collector_stream.bin`, whose byte zero is the zstd frame magic.

## Recording composition (`--package-raw-stream`, DEFAULT OFF)

A recording is composed of three things: the **delta cache**
(`hour_index.h4ix` + `checkpoints.h4cp`) which is what replay consumes, the
**raw wire stream** (`collector_raw/collector_stream.bin`, role
`collector_compact_stream`), and fixed overhead. Projected to a full campaign
those are 8.376 GB, 6.002 GB and 0.087 GB (`documents/DELTA_WIRE_DESIGN.md`
§8.5).

By default the recorder does **not** package the raw stream. The run-dir
`collector_stream.bin` is still captured and is still what the certified cache
is folded from — only the sealed container omits it, which is the ~40% cut.
Replay is unaffected: hours reconstruct from the cache, and the cache's source
binding (length + SHA-256) is taken from the recorder manifest's
`stream_storage`, a container-authenticated entry. `CertifiedReplayCache::open`
then cross-checks that identity — the length against the H4IX header, the
SHA-256 against both the H4IX and the H4CP header — before any hour is served.
(H4CP carries the source digest but not the source length, so the length rests
on H4IX alone.)

What a cache-only recording does **not** do, and cannot, is re-hash the source's
own bytes: they are not in the container. The packaged composition does that via
`Container::authenticate_entry`. So cache integrity is identical either way,
while the claimed H4CR source identity is *attested by the recorder manifest*
rather than *independently demonstrated from source bytes*. Use
`--package-raw-stream` where that independent demonstration is required.

The stream is optional only where the cache replaces it. A run that derives
neither a certified cache nor per-hour snapshots (the legacy `--skip-host-fold`
route) is refused at seal time rather than sealing a container with no
replayable source at all.

`--package-raw-stream` turns the stream back on, for verification and debugging
runs whose consumers need the wire records themselves. It is also accepted by
`--fold-and-repack-existing-run-dir`.

## Offline recovery (`--fold-and-repack-existing-run-dir`)

Recovery re-folds a run directory's raw stream and seals the container the
capture did not. It admits exactly two shapes, and the manifest says which:

* **deferred fold** — `host_finalize.fold.skipped=true`. The capture finalized
  with `--skip-host-fold` and published a container whose fold this run replaces.
* **unfinalized capture** — no `host_finalize` at all. The capture died, or its
  pack failed, before finalize; `replay_file` is null and
  `replay_artifact.pack_failed` is true. There is no container yet. This is the
  case recovery exists for, and the raw stream plus the `stream_storage` receipt
  written at pipe close are what make it recoverable.

A run that already completed a real finalize is refused — repacking it would
re-seal an already-sealed recording — as is one that never recorded the identity
of the bytes it wrote, because nothing could bind a container to them
afterwards. The stream identity is always READ from the capture's own receipt,
never recomputed from whatever is on disk now.

The composition recovery produces is chosen by `uses_binary_replay_persistence`,
the same predicate the live seal path uses, so a repacked recording is the one
its capture would have sealed in process. There is deliberately no second,
manifest-derived copy of that decision and no legacy per-hour JSON fallback:
selecting on `production_profile.enabled` alone is what silently repacked
ordinary compact captures into a composition they never had. `--workers` is
still accepted, but the certified-artifact build is a single streaming pass, so
it does not change the work.

A successful recovery repairs the manifest to name the container it produced and
keeps the capture-time failure under `replay_artifact.deferred_recovery`; a
failed one puts the fail-closed "this run produced no container" record back.

Every sealed recording **states** its composition — never implies it by an
entry's absence. `index.json`, `timeline.json` and the recorder manifest each
carry `raw_stream_packaging`, spelled `packaged` or `omitted`; the cache-only
spelling is accompanied by a `raw_stream_absence_reason`. Readers reject any
other spelling, a non-string value, or a disagreement between those documents,
and the recorder refuses to seal a container whose manifest declares a
composition different from the one it is producing.

Consumers that need the raw wire records — `fold_check`'s completeness audit,
the raw-stream decoder seams, `replay_decoder`'s container-stream API, the
rollover package-set binding and the compact replay verification gate — refuse a
cache-only recording by name, quoting the reason and the flag, rather than
reporting a bare missing entry.

## Capture profiles (`--profile production|debug`)

`--profile` selects one coherent preset cluster instead of hand-assembling the
individual flags. It is defined once in `compact_contract::CaptureProfile`
(single source of truth shared with the collector DLL) and written verbatim as
the `profile` field of `collector_control.json`.

**Both profiles are identical where it matters:** the same compact stream, the
same compact domain reader code, the same `DOMAIN_DATA` schemas, and the same
fold. Only emission/diagnostics verbosity differs. `--stream-format json` is a
divergent dead path that bypasses the production readers and is **never** a
profile — both profiles are compact.

**Both profiles preserve every fail-closed data-loss guarantee**, always-on and
never a preset choice: hourly cadence (`scheduled_snapshot_step_hours=1`),
skipped-hour accounting, per-domain `SOURCE_UNAVAILABLE` / partial-data
signaling, and the per-hour `DOMAIN_PERF` coverage heartbeat that the host's
coverage gate consumes. Only instrumentation **cost** is toggled.

| Knob | `production` | `debug` | Legacy (no `--profile`) |
|---|---|---|---|
| `stream_format` | compact | compact | json (`--production` -> compact) |
| `collector_stream.bin` storage | zstd | raw | raw (`--production` -> zstd) |
| production domain set | scheduled | scheduled | only if `--production` |
| `compact_country_frame` | on | off (full provenance) | off |
| `reader_concurrency` | 12 | 0 (unbounded) | 0 (`--production` -> 12) |
| `extraction_perf_gate` (RC per-read counters) | **off** | on | off |
| `sidecar_log` (phase-timing logs) | off | on | off |
| `lightweight_root_frame` | off | off | off |
| `min_game_hours` | 48 | 12 | 12 |

- **Override semantics:** any explicitly-passed individual flag wins over its
  preset value (`--extraction-perf-gate`, `--sidecar-log`, `--compact-country-frame`,
  `--lightweight-root-frame`, `--reader-concurrency N`, `--min-game-hours N`,
  `--stream-format ...`, `--stream-compression zstd|none`). The collector DLL applies the same fallback for any
  preset field a control file leaves unset (e.g. the Python dev harness).
- **Omitting `--profile` applies no preset.** A plain legacy probe stays JSON/raw; legacy `--production` now resolves to compact/zstd so production artifacts cannot be multi-GB raw streams.
- **`DOMAIN_PERF` records are emitted in BOTH profiles** (they are NOT gated on
  `extraction_perf_gate`). They are ~0.1 % of stream bytes but are load-bearing:
  the host coverage gate (`compact_domain_coverage_errors`) requires exactly one
  `DOMAIN_PERF` record per scheduled domain per frame hour and reads its
  per-domain `status_flags` for `SOURCE_UNAVAILABLE`/partial-data detection. Only
  the per-read RC counters (`rc_bump`, ~3.17 M calls/frame) and the perf
  diagnostic-unit trailer are instrumentation cost, and those are what
  `extraction_perf_gate=false` turns off.
- **`lightweight_root_frame` stays off in both profiles.** For the production
  domain set it is inert: the scheduled country-identity-dependent domains
  (fuel/production/focus/tech/politics/decisions/diplomacy/construction/supply/…)
  force the country-identity scan via `lightweight_root_needs_country_identity`,
  and the identity rows (tag <-> tag_db_id) re-key downstream `DOMAIN_DATA` by
  TAG. Setting the flag would buy nothing and risk dropping replay-consumed data.

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
- Project capture now supports `--special-projects-bridge` /
  `--special-projects-bridge-file` for bounded cache/preflight discovery outside
  the hot loop. The DLL/Rust compact path then rereads and validates those
  roots hourly. `--special-projects-scan` remains a debug fallback only; scan
  failure or timeout is not production completion.
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

Production speed-five launcher:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File extractor\native\scripts\Start-Hoi4ProductionSpeed5Capture.ps1 -Mode ShortSpeed5
```

The launcher rebuilds the release collector/host unless `-SkipBuild` is passed,
auto-detects the single live `hoi4.exe` PID, uses the compact stream production
profile, enables the extraction perf gate, includes `--compact-country-frame`,
sets the tuned production reader concurrency, and passes every required static
artifact from `extractor/game_defs/out`. `-Mode FullGame` defaults to a long run
and performs a conservative disk-space estimate; use `-OutDir` on a large drive
for full-game speed-five capture. Two control-field pass-throughs feed the live
validation session: `-D53RelationReadMode offsets|scan|shadow` (writes
`d53_relation_read_mode` into the DLL control file) and `-EmitModeAll delta|full`
(forces all five delta domains `--d41/44/48/51/53-emit-mode`). Both default to
the production behavior when omitted.

## Live validation session

`Invoke-LiveValidationSession.ps1` runs the speed-5 deep-war validation
runsheet against a client that is already advancing at speed 5. The graded
capture window is passive: from attach through authenticated teardown it may
observe the target and use the pinned collector hooks, but it must not pause,
resume, change speed, focus, minimize, launch, stop, or otherwise drive the
client. The host retains the authenticated H4RD/H4RA pipe as a collector-only
cooperative-stop capability. It never terminates the recorder after injection:
the direct harness uses its bounded `--duration-s`, and the product stop request
closes that retained pipe so the collector unhooks and drains H4CR. Successful
`CAPTURE_TEARDOWN_V1.json` publication requires H4CR EOF, collector-module
absence, and byte-for-byte restoration of all 36 pinned hook targets against
their pre-injection baseline. One command:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File extractor\native\scripts\Invoke-LiveValidationSession.ps1
```

What it does (all captures reuse `Start-Hoi4ProductionSpeed5Capture.ps1`):

1. **Preflight** — require exactly one `hoi4.exe`, build release
   collector + recorder_host, independently observe exact speed 5, and run a
   ~12 s passive probe capture that must show `game_hours_advanced > 0`. If the
   precondition is not met, abort without changing the client. Coarse disk-space
   floor check.
2. **Stage 1 — d53 shadow gate** — a ~75 s capture with
   `-D53RelationReadMode shadow`. **PASS iff
   `domain53_relation_shadow_mismatches == 0` (absent counts as 0) on every d53
   frame** and the run actually ran in shadow mode. Any mismatch is a HARD BLOCKER
   (the offsets relation reader diverged from the legacy scan) and the session
   stops before the production capture.
3. **Stage 2 — paired A/B** — a `-Mode ShortSpeed5` capture at head, compared
   against the manifest supplied with `-BaselineManifest`. The script's built-in
   baseline is not retained in this repository, so callers must supply an
   existing manifest.
   Writes `live_validation_report.md` (+ a compact console table): frame
   avg/p95/max, game-hours advanced, skipped hours + %, stream bytes +
   bytes/captured-hour, per-domain `read_time_us` top-10 vs baseline, the five
   delta domains' emitted-vs-observed rows, `host_finalize` peak RAM + finalize
   seconds, and a VERDICT block (skipped==0, frame avg vs the 20 ms / 45 ms
   targets, size-reduction factor vs baseline).
4. **Stage 3 (optional, `-IncludeEmitModeAB`)** — a short capture with all five
   delta domains forced to `emit_mode=full`, to attribute the size reduction.

Pass criteria: Stage 1 zero shadow mismatches (blocker on any mismatch); Stage 2
zero skipped hours (zero-data-loss); the report's VERDICT block reports frame
avg vs targets and size reduction vs baseline.

Modes for testing without the game:

- `-DryRun` — validates argument assembly + baseline parsing and prints the exact
  launcher invocations + a comparison skeleton, without attaching to `hoi4.exe`.
- `-SelfTest` — exercises the pure manifest-parsing / verdict functions against
  the real baseline manifest plus synthetic improved/shadow fixtures.

Example:

```powershell
cargo run --manifest-path extractor/native/Cargo.toml -p recorder_host --bin collector_record -- `
  --pid <hoi4-pid> `
  --dll extractor\native\target\debug\collector.dll `
  --out-dir replay\snapshots\_native_collector_session\clock_lightweight `
  --duration-s 180 `
  --sample-ms 10 `
  --min-game-hours 12 `
  --scheduled-domain clock `
  --lightweight-root-frame
```

Project probe example:

```powershell
cargo run --manifest-path extractor/native/Cargo.toml -p recorder_host --bin collector_record -- `
  --pid <hoi4-pid> `
  --dll extractor\native\target\debug\collector.dll `
  --out-dir replay\snapshots\_native_collector_session\projects_lightweight `
  --duration-s 15 `
  --sample-ms 10 `
  --min-game-hours 120 `
  --scheduled-domain projects_live `
  --lightweight-root-frame `
  --special-projects-bridge-file extractor\native\runs\<run>\special_projects_bridge.txt
```

The command exits nonzero when the manifest fails the requested hourly gate.

Current compact project proof:

```powershell
cargo run -p recorder_host --release --target-dir target_projects_bridge_v4 -- `
  --pid 45880 `
  --dll target_projects_bridge_v4\release\collector.dll `
  --out-dir runs\compact_projects_bridge_20260701_v4 `
  --duration-s 20 `
  --sample-ms 0 `
  --min-game-hours 20 `
  --stream-format compact `
  --extraction-perf-gate `
  --special-projects-bridge-file runs\compact_projects_bridge_20260701_v1\special_projects_bridge.txt `
  --scheduled-domain projects_live `
  --scheduled-domain focus_live `
  --scheduled-domain decisions_live
```

`compact_projects_bridge_20260701_v4` advanced 35 game hours with zero skipped
hours, emitted 36 `projects_live` compact data records, kept the stream
ordered/decodable, had no source/partial/bridge/worker failures, captured 864
facility rows and 324 currently worked project rows, and verified
`native_session.battleplan`. The manifest intentionally reports
`projects_root_source = "control_bridge"` and
`projects_root_discovery_debt = "temporary bounded preflight/cache input; not
DLL-complete project root discovery"`.

Compact release country-domain proof example:

```powershell
cargo build --release --manifest-path extractor/native/Cargo.toml -p collector -p recorder_host

extractor\native\target\release\collector_record.exe `
  --pid <hoi4-pid> `
  --dll extractor\native\target\release\collector.dll `
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
`naval_combat_members`, `air_wings`, `air_wing_loadouts`, `ships`, and
`oob_command` are captured
only if explicitly requested. This keeps independent country-domain proof gates
from being slowed by unrelated late-game combat traffic. Raw clock deltas follow
the same rule for filtered sessions: request `--scheduled-domain clock` when a
raw clock sidecar is the thing under test. With no domain filter, the DLL keeps
the legacy default of enabling all domains.

`air_wings` now has a no-bridge compact source for the verified 870ebf build.
Compact domain id 30 decodes packed owner/wing/generation/count/mission/effective
plane rows plus delete rows, and rejects malformed lengths, impossible bounds,
mission-region flag mismatches, non-zero deleted-row live fields, and non-zero
reserved bytes. The legacy `air_wing_body_bridge` remains debug/proof fallback
only; it is not part of the production recording path.

`air_region_activity` is compact domain id 31. It decodes DLL-derived region/
owner/mission buckets from the anchored air-wing sample, with wing counts, raw
plane counts, effective active planes, and delete rows. It rejects malformed
lengths, duplicate region/owner/mission rows, impossible ids/counts, unknown
flags, non-zero deleted-row counts, non-zero reserved bytes, and trailing bytes.

`air_wing_loadouts` is compact domain id 32. It decodes DLL-derived home
airbase ids from `pool+0x18` `{id,type=65}` handles plus type-70 equipment
child rows from `wing_body+0x1E8`. The host rejects malformed row/child lengths,
duplicate owner/wing/generation rows, invalid airbase ids, invalid equipment
variant ids, invalid raw amounts, duplicate variants per wing, non-zero
deleted-row live fields, non-zero reserved bytes, and child ranges that do not
match the emitted equipment table. A proof run advanced 113 game hours with
0 skipped and no source/partial/bridge/worker
failures while scheduling `air_wings`, `air_region_activity`, and
`air_wing_loadouts` together.

`air_variant_defs` is compact domain id 33. It decodes DLL-derived equipment
variant definition rows collected from the same anchored air-wing equipment
banks. Rows are keyed by `variant_id`, carry optional display/model strings, and
own child module rows `{slot_handle, module_token}`. The host rejects malformed
lengths, duplicate variant ids, unknown flags, impossible slot handles,
overlong/empty required strings, non-zero reserved bytes, overlapping/missing
module ranges, and trailing bytes. A proof run advanced 109 game hours with
0 skipped and no source/partial/bridge/worker
failures while scheduling `air_wings`, `air_region_activity`,
`air_wing_loadouts`, and `air_variant_defs` together.

`air_variant_stats` is compact domain id 35. It decodes DLL-derived type-70
variant stat-cache rows from the same anchored live air-wing equipment bank
path. Rows are keyed by `variant_id` and carry raw equipment token/type handles,
raw interface classifier, speed cache, range, attack, agility, max speed,
reliability, air defence, thrust, fuel consumption, and build-cost raw values
with explicit fixed-point scale metadata. The host rejects malformed lengths,
duplicate variant ids, zero equipment token/type handles, out-of-range raw
stat fields, unknown flags, non-zero reserved bytes, and trailing bytes. A proof
run advanced 105 game hours with 0 skipped and no source/partial/bridge/worker
failures while scheduling `air_wings`, `air_region_activity`,
`air_wing_loadouts`, `air_variant_defs`, `air_variant_stats`, and
`air_base_status` together. The generated `.battleplan` also passed an
independent `pack_replay verify`.

`air_variant_reserves` is compact domain id 36. It decodes DLL-derived country
stockpile rows from the anchored country production-manager stockpile and
filters them to aircraft `variant_id`s observed in live air-wing equipment
banks. Rows are keyed by `(tag_db_id, variant_id)` and carry raw fixed-point
stockpile amount plus delete markers. The host rejects malformed lengths,
duplicate country/variant rows, impossible country or variant ids, out-of-range
amounts, unknown flags, non-zero reserved bytes, deleted rows with non-zero
amounts, bad scale metadata, and trailing bytes. A proof run advanced 103 game
hours with 0 skipped and no source/partial/bridge/worker
failures while scheduling `air_wings`, `air_region_activity`,
`air_wing_loadouts`, `air_variant_defs`, `air_variant_stats`,
`air_variant_reserves`, and `air_base_status` together. The generated
`.battleplan` also passed an independent `pack_replay verify`. This proves
stockpile amounts for aircraft variants already discovered through active live
air wings; reserve-only aircraft variants still need a broader equipment
catalog root before the reserve slice can claim complete aircraft inventory
coverage.

`stockpile_variant_defs` is compact domain id 37. It decodes DLL-derived
type-70 variant definitions rooted in the country production-manager stockpile,
using the same row/module/string payload layout and validation rules as
`air_variant_defs`. The stream is pointer-free and keyed by `variant_id`; it is
intended to cover stockpiled variants, including aircraft variants not currently
assigned to live air wings, without a heap-wide type-70 scan in the production
recording loop. A proof run advanced 103 game hours with 0 skipped and no
source/partial/bridge/worker
failures while scheduling the full current air bundle plus
`stockpile_variant_defs`; the generated `.battleplan` also passed an independent
`pack_replay verify`.

`air_superiority_inputs` is compact domain id 38. It decodes DLL-derived
region/owner/mission/variant input rows joined from the anchored live air-wing
activity snapshot, live wing loadout equipment banks, and validated type-70
variant stat rows. Rows are keyed by `(region_id, owner_tag_db_id,
mission_mask, variant_id)` and carry raw equipment token/type/interface
classifiers, source wing count, raw fixed-point equipment amount, and delete
markers. The host rejects malformed lengths, duplicate keys, invalid ids/counts
or amounts, zero equipment token/type handles, bad scale metadata, unknown
flags, non-zero reserved bytes, and deleted rows carrying live fields. This is
input capture for reconstructing/modeling region air power; it does not claim
the exact cached displayed HOI4 air-superiority percent. A proof run advanced
103 game hours with 0 skipped and no source/partial/bridge/worker
failures while scheduling the full current air bundle plus
`air_superiority_inputs`; the generated `.battleplan` also passed an
independent `pack_replay verify`.

`air_variant_upgrades` is compact domain id 39 and
`stockpile_variant_upgrades` is compact domain id 40. They decode DLL-derived
type-70 variant upgrade rows from active air-wing variant pointers and country
stockpile variant pointers respectively. Rows are keyed by `variant_id` and
own child `{upgrade_index, level}` entries. The host rejects malformed lengths,
duplicate variant ids, invalid upgrade indexes or levels, duplicate indexes
inside one variant row, unknown flags, non-zero reserved bytes, bad row/entry
ranges, and trailing bytes. The stream intentionally carries raw indexes, not
upgrade names, because live upgrade-name key resolution is still unproven.
A proof run advanced 99 game hours with 0 skipped and no
source/partial/bridge/worker
failures while scheduling the full current air bundle plus both upgrade
domains. It emitted 446 stockpile upgrade entries; active-air upgrade rows were
valid but zero-entry in this vanilla sample. The generated `.battleplan` also
passed an independent `pack_replay verify`.

`air_base_status` is compact domain id 34. It decodes DLL-derived air-base rows
from the anchored strategic-air state-indexed air-base table. Rows are keyed by
airbase id and carry state id, materialized capacity, state air-base building
level, based wing count, and based plane count. The host rejects malformed
lengths, duplicate airbase ids, invalid state/capacity/level/usage bounds,
unknown flags, non-zero deleted-row live fields, non-zero reserved bytes, and
trailing bytes. A proof run advanced 106 game hours with 0 skipped and no
source/partial/bridge/worker
failures while scheduling `air_wings`, `air_region_activity`,
`air_wing_loadouts`, `air_variant_defs`, and `air_base_status` together.

Compact extraction gate example:

```powershell
extractor\native\target\release\collector_record.exe `
  --pid <hoi4-pid> `
  --dll extractor\native\target\release\collector.dll `
  --out-dir extractor\native\runs\compact_fuel_production_speed5_v1 `
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

`compact_land_oob_weather_870ebf_speed5_20260630_v1` refreshes the active
collector profile for the 2026-06-30 vanilla executable
`870EBF1F615262FACCA48A3B947775C41F0A5338F3068C40958209F5C41C65AD` and proves
the current land-command bundle on that build. The DLL profile now uses
`CGameStateDailyUpdate=0x1D4390`, `CGameState=0x3316E90`,
`CountryDataBase=0x33189C8`, and `GetLongString=0x526A40`, with refreshed
land/OOB/weather vtables from anchored live objects. The 30-second compact
speed-five gate scheduled `divisions`, `division_equipment`, `army_orders`,
`oob_command`, and `weather_live`; it advanced 57 game hours with 58 frame-end
records, skipped 0, had full per-domain data coverage, ordered/decodable
records, no source-unavailable/partial/bridge/worker flags,
`division_root_source = "dll_native"`, avg frame 19.299 ms, p95 22.352 ms, max
23.016 ms, 2,800,072 compact bytes, and an independent `pack_replay verify`
pass. The new `oob_command` data records captured 433 active army rows, 110
army-group rows, 388 leader rows, 1,628 division membership rows, 227 army-group
membership rows, owner rows, engine member counts, and missing-member-count
diagnostics through the DLL/Rust compact path. This is active land-command OOB
coverage only; empty armies, fleets, task forces, theatres, and full command
roots remain open.

`compact_country_870ebf_speed5_20260701_v2` refreshes the same active profile's
country-state validators for politics, ideas, leaders, decisions, research, and
equipment variants. The 45-second compact speed-five gate scheduled
`fuel_live`, `production_live`, `focus_live`, `tech_live`, `politics_live`,
`politics_residuals`, `decisions_live`, and `diplomacy_live`; it advanced 86
game hours with 87 frame-end records, skipped 0, had full per-domain data
coverage, ordered/decodable records, no source-unavailable/partial/bridge/
worker flags, avg frame 14.702 ms, p95 17.691 ms, max 23.922 ms, 640,154
compact bytes, and an independent `pack_replay verify` pass. Recorder semantic
counters show real typed payloads for all eight domains: 2,480 fuel rows, 5,727
stockpile rows, 2,434 military lines, 1,465 naval lines, 1,462 construction
rows, 1,005 active research slots, 1,832 popularity rows, 356 politics-residual
country rows, 397 decision-country rows, 50 timed decisions, and 355 diplomacy
country rows.

`compact_full_current_870ebf_speed5_20260701_v2` is the current wide proof for
the first 16 implemented compact domains on the 2026-06-30 vanilla executable
profile.
After refreshing current-build map and land-combat validators, the 60-second
speed-five gate scheduled the eight country domains plus `divisions`,
`division_equipment`, `army_orders`, `oob_command`, `land_combats`,
`map_control`, `map_detail`, and `weather_live`. It advanced 108 game hours
with 109 frame-end records, skipped 0, had full per-domain data coverage,
ordered/decodable records, zero parse/order errors, no source-unavailable/
partial/bridge/worker flags, avg frame 35.455 ms, p95 38.570 ms, max 87.869
ms, 6,985,286 compact bytes, and an independent `pack_replay verify` pass.
Recorder semantic counters covered real payloads for every scheduled domain,
including country economy/politics/research/decisions, divisions,
division-equipment entries, active army orders, active land OOB, land-combat
participants, map control, map resources/buildings, and weather regions.

`compact_ships_native_870ebf_20260701_010232` completes compact `ships`
recording for the same verified executable profile. The DLL collector walks the
anchored country/fleet/task-force/ship path without a Python bridge, while the
Rust host verifies ordering, decodability, cadence, byte volume, and replay
packing. The 20-second ships-only speed-five gate advanced 39 game hours with
40 frame-end records, skipped 0, had no source-unavailable/partial/bridge/
worker flags, avg frame 33.733 ms, p95 35.008 ms, max 35.618 ms, 147,677
compact bytes, `ship_root_source="dll_native"`, and an independent
`pack_replay verify` pass. Recorder counters observed 55,195 ship rows and
6,318 task-force rows, emitting 2,692 changed ship rows including 65 deletes.

`compact_full_plus_ships_870ebf_20260701_010320` is the current widest
production-shaped gate. It scheduled the prior 16 compact domains plus `ships`
for 30 seconds, advanced 53 game hours with 54 frame-end records, skipped 0,
had full per-domain data coverage, ordered/decodable records, zero parse/order
errors, no source-unavailable/partial/bridge/worker flags, avg frame 47.441 ms,
p95 57.250 ms, max 73.141 ms, 5,237,329 compact bytes, and an independent
`pack_replay verify` pass. `ships` emitted 3,133 changed rows while observing
74,160 ship rows and 8,473 task-force rows; ship read p95 was 43.519 ms.

`compact_oob_ships_native_870ebf_20260701_nav_oob_v2` widens compact
`oob_command` with DLL-native naval hierarchy rows while keeping `ships`
scheduled. The 20-second focused gate advanced 38 game hours with 39 frame-end
records, skipped 0, had full per-domain data coverage, ordered/decodable
records, zero parse/order errors, no source-unavailable/partial/bridge/worker
flags, avg frame 50.756 ms, p95 52.189 ms, max 52.223 ms, 216,303 compact
bytes, and an independent `pack_replay verify` pass. Recorder counters showed
real naval hierarchy payloads: 122 fleet rows, 202 task-force rows, 414
task-force member ids, 1,615 ship member ids, 3,939 observed fleet rows, and
6,268 observed task-force rows.

`compact_full_plus_naval_oob_870ebf_20260701_v1` was the widest
production-shaped gate immediately after the OOB schema widening. It scheduled
all 17 compact domains for 30 seconds, advanced 53 game hours with 54 frame-end
records, skipped 0, had full per-domain data coverage, ordered/decodable
records, zero parse/order errors, no source-unavailable/partial/bridge/worker
flags, avg frame 59.167 ms, p95 70.233 ms, max 80.460 ms, 4,655,364 compact
bytes, and an independent `pack_replay verify` pass. `oob_command` emitted 950
changed rows, including 124 fleet rows, 195 task-force rows, 404 task-force
member ids, 1,625 ship member ids, 5,441 observed fleet rows, and 8,638
observed task-force rows; OOB read p95 was 55.044 ms.

`compact_full_plus_oob_owner_870ebf_20260701_v2` was the widest
production-shaped gate after adding owner attribution to compact fleet and
task-force rows. It scheduled all 17 compact domains for 30 seconds, advanced
53 game hours with 54 frame-end records, skipped 0, had full per-domain data
coverage, ordered/decodable records, zero parse/order errors, no
source-unavailable/partial/bridge/worker flags, avg frame 58.891 ms, p95
72.759 ms, max 87.120 ms, 4,651,183 compact bytes, and an independent
`pack_replay verify` pass. `oob_command` captured 997 changed rows and 964
owner rows, including 152 fleet rows, 233 task-force rows, 536 task-force
member ids, 1,661 ship member ids, 474 missing-member-count rows, 5,940
observed fleet rows, and 9,243 observed task-force rows; OOB read p95 was
59.421 ms.

`compact_full_plus_ship_owner_870ebf_20260701_v1` was the widest
production-shaped gate after adding owner attribution to compact `ships` rows.
It scheduled all 17 compact domains for 30 seconds, advanced 53 game hours with
54 frame-end records, skipped 0, had full per-domain data coverage,
ordered/decodable records, zero parse/order errors, no
source-unavailable/partial/bridge/worker flags, avg frame 58.799 ms, p95
68.350 ms, max 93.165 ms, 5,004,085 compact bytes, and an independent
`pack_replay verify` pass. `ships` captured 2,110 changed rows, including 2,080
owner-attributed ship rows, 2,080 task-force rows, 2,080 definition rows, 79,068
observed ship rows, and 9,570 observed task-force rows; ship read p95 was
47.590 ms.

`compact_full_plus_tf_location_870ebf_20260701_v1` is the current widest
production-shaped gate after adding compact task-force current-province rows to
`oob_command`. It scheduled all 17 compact domains for 30 seconds, advanced 52
game hours with 53 frame-end records, skipped 0, had full per-domain data
coverage, ordered/decodable records, zero parse/order errors, no
source-unavailable/partial/bridge/worker flags, avg frame 77.385 ms, p95 90.114
ms, max 93.209 ms, 5,090,054 compact bytes, and an independent `pack_replay
verify` pass. `oob_command` captured 203 task-force location rows, 230
task-force rows, and 960 owner rows; `ships` captured 2,615 changed rows,
including 2,462 owner-attributed ship rows. The focused two-domain gate
`compact_oob_ships_tf_location_870ebf_20260701_v4` separately advanced 39 game
hours with 0 skipped, no source/partial/bridge/worker flags, and 187 OOB
location rows.

Implementation note: see `COMPACT_HOI4REPLAY_NEXT_STEP.md` for the bounded
Rust-native packaging path and the test contract for putting `collector_stream.bin`
inside a manifest-covered `.battleplan` without using Python or full JSON root
frames as the production path.
