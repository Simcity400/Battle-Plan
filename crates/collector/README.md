# Collector

`collector` is the DLL the recorder injects into `hoi4.exe`. It captures scheduled native data at the hourly sampling boundary, then hands owned payloads to off-thread decoding, diffing and replay output.

Native layouts, offsets and guarded readers belong in [`engine`](../engine/README.md). Replay schemas, cadence and encoding belong in [`replay`](../replay/README.md).

## Start here

Read [AGENTS.md](../../AGENTS.md), then [cadence.rs](../replay/src/cadence.rs) and [sampling.rs](src/sampling.rs). Follow the implementation for current dispatch points, roles and instrumentation; do not maintain another table catalogue or numbered phase map here.

| Area | Start here |
|---|---|
| Hourly boundary and capture sequence | [hook.rs](src/hook.rs), [sampling.rs](src/sampling.rs) |
| Executor, jobs and priorities | [acquisition.rs](src/acquisition.rs), [acquisition_jobs.rs](src/acquisition_jobs.rs) |
| Table families, registration and buffers | [extended/](src/extended/), [extended/mod.rs](src/extended/mod.rs), [table_capture.rs](src/table_capture.rs) |
| Map tables and shared division inputs | [map_capture.rs](src/map_capture.rs), [division_census.rs](src/division_census.rs), [division_detail.rs](src/division_detail.rs) |
| Raw-copy storage and replay | [raw.rs](src/raw.rs), [raw_memory.rs](src/raw_memory.rs), [raw_plan.rs](src/raw_plan.rs) |
| Off-thread decode, conversion and output | [decode.rs](src/decode.rs), [projection.rs](src/projection.rs), [spooling.rs](src/spooling.rs), [map_spool.rs](src/map_spool.rs) |
| Runtime and read accounting | [runtime.rs](src/runtime.rs), [read_accounting.rs](src/read_accounting.rs) |

## How an hour is captured

[hook.rs](src/hook.rs) detours `CGameState::HourlyUpdate` at its entry. The game mutates state immediately after that entry: **there is no free, stable interval after the trampoline**. `sampling.rs` resets the frame, takes the country snapshot, dispatches and joins acquisition jobs, then returns into the game's hourly update. Required native reads must finish before that return, including reads performed by workers.

There are two capture paths. **Raw-copy jobs** stage spans in a `RawLog`; the decode thread replays the typed walk against owned memory. The pack's `raw_mode` tells `capture_tables` not to repeat that walk against live memory. **Typed-walk jobs** fill pack buffers directly, borrowing them through `take_*_request` and returning them through `restore_*_request`. Packs own their rows and arenas; conversion happens off the game thread.

Owned copies must be sufficient for deferred work: decoding must never follow live game pointers. Guard every native read, and preserve source order, identity checks, meaningful pointer occurrences, bounds and the first independent failure. Address equality alone does not identify a table or read occurrence; shared acquisition and raw-log lookups must retain each consumer's scope, including empty and overflow paths.

A failed or short read is not an empty table, a zero value or a complete row. Never call engine functions to obtain values, wait for memory to change or retry to hide a failure.

## Scheduling, buffers and failures

Keep typed walks in pool jobs. Document necessary game-thread exceptions beside the code; for example, the state-flags walk depends on the States rows. Dispatch according to dependencies, not merely available workers:

| Dependency | Dispatch rule |
|---|---|
| Live memory only | Eligible for the first dispatch burst; use the appropriate ordinary job. |
| Shared division inputs | Wait until the input packet is available to its consumers. |
| Raw-copy verdicts | Join the raw jobs first, then dispatch `GatedPacks`. |
| Map-sample visitors, such as wing sources and order roots | Dispatch `LatePacks` after the map sample. |

**The Gated ordering prevents a specific ownership bug.** Multi-pack jobs swap the live packs with spare sets. Dispatching before a required raw verdict is installed makes the flag land on the spare set, not the pack already loaned to the worker; that pack then walks without the verdict.

`Role::priority()` orders queued jobs by their join segment, lower first and FIFO among equals. Short, early-joined jobs must not wait behind large, late-joined jobs. A join waits only for its own job: it must not execute unrelated queued work, and jobs must not submit other jobs.

Reuse spare sets and return loaned buffers through the matching restore path. `Captured::vacant()` and `Part::vacant()` allocate nothing. `with_capacities(&Default::default())` allocates real buffers and belongs in `spare()`, not a per-hour placeholder. Keep allocation and capacity growth off the game thread.

Keep these outcomes distinct:

| Outcome | Required behaviour |
|---|---|
| Submission rejected, such as a full pool | Run inline exactly once at that job's join. |
| Accepted job panics or is stopped | Terminal for that hour: publish `Unavailable` with the failure reason, do not retry live reads, and do not convert partial rows. |
| Raw staging overflows | `raw_ok` is false; use the existing typed fallback and grow staging capacity off-thread. This is not an accepted-job panic or stop. |
| Bounded walk truncates | Publish `Truncated { len, unread_rows }`, preserve growth demand and grow buffers off-thread. |

The legacy pack-migration recipe's wording that a “failed job” captures inline conflicts with the explicit accepted-job failure contract above. When working on `*_ticks.take()` fallbacks, check submission rejection, staging overflow and terminal failure separately; do not treat that recipe as permission to retry every failure.

## Cadence and freshness

[cadence.rs](../replay/src/cadence.rs) is the authority. Map data is hourly, with established four-hourly exceptions for slow-changing inputs such as relations and buildings. Panel data is daily across 24 balanced slots; keep whole shared-source families together. Do not lower cadence to make a benchmark faster.

Static means the **whole table** cannot change after a save loads. Static tables are captured on full hours: branch start, keyframe, load or gap. Character portraits remain hourly because new generals can appear even when an existing character's portrait tokens do not change.

Off-slot hours record `SKIPPED_REASON`; replay carries the previous data forward. Truncated staggered tables and failed static tables set `Shared.force_next`, forcing a full capture next hour with buffers grown off-thread. The lexer's maximum token ID is checked hourly; a change triggers another token-dictionary capture.

Record `DivisionEtaInputs` and let replay derive ETA. `DivisionEta` is retired. **Never source ETA operands from four-hourly or daily tables.**

## Reducing capture cost without changing data

**Reduce guarded calls, not just bytes.** `engine::Memory::copy` is an in-process, SEH-guarded copy. `copy_records` and `copy_scattered_prefetched<N>` gather multiple spans inside one guarded frame, avoiding repeated guard overhead. Divide per-table calls by row count to find repeated per-object work. Use a `const RecordLayout` for fixed-shape fields (`DIVISION_SEED_FIELDS` and `UNIT_ORDER_LAYOUT` are examples), scatter reads for independent fields, and `read_slice` for contiguous arrays. Batch predictable pointer enumeration; retain individual reads for unpredictable pointer chases. Preserve fault scope and ordering when batching.

**Reuse inputs at their proved lifetime.** Existing patterns include supply outer-map reuse (`InnerMemo`), ship doctrine and modifier metadata (`OrganisationMemo`, `DefinitionMemo`), province authentication (`ProvinceMemo`) and geometry (`StaticGeometry`). Key reuse by what invalidates it, such as module base, game state, play session or census header—not just a reused address. Reloadable defines, weather structures and mutable definitions are not automatically session-static. Shared inputs must preserve each consumer's identity and failure behaviour.

**Split only measured bottlenecks.** Workers contend for cores and memory; more parallelism is not automatically faster. For country-grouped walks, interleave outer units by every n-th slot rather than taking contiguous ranges, because large countries occupy neighbouring slots. Each share repeats header and first-fault work. Keep separate rows and arenas per `Part`, put once-per-family work in part 0, and restore single-walk row order at conversion. Merge outcomes so any `Unavailable` wins; otherwise sum lengths and unread counts. Preserve duplicate occurrences and capacity demand.

## Changing a capture path

**Add a table.** Add its replay row and ID in [rows.rs](../replay/src/rows.rs) or [extended_rows.rs](../replay/src/extended_rows.rs), its cadence entry, and an `extended/<pack>.rs` pack with `N`, `TABLES`, `READ_GROUPS`, `Capacities` and `Captured`'s `capture`, `outcomes` and `tables`. Register it in `packs!` in [extended/mod.rs](src/extended/mod.rs), then wire acquisition and conversion through the appropriate job. Change the [format](../replay/src/format.rs) version only when the wire changes; keep older versions readable.

**Move a pack into a job.** Wire its `Captured` through the matching multi-pack struct, `spare()`, `capture`, `swap_*` and take/restore paths; remove the former inline walk without breaking the failure distinctions above. A dedicated job also needs `Job`, `Report`, `Role`, `read_anchor`, `priority`, `ROLE_COUNT`, `ROLES`, `Job::role`, and dependency-correct dispatch/join points in `sampling.rs`.

**Change native readers in `engine`.** Read the [reverse-engineering guide](../../tools/re/README.md) first. Keep one source file per native struct, with evidence beside constants: native writer, serializer or function, width, signedness and confirmation. Use `C:\Tools\hoi4_mcp\hoi4re.cmd` and save established names, fields and notes. If decompilation drops arguments or arithmetic, inspect indexed instructions; distinguish virtual addresses from image-relative RVAs. Old readers, including `archive/pre-rewrite-2026-09-09`, are research leads, not proof. Stored fields, calculated getters and renderer caches are different sources; similar save values are not interchangeable evidence.

## Testing and measurement

Use existing synthetic-memory fixtures to check aliases, invalid pointers, short reads, exact first failures, capacity and growth demand, and owned-versus-direct equivalence. Share splits repeat first-fault reads and cache keys add reads; update counted expectations deliberately. Run `cargo test -p collector` and `cargo test -p engine` while iterating, then the full fmt, Clippy, build and workspace-test checks in [AGENTS.md](../../AGENTS.md). New capture domains need collector/replay integration and an oracle comparison or an explicit limitation. See the [recorder guide](../recorder/README.md#inspect-and-compare) for hourly-sample versus later-save limits.

Use [tools/bench.ps1](../../tools/bench.ps1), frozen builds and a same-run no-recorder baseline. Do not compile during timed windows or trust throughput percentages from a locked desktop or collapsed baseline. Acquire `%TEMP%\claude\hoi4_game.lock\owner` before controlling the game; release it by renaming the lock directory to `hoi4_game.lock.released-<stamp>-<why>`. Leave the game running and paused.

The blocking acquisition time is the primary diagnostic. In the documented instrumentation, `status.json` exposes `acquisition_work.mean_ns` (`collect_wait`), `block_phases.segments_mean_ns` and `join_wait_by_role_mean_ns`; `SEGMENTS_SCOPE` names the segments. Inspect p50, p95 and maximum as well as means. Earlier benchmark evidence found extra smoothing frames during long ticks, with a threshold around a 16 ms block; remeasure that threshold rather than treating it as universal.

Skipped hours contribute zero to per-table means. For a table captured once every 24 hours, its per-walk cost is 24 times the all-hour mean; extra full captures change that frequency. Compare job durations per completed call, not just averaged join waits. The documented `workers.roles[].ticks` uses 10 MHz ticks; use the checked-out branch's actual units and counter definitions.

Attribute native reads with `read_accounting::table(TableId)` and keep `table_ns` timings truthful. Decoding owned memory is not another native read. Performance counters belong in `status.json`, not replay schemas or the spool; do not add runtime verification windows or second reads to those outputs.

Record measured block and throughput changes in the commit or PR, separately from fixture read-count savings. Say when fewer calls do not improve the block. Run the fidelity checks, including hourly windows, the war matrix and the supported text-save oracle: faster incomplete capture is not an improvement. Keep benchmark histories, current hotspot rankings and migration progress out of this guide.
