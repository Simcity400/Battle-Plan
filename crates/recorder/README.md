# recorder

Passive CLI to verify HOI4, inject/monitor/detach the collector and finalize `.battleplan` files. Offline commands inspect, compare text saves and read metrics. The library's `controller` runs the same capture in-process for the [Battle Plan Recorder](../recorder-app/README.md) tray program.

Attachment prepares the process-wide executable tables from the verified process executable before injection. Offline save and movement comparisons prepare the same tables from their selected installation. Inspection and finalization use recording metadata without loading an installation.

## Code boundaries

| File | Responsibility |
|---|---|
| [lib.rs](src/lib.rs), [main.rs](src/main.rs), [recording.rs](src/recording.rs) | CLI, process discovery, attach/refresh/detach, discard and completion. `recording::usage` owns options. |
| [controller.rs](src/controller.rs) | The in-process capture API (stop/discard control, stages, outcome, admission per campaign and per load, and target folder), the read-only campaign probe (campaign, hour and load signal, and the game folder it runs from), a session's recorded content identity and plain problem text. |
| [windows.rs](src/windows.rs) | Windows process, memory, module and event operations. |
| [replay_io.rs](src/replay_io.rs) | Recovery, finalization (publishing only a Battle Plan Recorder session's admitted loads), the admission record, append and inspection. |
| [oracle.rs](src/oracle.rs), [oracle/](src/oracle/) | Save joins, native evidence and unsaved limitations; [report.rs](src/oracle/report.rs) owns coverage reporting. |
| [movement_oracle.rs](src/movement_oracle.rs), [metrics.rs](src/metrics.rs) | Live movement-map comparison and offline interval metrics. |

Native reads belong in [engine](../engine/README.md) and [collector](../collector/README.md), the format in [replay](../replay/README.md), content/save parsing in [installed-content](../installed-content/README.md). Shared development, mod, scale and storage rules live in [AGENTS.md](../../AGENTS.md).

## Record a campaign

Run from the repository root. Build outside timed measurements:

```powershell
cargo build -p recorder -p collector --release
```

While iterating on throughput, `--profile release-incremental` rebuilds a collector edit in seconds instead of minutes (output in `.cargo-target
elease-incremental`); its code can run at a different speed, so smoke, checkpoints and reported benchmark results use `--release`.

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

Coordinate ownership before attaching. Use one game and one recording controller (this CLI or Battle Plan Recorder, not both); multiple game processes, unsupported executable hashes and an already-loaded collector are refused. Never detach or stop another owner's session. The recorder never pauses, saves or sends input to the game.

Game exit finalizes and, without `--once`, waits for another process. Ctrl+C detaches/finalizes/exits. While attached, `record` reads the collector's `status.json` once a minute to print it; Battle Plan Recorder's in-process capture also reads it every two seconds for its panel and campaign admission. Another terminal can target your session's output directory:

```powershell
& $recorder detach --out $capture
```

The detach command waits for the session's checked completion while the recorder
controller remains alive and progress continues. During drain it follows collector
status publications and the collector's step log; after collector absence or game exit it follows actual
metadata, frame and replay-write progress in `finalization-progress.json`. An inactivity (180 s, as the controller allows the
collector)
timeout reports stopped progress and retains the spool. Total finalization time
has no deadline. Controller exit without completion fails immediately; normal
game exit can still finish recovery. Completion retains partial/drain failures.

The collector appends each attach and detach step to `progress` in the session
directory: milliseconds since attach began, the step (each hook family installed
or restored, each worker join and queue drain) and an optional count. The controller
waits for readiness while attach takes steps (180 s without one ends it) and for
the collector's done signal and unload while detach takes steps (180 s without one: one hook publication holds every game thread and on a loaded machine has taken 44 s),
at most 15 minutes for either, and names the last step when it gives up. After
each it prints the total and the slowest steps.

Each process session, in-process load or unexplained hour discontinuity starts a branch. Parentage is probable or unknown; one-second load timestamps can collide. Different campaign IDs produce separate replay files.

Game-score, operation-outcome and politics capture default to enabled. Record-only `--no-game-score`, `--disable-operation-outcomes` and `--disable-politics-capture` preserve named unavailable coverage, never recorded zero.

Finalization uses two streaming spool passes and compresses blocks before the shared decoder size bound, which applies to each table. Split hours become keyframes, anchored like any other, with original metrics and sample hours; an oversized frame or a split with a truncated table is refused by name. Allocation and I/O failures abort finalization and retain the spool. Repeated finalization compares complete decoded values, availability and metrics, then syncs the matching destination file and parent directory before acknowledging success without changing recording bytes or modification time. A sync failure retains the spool and reports failure.

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

### Load check

With your recorder attached, load through the game menu. Pause at the load hour, make a **new** text save, step one hour, detach and compare on the new `LoadSignal` branch. Loading reruns weather, so the loaded file cannot substitute. Retain each event family's first-interval coverage verdict; a full refresh cannot supply events from before observation began. Use a fully observed subsequent hour with a requested refresh and a new matched save for the complete check.

### Comparison boundaries

- Daily/sliced groups compare only their actual reads/owners. Carried groups report their read hour instead of comparing stale rows with current save values. [report.rs](src/oracle/report.rs) owns freshness rules; a sibling table's timing is not proof.
- Columns stored at the game's display precision ([display steps](../replay/src/display.rs)) compare with the save's exact value rounded by the same step (the remaining supply grace by the content's `SUPPLY_GRACE`), and the report lists each such column with its precision; a recording before those steps compares them exactly.
- Historical appended-field absence, current read refusal and native empty/zero stay distinct. Readable prefixes still compare, and inspection uses source-format byte sizes. Retired tables and fields are named as compatibility gaps and in recorded-table coverage; no comparison reads them, so they are never availability issues. See [replay compatibility](../replay/README.md#compatibility-and-recovery).
- Partial state-history membership skips only dependent joins, names/counts excluded states and makes coverage incomplete. Independent saved fields, identities and locations still compare. History effects are parsed, never executed; see [save parsing](../installed-content/src/save.rs) and [construction comparisons](src/oracle/construction.rs).
- Template comparisons preserve omitted keys versus emitted empty strings and the native first-NUL prefix. Air transport comparisons preserve complete wing pool/raid identity; unavailable historical operands remain limitations, while current reached saved-operand faults remain differences.
- Stored names compare the native first-NUL text, retaining counted limitations for captured trailing bytes. State, province, faction, unit and ace-callsign names preserve the stored-length gate and omitted versus emitted-empty keys; unconditional name writers require key presence in ordinary saves.
- Active ideas compare the recorded lexical dictionary's canonical writer spelling in roster order; absent or ambiguous dictionary joins fail the comparison. Advisor-token set membership uses its native case-insensitive hash/key identity, while saved advisor role strings compare exactly. The set's retained spelling depends on unsaved insertion history.
- Research definition names authenticate against the recorded dictionary entry for the exact serialized technology handle, preserving repeated database occurrences and canonical writer spelling. Missing, ambiguous or contradictory bindings are named differences and fail the comparison.
- Native omission rules also apply to advisor idea strings, deployment name flags and state resource blocks. An omitted advisor idea string contributes its empty token key. Unowned states retain the nonpositive owner class; their provinces keep absent ownership and independent explicit controllers. Factory sums use captured control where the state writer omits it, with that input dependency reported. Naval-base membership follows saved navy rosters, and each base occurrence compares its ordered country strings, including repeats and zero-level bases. Raw aliases use their own tag-string slots; unreadable saved operands remain differences.
- Construction self-checks use a line's remaining quantity independently of other target growth: native script effects can add levels without spending the waiting line's IC. Native count-reset observations separate cleared quantity from completed construction, including transfers that lower target levels. Historical prefixes check exact progress and integral completed-cost debits within a witnessed transfer's cleared count. A decrease without completed target levels or a native reset fails. Progress and spending still compare exactly.
- A division's supply footer compares only its disruption with the save's positive-only `disrupted_supply`; its demands, receipts and origin rows are unsaved runtime caches of the hourly supply solve, like those of railway guns and task forces, and need matched native hovers.
- A division's exile compares DivisionStatus's exile bit with the saved `exile`. Controller and owner area labels, area representatives and contact order have no save counterpart; the [selected-route checks](src/oracle/selected_routes.rs) prove only the recording's invariants (labels are smallest members naming themselves, one node on each label province, representatives are members, contacts are other areas' labels) and treat a refused label or node as a difference.
- Division base speed, railway-gun and task-force route queries, combat-vector order and retained snow/mud display boundaries have runtime-only operands. Match native observations at their supplying hour; saved-value agreement cannot certify those displays. Division arrival times are playback estimates from the recorded route and base speed; the save has no arrival time to compare.
- AirRegionCombat compares the save's regional air-combat holders after playback's daily prune ([air_region_derive](../replay/src/air_region_derive.rs)) at the frame's hour with the save's `AIR_LOG_TIMEOUT_HOURS`: owner, record count and order, each record's wing, date, planes, mission mask, selection, copied flag and attribution, and both damage vectors with each amount. A holder whose container country owns no state is not pruned by the daily pass, so its retained records are a named limitation; equipment pools and empty saved holders are named limitations too.
- Naval task-force participant membership and combat identities compare with the save. The first combat's role depends on its unsaved attacker country set and the unit's current logical owner; participant side cannot certify it. Empty versus nonempty recorded combat identities constrain role presence only as a recording consistency check.
- Domain joins and unsaved limits live in [oracle/](src/oracle/). Self-checks cannot certify runtime caches, events, source splits or displays. Changed unsaved inputs need matched native observations at the same identity and supplying read, plus smoke; later rates, net pool changes and reconstruction cannot replace them.

The [retained naval details oracle](src/oracle/combat_details.rs) compares every readable detail row and reports unexpected rows as differences. A saved result with no detail row makes coverage incomplete after those comparisons; zero differences cannot certify missing HQ inputs. HQ inputs themselves are omitted from the native save.
- Only collector-verified sessions compare, including mods; there is no override. Unhashed content reads retain named uncovered-read notes.

Saves hold no strike history, so NuclearStrikes has no save comparison. A recorded event loss (a same-process reattach) makes the partial first interval after it a named limitation in the naval result source check instead of a difference, restarts the rocket output continuity check at it, stops peace finalizer completeness from crossing it (a named limitation), and leaves an air loss day window that includes it uncertified with that reason; a partial interval without a recorded loss stays a difference. `inspect` prints each event loss. The [Event feed](src/oracle/event_feed.rs) and [rocket output](src/oracle/rocket_output.rs) self-checks validate recorded continuity/accounting; saves contain neither native actions, their causes nor historical applied IC/deliveries. After a world reset or load their coverage restarts at the next hourly boundary as a partial interval; integration needs matched native observations and the fresh-save/load and smoke workflows. Event feed partial or unavailable intervals are named coverage limitations, with an incomplete comparison at the selected save hour; a save cannot supply a complete-history expectation. Requested full refreshes preserve native event observation. Signed equipment gains and bombing corrections stay exact; air event dates can precede supplying reads.

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

[bench.ps1](../../tools/bench.ps1) converts its exclusive end first. The JSON schema/calculation lives in [metrics.rs](src/metrics.rs). `callback` is recorded hourly acquisition wall time excluding the native tick, not summed table timers, spool service or game throughput. `pool_bytes_max` is the interval's recorded pool maximum. Both are exact in replay blocks. Replay blocks record which tables were read but not their read times, so `inspect` reports those as `not_recorded` (older recordings keep theirs); the finalization summary reads exact nanoseconds from the spool.

Every hour must occur exactly once. Gaps, undated loss, partial/recovered files, damage, unsupported layouts and invalid bounds fail. Unavailable/truncated tables and dropped frames also fail; counts are affected decoded hours including carried tables, not native events. Historical lifetimes follow the code catalog: a table the recording's format does not record, or one the current format retired, is no capture failure.

### Offline size attribution

[sizes.rs](src/sizes.rs) writes a current-format recording's frames again with the current block writer, in memory, and attributes every compressed byte to its block section: frame header, diagnostics, and each table's keyframe and later entries, beside the recording's stored size and its plain frame bytes. It prints bytes per recorded hour and ranks tables by compressed bytes, so encoding and layout changes can be compared without a game run; `--verify` also decodes both and requires every hour to match. `--columns` splits each table's stored bytes between its section columns (control, keys, masks and each field by its row member's name), attributing a section's compressed bytes in proportion to each column compressed alone, beside each column's keyframe bytes, how many delta rows per hour were written whole, changed some fields or were deleted, and how often each field changed. `--levels <levels>` compresses every rewritten section again at each zstd level and reports all sections' and each table's bytes per hour and the single-threaded time per level. Older formats are refused because frames are written again only in the format they were recorded in.

```powershell
& $recorder sizes $replayFile [--json] [--verify] [--columns] [--levels <level>[,<level>...]]
```

### Derivation check

[derived.rs](src/derived.rs) resolves the recording's verified content stack and compares every value playback derives with what the recording holds, row by row and hour by hour: held day/night clocks whose hour proves the update ran, [province temperatures](src/derived/temperature.rs), the [selected army's attack operands](src/derived/attack.rs): wherever a DivisionStatus row holds both the stat-block operands and the native operands the collector's `attack_witness` diagnostic read, each terrain's attack derived from the stat block's entry and the content terrain's adjuster, and each scalar and the acclimatization factor, must equal the native one; and [power-balance description trees](src/derived/power_balance.rs), which it compares with the native witness trees of recordings made before the derivation and of a `breakdown_witness` collector build's sampled slice. A recording that stores the temperature pass's inputs must reproduce each hour's read checksum, and every anchor whose own pass the hour before proves must equal its derivation word for word; one that recorded every hour's temperature is checked where the pass needs neither seed nor drift: changes only in the rotating window from a single cursor, only at active provinces with an active neighbour, none while suspended, and each a smoothed stage within the content's maximum change. It counts equal values, mismatches (listing the first with their province and hour), held rows by suspension state and derivations the content cannot supply. A recording from a `naval_tree_oracle` collector also has each [naval source tree](src/derived/naval_trees.rs) occurrence's fresh native read: the identity playback joins from NavalCombatDetails and NavalCombatShips must be the one the capture filed, and the recorded (carried or read) trees must equal the fresh read. `--census` counts each table's changed rows and fields per hour; `--sizes` writes the weather table again with the current writer, as recorded and with each derivation applied, and attributes its compressed bytes as `sizes` does. `--weather-factor <steps>` (comma-separated fixed5 raw steps, or `default` for 25 and 5) writes DivisionStatus again as recorded and with `weather_factor` truncated toward zero to each step, a factor below one step staying exact, and prints each variant's B/h, changed rows and factor changes per hour, and how often the attack list's region weather entry at each division's own province would be shown, hidden or printed differently ([weather_factor.rs](src/derived/weather_factor.rs)).

```powershell
& $recorder derived $replayFile [--game-dir <directory>] [--user-dir <directory>] [--census] [--sizes] [--weather-factor <steps>]
```

## Recovery and checking changes

Wait for detach/finalization before reading completion. `status.json`/`completed.json` retain failure/hour, partial and drain facts; draining cannot certify capture success. The recorder polls the fixed terminal witness while the collector is loaded, publishes `capture_failed`, and requests detach automatically. Refresh refuses immediately after terminal failure. Authenticated raw-allocation sidecar details supply the attempted family and byte request in status. Finalization still waits for proved collector absence or game exit before recovering the committed prefix. Partial replays open verified prefixes with visible failure notices.

After confirmed collector absence or game exit, set `$sessionDirectory` to a retained session and recover with:

```powershell
& $recorder finalize $sessionDirectory --out $capture
```

A session the user discarded in Battle Plan Recorder keeps only its `completed.json` (with `"discarded": true`); its spool, copied collector and evidence are removed after collector absence or game exit is proved, and `finalize` refuses it. Discard never touches a recording the session would have continued.

A Battle Plan Recorder session publishes only the loads it admitted. Before the collector starts it writes `bin/recorder.json`, which marks the session, and `admitted.json`, which lists the load it attaches for when there is one. It rewrites `admitted.json` atomically as each load is decided: an admitted load with the campaign, the collector's load signal and the folder the session publishes into, and a refused load with its reason (`off`, `elsewhere`, or `settings` when the recorder settings couldn't be read or updated), kept only for reporting. When the session saves, after the drain and before finalizing, it reads the spool's clock witnesses once and decides every load in it that was never asked about, in load order across campaigns by clock witness time (every campaign's witnesses share the session's clock). Every `finalize` of such a session (after a crash, repeated, or after a collector failure) publishes the same thing: for each campaign, the recovered spool's branches whose load is admitted. A branch's load is the first load signal its clock witnesses carry; the branch a validated failure witness names takes the witness's load signal when it failed before its first clock witness. Other branches (their starts, gaps, frames, clock witnesses and the controls after them) are left out, and their frames are passed over when frames are read. A failure witness is applied only when the spool's last branch, and the load that failed, are admitted; otherwise the published hours end cleanly before it. Each load left out is returned and listed under `omitted` in `completed.json`, with its campaign, load signal (null when unidentified), hours, and reason (`off`, `elsewhere`, `settings` or `not_admitted`); finalization prints them too. A session that spooled hours is never recorded as `nothing_recorded`, even when none of them are published. A marked session whose `admitted.json` is missing or unreadable refuses finalization, so nothing is published. The spool always stays in the session folder. Sessions without the marker, including every command-line session, publish every load as before.

Recovery validates [terminal evidence](../replay/src/witness.rs), identity/context and committed spool bounds before keeping CRC-valid prefixes. Only attributable attempted hours become gaps; missing context invents no hour/branch. Invalid evidence and unsupported spools remain on disk with a refusal. Repeated finalization must preserve an existing good replay. An existing replay that needed read-time recovery is refused before finalization compares or appends; start a new file instead.

Allocation-failure checks require both binaries' `alloc-fault-injection` feature and coordinated `--alloc-fault <phase>:<positive-ordinal>[:<committed-hours>]`; production builds refuse it. Confirm game survival and partial-replay visibility, then run relevant fresh-save/load and smoke checks.

Update affected inspection, metrics, oracle and playback consumers together. New inputs need a save comparison or named limitation. Follow scoped [AGENTS.md checks](../../AGENTS.md), then relevant fresh-save/load, smoke and native observations above. Documentation-only changes need link/diff checks, not live captures.

The optional installed-save parser regression is ignored by default. To run it explicitly, set `BATTLE_PLAN_LAND_SAVE` to your text save and `BATTLE_PLAN_LAND_EXPECTATIONS` to a JSON file with `division_id` and `division_type` (unsigned 32-bit words), `manpower` (ordered `[tag, signed_integer]` pairs), `history_count` (unsigned count), and `first_date`/`last_date` (date strings accepted by the save parser). It checks the exact selected division values, history count and date bounds after parsing all divisions/history. Missing inputs or invalid expectations fail; an ignored test is not live validation.

```powershell
$env:BATTLE_PLAN_LAND_SAVE = $saveFile
$env:BATTLE_PLAN_LAND_EXPECTATIONS = Join-Path $capture 'land-expectations.json'
cargo test -p recorder oracle::land::tests::installed_save_divisions_and_history_match_expectations -- --ignored --exact
```
