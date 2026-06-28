# live_capture

Python recorder layer for the native Battle Plan app.

`live_capture/` currently turns read-only memory-reader results into
schema-conformant snapshots and portable `.battleplan` recordings. Under the
in-process collector plan, it also becomes the external recorder/adapter that
consumes collector frames and writes the same replay format. It must keep
measured zero, captured null, and absent data distinct.

Runtime capture has one authoritative source: the live host `hoi4.exe` process.
Saves are optional development fixtures and validation artifacts only; they are
not required to record or replay a live session.

## Role

This package owns capture assembly, durable snapshot writes, continuity
metadata, recorder UI/CLI entrypoints, collector-frame ingestion, and
`.battleplan` writer integration. See `../replay_ui/MASTER_PROGRESS.md` for
current product status and acceptance gates, and
`../IN_PROCESS_COLLECTOR_PLAN.md` for the collector migration plan.

## Main Pieces

| Module | Role |
| --- | --- |
| `capture.py` | Current capture loop, live process attachment hooks, single-shot and interval capture. |
| `collector_attach.py` | External no-op attach helper for loading the collector DLL into a supported `hoi4.exe` and receiving one pipe frame. |
| `collector_country_bridge.py` | Save-free country-object/country-record preflight bridge with same-process cache; feeds compact control contracts while keeping heap scans out of recorder runs. |
| `collector_frames.py` | Synthetic/decoded in-process collector frame validation and snapshot adapter. |
| `collector_map_bridge.py` | State/province map-control preflight helpers and caches; state roots are fast when cached, province array relocation remains the open blocker. |
| `collector_profile_check.py` | Profile preflight: running-exe hash match, root-probe sentinel/address/domain validation, and transport diagnostics. |
| `collector_probe_record.py` | Low-level live collector probe recorder used for bounded cadence proofs and transport debugging. |
| `collector_session.py` | Recorder-facing collector session wrapper that writes lifecycle frames, durable snapshots, `.battleplan` output, and cadence/transport diagnostics without requiring saves. |
| `collector_profiles/` | Build-gated collector profiles: executable hash, offsets, sentinels, roots, and enabled domains. |
| `registry.py` | Import-guarded reader plug registry. Failed/missing plugs omit domains instead of fabricating data. |
| `assembler.py` | Folds reader results into replay snapshots while preserving no-fabrication semantics. |
| `continuity.py` | Campaign/segment/capture sequence tagging and rollback/time-jump handling. |
| `durable_store.py` | Atomic snapshot/index writes and single-writer locking. |
| `recorder/` | GUI/CLI recorder, cadence scheduling, manifest diagnostics, collector transport adapter, packaging spec. |

## Current Recording Reality

- The live host `hoi4.exe` collector is the only runtime-authoritative source.
  Save oracle output is validation/development data only.
- Canonical replay snapshots are hourly-or-better. Daily rows can be rollups,
  not replay coverage.
- `collector_session` currently wraps repeated bounded DLL workers as one
  logical session. That bridge writes lifecycle frames, durable snapshots,
  manifests, and `.battleplan` output, but the target remains a true
  long-running collector session.
- The session writes a validated `collector_frames.jsonl` sidecar containing
  every accepted DLL frame and embeds the same byte stream into finalized
  `.battleplan` files at `payload/<game>/collector_raw/collector_frames.jsonl`.
  The replay store still keeps one canonical snapshot per unique game hour, so
  the raw stream is where duplicate 10 ms samples survive for future combat,
  movement, and event extraction.
- Country identity/scalars use a preflight cache/control-contract path:
  `tag_db_id:country_object_ptr` plus `tag_db_id:country_record_ptr`, validated
  by the DLL per frame. `country_object_scan` must stay false for normal and
  speed-five proof runs.
- The collector-frame adapter can fold `state_ownership`, `state_control`, and
  `province_control` rows when a collector frame explicitly claims those
  domains. Session manifests summarize raw-frame country/state/province row
  counts.
- `collector_map_bridge` can preflight and cache all state owner/controller
  roots; same-process cache hits are fast enough for recording setup. Scheduled
  DLL emission of those rows is wired and should be proven in live speed-five
  sessions before treating it as fully accepted.
- Province-control dense-array relocation/cache is not yet a normal recording
  source. Do not treat save fallback or slow cold discovery as live replay
  coverage.

## Recorder Smoke / Tests

```powershell
python live_capture/recorder/recorder_main.py --smoke
python -m live_capture.collector_attach --pid <hoi4-pid> --dll replay_native/target/debug/collector.dll --pipe
python -m live_capture.collector_profile_check --pid <hoi4-pid> --profile live_capture/collector_profiles/vanilla_latest_2026_06_28.json --dll replay_native/target/debug/collector.dll
python -m live_capture.collector_session --pid <hoi4-pid> --dll replay_native/target/debug/collector.dll --out-root replay/snapshots/_collector_session --game-id collector_live_speed5_session --duration-s 8 --sample-ms 10 --dwell-s 0.25 --worker-bursts 3 --burst-interval-s 0.25 --until-game-hours 12 --max-wall-s 30
python -m live_capture.collector_probe_record --pid <hoi4-pid> --dll replay_native/target/debug/collector.dll --out-root replay/snapshots/_collector_probe --store-root replay/snapshots --game-id collector_live_speed5_worker --dwell-s 0.25 --worker-duration-s 8 --worker-sample-ms 10 --until-game-hours 12 --max-wall-s 15 --unique-store-hours-only
python -m unittest live_capture.tests.test_collector_session live_capture.tests.test_recorder_cadence live_capture.tests.test_recorder_worker_cadence live_capture.tests.test_recorder_manifest_diagnostics live_capture.tests.test_replay_writer
python -m pytest live_capture
```

Run `collector_profile_check` as a separate preflight immediately before worker
recording. Do not combine it into the same recorder process; back-to-back hook
loads are more reliable as separate commands.

Use `collector_session` for recorder-facing capture work. It wraps the bounded
collector worker with explicit `session_start`/`session_stop` lifecycle frames,
feeds decoded collector frames through the durable store, finalizes a
`.battleplan`, packages the raw collector stream, and records cadence plus
transport diagnostics in the session manifest. Use `collector_probe_record`
directly only when proving collector timing, reproducing transport behavior, or
collecting raw probe fixtures.

Repeated bounded workers are one logical recorder session. The raw probe
manifest keeps every verified observation, but the replay writer processes one
selected frame per unique game hour so duplicate 10 ms samples do not dominate
post-processing. Worker sequence ids restart per bounded worker; diagnostics
therefore report both global reset evidence and burst-local monotonicity. If a
later worker burst fails after earlier verified frames, the session writes the
prefix replay and manifest, then exits fail-closed with the attach error.

Country identity rows include the live `tag_db_id` from the collector tag table
even when country-object scalar scanning is disabled. Treat that id as the safe
join key for future state/province/division/country-object work; do not enable
heap-wide country-object scans in normal recorder runs.
The worker control file and probe manifest explicitly record
`country_object_scan: false` for this reason.

When `collector_probe_record` is asked for `--until-game-hours`, hourly cadence
is a required proof: the run must advance at least the requested number of
in-game hours, keep game time monotonic, and skip zero scheduled hours. The
manifest records `hourly_requirement`; the CLI exits nonzero on proof failure.
Use `--allow-hourly-gaps` only for diagnostic runs where a failed proof should
still exit 0.

For speed-five acceptance, use `--worker-sample-ms 10` until a measured
less-aggressive interval is documented, and keep `--max-wall-s` set. A run that
catches every hour but exceeds `--max-wall-s` fails with
`max_wall_s_exceeded`; that means cadence worked but speed-five throughput was
not proven. The control file intentionally does not enable
`country_object_scan`; heap-wide discovery is not allowed in proof runs. Worker
proof runs use a per-run named-pipe token and keep sidecar file logging disabled
so stale workers and local file I/O cannot contaminate the timing signal.

## Capture Semantics

- A domain is captured only if its plug runs to completion.
- `0` and `0.0` are measured zero. `null` is captured-but-not-measured or
  unavailable. An omitted domain is not captured.
- Failed plugs do not leak partial state into snapshots.
- Unknown or untrusted dates are kept out of the canonical replay index.
- Canonical snapshot filenames are keyed by trusted in-game time and segment.
- The single-writer lock prevents concurrent capture loops from corrupting a
  store.

## Store Layout

```text
<root>/<game_id>/s<segment>.<hour>.snapshot.json
<root>/<game_id>/index.json
<root>/<game_id>/_noncanonical/
<root>/_continuity/<game_id>.state.json
<root>/_continuity/<game_id>.lock
```

Portable recordings are finalized as `.battleplan` containers and can be opened
by the native app through the same snapshot-store interface.

## Safety

- Live access is read-only: legacy external reads stay under `memory_reader/`,
  and collector-frame ingestion consumes already-decoded DLL output.
- Unknown HOI4 builds fail closed.
- Save-derived data is development validation/discovery data only; do not use it
  as runtime fallback and do not present it as a fresh live read.
- Collector transport gaps are manifest data, not silently repaired by saves.
- Local capture stores, partial recordings, and finalized recordings are
  gitignored.
- Do not commit personal paths, raw dumps, or scratch probe outputs.
