# Prepared slice evidence

The [coverage inventory](../../docs/rebuild-acceptance.md) distinguishes the
implemented slice checks from the remaining product, map and frontend comparisons
and lists central synthetic scale checks and measurement targets.

`dashboard-acceptance` runs the actual preparation pipeline and host-direct query
dispatch without a browser. It compares the requested branch, tags and hours with
the existing production readers. Evidence and process caches live in a supplied
directory outside Git. Recordings and game/mod files remain read-only; an append
exercise copies recordings into that evidence directory.

Every harness mode runs inside a thread made by `prepared::runner::worker_thread`,
the same 2 MiB constructor used by production preparation workers and service
loaders. Options, content setup, sources, preparation and queries are constructed
inside that thread. The harness main thread receives only the final outcome.
Prepared branch state keeps its observers on the heap, and stack layout/regression
tests check both restoration and query reads on the production stack budget.

Build the harness with the integrated dashboard, replay, store and domain changes:

```powershell
cargo build --locked --release -p dashboard --bin dashboard-acceptance
```

Launch from the repository root. Each measurement run should use its own evidence
directory. Reusing a directory deliberately measures prepared reuse.

```powershell
./tools/accept-rebuild-slice.ps1 `
  -Recording 'C:/captures/reference.battleplan' `
  -Evidence 'C:/captures/acceptance/reference-run' `
  -GameDir $env:BATTLE_PLAN_GAME_DIR `
  -UserDir $env:BATTLE_PLAN_USER_DIR `
  -Requests 'C:/captures/acceptance/requests.json' `
  -Repetitions 20
```

`GameDir` and `UserDir` also default to `BATTLE_PLAN_GAME_DIR` and `BATTLE_PLAN_USER_DIR`. Both
are required for focus titles, PNGs and descriptions. Omitting both still
exercises recording-only data and records presentation as skipped. A skip keeps
the requested checks from passing. `AllowUnverified` follows the application's
existing explicit unverified-content option.

The launcher gives the child process private `LOCALAPPDATA`, `APPDATA`, `TEMP`
and `TMP` directories. Content verification uses `cache/content` there. It does
not change the invoking shell's environment. The binary independently checks its
cache roots and rejects evidence inside its current Git worktree.

`PrepareOnly` runs this same pipeline, prepared restoration and unchanged-reuse
checks without product oracles or query timings. It requires ready preparation
sections, marks `preparationOnly: true` and `parityPerformed: false` in its report,
and leaves the isolated prepared directory available as an explicit browser input
seed. It is setup evidence, not a numeric or presentation parity result. The
[browser harness](../../ui/e2e/README.md) copies that directory privately for each
role and records the seed provenance.

`--preparation-measure serial` or `--preparation-measure parallel` runs all
production observers with the binary's mimalloc allocator and writes
`preparation-measurement.json`. `--measure-seconds` sets the measurement time box,
which ends at a manifest commit. Use fresh isolated evidence and supply the real
game and user folders. This mode runs preparation only; it performs no product
oracle scans or queries. Serial timing separates selected reading/reconstruction
from frame observation; publication is included in total elapsed time. The external
measurement launcher supplies process memory, machine CPU and wall/silence guards.

Run `tools/measure-preparation.ps1` with the release `dashboard-acceptance.exe`,
the recording, a fresh evidence directory and `-Mode serial` or `-Mode parallel`.
Parallel mode combines one-frame decode-ahead with lockstep family lanes; lane
count is available parallelism minus two, capped by independent groups. Serial
mode keeps both selected reconstruction and observation sequential. Content and
writer calls retain coordinator order; all lanes join before advancing a frame.
The launcher defaults to a 120-second committed preparation time box, a 300-second
wall guard, a 120-second output-silence guard and a 16 GiB private-memory guard.
It writes per-phase process-memory/CPU evidence and preserves logs on a guard stop.

Measured runs also write `familyTimings`, sorted by each row's combined exclusive
observation and flush/publication wall time. Nested spans subtract their children,
so coordinator scopes sum without double counting. Rows contain observe/flush seconds,
call counts, mean seconds per call and percentage of total preparation wall time.
Rows cover setup, content/context and observer construction, checkpoint/resume
reads, branch/block/frame overhead, stream acquisition, progress output,
publication and local destruction. Naval observation and detail observation have
separate rows, as does focus frame writing. `residualSeconds` is elapsed preparation
wall time minus coordinator spans. Worker spans overlap coordinator work and each
other: subtract `workerObserveSeconds` from summed family durations before elapsed
accounting. `observationCriticalPathSeconds` records actual frame wall time;
`frame.overhead` includes dispatch and joining. `workerLanes` reports the actual
lane count. Family percentages may sum above 100 in parallel mode. Stream acquisition measures consumer
time outside observation; overlapping producer time is not added again. Ordinary
preparation allocates no family timing table and reads no per-call clocks.
Transient CPU-counter timeouts retain the last successful CPU sample; the process
report records their count and last error.

Campaign preparation measurements include growing prefixes at 8,760, 43,800 and
87,600 indexed game hours (1, 5 and 10 years). Pass
`-ThroughHours 8760,43800,87600` to the launcher, or repeat
`--through-hours` in the binary. Prefix mode ignores the short `MeasureSeconds`
time box and stops at the first block commit reaching the largest requested
prefix. Measurement mode commits each block; ordinary preparation keeps its
existing commit budgets. A block-0 or short burn-in run is a diagnostic only.

`blocks.jsonl` is flushed and synced after each commit. The final preparation
report includes those rows in `blocks`: ordinal and hour bounds, observed hours,
wall time and throughput, serial decode seconds, sorted exclusive timing rows and
residual, accepted payload bytes, physical store bytes by section, and a native
private-byte sample at the commit. Parallel decode seconds are unavailable.
The journal first appends a commit row, then a storage enrichment row for the same
ordinal after read-only disk accounting; the final array contains one merged row
per commit. Resume completes a pending last storage enrichment from the accepted
cache before admitting another block.
Evidence-writing time has its own aggregate timing row and is excluded from block
throughput. Input/journal/resume preflight is included in the first newly measured
block's `measurement.preflight` row, so its input binding cannot silently warm
setup caches outside measured wall time. Final post-commit cleanup appears in
aggregate timings.

Storage enrichment also writes `payload-<ordinal>.jsonl`, referenced by
`payloadChangesFile`. It records each new/replaced live section/range's length and
BLAKE3 digest. The initial commit covers all live ranges; later commits compare
immutable file IDs with a streamed prior-manifest mirror and hash only changed
payloads. Digests stream to disk, and retained prepared histories are not loaded
or rehashed. This evidence work is excluded from block throughput. Run
`tools/compare-preparation.ps1 -SerialEvidence <serial> -ParallelEvidence <parallel>`
to compare every committed block's payload changes without collecting histories
in memory. Manifest UUIDs and physical file names are excluded.

Each `prefix-<h>.json` records cumulative throughput, last-10-block throughput,
first-10 versus last-10 per-family seconds per 1,000 observed hours and growth
ratios, peak sampled private bytes, and store bytes per 1,000 hours. Windows use
up to 10 blocks when fewer exist; a zero initial family cost gives a null ratio.
Indexed spans determine the prefix stop, while observed frames determine rates.
Branches contribute their indexed spans in recording order.

`-Resume` reuses the evidence directory's existing private cache roots, verifies
the recording/content/mode stamps, and appends new blocks with a separate `runId`
and `resumed` flag. Existing block rows and prefix snapshots remain; machine-load
samples append to CSV and each resumed process gets separate logs. A nonempty
directory is refused without Resume. An evidence/cache cursor mismatch refuses
to invent missing timings. An unterminated final journal append is discarded.

`-ProcessorAffinity <decimal-mask>` pins the measured process and records the mask
in `process-measurement.json`. Machine-load samples stream to the referenced
`memory-samples.csv` and are streamed into the process report's `machineLoadSamples`
array after exit; a small metadata file supports resume without loading that
array. The launcher retains only phase aggregates and up to 31
block times in memory. With ThroughHours, wall and silence guards apply per block
using the larger of their configured floor and 10 times the rolling median block
time. The first block uses the configured floors. Memory guards stay in force.

## Requests

The optional JSON file selects concrete requests:

```json
{
  "branchId": "1",
  "tags": ["opaque country tag"],
  "hours": [0, 23, 24, 719, 720, 721],
  "viewports": [{"from": 0, "to": 721, "width": 800}],
  "production": [{
    "branchId": "1",
    "tags": ["opaque country tag"],
    "top": null,
    "measure": "day",
    "count": "units",
    "grouping": "type",
    "included": ["recorded content type key"],
    "refits": false
  }]
}
```

Tags, selectors and dates come from the recording and its content stack. The
example's placeholders must be replaced. Country lists, selector lists and
requested hours have no harness cap. Ordered duplicate tags remain in measure
requests; production follows its normal manual deduplication. Production cases
use the selected branch and may use an empty tag list plus `top` for global ranks,
or neither for the type catalogue.

Without an explicit branch, the first recorded branch is selected. Without tags,
the recorded client country is selected when present, otherwise the first initial
catalogue entry. Default hours are branch ends/midpoint and every indexed block's
first/last hour. Default viewports cover the branch and a single midpoint hour at
800 pixels. The resolved requests are written to `requests.json`.

Without explicit production cases, the harness exercises total/day/week/month,
units/IC, type/variant/domain grouping, refits off/on, all types, each recorded
type individually, an ordered duplicate selector and one missing opaque selector.
It also requests the widget's Top 10 selection. Every case checks both ordinary
and ranked pages. `production-requests.json` records the complete resolved matrix;
supplying an explicit matrix makes shorter targeted runs possible.

## Comparisons and measurements

- Focus hour availability, country identity, order/current/continuous/paused,
  exact progress/daily gain and every completion ordinal, including repeated tokens.
  Presentation and details compare the old production interpreter at each
  completion's confirming date (or the first observed order for undated entries)
  and the active card's requested date. All page rows are reached. Named table
  refusals, counts and null fields remain distinct.
- Equipment-produced keys/types/warnings, exact combined values, independent
  equipment values, included-total knownness, complete page counts and country
  order. Ranked manual selection excludes unknown values and retains known zero;
  Top selections retain the old nonzero rule. The old product's safe-number
  refusal is recorded as an oracle error, never an automatic approved improvement.
- Political/command power exact run bounds/raw values/native identity/segment starts and dated
  drops, every requested drop page and through-hour count, viewport extrema and
  continuity summaries, complete mark-cluster ordinals/counts and balance-only
  keyboard dates. Summary expectations sweep the old product intervals instead
  of calling the prepared summary producer. Keyboard expectations use the old
  widget's run/drop stops and shared chart navigation rules; added unknown-boundary
  stops are reported as differences.
- Prepared map checkpoint search, Campaign/Clock/DivisionStatus exact row and
  availability states, acquisition provenance, requested hour and gap reason.
  Checkpoint-seeded seek and next-hour step use the real replay decoder. Evidence
  includes checkpoint-header reads, checkpoint payload bytes, source-block reads,
  retained source-block frame count and applied-frame bounds.
- Complete preparation, unchanged reuse and reopened prepared restoration. The
  run records accepted source-block cursors, newly prepared block counts, prepared
  bytes, content-stamp cardinality and preparation/reopen duration.

For append evidence, add `-Prefix 'C:/captures/prefix.battleplan'`. The prefix must
have the same indexed source-block prefix as `Recording`, with at least one new
block if tail work is expected. The harness prepares a copy of the prefix, copies
the complete recording over that scratch path, and opens its new revision. It
compares index metadata and checks that `newBlocks` equals only the added block
count. The harness does not hash or compare old compressed payload bytes. Its
block count measures pipeline work, not all filesystem reads; replay/pipeline
synthetic tests cover rejection and the metadata-only growth policy separately.

Each query records first-call duration and repeated in-process p50/p95/p99 plus
complete serialized reply bytes. The timed path includes coverage checks,
content-stamp/manifest reads and JSON serialization. Parsing, evidence writing
and old oracle work are outside that query timer. These are in-process cache
measurements; the harness does not evict the OS cache. Map timers measure
checkpoint decoding and selected-table reconstruction without native rendering.

The launcher samples process private memory, working set and CPU time every
100 ms. These samples include old oracle scans and their caches. They establish
observed process use, not a prepared-only allocation bound or VRAM use.
`phases.jsonl` records timestamps separating preparation, prepared reopening and
each parity/timing family so samples can be attributed to those intervals.

`report.json` records all per-request timings and measurement limits.
`differences.jsonl` gives exact field/array-position differences, including a
separate presence flag for omitted versus null values. `replies.jsonl` retains
the requests, replies and oracle expectations. No difference or output inventory
is truncated. `map-reads.jsonl`, `process-samples.jsonl`, `process-report.json`,
`stdout.txt` and `stderr.txt` retain the other evidence.

Exit status is 0 only when requested comparisons have no differences, errors or
skips; 1 means some checks did not pass; 2 means setup or execution failed before
a complete report. Target numbers in the report are measurement targets, with no
automatic performance approval. Frontend frame/paint timing, native clocks,
GPU presentation and the viewer's private checkpoint consumer require their own
checks. Queue the viewer's `map_checkpoints::tests`, `hours::tests` and
`map::presentation::tests` alongside this harness.

## Progressive timeline measurements

`--progressive setup --prefix-blocks N` prepares only a prefix of a scratch copy.
Cancellation occurs at the first accepted publication whose cursor reaches N;
publication batches can accept more than N blocks. Setup rejects a completely
prepared recording. `prefix.json` records the actual accepted cursor.

`--progressive rough` and `--progressive zoom` open that copy and its isolated
prepared store in separate fresh processes. They never resume preparation. The
rough request covers the branch at 800 pixels; the zoom covers thirteen complete
unprepared blocks. Both use the production balances `series-summary` query for
political power, without event marks. Recording-derived default country selection
and optional `--requests` tags use the existing harness request rules.

Supply `--measurements` with a JSON-lines path inside `--evidence`. Coverage,
rough-query and exact-query rows record hours, blocks selected and wall milliseconds.
Block selection follows the tail's index/coverage overlap rule and includes the
preceding observation when the exact window needs it. It is an index-derived
selection count, not measured filesystem reads.
The rough row includes source-open-to-first-summary milliseconds, excluding
prefix preparation, scratch copying and evidence writes. Reply files retain the
actual summaries; the harness rejects rough replies without approximate spans
and zoom replies with approximate spans. Process isolation means cold application
state and no prepared tail; it makes no OS-cache eviction or frontend paint claim.

The external progressive launcher builds nothing and enforces a 600-second total
run limit, 120 seconds without child output and 16 GiB private bytes. It records
100 ms process samples and fills each measurement's peak private bytes from the
corresponding fresh query process. These are sampled process peaks, not allocation
bounds. The owner builds the release binary and runs the launcher against the
protected 50-year input; only its evidence-directory copy is prepared.

## Readiness

`dashboard-acceptance --readiness` measures how long after opening a recording every registered
widget's default view and the map's first frame can draw while background preparation runs. It
opens through the data service as the desktop does, with an empty isolated prepared store and the
content stack from the game and user folders; by default an earlier open with preparation off warms
the content hash index, and `--readiness-cold-content` skips it. `src/readiness/widgets.json`
lists each widget's default requests (`--readiness-table` replaces it for one run); a test fails
when a registered widget has no entry. All requests are in flight at once, as mounted widgets issue
them, and a pending prepared request without a first view is asked again when the coverage its last
reply waits for (its `gate`) moves. A shown unfinished reply refreshes on gate movement no sooner
than 2 seconds after its last reply, then 4 seconds, then 8 seconds for every later refresh.
Completion of every gate on the selected branch bypasses the interval. New navigation, ranges and
selections ask immediately. The probe mirrors the shared client loop and marks shown-reply
refreshes for the service's prepared and sampled queues. Both those queues and the browser
connection queue admit at most eight consecutive first asks while refreshes wait, then one refresh;
each class remains FIFO. Terminal preparation states trigger one final ask regardless of gate
movement or refresh interval. A final null reply is unavailable; a final partial reply retains its
extent and error, and stops polling without becoming exact.
Status is polled every 500 ms, matching the client. A reply is
ready when it is exact; answered beyond accepted coverage by the product's own sampling (flagged
approximate spans count); or answered from its gate's prepared prefix (`prefix`): it names requested
hours after the prefix `pending`, names values `preparing`, or is an event list cut at accepted
coverage. A null reply is pending. Each request may name its first view's class (`firstView`):
`a` exact at the hour, `b` sampled and marked approximate, `c` the newest window with a proven
look-back, `d` the prepared prefix. A ready reply follows that refresh cadence until it is
exact (or unavailable). JSON lines report preparation progress (each status row's state,
counts and text, including the newest battle windows' per-branch earlier-block reads,
look-backs, failed checks and outcomes), each request's first-view (`firstViewMs`), exact (`exactMs`) and query times, each widget's first-view and exact times and
gating request, the map, and the overall time, until every widget is exact or
`--readiness-limit` seconds (default 1200) pass, or stopped preparation's final asks have drained.
The map worker is joined before reporting the summary; its successful first frame records both
map widget readiness times even when it finishes after polling stops.

The probe follows an approximate production-lines reply with the widget's
separate `exact: true` product request. The first request keeps its first-view
time and names the follow-up as `exactRequest`; the widget's exact time waits
for that whole-history request, independently of preparation coverage.

The CPU snapshot at `allReady` records the first-view milestone. The final snapshot
records the end of the exactness probe. With `BATTLE_PLAN_READINESS_CPU_UNTIL_PREPARED`
set, the probe also waits for preparation to end before its final snapshot.
