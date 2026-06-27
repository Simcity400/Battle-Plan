# live_capture

Python capture and recorder layer for the native Battle Plan app.

`live_capture/` turns read-only memory-reader results into schema-conformant
snapshots and portable `.battleplan` recordings. It never writes to HOI4, saves,
or process memory. The capture path is build-gated, fail-closed, and designed to
keep measured zero, captured null, and absent data distinct.

## Current Status

- Snapshot assembly, durable store writes, continuity metadata, and `.battleplan`
  writer integration are implemented.
- The distributable recorder has selectable cadence modes:
  every in-game hour, every 2 hours, every 6 hours, and manual/paused one-shot.
- In-game-hour de-duplication and skipped-hour diagnostics are implemented and
  tested.
- Product parity still needs a live unpaused proof at the intended multiplayer
  speed: either no scheduled capture hours are missed, or skipped hours are
  explicitly recorded in the manifest.

See `../replay_ui/REPLAY_PARITY_COMPLETION_CHECKLIST.md` for the acceptance
checklist.

## Main Pieces

| Module | Role |
| --- | --- |
| `capture.py` | Capture loop, live process attachment hooks, single-shot and interval capture. |
| `registry.py` | Import-guarded reader plug registry. Failed/missing plugs omit domains instead of fabricating data. |
| `assembler.py` | Folds reader results into replay snapshots while preserving no-fabrication semantics. |
| `continuity.py` | Campaign/segment/capture sequence tagging and rollback/time-jump handling. |
| `durable_store.py` | Atomic snapshot/index writes and single-writer locking. |
| `recorder/` | GUI/CLI recorder, cadence scheduling, manifest diagnostics, packaging spec. |

## Recorder Smoke / Tests

```powershell
python live_capture/recorder/recorder_main.py --smoke
python -m unittest live_capture.tests.test_recorder_cadence live_capture.tests.test_recorder_worker_cadence live_capture.tests.test_recorder_manifest_diagnostics live_capture.tests.test_replay_writer
python -m pytest live_capture
```

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

- Memory access is read-only and delegated to `memory_reader/`.
- Unknown HOI4 builds fail closed.
- Save-derived data is validation/discovery/backstop data; do not present it as a
  fresh live read.
- Local capture stores, partial recordings, and finalized recordings are
  gitignored.
- Do not commit personal paths, raw dumps, or scratch probe outputs.
