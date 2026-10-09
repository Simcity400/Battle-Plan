# recorder

Passive CLI to verify HOI4, inject/monitor/detach the collector and finalize `.battleplan` files. Offline commands inspect, compare text saves and read metrics.

## Code boundaries

| File | Responsibility |
|---|---|
| [main.rs](src/main.rs), [recording.rs](src/recording.rs) | CLI, process discovery, attach/refresh/detach and completion. `recording::usage` owns options. |
| [windows.rs](src/windows.rs) | Windows process, memory, module and event operations. |
| [replay_io.rs](src/replay_io.rs) | Recovery, finalization, append and inspection. |
| [oracle.rs](src/oracle.rs), [oracle/](src/oracle/) | Save joins, native evidence and unsaved limitations; [report.rs](src/oracle/report.rs) owns coverage reporting. |
| [movement_oracle.rs](src/movement_oracle.rs), [metrics.rs](src/metrics.rs) | Live movement-map comparison and offline interval metrics. |

Native reads belong in [engine](../engine/README.md) and [collector](../collector/README.md), the format in [replay](../replay/README.md), content/save parsing in [installed-content](../installed-content/README.md). Shared development, mod, scale and storage rules live in [AGENTS.md](../../AGENTS.md).

## Record a campaign

Run from your worktree root. Build through the wrapper, outside timed measurements:

```powershell
bash tools/build-slot.sh "dev/recorder" "cargo build -p recorder -p collector --release"
```

Keep `collector.dll` beside `recorder.exe`. Replace these example paths with your own. A **bare `recorder.exe` attaches to the running game or waits for one**. Use `--help` for options and explicit subcommands for operations; never launch it bare as a probe.

```powershell
$recorder = '.\.cargo-target\release\recorder.exe'
$capture = 'captures\recorder-check-YYYYMMDD'
$userDirectory = 'captures\smoke-profile'
& $recorder --help
& $recorder record --once --out $capture --user-dir $userDirectory
```

Pass paths as separate quoted arguments/variables. `record` discovers the installation from the process, has no `--game-dir` option and prints its selected Paradox user directory.

Coordinate ownership before attaching. Use one game and one recording controller; multiple game processes, unsupported executable hashes and an already-loaded collector are refused. Never detach or stop another owner's session. The recorder never pauses, saves or sends input to the game.

Game exit finalizes and, without `--once`, waits for another process. Ctrl+C detaches/finalizes/exits. Another terminal can target your session's output directory:

```powershell
& $recorder detach --out $capture
```

Each process session, in-process load or unexplained hour discontinuity starts a branch. Parentage is probable or unknown; one-second load timestamps can collide. Different campaign IDs produce separate replay files.

Game-score, operation-outcome and politics capture default to enabled. Record-only `--no-game-score`, `--disable-operation-outcomes` and `--disable-politics-capture` preserve named unavailable coverage, never recorded zero.

## Inspect and compare

Use descriptive capture directories; private reports retain source/binary identities, content stack, input save, purpose and interval. Keep the emitted campaign filename and spool together. Inspection shows identity/branches, hours, gaps, partial outcomes, compatibility, coverage and costs:

```powershell
# Use the actual emitted filename.
$replayFile = Join-Path $capture 'CAMPAIGN_ID.battleplan'
& $recorder inspect $replayFile | Tee-Object -FilePath (Join-Path $capture 'inspect.txt')
```

Inspection establishes no save agreement. Set `$oracleHour`/`$branchId` to the run's exact integers for character inspection:

```powershell
& $recorder inspect $replayFile --characters 'SEARCH_TEXT' --hour $oracleHour --branch $branchId
```

### Fresh-save oracle

1. Own the game lock described below. Use one fully loaded game, your attached recorder and the same verified content stack. Set `save_as_binary=no` in the selected user directory before launch; binary saves are refused. Save on the recording client so its viewing choice can be compared.
2. Pause at engine hour `H`. Request a full refresh and wait for acknowledgement:

   ```powershell
   & $recorder refresh --out $capture
   ```

   This asks the next hourly sample to read every scheduled table; it sends nothing to the game. A keyframe re-encodes held rows and is not a full refresh.
3. Make a new text save through the game UI while paused at `H`. Avoid state-changing commands between save and sample. Match the instant: a later same-hour save can differ, and allocation counters do not prove unchanged state.
4. Step exactly one game hour, pause and wait for the collector to post `H` at its end-of-hour boundary. Detach and wait for finalization. Retain the session's `status.json` and `completed.json`.
5. Inspect the replay. Set `$branchId` to its actual branch, `$oracleHour` to `H`, `$saveFile` to the fresh save and `$gameDirectory` to the matching installation. Run the offline oracle:

   ```powershell
   & $recorder oracle $replayFile $saveFile --hour $oracleHour --branch $branchId --game-dir $gameDirectory --user-dir $userDirectory 2>&1 | Tee-Object -FilePath (Join-Path $capture 'oracle.txt')
   $oracleExit = $LASTEXITCODE
   ```

6. Report the exit status, `Differences`, coverage, failed/skipped groups, content exclusions and limitations. A complete requested-refresh check needs a successful exit, zero differences, zero table availability issues, zero failed/skipped groups and no membership exclusions. **Never waive a difference.** Retain failing artifacts, investigate, fix and repeat the matched check; report what remains unverified.

### Load check

With your recorder attached, load through the game menu. Pause at the load hour, make a **new** text save, step one hour, detach and compare on the new `LoadSignal` branch. Loading reruns weather, so the loaded file cannot substitute. Report the first EquipmentDestruction interval's partial coverage; use a requested refresh for the complete check.

### Comparison boundaries

- Daily/sliced groups compare only their actual reads/owners. Carried groups report their read hour instead of comparing stale rows with current save values. [report.rs](src/oracle/report.rs) owns freshness rules; a sibling table's timing is not proof.
- Historical appended-field absence, current read refusal and native empty/zero stay distinct. Readable prefixes still compare, and inspection uses source-format byte sizes. See [replay compatibility](../replay/README.md#compatibility-and-recovery).
- Partial state-history membership skips only dependent joins, names/counts excluded states and makes coverage incomplete. Independent saved fields, identities and locations still compare. History effects are parsed, never executed; see [save parsing](../installed-content/src/save.rs) and [construction comparisons](src/oracle/construction.rs).
- Domain joins and unsaved limits live in [oracle/](src/oracle/). Self-checks cannot certify runtime caches, events, source splits or displays. Changed unsaved inputs need matched native observations at the same identity and supplying read, plus smoke; later rates, net pool changes and reconstruction cannot replace them.

The [retained naval details oracle](src/oracle/combat_details.rs) compares every readable detail row and reports unexpected rows as differences. A saved result with no detail row makes coverage incomplete after those comparisons; zero differences cannot certify missing HQ inputs. HQ inputs themselves are omitted from the native save.
- Only collector-verified sessions compare, including mods; there is no override. Unhashed content reads retain named uncovered-read notes.

The [Event feed](src/oracle/event_feed.rs) and [rocket output](src/oracle/rocket_output.rs) self-checks validate recorded continuity/accounting; saves contain neither native actions nor historical applied IC/deliveries. After a world reset or load their coverage restarts at the next hourly boundary as a partial interval; integration needs matched native observations and the fresh-save/load and smoke workflows. Event feed partial or unavailable intervals are named coverage limitations, with an incomplete comparison at the selected save hour; a save cannot supply a complete-history expectation. Requested full refreshes preserve native event observation. Signed equipment gains and bombing corrections stay exact; air event dates can precede supplying reads.

### Movement load oracle

Movement-map changes need this check once per content stack. Own the game, load fully and pause; an attached recorder may stay attached. Run outside timed capture:

```powershell
& $recorder movement-oracle --user-dir $userDirectory --report (Join-Path $capture 'movement-oracle.json')
```

It opens the single game read-only, requires proved mount order and compares every province's ordered neighbours, terrain flags/ordinal with `MovementMap`. Differences/unreadable provinces fail and are reported. Runtime invasion/diplomacy inputs need matched native route observations.

## Live checks and performance

### Game ownership

The shared lock is the directory at `Join-Path $env:TEMP 'claude\hoi4_game.lock'`, with an `owner` file. Manual checks must acquire it exclusively and record ownership before driving the game. Never remove another owner's lock or take over their game/recorder. Release your lock only after your processes stop safely.

`Smoke` acquires the lock itself and refuses an existing lock or running game/recorder; start with those resources free. On failure it attempts safe detach and keeps its game paused for diagnosis. The lock remains while either process survives. Exit normally after safe collector removal. If an action is denied, stop and report it.

### Smoke artifacts and the checkpoint

[smoke.ps1](../../tools/smoke.ps1) drives the game separately from the passive recorder, then checks finalization, fresh-save agreement and matched renders.

1. Read its help. Prepare an isolated text-save user directory, an existing local save and matching `continue_game.json`. `-ExpectedSave` is the local `.hoi4` **filename**, not a path. Supply calibrated `-UiConfig` coordinates, dimensions and expected hour/seed, and a **new** `-Out` directory under `captures`.
2. Reuse final binaries at the current clean HEAD. `-BuildReceipt` identifies that exact `source_commit` and exactly `recorder.exe`, `collector.dll`, `viewer.exe`, each with `name`, SHA-256 `sha256` and `bytes`. `-BinaryDirectory` defaults to this worktree's `.cargo-target/release`; set it for retained artifacts. The script verifies source/copied hashes and clean HEAD. A receipt links source to artifacts; it makes no live-pass claim.
3. Run from the worktree root with your prepared paths:

   ```powershell
   powershell -NoProfile -File .\tools\smoke.ps1 -Action Smoke -BuildReceipt 'captures\checkpoint\build-receipt.json' -BinaryDirectory 'captures\checkpoint\bin' -ExpectedSave 'baseline.hoi4' -UserDirectory $userDirectory -UiConfig 'captures\smoke-config\ui.json' -Out 'captures\smoke-YYYYMMDD' -WaitForUiReview
   ```

4. At UI review, inspect the paused screenshot and dismiss notifications through guarded driver actions. Keep the map unobscured and paused; write `ui-review-approved.json` in Out with the exact requested `hour` and `unobscured: true`. This cannot waive correctness. Park the capture cursor over inert UI chrome without a tooltip; avoid edge scrolling during timed advance.
5. Inspect `smoke-result.json`, tool logs, boundary save/status, completion and image/score outputs. Success requires clean detach/drain, collector absence, normal exit, unchanged input save, no transport loss, zero oracle differences and complete coverage. Review render results and report failures/unverified checks. Smoke advances at top speed and saves at the first hour past the minimum; its timing is not paired performance acceptance.

Only a coordinator-called checkpoint with inherited `GATE=1`, `JOBS=4` and a build window uses:

```powershell
powershell -NoProfile -File .\tools\smoke.ps1 -Action Smoke -Checkpoint -ExpectedSave 'baseline.hoi4' -UserDirectory $userDirectory -UiConfig 'captures\smoke-config\ui.json' -Out 'captures\checkpoint' -WaitForUiReview
```

It runs the gate once, builds three release packages and writes `build-receipt.json`. `-Checkpoint` excludes receipt/binary-directory reuse and never sets `GATE`. Later checks reuse the receipt at unchanged clean HEAD without builds/tests; `smoke-result.json` with matching source/binary fields also qualifies.

Cost checks use matched saves/content and baselines. Keep builds/movement-oracle outside timing; separate cold/steady work. Report game-hours/second, callback p95, memory/pools, encoded size and finalization. See [AGENTS.md](../../AGENTS.md#live-checks) and [collector](../collector/README.md).

## Exact recorded metrics

Build the recorder from final integrated source before measurement. Both offline forms read an inclusive interval:

```powershell
& $recorder metrics $replayFile $branchId $firstHour $lastHour
# bench.ps1 -MetricsTool uses this positional form.
& $recorder $replayFile $branchId $firstHour $lastHour
```

[bench.ps1](../../tools/bench.ps1) converts its exclusive end first. The JSON schema/calculation lives in [metrics.rs](src/metrics.rs). `callback` is recorded hourly acquisition wall time excluding the native tick, not summed table timers, spool service or game throughput. `pool_bytes_max` is the interval's recorded pool maximum.

Every hour must occur exactly once. Gaps, undated loss, partial/recovered files, damage, unsupported layouts and invalid bounds fail. Unavailable/truncated tables and dropped frames also fail; counts are affected decoded hours including carried tables, not native events. Historical lifetimes follow the code catalog.

## Recovery and checking changes

Wait for detach/finalization before reading completion. `status.json`/`completed.json` retain failure/hour, partial and drain facts; draining cannot certify capture success. Partial replays open verified prefixes with visible failure notices.

After confirmed collector absence or game exit, set `$sessionDirectory` to a retained session and recover with:

```powershell
& $recorder finalize $sessionDirectory --out $capture
```

Recovery validates [terminal evidence](../replay/src/witness.rs), identity/context and committed spool bounds before keeping CRC-valid prefixes. Only attributable attempted hours become gaps; missing context invents no hour/branch. Invalid evidence and unsupported spools remain on disk with a refusal. Repeated finalization must preserve an existing good replay.

Allocation-failure checks require both binaries' `alloc-fault-injection` feature and coordinated `--alloc-fault <phase>:<positive-ordinal>[:<committed-hours>]`; production builds refuse it. Confirm game survival and partial-replay visibility, then run relevant fresh-save/load and smoke checks.

Update affected inspection, metrics, oracle and playback consumers together. New inputs need a save comparison or named limitation. Follow scoped [AGENTS.md checks](../../AGENTS.md#builds-and-merging), then relevant fresh-save/load, smoke and native observations above. Documentation-only changes need link/diff checks, not live captures.
