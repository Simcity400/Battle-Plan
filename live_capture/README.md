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
| `collector_frames.py` | Synthetic/decoded in-process collector frame validation and snapshot adapter. |
| `collector_profile_check.py` | Profile preflight: running-exe hash match, root-probe sentinel/address/domain validation, and transport diagnostics. |
| `collector_probe_record.py` | Live collector probe recorder that writes verified frames into the durable snapshot store. |
| `collector_profiles/` | Build-gated collector profiles: executable hash, offsets, sentinels, roots, and enabled domains. |
| `registry.py` | Import-guarded reader plug registry. Failed/missing plugs omit domains instead of fabricating data. |
| `assembler.py` | Folds reader results into replay snapshots while preserving no-fabrication semantics. |
| `continuity.py` | Campaign/segment/capture sequence tagging and rollback/time-jump handling. |
| `durable_store.py` | Atomic snapshot/index writes and single-writer locking. |
| `recorder/` | GUI/CLI recorder, cadence scheduling, manifest diagnostics, collector transport adapter, packaging spec. |

## Recorder Smoke / Tests

```powershell
python live_capture/recorder/recorder_main.py --smoke
python -m live_capture.collector_attach --pid <hoi4-pid> --dll replay_native/target/debug/collector.dll --pipe
python -m live_capture.collector_profile_check --pid <hoi4-pid> --profile live_capture/collector_profiles/vanilla_latest_2026_06_28.json --dll replay_native/target/debug/collector.dll
python -m live_capture.collector_probe_record --pid <hoi4-pid> --dll replay_native/target/debug/collector.dll --out-root replay/snapshots/_collector_probe --store-root replay/snapshots --game-id collector_live_speed5_worker --dwell-s 0.25 --worker-duration-s 8 --worker-sample-ms 10 --until-game-hours 12 --max-wall-s 15 --unique-store-hours-only
python -m unittest live_capture.tests.test_recorder_cadence live_capture.tests.test_recorder_worker_cadence live_capture.tests.test_recorder_manifest_diagnostics live_capture.tests.test_replay_writer
python -m pytest live_capture
```

Run `collector_profile_check` as a separate preflight immediately before worker
recording. Do not combine it into the same recorder process; back-to-back hook
loads are more reliable as separate commands.

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
