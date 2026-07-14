# fast-turnaround lane report

## Mission

Keep the capture-run path within the binding 15-minute turnaround budget by deferring the expensive host fold, while retaining an explicit inline-fold opt-in and a standalone offline fold command.

## Changes

- `extractor/native/scripts/Start-Hoi4ProductionSpeed5Capture.ps1:49-53` adds `-RunHostFold`, `-MaxTurnaroundMinutes` (default 15), and `-AcknowledgeLongRun`.
- `extractor/native/scripts/Start-Hoi4ProductionSpeed5Capture.ps1:210-230` applies a 120-second launch/pack margin and rejects projected turnaround above the configured limit before static-artifact checks, PID discovery, output creation, build, or capture. `-AcknowledgeLongRun` bypasses only this guard.
- `extractor/native/scripts/Start-Hoi4ProductionSpeed5Capture.ps1:307-308` passes recorder flag `--skip-host-fold` whenever `-RunHostFold` is absent. The branch is mode-independent, so Smoke, ShortSpeed5, FullGame, and Debug share the deferred default.
- `extractor/native/scripts/Start-Hoi4ProductionSpeed5Capture.ps1:335-360` constructs and prints the exact copyable deferred command on both dry-run and successful capture completion:
  `& "<repo>\extractor\native\scripts\Invoke-DeferredFold.ps1" -RunDir "<run-dir>" -Workers 4`
- `extractor/native/scripts/Invoke-DeferredFold.ps1:1-126` is the new offline wrapper. It validates the run directory, reads `collector_stream.bin` and `native_session_manifest.json`, uses the manifest `replay_game_id`, builds `stream_fold` unless `-SkipBuild` is supplied, and writes derived snapshots plus `deferred_fold_report.json`. `-Workers` defaults to 4 and is passed explicitly as `--workers` (`:6`, `:59-65`, `:98-107`).

No capture profile, emitter/read-mode flag, collector code, recorder-host code, or fold implementation changed.

## Correctness argument

This is control-plane separation only. The recorder already implements explicit `--skip-host-fold` handling and marks the packed container/manifest as skipped (`recorder_host/src/main.rs:1398-1420`). The deferred wrapper calls the existing `stream_fold` binary, whose CLI dispatches to `fold_compact_stream_file_parallel` with the requested worker count (`recorder_host/src/bin/stream_fold.rs:51-97`). Therefore this lane does not alter stream bytes, domain emission, fold semantics, snapshot construction, or capture profiles.

The wrapper fails closed on a missing stream, missing/unparseable manifest, missing `replay_game_id`, existing report, or non-empty deferred snapshot directory. It never guesses a game id and never mixes a new fold with stale output.

## Validation evidence

Validation used Windows PowerShell 5.1 with explicit dummy PID `4242` and `-DryRun`; no live process discovery, injection, capture, or fold was performed.

- Parser: `System.Management.Automation.Language.Parser::ParseFile` reported `PARSE_OK` for both PowerShell scripts (2/2 files, 0 parse errors).
- Default modes: Smoke, ShortSpeed5, Debug, and acknowledged FullGame each exited 0; every generated recorder command contained exactly the shared `--skip-host-fold` default path and printed the deferred command.
- Inline opt-in: Smoke with `-RunHostFold` exited 0 and its recorder command did not contain `--skip-host-fold`; the profile remained `production`.
- Guard boundary: `DurationSeconds=780` plus 120 seconds equals 900 seconds and exited 0 at the default 15-minute limit. `DurationSeconds=781` plus 120 seconds equals 901 seconds and exited 1 with the acknowledgment instruction.
- Default FullGame: 21,600 capture seconds plus 120 seconds projected 362 minutes and exited 1 before generated-catalog checks. Adding `-AcknowledgeLongRun` exited 0 and retained default deferred folding.
- Deferred wrapper: dry-run read fixture game id `fast_turnaround_validation`, reported `workers=4`, and emitted a `stream_fold ... --workers 4` command. A separate dry-run with `-Workers 7` emitted `--workers 7`.
- Scope after fixture cleanup: only the launcher, the new deferred wrapper, and this lane report are modified/untracked.

`cargo build --release` was not run because no Rust or `recorder_host` source was touched; dry-run printed but did not execute the build command.

## Refuted premise / NULLs

No new fold implementation was required. The existing `stream_fold` CLI already exposes the parallel strided fold and a worker-count flag, so the safe implementation is a PowerShell orchestration wrapper.

## Still to validate

Per the lane restriction, no live capture or real 34 GB deferred fold was run. The orchestrator should independently review the diff and may mechanically spot-check the printed command on the next real capture; this implementer did not perform its own adversarial review.

## Fix round

### Defects fixed

- extractor/native/recorder_host/src/main.rs:1092,1540-1977 adds the input/packaging-only --fold-and-repack-existing-run-dir mode. It reads the existing compact run manifest and stream, requires the explicit skipped-fold state, reuses fold_compact_stream_file_parallel plus the same streaming container writer as inline finalization, atomically replaces the run's named .battleplan, strictly updates standalone host_finalize.fold telemetry, and removes its unique snapshot spill directory on success or failure. world_fold.rs is untouched.
- extractor/native/recorder_host/src/main.rs:1295,1672-1806,3820-3997 records the packaging creation label, compression profile, static-artifact names, and sanitized payload hashes before the initial pack. Deferred repack requires those exact inputs and fails closed if a static artifact changed. Runtime-only peak-memory fields remain authoritative in the standalone post-pack manifest rather than the embedded pre-pack copy, making the container bytes deterministic across inline and deferred packaging from the same run inputs.
- extractor/native/scripts/Invoke-DeferredFold.ps1:1-199 now invokes that repack mode against the run directory instead of writing deferred_fold_snapshots and deferred_fold_report.json. It validates the pending container/manifest, reconstructs only the recorded static-artifact arguments, verifies final fold telemetry, and reports the updated replay path.
- extractor/native/scripts/Start-Hoi4ProductionSpeed5Capture.ps1:219-235 adds the inline-fold estimate (DurationSeconds * EstimatedMegabytesPerSecond) / 16.6 to the turnaround guard only when -RunHostFold is set. Lines 375-382 print the measured post-run turnaround and emit both Write-Warning and a yellow WARNING: line when the actual limit is exceeded.
- extractor/native/scripts/Invoke-DeferredFold.ps1:5-8,141-144 uses zero as “no override” and omits --workers; recorder_host therefore executes the shared default_fold_workers() logic (cores/2 clamped 1-8). A positive -Workers still passes an explicit override. The launcher no longer prints -Workers 4.

### Correctness evidence

- tests::deferred_repack_container_is_byte_equivalent_to_inline_fold (extractor/native/recorder_host/src/main.rs:27601) creates the same one-hour H4CR fixture in inline and skipped/deferred run directories, runs both paths with identical packaging inputs, and asserts equality of the complete .battleplan byte vectors. It then opens and verifies the deferred container, confirms one indexed snapshot, confirms embedded and standalone host_finalize.fold.hours_emitted == 1 with no skipped marker, and confirms no .native_session_snapshots-* directory remains.
- Release fixture command: cargo test --release -p recorder_host --bin collector_record deferred_repack_container_is_byte_equivalent_to_inline_fold -- --nocapture.
  Verbatim result: test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 187 filtered out; finished in 0.11s.
- Default-worker parser coverage at extractor/native/recorder_host/src/main.rs:27579 asserts the omitted-worker mode equals default_fold_workers() and that --workers 7 remains 7. PowerShell dry-run evidence printed workers=default_fold_workers (cores/2 clamped 1-8) with no --workers, and the override printed workers=7 (override) plus --workers 7.
- Inline guard dry-run with DurationSeconds=150, EstimatedMegabytesPerSecond=70, and -RunHostFold exited 1 before artifact checks with: Projected capture-run turnaround is 15.042 minutes (-DurationSeconds plus a 2-minute margin and estimated host fold at 16.6 MB/s), exceeding -MaxTurnaroundMinutes 15.

### Gate results

- Required full gate, from extractor/native: cargo test --release --no-fail-fast.
  - First run exited 101 at the documented -p diplomacy_intel_operatives_live --lib timing-flake target. Immediate isolated rerun was green: test result: ok. 113 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.27s, including payload_phase_times_populated_and_bounded_by_outer_build ... ok.
  - Full rerun: EXIT 0.
  - Count-only confirmation rerun: {"exit":0,"targets":89,"passed":1222,"failed":0,"ignored":9,"measured":0,"filtered":0}.
- PowerShell parser gate:
  - PARSE_OK extractor/native/scripts/Start-Hoi4ProductionSpeed5Capture.ps1
  - PARSE_OK extractor/native/scripts/Invoke-DeferredFold.ps1
- git diff --check: exit 0, no output.
- Final scope is the two PowerShell scripts, recorder_host/src/main.rs, and this lane report only. No world_fold.rs, replay_interface/, documents/AGENTS.md, or plan/queue document changed.

### Remaining validation

No live the game capture was run. All requested behavior, including exact final-container byte equivalence, was exercised offline with the small H4CR fixture.
