# Collector

`collector` is the Windows x64 DLL the [recorder](../recorder/README.md) injects into the supported HOI4 process. It captures native state and hands owned data to off-thread decoding and replay output.

Read the [shared rules](../../AGENTS.md). Native layouts, guarded reads and evidence belong in [engine](../engine/README.md), schemas and cadence in [replay](../replay/README.md), and content resolution in [installed-content](../installed-content/README.md). Per-capture behaviour and addresses stay beside code.

## Source map

| Area | Entry points |
|---|---|
| Hourly boundary and DLL lifetime | [sampling.rs](src/sampling.rs), [runtime.rs](src/runtime.rs) |
| Patching and bridges | [hook.rs](src/hook.rs), [hook/tap.rs](src/hook/tap.rs) |
| Jobs and owned copies | [acquisition.rs](src/acquisition.rs), [acquisition_jobs.rs](src/acquisition_jobs.rs), [raw.rs](src/raw.rs), [raw_plan.rs](src/raw_plan.rs) |
| Packs, loans and sizing | [extended/mod.rs](src/extended/mod.rs), [extended/packs.rs](src/extended/packs.rs), [country_scale.rs](src/country_scale.rs), [table_capture.rs](src/table_capture.rs) |
| Decode and output | [decode.rs](src/decode.rs), [spooling.rs](src/spooling.rs), [status_writer.rs](src/status_writer.rs) |
| Politics and characters | [extended/governance.rs](src/extended/governance.rs), [extended/characters.rs](src/extended/characters.rs), [politics_changes.rs](src/politics_changes.rs) |
| Economy | [extended/economy.rs](src/extended/economy.rs), [extended/production.rs](src/extended/production.rs), [extended/construction.rs](src/extended/construction.rs) |
| Military | [division_census.rs](src/division_census.rs), [extended/naval_details.rs](src/extended/naval_details.rs), [extended/air.rs](src/extended/air.rs) |
| Intel and visibility | [extended/intel.rs](src/extended/intel.rs), [extended/fog.rs](src/extended/fog.rs) |
| Map and world | [map_capture.rs](src/map_capture.rs), [extended/weather.rs](src/extended/weather.rs), [extended/session.rs](src/extended/session.rs) |

## How it works

Hourly callback → acquisition jobs → owned buffers → decode/conversion → diff/encode → recoverable spool. `DllMain` defers initialization to a worker outside the loader lock.

The hook enters `CGameState::HourlyUpdate` before the clock advances; the sample is the settled end of the labelled hour. Every live read, including worker reads, finishes before the trampoline runs. There is no stable interval after it. The boundary holds the simulation thread, not every native writer: retain consistency guards for concurrent caches and dictionaries, refusing overlapping changes without waiting or retrying.

Raw-copy jobs stage spans in a `RawLog` for typed decoding over owned memory; typed jobs fill preallocated rows and arenas. Deferred work never follows live game pointers. Use matching take/restore paths and allocation-free parked spares. Install raw verdicts before loaning dependent packs; join shared-source producers before dispatching consumers.

Between-tick observers use a bounded store/log → hourly drain into owned frames → off-thread aggregation and encoding. Callbacks preserve original execution, never refresh caches or write game data, and use try-only storage leases. Preserve cursors, epochs, identities and branch continuity; loss breaks coverage until explicit recovery. Swap storage only at an empty, writer-free boundary, free retired storage off-thread, and retain sampled events until their output outcome is known. Between-tick facts after the last captured hour are outside recorded coverage; detach invents no sample. See [output_log.rs](src/output_log.rs) and [event_ledger.rs](src/event_ledger.rs).

## Acquisition rules

[world_lifetime.rs](src/world_lifetime.rs) observes native reset, deletion and construction before retirement or publication. Invalidation closes admission for occurrence-aware observers; the next readable hourly boundary starts a partial interval, forces fresh acquisition and splits replay branch continuity even if stored clock fields agree. Event feed, production, air loss, bombing, naval result and operation completion post reads require the same occurrence. The signal does not lock native objects or certify successful load completion; other families retain their native identity and consistency guards. The signal remains published until the other observer families and their older native invocations are quiet.

[Event feed](src/event_feed_observer.rs) copies pointer-free facts into each hourly frame and keeps them until the spool returns an exact receipt for that batch; an uncertain write keeps its facts. At detach, facts after the last recorded hour and known-failed batches are released and counted in session status (`event_feed_detach`); only uncertain receipts, tickets and native flights keep the collector loaded. [Rocket output](src/extended/rocket_output.rs) shares the [production hook owner and drain](src/line_output.rs), with independent known amounts and refusals.

Collect the least data needed by a visible feature. Prefer stored values, including stale caches and retained name bytes. Capture operands for on-demand results, viewpoint/camera dependence, viewer-language text or results much larger than their inputs. Keep cached text's language provenance.

Keep the ordinary-hour game-thread block within the **2 ms target** documented in [acquisition.rs](src/acquisition.rs), including buffer backpressure and joins. Load and invalidating full-refresh hours have the approved exemption; measure them separately. Collector coordination allocates and frees nothing on the game thread. Conversion, encoding, diffing, I/O and growth stay off it. Allocating acquisition jobs are worker-only through fallback and unwind; growth continues from acquired headers without restarting reads.

Acquire each scheduled table once from the first hour. Freeze activity before dispatch; inactive tables do not start or retire work. Charge shared sources once. Joins handle only their own job; workers never submit or wait on other jobs. Preserve FIFO retirement and return loans before recycling their producer.

[cadence.rs](../replay/src/cadence.rs) owns scheduling; [slicing.rs](src/slicing.rs) owns sliced-family selection. Preserve full-refresh and invalidation rules. Encoding keyframes use held rows without extra reads. Successful acquisition establishes freshness even when values encode unchanged; carried data keeps its prior read hour. Failed or unpublished reads cannot certify reuse or erase independent sibling outcomes. [fixed_layout.rs](src/fixed_layout.rs) authenticates native modifier and lexer layouts; apply its refusals after merging so carried rows cannot bypass the checks.

Size rows, arenas, logs and watches from live registries and content demand before installation; raise capacity off-thread as demand grows. `country_scale.rs` coordinates hints. Short walks report exact demand; failed sizing observations are named and counted, adding no fabricated zero hint. A failed or racing hint loses no data, so it never becomes the session warning; any resulting shortage surfaces as its capture's own named refusal. Use checked growth for large storage, with no chosen population, text or tree-depth cap. Scale requirements live in [AGENTS.md](../../AGENTS.md).

Observer overflow refuses the interval and requests off-thread growth even when population is unchanged. Drains/recycled frames accept prepared larger storage; callbacks never grow or retire it.

Politics staging starts from native daily record and live roster demand with room for the bounded frame backlog. Off-thread sizing polls on the existing input-demand cadence even when country count is unchanged. Rejected records count toward each lane's full burst until its sealed handoff is consumed; failed sizing hints appear in `country_scale_demand.politics_records` and preserve capacity. Politics staging replacement still invalidates observation and retains old logs until detach.

Observer demand comes from census and observed shortage, never a pooled vector's allocation headroom. Frames follow producer sizing without returning their capacity as demand. Offers with multiple arenas preserve pending growth in every dimension.

[Equipment destruction](src/equipment_destruction.rs) stages larger storage in a separate worker slot and installs it only at an empty, writer-free boundary or drain. Retired vectors stay parked for off-thread release. A clean hourly boundary discards a refused interval and reopens observation; its drain remains unavailable, and the following fully observed interval can be complete. Queued provenance and return-credit proofs retain their independent lifetime checks. Escaped native calls prevent recovery and detach.

Guard every native read and preserve order, duplicate occurrences, identity, bounds, consumer scope and first independent faults. Address equality alone is not identity. Never call a getter for a value, retry failed reads or certify partial data as complete. Preserve these distinctions:

- Rejected requests run inline once at their join only when eligible; worker-only rejection publishes unavailable without callback reads. An accepted job's panic/stop is terminal for that hour, without retry or partial conversion.
- Raw staging exhaustion retains growth demand; typed fallback is allowed only before a prepared source observation. Truncation retains rows, unread counts and demand with an explicit outcome.
- Unscheduled data carries its prior sample. Failed operands remain absent rather than empty or zero, and known siblings survive.

Refusals, unsupported forms, shortages and losses are named and counted, never silent. A coordination panic stops acquisition for the attachment and records losses. Checked allocation failure stops admission, abandons incomplete work and lets the original tick continue. [allocation.rs](src/allocation.rs) writes the fixed [terminal record](../replay/src/witness.rs), authoritative over status. Encoder baselines advance only after a complete spool record.

[Retained naval result details](src/extended/naval_result_details.rs) omit only the affected result's detail row when its borrowed HQ inputs fail. The independent result census remains available; absent details are unknown. The capture restores its ship, plane, hit and influence arenas, counts refusals in collector status and keeps allocation failure terminal.

Reuse needs proved writer coverage and receiver lifetime; loads, gaps, lost watches and failed publication revoke certificates. Prepared geometry still acquires fresh values and resumes changed controls from the acquired prefix. Mutation-certified reuse needs fresh-read verification and coordinated burn-in; mismatches disable it for the attachment. Preserve default verification in [Cargo.toml](Cargo.toml). Quiet traces prove only exercised paths; do not add ordinary production comparison windows or second captures.

## Hook and patch safety

- Authenticate the executable, instruction span and sentinel bytes. Keep sentinels inside the owned function, exclude incoming branches and native return PCs from overwritten instructions, and preserve continuation and handler scope.
- Publish/restore a family under one thread suspension, validating every site before any write. Prepare storage, protections and handles first; query a prepared system-information buffer after suspension and hold newcomers until the set is stable. A short census buffer resumes every hold before growth and retries within the same bounded rounds; insufficient prepared thread storage refuses before writing. No allocation, formatting, locks or destruction while suspended. Flush the instruction cache and verify bytes. Failed publication rolls back the family and retains executable storage until safe. Never overwrite foreign bytes at sites the family did not own.
- Use existing relocation/trampolines. Preserve arguments, return values, live registers, flags and SIMD state, Win64 alignment and shadow space. Interior bridges clear DF for observers and restore native state before relocated instructions execute.
- Register correct Windows unwind records before publication; preserve canonical prologues/epilogues and native return locations, and unregister before freeing their page. [Tap](src/hook/tap.rs) uses `extern "system-unwind"`; unsupported forms refuse installation. SEH-guarded reads and caught Rust panics do not prove foreign exception safety through originals called from a non-unwinding ABI.
- In-flight counts cover native originals outside DLL code; suspended-IP checks cover uncounted stub entry/exit. Buried return PCs can reference patched code. Rust cleanup cannot prove a stub finished foreign unwind: retain its flight without a safe completion witness. Generic Tap flights finish after the copied span, preserving flags through a separately registered completion frame. Every observer joins both runtime quiescence checks, including all relay, trampoline and bridge ranges.
- Escaped equipment, LAND and operation callbacks retain their active claims across load/detach, including disabled capture; only proved normal completion retires them. An unresolved OS resume retains suspension handles and publication storage until resume or thread exit, regardless of detach deadlines.

## Detach and failure handling

[Runtime](src/runtime.rs) owns the order:

1. Cancel buffer waits, restore the hourly entry, close acquisition admission and wait for admitted leases.
2. Restore all owned observer families; one failure must not skip others. Wait for callbacks, originals and executable ranges, excluding only registered collector workers from this first IP check.
3. Join acquisition workers, drain decode FIFO, drain/join the spool, and finish status and required durability work.
4. Repeat full quiescence without worker exclusions, release stores/trampolines/unwind registrations, clear shared state, then self-unload. The recorder confirms DLL absence before finalization.

Observer restoration and quiescence share a bounded wait for every family; a timeout names the family and retains the collector. Unproved restoration, publication, unregistering or quiescence retains reachable code/storage and reports refusal. Never reset stranded flights to pass detach. Process-detach callbacks do not flush output. Preserve recoverable spool prefixes on process, allocation and write failures; recorder/replay own partial-file recovery.

## Checking changes

New packs need replay rows/cadence, registration in `extended/mod.rs`, live-sized owned buffers, dependency-safe jobs and off-thread conversion. Follow adjacent take/restore paths and update affected writers/consumers together under the [replay guide](../replay/README.md).

Use existing synthetic-memory, pipeline, scale and fault tests for aliases, first faults, truncation/growth, sibling isolation and owned-versus-direct equivalence. Hook changes need publication, register/unwind and lifetime checks. Follow the [shared build checks](../../AGENTS.md); documentation-only changes need link and diff checks.

Measure the whole game-thread wall block and tails, including buffer waits; overlapping worker timings cannot be summed. Keep native reads, off-thread work, retained memory, encoded size and throughput separate. Charge reads once through [read_accounting.rs](src/read_accounting.rs); diagnostics stay in `status.json`. Batch without changing fault scope/order and split only measured bottlenecks.

Integration needs the [fresh-save oracle](../recorder/README.md#inspect-and-compare) and [smoke/performance workflow](../recorder/README.md#live-checks-and-performance): matched saves, frozen builds, same-run baseline, ordinary/full hours, constrained pools and downstream buffer pressure. Fixture timing is not live slowdown; a passing build is not capture acceptance.
