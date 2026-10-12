# recorder

Passive CLI to verify HOI4, inject/monitor/detach the collector and finalize `.battleplan` files. Offline commands inspect, compare text saves and read metrics.

Attachment prepares the process-wide executable tables from the verified process executable before injection. Offline save and movement comparisons prepare the same tables from their selected installation. Inspection and finalization use recording metadata without loading an installation.

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

Run from the repository root. Build outside timed measurements:

```powershell
cargo build -p recorder -p collector --release
```

Keep `collector.dll` beside `recorder.exe`. Replace these example paths with your own. A **bare `recorder.exe` attaches to the running game or waits for one**. Use `--help` for options and explicit subcommands for operations; never launch it bare as a probe.

```powershell
$recorder = '.\.cargo-target\release\recorder.exe'
$captureRoot = Join-Path $env:LOCALAPPDATA 'HOI4Replay\captures'
$capture = Join-Path $captureRoot 'recorder-check-YYYYMMDD'
$userDirectory = Join-Path $captureRoot 'smoke-profile'
& $recorder --help
& $recorder record --once --out $capture --user-dir $userDirectory
```

Pass paths as separate quoted arguments/variables. `record` discovers the installation from the process, has no `--game-dir` option and prints its selected Paradox user directory. Without `--out`, recording and its control commands use `HOI4Replay\captures` under `LOCALAPPDATA`, or the user's temporary directory when that variable is absent or empty. Movement reports default to its `movement_oracle` subdirectory. Keep captures and reports outside Git.

Coordinate ownership before attaching. Use one game and one recording controller; multiple game processes, unsupported executable hashes and an already-loaded collector are refused. Never detach or stop another owner's session. The recorder never pauses, saves or sends input to the game.

Game exit finalizes and, without `--once`, waits for another process. Ctrl+C detaches/finalizes/exits. Another terminal can target your session's output directory:

```powershell
& $recorder detach --out $capture
```

The detach command waits for the session's checked completion while the recorder
controller remains alive and progress continues. During drain it follows collector
status publications; after collector absence or game exit it follows actual
metadata, frame and replay-write progress in `finalization-progress.json`. An inactivity
timeout reports stopped progress and retains the spool. Total finalization time
has no deadline. Controller exit without completion fails immediately; normal
game exit can still finish recovery. Completion retains partial/drain failures.

Each process session, in-process load or unexplained hour discontinuity starts a branch. Parentage is probable or unknown; one-second load timestamps can collide. Different campaign IDs produce separate replay files.

Game-score, operation-outcome and politics capture default to enabled. Record-only `--no-game-score`, `--disable-operation-outcomes` and `--disable-politics-capture` preserve named unavailable coverage, never recorded zero.

Finalization uses two streaming spool passes and compresses blocks before the shared decoder size bound. Split hours become self-contained keyframes with original metrics and sample hours; an oversized frame or a split with a truncated table is refused by name. Allocation and I/O failures abort finalization and retain the spool. Repeated finalization compares complete decoded values, availability and metrics, then syncs the matching destination file and parent directory before acknowledging success without changing recording bytes or modification time. A sync failure retains the spool and reports failure.

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

1. Use one fully loaded game, your attached recorder and the same verified content stack. Set `save_as_binary=no` in the selected user directory before launch; binary saves are refused. Save on the recording client so its viewing choice can be compared.
2. Pause at engine hour `H`. Request a full refresh and wait for acknowledgement:

   ```powershell
   & $recorder refresh --out $capture
   ```

   This asks the next hourly sample to read every scheduled table; it sends nothing to the game. A keyframe re-encodes held rows and is not a full refresh.
3. Set the intended game speed before making a new text save through the game UI while paused at `H`. Hold speed and all other state unchanged through the supplying callback; pause separately. If any setting or command changes after the save, repeat the save and refresh request. Match the instant: a later same-hour save can differ, and allocation counters do not prove unchanged state.
4. Step exactly one game hour, pause and wait for the collector to post `H` at its end-of-hour boundary. Detach and wait for finalization. Retain the session's `status.json` and `completed.json`.
5. Inspect the replay. Set `$branchId` to its actual branch, `$oracleHour` to `H`, `$saveFile` to the fresh save and `$gameDirectory` to the matching installation. Run the offline oracle:

   ```powershell
   & $recorder oracle $replayFile $saveFile --hour $oracleHour --branch $branchId --game-dir $gameDirectory --user-dir $userDirectory 2>&1 | Tee-Object -FilePath (Join-Path $capture 'oracle.txt')
   $oracleExit = $LASTEXITCODE
   ```

6. Report the exit status, `Differences`, coverage, failed/skipped groups, content exclusions and limitations. A complete requested-refresh check needs a successful exit, zero differences, zero table availability issues, zero failed/skipped groups and no membership exclusions. **Never waive a difference.** Retain failing artifacts, investigate, fix and repeat the matched check; report what remains unverified.

[Selected transport comparisons](src/oracle/selected_transport.rs) check the complete saved unit census, exact signed combat count and source province for zero-combat transfers. Convoy comparisons retain original cache observations, nonzero pool order and signed stock; physical navy access uses the native signed-positive projection. Runtime priority, reservations, risk graphs and pointer lifetimes retain named limitations. Missing saved operands and current capture failures remain differences.

### Load check

With your recorder attached, load through the game menu. Pause at the load hour, make a **new** text save, step one hour, detach and compare on the new `LoadSignal` branch. Loading reruns weather, so the loaded file cannot substitute. Retain each event family's first-interval coverage verdict; a full refresh cannot supply events from before observation began. Use a fully observed subsequent hour with a requested refresh and a new matched save for the complete check.

### Comparison boundaries

- Daily/sliced groups compare only their actual reads/owners. Carried groups report their read hour instead of comparing stale rows with current save values. [report.rs](src/oracle/report.rs) owns freshness rules; a sibling table's timing is not proof.
- Historical appended-field absence, current read refusal and native empty/zero stay distinct. Readable prefixes still compare, and inspection uses source-format byte sizes. See [replay compatibility](../replay/README.md#compatibility-and-recovery).
- Partial state-history membership skips only dependent joins, names/counts excluded states and makes coverage incomplete. Independent saved fields, identities and locations still compare. History effects are parsed, never executed; see [save parsing](../installed-content/src/save.rs) and [construction comparisons](src/oracle/construction.rs).
- Template comparisons preserve omitted keys versus emitted empty strings and the native first-NUL prefix. Transport comparisons preserve signed convoy allocation and complete wing pool/raid identity; unavailable historical operands remain limitations, while current reached saved-operand faults remain differences.
- Stored names compare the native first-NUL text, retaining counted limitations for captured trailing bytes. State, province, faction, unit and ace-callsign names preserve the stored-length gate and omitted versus emitted-empty keys; unconditional name writers require key presence in ordinary saves.
- Active ideas compare the recorded lexical dictionary's canonical writer spelling in roster order; absent or ambiguous dictionary joins fail the comparison. Advisor-token set membership uses its native case-insensitive hash/key identity, while saved advisor role strings compare exactly. The set's retained spelling depends on unsaved insertion history.
- Research definition names authenticate against the recorded dictionary entry for the exact serialized technology handle, preserving repeated database occurrences and canonical writer spelling. Missing, ambiguous or contradictory bindings are named differences and fail the comparison.
- Native omission rules also apply to advisor idea strings, deployment name flags and state resource blocks. An omitted advisor idea string contributes its empty token key. Unowned states retain the nonpositive owner class; their provinces keep absent ownership and independent explicit controllers. Factory sums use captured control where the state writer omits it, with that input dependency reported. Naval-base membership follows saved navy rosters, and each base occurrence compares its ordered country strings, including repeats and zero-level bases. Raw aliases use their own tag-string slots; unreadable saved operands remain differences.
- Construction self-checks use a line's remaining quantity independently of other target growth: native script effects can add levels without spending the waiting line's IC. Native count-reset observations separate cleared quantity from completed construction, including transfers that lower target levels. Historical prefixes check exact progress and integral completed-cost debits within a witnessed transfer's cleared count. A decrease without completed target levels or a native reset fails. Progress and spending still compare exactly.
- Current route queries, combat-vector order and retained snow/mud display boundaries have runtime-only operands. Match native observations at their supplying hour; saved-value agreement cannot certify those displays.
- Naval task-force participant membership and combat identities compare with the save. The first combat's role depends on its unsaved attacker country set and the unit's current logical owner; participant side cannot certify it. Empty versus nonempty recorded combat identities constrain role presence only as a recording consistency check.
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

Use one game and one recorder for a live check. Coordinate with anyone sharing the machine, and never detach or stop another session. The live tools use a configurable `-GameLock`, defaulting to `HOI4Replay\hoi4_game.lock` under the user's temporary directory. An existing lock or running game/recorder is refused. On failure the tools attempt safe detach and keep the game paused for diagnosis; the lock remains while either process survives.

### Smoke artifacts

[smoke.ps1](../../tools/smoke.ps1) drives the game separately from the passive recorder, then checks finalization, fresh-save agreement and matched renders.

1. Read its help. Prepare an isolated text-save user directory, an existing local save and matching `continue_game.json`. `-ExpectedSave` is the local `.hoi4` **filename**, not a path. Supply calibrated `-UiConfig` coordinates, dimensions and expected hour/seed, and a **new** `-Out` directory under your selected `-CaptureRoot`.
2. Reuse final binaries at the current clean HEAD. `-BuildReceipt` identifies that exact `source_commit` and exactly `recorder.exe`, `collector.dll`, `viewer.exe`, each with `name`, SHA-256 `sha256` and `bytes`. `-BinaryDirectory` defaults to this checkout's `.cargo-target/release`; set it for retained artifacts. The script verifies source/copied hashes and clean HEAD. A receipt links source to artifacts; it makes no live-pass claim.
3. Set `$gameDirectory`, `$buildReceipt`, `$binaryDirectory`, `$uiConfig` and `$smokeOut` to your installation and prepared artifacts, then run from the repository root:

   ```powershell
   powershell -NoProfile -File .\tools\smoke.ps1 -Action Smoke -GameExe (Join-Path $gameDirectory 'hoi4.exe') -CaptureRoot $captureRoot -BuildReceipt $buildReceipt -BinaryDirectory $binaryDirectory -ExpectedSave 'baseline.hoi4' -UserDirectory $userDirectory -UiConfig $uiConfig -Out $smokeOut -WaitForUiReview
   ```

   `BATTLE_PLAN_GAME_DIR` can supply the installation instead of `-GameExe`. `-InputShimDir` is optional; supply it only for checks that require your prepared input helper. `-Checkpoint` runs the workspace checks, builds the three release packages and writes a new receipt; it excludes receipt/binary-directory reuse.
4. At UI review, inspect the paused screenshot and dismiss notifications through guarded driver actions. Keep the map unobscured and paused; write `ui-review-approved.json` in Out with the exact requested `hour` and `unobscured: true`. This cannot waive correctness. Park the capture cursor over inert UI chrome without a tooltip; avoid edge scrolling during timed advance.
5. Inspect `smoke-result.json`, tool logs, boundary save/status, completion and image/score outputs. Success requires clean detach/drain, collector absence, normal exit, unchanged input save, no transport loss, zero oracle differences and complete coverage. Review render results and report failures/unverified checks. Smoke advances at top speed and saves at the first hour past the minimum; its timing is not paired performance acceptance.

Cost checks use matched saves/content and baselines. Keep builds/movement-oracle outside timing; separate cold/steady work. Report game-hours/second, callback p95, memory/pools, encoded size and finalization. See [AGENTS.md](../../AGENTS.md) and [collector](../collector/README.md).

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

Wait for detach/finalization before reading completion. `status.json`/`completed.json` retain failure/hour, partial and drain facts; draining cannot certify capture success. The recorder polls the fixed terminal witness while the collector is loaded, publishes `capture_failed`, and requests detach automatically. Refresh refuses immediately after terminal failure. Authenticated raw-allocation sidecar details supply the attempted family and byte request in status. Finalization still waits for proved collector absence or game exit before recovering the committed prefix. Partial replays open verified prefixes with visible failure notices.

After confirmed collector absence or game exit, set `$sessionDirectory` to a retained session and recover with:

```powershell
& $recorder finalize $sessionDirectory --out $capture
```

Recovery validates [terminal evidence](../replay/src/witness.rs), identity/context and committed spool bounds before keeping CRC-valid prefixes. Only attributable attempted hours become gaps; missing context invents no hour/branch. Invalid evidence and unsupported spools remain on disk with a refusal. Repeated finalization must preserve an existing good replay.

Allocation-failure checks require both binaries' `alloc-fault-injection` feature and coordinated `--alloc-fault <phase>:<positive-ordinal>[:<committed-hours>]`; production builds refuse it. Confirm game survival and partial-replay visibility, then run relevant fresh-save/load and smoke checks.

Update affected inspection, metrics, oracle and playback consumers together. New inputs need a save comparison or named limitation. Follow scoped [AGENTS.md checks](../../AGENTS.md), then relevant fresh-save/load, smoke and native observations above. Documentation-only changes need link/diff checks, not live captures.

The optional installed-save parser regression is ignored by default. To run it explicitly, set `BATTLE_PLAN_LAND_SAVE` to your text save and `BATTLE_PLAN_LAND_EXPECTATIONS` to a JSON file with `division_id` and `division_type` (unsigned 32-bit words), `manpower` (ordered `[tag, signed_integer]` pairs), `maximum` and `remaining` (signed fixed5 raw integers), `history_count` (unsigned count), and `first_date`/`last_date` (date strings accepted by the save parser). It checks the exact selected division values, history count and date bounds after parsing all divisions/history. Missing inputs or invalid expectations fail; an ignored test is not live validation.

```powershell
$env:BATTLE_PLAN_LAND_SAVE = $saveFile
$env:BATTLE_PLAN_LAND_EXPECTATIONS = Join-Path $capture 'land-expectations.json'
cargo test -p recorder oracle::land::tests::installed_save_divisions_and_history_match_expectations -- --ignored --exact
```
