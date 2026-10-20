# dashboard

Native widget data: opened recordings, typed products, the data service and the
host/viewer protocol. This crate reads recordings and their content stack; it
neither reads live game memory nor draws map pixels. See the
[architecture](../../docs/dashboard.md), [desktop host](../desktop/README.md) and
[widget framework](../../ui/src/widgets/README.md) for the surrounding flow.

The host owns a bounded background preparation queue. Published library recordings
are prepared without being opened, newest first, and opening a source also starts preparation
independently of the opening request; Status, Start, Cancel and Clear
commands operate on that recording through its current source binding. Prepared
queries use the same host service and cancellation boundary as ordinary products,
and read only their requested pages. The reply echoes source, revision and product.
The observation LRU retains whole query replies and their series caches for
numeric summaries and states, air superiority, party history, force histories,
trade and resource balance, wars and casualty breakdown, military air ledgers,
special projects, and the other cached query routes. Their binding names the
supplier coverage pages, the identity page and its coverage, and presentation
inputs. An unrelated store publication keeps the answer; replacing a supplier
page or advancing its gate recomputes it. Refusals are checked before reuse and
current content warnings are appended when serving the reply.
Clear cancels writers across opened revisions of the path before deleting the
prepared files; existing open range readers retain their bytes.

`Service::configure_content` verifies the selected game folder and prepares the process-wide executable tables before reading game settings or exposing content options. Content-dependent consumers share the decoded registry, equipment and rule metadata. Built-in variable names come from the installation's shipped documentation. Recording-only source opening, metadata and numeric histories remain independent of these tables.

## Layout

| Module | What it does |
|---|---|
| [dashboard-text](../dashboard-text/README.md) | The text interpreter, name rules, content catalogue and recording metadata helpers, re-exported here at their old paths. |
| [dashboard-base](../dashboard-base/README.md) | Sources, content, the catalogue loader, the prepared store's shared pages, series, charts, group sums and measures, ranking, the product contract and the shared products including country history, re-exported here at their old paths. |
| [dashboard-core](../dashboard-core/README.md) | Host/viewer protocol, data transport, recording metadata, host configuration, map settings and whole-branch block scans, re-exported here at their old paths. |
| Product crates | One crate per heavy product or product family, above dashboard-base, re-exported here at `products::<name>`: [dashboard-air-operations](../dashboard-air-operations/README.md), [dashboard-doctrine-mastery](../dashboard-doctrine-mastery/README.md), [dashboard-naval-regions](../dashboard-naval-regions/README.md), [dashboard-naval-yards](../dashboard-naval-yards/README.md), [dashboard-weather](../dashboard-weather/README.md), [dashboard-air-aces](../dashboard-air-aces/README.md), [dashboard-air-superiority](../dashboard-air-superiority/README.md), [dashboard-army](../dashboard-army/README.md), [dashboard-construction](../dashboard-construction/README.md), [dashboard-country-fuel](../dashboard-country-fuel/README.md), [dashboard-country-nukes](../dashboard-country-nukes/README.md), [dashboard-economy](../dashboard-economy/README.md), [dashboard-equipment-losses](../dashboard-equipment-losses/README.md), [dashboard-equipment-stockpile](../dashboard-equipment-stockpile/README.md), [dashboard-forces](../dashboard-forces/README.md), [dashboard-game-score](../dashboard-game-score/README.md), [dashboard-game-window](../dashboard-game-window/README.md), [dashboard-intelligence](../dashboard-intelligence/README.md), [dashboard-land-battles](../dashboard-land-battles/README.md), [dashboard-military-air-ledger](../dashboard-military-air-ledger/README.md), [dashboard-naval](../dashboard-naval/README.md), [dashboard-occupation](../dashboard-occupation/README.md), [dashboard-party-history](../dashboard-party-history/README.md), [dashboard-peace-deals](../dashboard-peace-deals/README.md), [dashboard-politics](../dashboard-politics/README.md), [dashboard-politics-ledger](../dashboard-politics-ledger/README.md), [dashboard-raids](../dashboard-raids/README.md), [dashboard-resource-balance](../dashboard-resource-balance/README.md), [dashboard-special-projects](../dashboard-special-projects/README.md), [dashboard-stability](../dashboard-stability/README.md), [dashboard-subjects](../dashboard-subjects/README.md), [dashboard-unit-leaders](../dashboard-unit-leaders/README.md), [dashboard-wars](../dashboard-wars/README.md), [dashboard-world-tension](../dashboard-world-tension/README.md). |
| [dashboard-keyed](../dashboard-keyed/README.md) | Keyed observation families: one observer per family turns each hour into named exact values; the shared block extract, fold, series and query window prepare and read them (air missions and statistics, weather, minefields, naval supremacy, doctrine mastery, occupation reserve manpower, intel network coverage, naval repairs and refits). Registered in [prepared/pipeline/independent/keyed.rs](src/prepared/pipeline/independent/keyed.rs) and routed in [prepared/query.rs](src/prepared/query.rs). |
| [prepared_focus.rs](src/prepared_focus.rs) | The focus order prepared on its own (block facts and their fold), focus queries and focus art evaluated at the hour asked (`prepared_focus/`). |
| [products/](src/products/mod.rs) | Product registry and shared domain helpers; each product's behaviour lives beside its code or in its widget README. |
| [content_check.rs](src/content_check.rs) | Offline preparation and diagnostics for every registered product. |
| [service.rs](src/service.rs), [service/](src/service/) | Source leases, loader jobs, request admission and the bounded response pump. |
| [reply_cache.rs](src/reply_cache.rs) | Whole-branch product replies kept on disk between sessions. |
| [prepared/prepass.rs](src/prepared/prepass.rs) | Block pre-pass: upcoming blocks read once for every family reading them, each family's extract handed what its own read of the block alone (every hour or by its changes) hands it, on workers for families folded in block order; the shared pool of families prepared on their own. |
| [reply_dto.rs](src/reply_dto.rs), [lib.rs](src/lib.rs) | Land-owner and focus reply types (research reply types live in [dashboard-base](../dashboard-base/README.md), recording metadata types in [dashboard-core](../dashboard-core/README.md)), public facade and TypeScript contract assembly. |
| [bin/dashboard-bridge.rs](src/bin/dashboard-bridge.rs) | Development stdio adapter for browser UI work. |
| [bin/export-types.rs](src/bin/export-types.rs) | Generates the frontend contract. |

## Products

A `Product` turns a `Source` plus typed parameters into a typed reply. History
products cover a whole branch, so playback is a local lookup. Hour-specific
artwork and formula details use separate transient requests; explicitly windowed
products reuse a branch scan rather than decoding it for each window.

Each product defines a unique `KEY`, `CONTENT_DEPENDENT`, `Params` and `Reply`;
`validate` checks parameters before loading. The default loader refuses direct
requests while preserving metadata and type declarations. Parameters deserialize and
export through `TS`; replies serialize and export through `TS` and implement
`ReplyWarnings`. Override `types()` when nested wire types need declarations.
Use the shared parameter and selection validators in `products/mod.rs`.

Country groups, wars, subjects and special projects retain their `Product`
metadata and parameter checks; their whole-branch `load` implementations compile
only for oracle tests. Their widgets query prepared state through the shared
prepared routing. The native scan APIs shared with event-feed, faction totals,
air superiority and casualty breakdown remain available to those callers.
Autonomy and world tension retain their registered loaders and metadata; their
widgets use the shared prepared window and reached-value queries.

Construction log, its manifest and spending, resource balance, trade flows,
equipment stockpile, production spending, research and peace outcomes answer
through prepared queries. Their branch-walk loaders, scans and loader-only
caches compile only for tests, preserving their independent parity oracles.
The production `impl Product` retains validation and exported types.
Equipment produced also serves whole-history UI requests and the offline
acceptance tool. Its shared production scan and cache remain live for the
scoreboard's equipment totals and supplying dates.

Equipment design detail's offline check reuses the complete equipment database
preparation. Names use recorded text and token spellings. The check reports the
recorded design, country, organisation and unit context needed for selected stats,
costs, validity and reveal; offline content alone supplies no saved design or
numeric result.

`CONTENT_DEPENDENT` is true when any reply field reads localisation, artwork or
content definitions. The generated `CONTENT_PRODUCTS` list controls frontend
content-cache invalidation. Service, transport and host route by key; widgets
and their data helpers choose products. The shared serialization boundary adds
current uncovered-content-read warnings through `ReplyWarnings`, including for
cached replies; prepared query replies gain them, with the stack's
artwork-effect notes, where they are served. Preserve independent known values
when a name or image refuses.

Preparation and recording queries share the opened source's decoded-section
cache ([dashboard-base](../dashboard-base/README.md#sampled-reads)). Retention is
sized from measured table columns and the live span between preparation, pre-pass
and query reader frontiers. Repeated reads of an admitted grouped block/table
prefix share immutable update cells across families and threads without copying
rows or deltas on a hit; each reader keeps its own selection and reconstruction.
The shared-extract regression also compares every published hourly payload with
section retention disabled and enabled, counting one section decode for a
consumer group while retained.

Keep caches in `source.state::<YourState>()`. Share scans between consumers of
the same inputs instead of decoding a branch again. Read a branch's blocks through
`blocks::scan_blocks`, which reads ahead in parallel; read one hour with
`Replay::seek_selected` and only the tables used (`installed_content::country_names::TABLES`
for country names). Whole-block or whole-table reads cost seconds per block. Numeric history and
content-dependent presentation can have separate cache keys. State is revision
local; append inheritance is an explicit `append_state` opt-in after the source
verifies the recorded prefix. Its clone shares immutable history and gives the
new revision an independent mutable continuation. Cache eviction changes reuse,
never the completeness of a reply.

`military-air-ledger`, `air-aces`, `air-ace-portraits`, `raids` and
`air-superiority` use prepared query adapters. Their whole-recording loaders,
recording availability probes and branch scan caches compile only for tests as
parity oracles; the dashboard's pipeline tests reach the military air ledger's
and raids' through those crates' `test-support` feature. Preparation and queries share the native observers, content
resolvers and reply builders.

Event-feed focus artwork groups completion hours into one chronological sparse
read through the shared query frame cache. Completion hours within a block share
its trigger-table decode and advance their decoder in order; later pages reuse
the cached block and hours. Plain completion art reads no recorded state.
Equipment-loss preparation reads DivisionCensus for membership coverage and
does not select Divisions.

### Add a product

1. Add `products/<name>.rs` implementing `Product`, with validation, cancellation
   checks and the complete-reply budget. Reuse shared history/content helpers.
   A heavy product gets its own crate above dashboard-base, laid out like
   [dashboard-politics-ledger](../dashboard-politics-ledger/README.md), so editing
   it rebuilds and tests only that crate before the dashboard.
2. Export the module and register it once in `PRODUCTS` in `products/mod.rs`.
   Implement `ReplyWarnings` for its reply, normally through `reply_warnings!`.
3. Add its dispatch in `content_check.rs`, reusing the actual naming/artwork
   loaders. State which inputs require a recording, even for recording-only products.
4. Export its nested wire types through `types()` and regenerate the contract.
5. Add/register the widget under `ui/src/widgets/`, following the
   [widget guide](../../ui/src/widgets/README.md). Put product-specific behaviour
   in its code or widget README, then run the affected checks below.

### Exact and derived peace outcomes

See [peace-deals.rs](../dashboard-peace-deals/src/products/peace_deals.rs) and the
[Territory transfers guide](../../ui/src/widgets/territory-transfers/README.md).

### Aircraft catalogue

[aircraft-catalogue](src/products/aircraft_catalogue.rs) answers the Campaign map's Fly aircraft picker a page at a time. It is the [content catalogue of aircraft](../installed-content-equipment-graphics/README.md) over the recording's registered descriptor roster, equipment database and graphics pools, with every model a recorded design saved on any branch: each daily read of `VariantDefinitions`, its type spelled by that hour's `LexicalTokens`. It is built once per source and content binding; each request filters a scope (`aircraft`, names something makes an aircraft's, or `all`, every registered entity and bare mesh and every name a reference makes) by a case-free query over names, titles and reference keys, and answers `count` entries from `first`, flyable ones first. Titles are the localised names of the referencing equipment, a ladder prefix's own (`ENG_fighter_equipment_2`) before the key's. Refused entries keep their reason and are counted by kind over the scope. An equipment database or graphics pools that do not read leave every name listed with that reason; nothing is capped, so every entry is reachable a page at a time.

## Background preparation

Background preparation defaults on. The optional `background_preparation` choice
in the host's `content.json` (see [config.rs](../dashboard-core/src/config.rs)) persists the global
preference while preserving the content folders. Settings can disable automatic admission; explicit
Prepare and Resume remain available and previously prepared files stay readable.
The desktop scans the shared recording library at startup and while idle, so exact
history is usually ready when a recording is opened. `Service::prepare_library`
prepares, through the same queue and store as opening, every recording among the
scanned library and the library work already admitted whose store is behind, one
at a time, most recently modified first. Every session's finalization replaces the
recording it saved into, so the latest sessions come first: a recording further
back is admitted only once every newer one is prepared, and newer work (a session
that ended meanwhile) pauses older library work. A recording whose store is up to
date needs nothing. The only bound is a free-space floor: library work neither
starts nor continues while the volume holding prepared data (`store::local_space`)
has less than one part in `BACKGROUND_FREE_SHARE` (ten) of its capacity free.
Preparation stops at that floor instead of predicting a store's size, which grows
with the recording, and the floor does not apply to preparation the user asks for.
Prepared data on disk is never evicted; Clear removes it.

Paused automatic work stops at its next block boundary, keeps its accepted prefix
and reports `Paused` (`prepared::pause`), not `Cancelled`, with its `PauseReason` in
the status reply's `pauseReason`: `requested` (preparation the user asked for),
`newerRecording`, `diskSpace` (the floor) or `game` (the game guard). The library
scan resumes it once the reason has passed and it is again the newest work behind
its store. A recording opened while the game runs is kept for after it and reports
`Paused` with reason `game` unless its store is known to be up to date, so its
panel follows the start.

The user's work is what the user holds now: a recording with an open view, or a job
an open or Prepare claimed that has not reached a terminal state. The claim lasts
across pauses of the same job (even one still stopping when the user opened it)
and ends at every terminal transition, which all go through `prepared::finish`
(Ready, Failed or Cancelled, including a submission the full queue refused).
Whose work an admission is gets decided in one place, `start_preparation`, under
the admission lock at admission time: the scan, refreshes and other automatic
callers pass a library admission, which runs as the user's exactly when the user
holds the recording then, and otherwise waits for the user's preparation and
stops at the free-space floor; only an open's own admission or Prepare passes the
user's. The scan orders held recordings first, whatever their age, but that order
classifies nothing, so a view closed while the scan runs, or a finished claim,
leaves library work. A store whose progress cannot be checked is not known to be
prepared and is admitted like any other; its preparation succeeds or fails with
its own reason.

Admission does no store or volume I/O under the admission lock. Every admitting
path (an open, Prepare, a refresh, the scan) restores the store first, outside the
lock, and passes the outcome in (`Checked`) with the source's store epoch it read
(`prepared::store_epoch`, read before the content). The epoch rule: the epoch
moves on, under the admission lock or with the running job that writes, whenever
anything may change what a check found (a content rebind, which also forgets
which revisions are finished; a Clear, for every revision of the path; a job
queued to write the store; and that job's end). A check's outcome is used only in
the critical section that acts on it, under the admission lock, and only while
the source's epoch is unchanged: `publish_prepared` and the hand-off inside
`prepared::start`. Otherwise it is dropped and the store is prepared, so a path
that forgets to move the epoch on fails safe. `prepared::start` never restores: a
complete store becomes Ready, an unreadable one fails with its reason, and
anything else is queued. The scan shows a recording Ready (with its restored
sections, no pause or error) only through that check, then retires it; a store
that reads complete but cannot be restored is prepared again. A recording opened
during a game whose store restores is shown Ready rather than paused. The scan
measures the volume once, before any admission; a refresh uses the scan's last
measurement and measures again, outside the lock, once it is older than the
interval between scans; an unmeasurable volume never stops work. A refresh reads
whether the job is the user's Prepare before it cancels it, so a content restart
restarts the user's Prepare as theirs, and it re-validates under the lock when it
admits: a Cancel or Clear by the user since it captured that (`user_stops`, or
the store cleared) drops the refresh, so a captured Prepare never overrides a
later user action. Newest-first is one rule, `enforce_newest_first`: every
library admission runs it inside `start_preparation` (from the scan or a
refresh), and every scan runs it at the end; of the jobs running as library work
right now, the newest recording's keeps running and the rest pause
(`newerRecording`); recording age comes from each source's revision stamp.

A store another service or process changes after it was checked is caught
eventually, without I/O under the lock: each check records the store's manifest
identity (`Store::manifest_identity`) before restoring, and after any admission
that may have published from the check (the scan's publication, an open, Prepare
or a refresh shown Ready from its restore) the identity is read again outside the
lock. If it changed and nothing else moved the epoch since, the Ready is
withdrawn to Idle, the revision is no longer stamped finished, and the source is
kept, so the next scan prepares it. A reader may see Ready until then. Clear deletes a store by its recording's path without
reading its manifest, so a damaged store can be cleared. A user's Cancel overrides
a pause, and the scheduler never turns a Cancel into one. Content and preference
refreshes leave paused work to the library scan.

Exactly this runs under the admission lock (`preparation_actions`): admission
decisions in `start_preparation` (the lease table, other jobs' progress and
suppression, all in memory) and the queueing in `prepared::start`;
`publish_prepared`; making way for the user (`take_over_preparation`, including
waiting for a replaced revision's writer to stop); pausing library work
(`pause_library`, `enforce_newest_first`, the game guard); cancelling automatic
work when the preference turns off; a content rebind and its epoch moves; a
refresh's ownership read, its restart wait and its re-validation; withdrawing a
publication another writer invalidated; and Clear, which cancels and waits
for writers and deletes the store while admissions are kept out. Apart from
Clear's deletion, no file or volume I/O runs under it: restores, volume
measurements, store identities, source opening and recording revision reads
(`recording_revision`, including when the preference turns back on) happen
before it is taken or after it is released.

Library work yields to the user. Opening a recording, or Prepare, first makes way
for it (`take_over_preparation`): the recording's own queued or running job is
joined, the same source and job rather than a second one, and becomes the user's;
a running job of another revision of the same recording is cancelled and waited
for before the new one is admitted, since one writer holds a recording's store;
library jobs of other recordings pause. Library admission waits while any
preparation the user asked for is queued or running, then resumes. A full queue is
retried once, after the paused jobs leave it. Library jobs run each
preparation thread one step below its level in a job the user waits for
(dashboard-base `prepared::priority`): the hourly walk's coordinator, its
synchronous stream decoder, family threads and flush helpers drop from normal
to below normal; other pre-pass, shared-read and lane workers drop from below
normal to lowest. Persistent runners enter each job's coordinator level while
it runs and restore their previous level afterward; they use no separate
Windows background mode. A job the user joins starts its later threads at their usual levels.
Unchanged and busy replies end admission retries. Recording revisions, completed
workers, changed preferences/content bindings, game transitions and crossing the
free-space floor permit a new attempt; timer ticks alone do not submit further
preparation requests.
It detects completed sessions through the recorder's atomic recording publication.
A running game suspends automatic admission and pauses automatic jobs (reason
`game`) at the existing block boundary, so a session that ends while the game keeps running is
prepared once the game exits. After game exit, accepted prefixes resume from their
unaccepted tails, including interrupted opened recordings outside the library,
whatever their age.
Manual preparation remains available while the automatic preference or game guard
is off. Clear and Cancel suppress automatic library retries for the recording path
until explicit Prepare; that suppression lasts for this app session and spans
appended revisions. No new preferences or persistent scheduling files are used.

`prepared::unprepared_blocks` compares the saved checkpoint cursor and index/semantic
metadata with the recording's current trailer index. Matching full file sizes can
still have an unaccepted tail. The helper reads no frames or observer accumulators;
source opening can read the initial country catalogue when no metadata cache exists.
Completed preparation attempts retain only their path/revision stamp in the service;
Work waiting for admission or interrupted by game presence retains its shared
source until completion. Failed
attempts, including failed opens, wait for explicit Prepare, a changed recording
revision or a content rebind.

Preparation branch state stores every observer behind a heap-owned pointer,
including observers restored from separate ranges. Its JSON fields and existing
store schemas remain the same. Branch construction, checkpoint restoration and
query continuations keep their aggregate handles small; serialized byte buffers
and recorded collections remain on the heap.

`prepared::runner::worker_thread` supplies the shared 2 MiB production stack for
preparation workers, service loaders and the desktop library admission monitor.
The acceptance harness uses the same constructor for all its setup, preparation
and query modes. The stack layout probe prints compiled observer sizes; synthetic
full-prepare/resume/append and serde/query checks use that exact constructor.

`prepared::start` admits real work to two background workers with a bounded queue.
`Source::open` holds the read-only replay file rather than buffering the recording;
source append inheritance compares recorded block index metadata. Prepared
directories derive from the canonical recording path, so separate recordings of
a campaign keep separate data. Accepted preparation saves the current source
catalogue and its optional resolved names. Matching recording/index stamps
restore these plain metadata pages without decoding an initial recording block;
stale or unsupported metadata cache entries fall back to the recording reader.
Growth reuses the cached initial catalogue after
checking the saved index and semantic metadata prefix. Trailer ranges, gaps and
event-loss intervals refresh directly; a new branch reads only its new catalogue
blocks. Cached country
names carry forward by their original observation; only new country observations
need name evaluation. Names validate against the resolved content binding;
changed content stamps discard cached names. Changed prefix metadata falls back
to the recording reader.
Whole-product cache entries for held files use the canonical path and opened
metadata revision.

The hourly walk prepares map checkpoints, country balances, army size, resource
balance, land battle logs, game score, autonomy and the land-owner catalogue.
Its hourly selection is the union of the country balances, land battles, game
score, autonomy and land-owner inputs, including the shared country-name
operands (`pipeline::selected`). Army and resource tables join a block's selection
only when that block's extract could not be folded (`block_selected`); their
successful folds use their own facts. A change of selection restores the accepted
stream cursor before reading the next block's self-contained keyframe. Metrics
and read evidence remain whole. The saved decoder contains the hourly selection.
Families prepared on their own are skipped. A partial registry returning any of those
families to the walk keeps the full table selection, as the hourly test oracle
does, so its content-dependent reads remain available.
Map progress observes every accepted hour, but map rows are read only at the
existing checkpoint hours (`prepared_map::Builder::accept_sparse`). A checkpoint
at its block's first frame reads only the selected keyframe sections; one inside
a block reads and reconstructs that block through the exact checkpoint hour.
The main stream admits the whole block before observation. Sparse checkpoint
reads preserve the same selection, frame ordinal, metrics, sample hours, partial
rows, refusals and compressed bytes as the hourly map observer. Short blocks,
gaps, interleaved branches, resume and append retain the accepted-frame stride.
One replay worker reconstructs a block sequentially through a channel retaining
one queued frame. For each observed frame, scoped worker lanes borrow disjoint
family accumulators while the coordinator runs the observations that write the
store or need another family's output of the same frame, staging writes in their
original order. The map writes through the store writer, so it stays on the
coordinator with the land-owner catalogue, whose errors stop preparation.
Country balances, game score, autonomy and land battles are lane jobs; resources
and army are lane jobs when their block extracts fall back. A job
keeps its coordinator position as its order, so a frame reports the first failing
call in serial order, and a job's refusal is recorded at the frame's hour, as on
the coordinator. Every lane finishes before the next frame or any block finalization,
publication, budget inspection, resume or branch change. No accumulator/history
is duplicated, and every lane shares one frame's existing table allocations.
One runtime frame fact separates allocation changes from sampled dates and the
current table/slot read evidence. Preparation computes it once before dispatch
and shares it with all lanes and the coordinator. Weak stamps retain no previous
rows; branch changes, missing hours and observer restores invalidate the stamps.
The fact also carries the hour's recorded updates: `FrameChanges::rows(table)`
names a changed table's rows, each with its changed fields, and its deleted keys
when the decoder applied exactly that delta to the previous hour's rows. It is
None whenever any row may have changed (keyframe, snapshot, gap, resume, failed
delta, or a table an observer marked changed itself), so observers then rescan.
Families adopting the fact still advance their own clocks and read owner-specific
SliceReads from the frame; unchanged allocation alone never proves no fresh read.
Lane count is available parallelism minus the decoder and coordinator, capped by
the number of independent groups; two or fewer available processors use serial
preparation. Scoped lanes close and join on cancellation, errors or unwinding.
Cancelled frames never move the accepted cursor or decoder resume state.

Resource balance and army size are prepared from blocks rather than observed
hour by hour. Pre-pass workers (`prepared/prepass.rs`: available parallelism
minus the decoder and coordinator, at least one) share the family pool and
its measured longest-chain order. Tasks supplying the walk run at its
coordinator level; other tasks yield one level below it. They claim upcoming
blocks in order, at most one more than the workers ahead of the block
the coordinator takes next and, beyond that block, within 512 MiB of claimed
blocks' recorded raw bytes and the bytes the facts of those extracted hold
(`Extract::bytes`: each kept frame's views once, recorded calls, and every
value kept beside them, such as scans, maps, sets, strings and serialized
publications, at capacity, with each shared allocation once; `Held`).
Each reads its block alone from the block's keyframe once for all its
families (`Replay::read_block_watched`: the tables they read decoded and
reconstructed once), hands each family's extract exactly what its own read of
the block alone hands it (every hour of its tables, as `Replay::read_block_alone`
reconstructs them, or the hours its fields change), with change facts from a
tracker of that block alone for that family (preparation starts every block
without the facts of the last, so these are the facts observers get). A family
whose own read would refuse the block's keyframe observes it hour by hour; when
the one read fails, each family reads the block alone. Before observing a block the
coordinator takes its outcomes. A family whose extract refused, failed or
panicked, or whose block could not be read alone, observes that block hour by
hour as a lane job, so its refusals and errors are the ordinary ones, in the
ordinary order. Otherwise, once the block's ordinary observation succeeded and
before its batch decision, the coordinator folds the family's facts into the
branch state, in block order. Cancellation ends the coordinator's wait; dropping
the walk's pool view retires its lanes, and the phase stops and joins the shared
workers. A standalone pre-pass stops and joins its own workers. Facts
depend on neither the number of workers nor timing. Measured runs time folds
under the family and extracts as worker time (`<family>.extract`), and count
blocks observed hourly (`<family>.sequential`) and folds that observed their
block again (`<family>.fallback`). Shared-pool block reads, including the walk's
prerequisites, are timed once under `prepass.read`; resource and army extracts
retain their family timing rows.

A series family adopts the pre-pass through shared recording
([prepared series](../dashboard-base/README.md#prepared-series)): its fold
observes the block's first hour itself on the carried state and makes the calls
the extract recorded from the second hour again on the carried accumulators.
When a check of the carried state fails it observes the rest of the block hour
by hour with the block's own facts.

Beside the families prepared on their own, each branch's newest land and naval
battle windows are published from its last block and a look-back proven to
hold every fact the page carries, so each family's manifest and that window
answer before forward preparation reaches them. Each window commits on its
own through the families' group commits; the land window's diagnostics follow
on its Beside job after the proven page is published. The land and naval battle
logs below describe the proofs.
Beside them too, each branch's whole production line
scan of the player's country, and of every country a request read whole this
session, is kept under sections of its own, one commit per scan, unless one is
kept for the branch's current last block (`prepared/pipeline/production_scans.rs`).
Only the source's preparation keeps them, through its own writer, admission,
game guard, cancellation and Clear; a request never writes one and queues no
preparation. Every preparation keeps the player's scan. Another country's scan
a request read whole after the source's preparation finished is kept the next
time that source is prepared in the same app session, or is scanned again
later. A scan that cannot be read is not tried again that session.

Families prepared on their own run beside the hourly walk
(`prepared/pipeline/independent.rs`). Each is a registry entry (`Own`): its
pre-pass family, the query products it gates, its branch state (`Branch`:
loaded from its own saved sections or new, folding a block's facts or observing
the block hour by hour, publishing the pages it held from each block it went
through, flushing, saving its pages and its continuation) and the sections the
hourly walk keeps for families that read its state in the same hour, with those
readers. Every registered family starts at once on one pool of pre-pass workers
(`prepass::Pool`, also serving the hourly walk's resource and army extracts) over the source's one opened recording, so each block's checksums are
verified once. A worker serves the longest remaining measured chain
first, including the walk, claiming its next block for every family whose next block it is and
within reach (the same read-ahead window per family). The cost is the live mean
extract time plus its own thread's mean fold and block-publication time and
amortized batch serialization, multiplied by its remaining blocks. The walk's
two extracts count together with its decoder, map and land observation, folds
and block-end publications as one downstream chain. Pre-pass and group-commit
waits are excluded. Unknown costs use the live mean; equal costs
prefer the earlier block and registry order. The worker reads the block once
for those families and runs their extracts costliest first. When fewer blocks
are coming within reach than workers wait (a session's newest block), a block
is split between them by those costs, each part read on its own. The byte budget counts
each claimed block's recorded raw bytes once however many families claimed it,
and the facts of every block extracted and not yet taken by the bytes its
extract reports (`Extract::bytes`; a block claimed reserves its family's mean
facts so far, replaced by its own once extracted). A measured run times the
blocks' reads beside the extracts as `prepass.read`. Each family folds its blocks in order on a thread of its own
and commits by the hourly batch rules (its byte budget counting what it holds
alone), with its pages, its own coverage page per
branch (`preparation-family-<name>-<branch>`, with its refusals) and its
progress in the checkpoint. Families commit through one store writer in group
commits: while other families run, a family writes its batch into a fork of
the open transaction on its own thread and queues it. The first family to queue
while no group forms leads one: it closes the transaction to new forks, waits
for the forks still being written, adopts every queued batch in registry order,
each whole or not at all (`store::Writer::adopt_all`), writes the checkpoint
once and publishes them in one store commit (one manifest rewrite, one set of
flushes). The lock is released while the store commits, so families keep
folding; one reaching its commit meanwhile forks the next transaction once it
opens and joins the next group. A family's write and adoption errors are its
own and leave the others' batches to commit; an error of the joint publication
(sources changed, cancellation, the checkpoint or the store's own writes)
belongs to no family's batch and is every member's, and, as any failed commit,
loses the writer. A family's pages, ranges and saved state do not depend on
which families committed with it: each section is one family's, numbered by its
own writes; only where sections new to the manifest are listed follows the
groups, and every read finds a section by name. Timings report each family's
commits, waits included, as `independent.commit`, and each store commit, by the
family that led it, as `independent.publish`. The hourly walk starts after
the first-view jobs beside the families (`Beside::first_view`, the newest
battle windows) have each published once or finished without a view; its block
registered pre-pass lanes are released then too. Family completion is
no gate. The walk stages each unchanged publication batch in a detached fork
(`store::Writer::stage`), including its parallel flushes, while families continue
folding and committing. Its batch joins the group only once its writes and
flushes are ready: adoption holds the writer briefly, validates the ranges the
walk read or replaced, and publishes all its pages with its checkpoint. Family
commits do not wait for a walk batch in preparation. Dropping an unfinished walk
batch leaves its pages private. First views retain their existing query lanes
and priorities. Each walk commit merges
its cursor, map and refusals with the latest accepted family and newest-window
progress, so neither publisher can overwrite another's progress. Status reads
a snapshot of the last successful store commit, including while another group
publishes without the lock or the walk prepares its detached batch. Each publisher
resumes its own committed prefix on reopen and recording growth: a family may
lead or trail the walk, and every cursor must stay within the unchanged saved
index prefix. Completed-store restoration requires every publisher to finish,
including work beside the forward publishers. `preparation-side-work-v1` keeps
the pending side jobs and requested production scan roster. A side job's final
group commit clears its pending entry; page publication alone does not finish
newest-window diagnostics. Reopen retries pending newest jobs even when forward
coverage or their first pages are complete, and resumes outstanding production
scans from the durable roster, including requests from an earlier Source.
`unprepared_blocks` counts the walk's tail alone for its measurement journal;
zero can still leave family or side work. The
`independent` timing row counts time outside the walk rather than counting its
overlapping wall time twice. Family and side-job rows, including their publications,
are worker time. Work beside the families
(`independent::Beside`: the newest battle windows) runs on threads of its own
from the start of the phase, however the families prepare, and publishes each
batch as soon as it has one through the same group commits: it forks the open
transaction, writes the batch, queues it and leads or joins a group as a family
does, with no progress of its own, in sections no family writes. Groups that
wait for every family still preparing count only the families' batches. A family whose block publication reads back what earlier
blocks wrote publishes each block at its end (`Branch::publish_block`) into
pages held for its next commit (`dashboard_base::prepared::pages::HeldPages`),
which answer its reads from its held writes, then from what it committed; their
bytes count against its batch budget, and an error refuses the branch at the
block's last hour. A commit first writes those pages in the order they were
written, then the pages the family held from the blocks it went through since
its last commit (`Branch::publish`), also when it refused the branch later in
the batch, as hourly preparation wrote them at those blocks. A family that refuses a branch goes on with its other branches;
one that fails or panics leaves the others to finish and commit, and preparation
then reports its error. A family's pages and saved state are those hourly
preparation publishes. Its gated products wait only for its coverage and refuse
from its page (`pipeline::product_coverage`; a series request by its family
kind), and every query `FAMILIES` lists as reading it refuses from that page too
(`pipeline::refusal`). A product whose replies read several families' pages
up to the hours they answer waits for the coverage they all have
(`independent::JOINT`: each one's own while prepared on its own, the branch's
otherwise): country factories and stockpiles, supply health, equipment
losses and territory with the identity history they are named through, factory faction totals with the faction rosters and the
identity history, territory faction totals with the faction rosters, the
identity history and territory, and the event feed with every family whose
changes it merges. A family prepared on its own may have committed pages beyond that
coverage: country factories and stockpiles then read theirs only through the
last batch committed by its last hour (`series::committed_coverage`,
`Reader::through_hour`) and the hours after as an unprepared tail, and a
factory faction total reaching past it waits, but for a reached hour under
hour membership. Readers in
[dashboard-base](../dashboard-base/README.md) ask both through the
`coverage::Gates` the pipeline, the query route and the service install
(`pipeline::install_gates`). Status shows its own progress: its section rows
count its accepted blocks, and `families` lists its coverage by branch, as its
coverage pages hold it (`independent::coverages`), beside the hourly walk's
`coverage`. A prepared reply that is pending (null) or partial (its `coverage`
missing or incomplete) carries a `gate`: the status entries whose coverage it
found short while answering (`coverage::waiting`), each a family prepared on its
own or `walk` for the walk's coverage, or, when it consulted none, those its
product waits for (`pipeline::gate`). Before a first view, clients ask it again
when one of them moves. A shown unfinished reply refreshes after a moved gate
no sooner than 2 seconds from its last reply, then 4 seconds, then 8 seconds
for every later refresh. Every gate completing on the selected branch bypasses
that interval. A terminal status (paused, failed, cancelled or idle) also asks
once before reporting its error, delivering any accepted point or prefix held by
the interval. Other polling failures drain deferred progress; source cancellation
discards it. New navigation, ranges and selections ask immediately. This
policy lives in the shared frontend prepared-query loop; the readiness probe
uses the same intervals and completion rule.

A product whose values need earlier observations samples no unprepared hour.
While its gate's coverage is partial it answers from the pages that coverage
holds, through its last hour, rather than null
([`prefix`](../dashboard-base/src/prepared/prefix.rs)): the reply's root carries
that `coverage` and `pending`, the requested recorded hours after the prefix
without recorded gaps (empty once nothing requested waits). A whole-branch
reader's hours after its last observation are pending there, not unobserved
(`Prefix::clip`). A value in an answered row that needs history the prefix has
not reached stays unknown with the reason `preparing`: a variant of the
product's unknown-reason enum (`ProductionSpendingUnknown`, `SpendingGap`), or
the row's `preparing` field names. So answer `idea-history`; the `events-page`
of every family, peace outcomes and research included (event lists hold only
accepted events, so the hours after are always pending); the event feed,
backwards from the joint coverage's last hour, each continuation keeping the
live hour of its first page; and the politics ledger's spending `totals`,
summed through the prefix's last hour. A war casualty state read past the
prefix answers at its last hour. The country groups answer the prefix carried
on by later keyframes, marked `approximate`. The wars answer the walk of the prefix's
change inputs, with the groups at a read hour past it from that hour's frame
alone (`current`). Null remains for a gate with no accepted
coverage. Readiness counts such a reply ready (`Outcome::Prefix`).

A product whose lanes hold a state of each frame answers the hours after its
prefix from each later block's keyframe
([`keyframes`](../dashboard-base/src/prepared/query/keyframes.rs)) instead: its
reply's `approximate` names the sampled hours, exact at each keyframe and
approximate over its block, and `pending` only the hours left
(`Prefix::sampled`), those of a keyframe it could not sample. So answer
`subjects`. Compact per-block observations use the shared source cache
([`observations`](../dashboard-base/src/prepared/query/observations.rs)); re-asks
join the observations still beyond coverage instead of reading or sampling them
again. Content bindings and recording revisions invalidate them; retained bytes
are bounded by their live block working set and observed allocation capacities.

The hourly walk asks the registry whether to observe, flush,
save and load each family (`Refused::live`), so it skips a family prepared on
its own unless another family reads that family's state in the same hour; then,
until every such reader (`Own::read_by`) is prepared on its own too, it keeps
its own read, the same observation saved under the reader's names
(`independent::kept_as`) and never published as the family's. The walk's saved
branch state leaves out the state of the families it skips. A checkpoint whose
families of their own differ restarts preparation.

A family whose own thread observes every block hour by hour on the block read
alone (the politics ledger, occupation, the naval family and the sampled
families)
declares the tables it reads (`Own::shared_tables`) and reads each block
through `Block::read_alone`. One that keeps nothing of an extract (occupation
and the sampled families that do not fold) has no pre-pass lane
(`Own::laned`): the pool reads none of its blocks for it, and its thread
observes each block as its fold of an extract did. Those families read each block once, together
([prepared/pipeline/independent/shared.rs](src/prepared/pipeline/independent/shared.rs)):
reader threads below normal priority (one per family, no more than the
machine's hardware threads, each reading one block at a time) read it ahead of
them for the tables of every family still preparing (`Replay::read_block_kept`),
and each family reconstructs its own tables from that read on its own thread
with a decoder of its own (`Replay::reconstruct_alone`), at its own pace. A
selected read decodes each table's updates on their own, so a family's frames,
their allocation sharing (its change facts), availability, sample hours,
metrics and recorded updates of its tables are its own read's; its other
tables stay unreconstructed and unavailable. A block whose read fails, or
holds a keyframe the decoder's all-table check would refuse for another
family's table, is read by each family alone. Each family wants the block it
takes next and one more, readers read the wanted blocks nearest to a family's
next first, and a block read stays kept while a slower family will still take
it, so it is read once for all of them. A reader gives a block up, unkept, as
soon as no family wants it: it checks before each table section it decodes
(each hour of an interleaved block). A family that refused a block's branch
wants nothing. `Block::read_through` reconstructs tables through an hour from
the same read, for a family that reads more of its tables at some hours.

Only the readers and the pre-pass pool's block tasks read a block for every
family; a family never does. A family takes a kept block that holds its
tables; or, while a reader is free to read its next block, waits for it; or
reads the block itself with its own tables alone (`Replay::read_block_alone`),
exactly as its own read alone, keeping nothing. Each kept block knows the
tables it holds (those of the families that may still read that block), so a
finished family's tables, tables of a family already past that block and
tables of a family currently refusing its branch are left out, and a family whose
tables a kept block lacks reads that block alone. The pool's block tasks read
most blocks before these families take them: a task reading a block one of
them will take within as many blocks past its next as the budget has shares
reads their tables in the same read (`Replay::read_block_watched_keeping`) and
deposits the block's frames before its extracts run (`prepass::Depot`), so the
readers read only the blocks no task deposited; a task that does not deposit
an offered block withdraws it. A deposited block's frames hold every table the
task read, each table's updates its own read's. Tables kept only for these
readers are decoded but have no reconstructed views in the task; each reader
reconstructs its own selection. A task never waits for these
families. A pool block's own decoded updates live only while its task reads
it, one block per worker; only the facts its extracts keep outlive it, counted
by their decoded bytes.

Every block read for every family is charged by the bytes its decoded updates
hold (as a block's facts count recorded updates; a block's recorded raw bytes
far undercount a block of predicted changes) from when it is kept until its
last holder drops it: a block released from those kept, so that no family
takes it any more, stays charged while a family still reconstructs from it.
Each read under way reserves one block's share of the 512 MiB budget,
`budget / (readers + 2)`, and starts only while what is charged and reserved,
with its own share, fits the budget; before any block was measured only one
read is under way, which measures the first. Reading for every family stays on
only while every block measured holds at most a block's share: once one holds
more, the source is read standalone for the rest of the run (readers and tasks
read for nobody, every family reads alone, as before the shared reader, and
kept blocks are released as their holders drop them). The bound, while
sharing: charged bytes <= budget + (reader threads + busy pool tasks + 1)
blocks, each at most `budget / (readers + 2)`, where only the reads under way
when a block first measures beyond its share may exceed it, and that ends
sharing; in standalone mode, only the families' own reads. Whenever what is
charged passes the budget, kept blocks are released, furthest from the family
taking them first, and read again when wanted. A deposit released here may
still be held by its pool task until the task ends, as that task's own block.
A measured run times the shared reads as `independent.shared_read` and counts
the deposited blocks as `independent.shared_deposited`; a family's own read
rows keep its wait and its reconstruction.

A pre-pass family may declare the fields it reads (`Family::fields`, as
`replay::container::TableFields`, a table optionally visited at every read). Its blocks
are then read only at the hours those fields, row keys or availability change
(`Replay::read_block_changes`): its extract observes the block's first hour and
each such hour with the change facts the block read alone has there
(`FrameChanges::rebuilt`, from the reader's evidence of the tables each hour
rebuilt; `FrameChanges::rows` is None for a changed table), and is told of the
quiet spans between (`Extract::quiet`). The pre-pass splits a span and gives the
extract a frame at each hour that follows a gap, decodes again a table it
watches whole or one that is incomplete, or reaches the extract's deadline
(`Extract::deadline`: an hour it must observe with no field changing). Such a
frame is built on the last frame observed, with the hour's metrics, sample
hours and a new allocation of each table its update rebuilt. Each later frame
the extract is given holds every table so built at that allocation until the
table's update rebuilds it again, so tables told apart by allocation follow
hourly preparation's. Each family's read watches exactly its own fields, so
the extract is given at each hour read the tables the block reader reports
changed there for those fields (`Extract::observe_reported`; none at a frame
built in a quiet span), and need not find them again from the hour before's
rows.

The pre-pass releases a last-observed frame's replaced table views before
reconstructing an hour the block reader already knows will visit its watch
(`BlockWatch::releasing`). Pending quiet hours are delivered first, so their
frames keep the preceding rows and built allocations; tables carried unchanged
into the next frame retain their allocation. The shared reader caches recorded
updates, not reconstructed views. Consumers needing a block's final frame keep
only that hour, and the naval two-read handoff names the second read's boundary
allocations through `ViewIdentity`, which keeps no rows alive.

A family prepared on its own whose content decides what its observation can
read chooses its run's reads from the content the phase binds (`Own::binds`,
`Own::bound`: a `prepass::Bound` of tables, fields and extract in place of
the family's declared ones). Its pool lane claims no block until the
family's own thread has bound them, before its first block, so every block
of the run is read with them; the family's facts say what its extract read.

Shared hourly readers choose their table union from that content too:
occupation leaves out the trigger kit when no bound law has an eligibility
trigger. Unscoped project extracts read no country presentation tables.
Supplying-read flags are computed once per hour and timed-table set through
the recording's cadence groups. The block reader's rebuilt tables become an
indexed mask, and quiet-hour columns are searched in their table order.
After reconstruction, families sizing a block share immutable view sizes,
while keeping independent allocation counts and byte totals. Measured runs
also report inclusive `extract-start`, `extract-size` and `extract-finish`
notes by family; these do not change the exclusive CPU labels.

Country groups are prepared on their own. Their extract
([products/country_groups/prepared.rs](../dashboard-base/src/products/country_groups/prepared.rs)) reads a block by the fields the scan
and the change publisher read: the countries' keys, slots and tags; the war
operands, not their casualties or cancel byte; every field of the other group
tables, whose every row change the scan counts as a change of the groups; and
every read of the dictionary. It runs a scan of the block alone to find the
hours that change the groups (its deadline is the next war's end) and keeps
those hours' frames, with countries reduced to the id, slot and tag the groups
read, and the spans of unchanged hours between them. The fold observes each kept
hour on the carried groups exactly as hourly preparation does, and advances each
unchanged span in constant time (`Scan::advance_quiet`), the same step hourly
observation takes for one such hour; when the carried scan does not hold the
inputs the block's scan held before a span, it observes the rest of the block
hour by hour. Their change publisher's continuation is saved as
`country-group-state-<branch>`. Air superiority reads the groups' state of the
same hour. Prepared on its own, it makes its own read of them (`Own::read_by`),
so the hourly walk no longer observes the groups; while air superiority is in
the hourly walk instead, the walk keeps that read for it. Either read is saved
under air superiority's names (`air-superiority-groups-<branch>`,
`air-superiority-group-changes-<branch>` and, from its own read,
`air-superiority-group-state-<branch>`), never published as the groups', and
makes the groups' calls on the groups' frames at the groups' hours, so a groups
refusal reaches air superiority at the hour hourly preparation records it.

Air superiority is prepared on its own
([products/air_superiority/block.rs](../dashboard-air-superiority/src/products/air_superiority/block.rs)).
Its extract reads a block by the groups' fields and its own: the countries'
keys, every link, every score field, each wing's mission region (not its plane
count), the theatres' names, the theatre groups' theatres and the theatre
rosters' keys. Its deadlines are each day's first hour, where the day's sides
are chosen, and the next recorded war end, where the groups change with no
field changing. It keeps the observed hours' frames (countries reduced to their
keys, wings to their mission regions), each with the hour's score projection
made on the worker through the score identity it read there, and for each hour
between, the theatres' slot read and whether a table the day's choice reads was
decoded again (war casualties move every hour of a war without changing the
groups). The fold observes each kept hour on the carried state, the groups and
then air superiority as hourly preparation does, taking the worker's projection
once the worker's score identity is found equal to the carried one; each hour
between takes the groups' and air superiority's unchanged-hour steps
(`Prepared::quiet_hour`); and each block ends as hourly preparation ends it, the
groups sealed and the day's choice held open. When the carried state does not
continue into an hour between, the rest of the block is observed hour by hour.
Scores move every hour of an air war, so those hours are observed on the
family's own thread, their projections made on the workers. Each projection
also carries how its pairs differ from the worker's previous one in the block;
when the carried observer applied that previous projection the hour before, it
observes only the changed and gone pairs (an unchanged pair would observe its
held score, naming nothing), in the order the whole walk observes them, and
otherwise walks every pair. A pair's keys are held while it is current and a
region's are held once named, so a key is named (its field serialized) only
when new. A measured run times the fold's parts (`air_superiority.fold.*`:
pairs walked whole or by their diff, fold-made projections, regions, wings and
theatres, day choices, the groups' steps and quiet hours). A groups refusal
refuses air superiority at that hour with the groups' error, recorded as coming
from the groups.

World tension reached first views extract the frame's independent total and
country aggregates through the preparation observer, without walking source-event
history. Current named amounts are exact; historical contributor zeroes and
event continuity remain preparing. Unresolved nonzero slots carry their exact
amounts in `preparingContributors`, while `unnamedRaw` is null with its field
listed in the read's `preparing`. A transient catalogue key is the JSON pair of
tag and slot, so a later prepared catalogue cannot redirect it to another
contributor. Campaign geometry joins the prefix with cached keyframe totals and
named amounts, marked approximate. An absent sampled contributor stays unknown;
its historical zeroes need preparation. Events answer the prefix with pending hours.
The reply keeps following identity coverage until native metadata is complete.

World tension is prepared on its own
([products/world_tension/block.rs](../dashboard-world-tension/src/products/world_tension/block.rs)). Its
extract reads a block by the countries' keys, every link and every field of the
tension, threat source and country aggregate tables, whose every row change the
observer takes as a change of its inputs; the countries' balances are not read.
It keeps the observed hours' frames (countries reduced to their keys) and, for
each hour between, whether the tension tables were read there: a read with equal
rows starts the held values again and counts the read's diagnostics. The fold
observes each kept hour on the carried observer as hourly preparation does and
takes the observer's unchanged-hour step for each hour between
(`Scan::advance_quiet`). The observer's slot-to-tag and link-to-slot maps,
threat sources' first and last reads, first contributions and warning counts
are its carried state, saved in its catalogue, so each block continues the
history of every earlier hour; a block that gives it no hour ends its segment,
as hourly preparation ends it after a block it read no frame of.

The country identity history is prepared on its own; its extract and fold are
described in [dashboard-base](../dashboard-base/README.md#country-identity-history).
Territory's owned-state observation drives the history and the event feed reads
it in the same hour; both are prepared on their own with their own reads of it
(territory's saved as `territory-identities-<branch>`, where the hourly walk
kept it while territory was in the walk), so the walk keeps none. An identity
refusal reaches each at the hour hourly preparation records it, as coming from
the identities.

Focus art uses the production interpreter at the hour a reply asks for
([prepared focus pages](#prepared-focus-pages)). Content inputs come from the recorded stack's physical files,
including installed mod archives and the selected language settings.
Their size/mtime stamps are validated once when source/content options bind,
then retained in source state. Prepared queries use this binding and the current
small store manifest without restatting installed content. Each prepared query
installs a store scope (`store::Store::scope`) for its source key, so the whole
query checks the recording and reads the manifest once, and shares pack handles.
Series readers keep what they read for the query
([prepared series](../dashboard-base/README.md#prepared-series)). Publication validates all
inputs before committing; the store keeps their stamp list in a separate immutable file.

Publication batches end at a branch boundary, after a branch's 1st, 2nd, 4th,
8th, 16th and 32nd blocks and then after every 32nd block (counted from the
branch's first block, wherever preparation started, resumed or grew), at a
64 MiB byte budget, after every block of a measured run, or at the recording end
(`pipeline::Batches`). The early ends let a view at a branch's start become
exact once its family has folded that branch's first block. The hourly walk and
every family prepared on its own end their batches at the same blocks. Wall
time never ends a batch, so the ranges a recording publishes do not depend on
how fast the machine prepares it. Progress and cancellation are checked per
block.
Domain data, branch accumulators, decoder resume state, accepted coverage and the
common cursor share one manifest commit. Saved hash-backed continuation
maps and sets serialize in key order, including tuple-key pair arrays and nested
holder sets. A source-tree test rejects unreviewed hash-backed serialized fields.
Saved continuations read every floating-point value back bit for bit. Each
hourly branch, census and sampled-family continuation buffers its JSON writes
before level-1 compression, sharing the economy continuations' encoder. The
saved bytes are the same as streaming each JSON write directly. Group commits
keep the map binding's current range when its serialized bytes are unchanged.
An observer restored from its saved state publishes and saves exactly what the same
observer continued in memory would: a commit never adds a run break, an event or
a changed value. What a family does not save it rebuilds from its next frame,
including the holders and relations of the hour before that it still holds open.
Families whose published warnings carry content refusals (the naval family's
sinking log, titles and details, land battles, occupation, leaders, aces,
casualties and special projects) observe and flush as their own consumer of the
shared catalogue (`catalogue::own_refusals`, [own refusals](../dashboard-text/README.md#own-refusals))
and save that tally with their state, so those warnings name only the refusals
of their own reads, whichever lane, process or other consumer read first.
Country-history tags register supply actors
in stable tag order. Thus equal observation/commit plans produce equal section
payloads across fresh processes and serial or lockstep observation. Cancellation or an I/O refusal leaves
the last committed prefix readable. Resume starts at its next source block;
growth compares the saved index, session, branch and gap metadata and prepares
only new blocks. Changed index metadata starts a replacement. The current batch
is buffered; saved accumulators retain only continuation state. When the next
batch continues the branch a batch flushed, preparation keeps that branch's
observers in memory rather than restoring them from what it just saved; a
pipeline test publishes every section the same both ways.

At a batch's end the families flush. When lanes run, the flush runs on scoped
threads (one per available processor): each family flushes into its own fork
of the batch's store writer (`store::Writer::fork`), appending to the fork's own
pack file, and a family's flush steps stay in their serial order on one thread,
as do the identity history, territory and the event feed, which its refusal
reaches. The coordinator then adopts each step's ranges, adds its timings and
applies its refusal check in the serial order, so the manifest's sections, range
numbers and range bytes are those of a serial flush, and the first error or panic
in that order wins. Families' sections are disjoint; adoption refuses overlapping
writes. Serial preparation flushes in that order on the coordinator. The branch
state, map, coverage and checkpoint are written after the families, on the
coordinator. The commit then flushes each writer's pack once.

A family whose own observation of a frame, or whose flush, fails with
`InvalidData` refuses that branch's recorded data for itself only. The
checkpoint records the family, its first refusing hour and its error text; the
family is neither observed nor flushed again on that branch, and every other
family continues. Families whose observation reads its state refuse with it and
name it: the identity history reaches territory and the event feed, and country
groups reach air superiority. Any other error (I/O, cancellation, a panic) still
stops preparation, and decoding errors are not family refusals. Serial and lane
observation record the same refusals. A refusal survives commit, resume, append
and restart; a resumed branch neither restores nor observes the family again.
Each branch's coverage page carries its refusals. Every prepared query of a
refused family, and every query that reads one (the scoreboard, faction totals,
the event feed and the identity-history readers), answers with exactly the
error its product route answered; the family's earlier pages are not served.
Status fails the family's rows with the branch, hour and reason. One table in
`prepared/pipeline.rs` names each family's status rows, dependents and queries.
The map and the land-owner catalogue are recording-wide, not branch families;
any of their errors still stops preparation.

Published coverage is branch-scoped (and, for a family prepared on its own,
family-scoped) and distinct from recorded missing hours and an unprepared tail. Progress reports accepted block counts, committed coverage hours per branch and active
payload bytes, without inferred rates or timer progress. Clear cancels and waits
for writers to close, removes names immediately, and lets held range readers keep
their existing bytes. Stale or unsupported prepared schemas restart preparation from the recording;
recording readers remain available.

Peace preparation refreshes current native ownership, war, subject, government
and existence operands only with their supplying table changes. Country changes
resolve those retained operands again; snapshot dates share immutable field maps.
Subject end hours remain native clock work. Open-card positions compact on
membership changes, and finalizer country-name proof spans materialise at changed
holders, finalizer reads and publication. Missing-input spans share an accepted
frontier and seal at changes, gaps, publication and reply construction.

Research preparation retains current ordered country operands, the slots whose
owners hold input rows and each slot's input read hours; a read joins its input,
bonus and modifier rows by owner when it is observed. When an hour continues the
previous one with the same links and its input and read-hour tables changed by
exactly their recorded rows (`FrameChanges::rows`), only the slots those rows
name are observed again; otherwise each index refreshes from its whole table when
that table changes. A slot whose carried read its owners' read hours do not all
prove, or whose progress shows at its read hour alone, is observed again the next
hour. Country changes to fields no holder reads keep the holders without
comparing every country. A deadline-ordered current-holder schedule closes carry
exactly at the native stagger deadline. Accepted unchanged hours advance one
frontier; supplying reads, deadlines, gaps and publication materialise the
existing native coverage and slot endpoints.

Stockpile choices keep ordered native ID/classification witnesses and the latest
logistics representative per type, with keyed static group evidence for unresolved
types and required available amounts. Late lexical names move their witness to
its new type, and the previous type elects its next highest ID directly.
Supplying reads combine those facts with only current numeric operands through
the same evaluator used for corrections. Equal native tables advance supplying
dates and diagnostic counts from their current read evidence without repeating
row joins or visiting retained numeric histories. Retired IDs keep their native
absence semantics without repeating their classification or group projection.

Trade preparation retains current exchange operands in recorded order, each with
its row key, and resolves them through the shared country lookup only when their
source operands change. When a known hour continues the previous one with the
same links and holders, and the exchanges changed by exactly their recorded rows
(`FrameChanges::rows`), only the flows those rows name before or after the hour
are summed again, in table order, and published; a change to fields no operand
reads changes nothing. Each active flow's exchanges are kept by row key for this,
under the holders and names of the last whole-table observation. Country changes
to fields no holder reads keep the holders without comparing every country, and a
lexical change naming every resource as before needs no rescan. A changed row
that is or was left out (an unresolved seller or buyer, an unnamed resource)
observes the hour from the whole tables, which count what it leaves out. Sparse
current flows and refusal groups feed their existing shared series. Active trade
and known-hour integrals advance hourly; retired cumulative endpoints materialise
once at publication, including across missing observations.

Resource preparation keeps current native pool and exchange operands at runtime,
by row key and grouped by the slot each row names. When an hour continues the
previous one with the same holders and both operand tables changed by exactly
their recorded rows (`FrameChanges::rows`), only the countries those rows name,
before or after the hour, are observed again, and only their changed pools and
deals publish; a change to fields no operand reads changes nothing. A slot whose
rows name one resource twice is observed again at every pool or name change, as
a whole-table observation would. Country changes to fields no holder reads keep
the holders without comparing every country. Otherwise each projection refreshes
only when its own table changes, and holder changes resolve the retained
operands through the shared country lookup. Resource definitions refresh with
their definition or lexical inputs; a lexical change naming every resource as
before needs no rescan. Unchanged countries keep one accepted frontier,
materialised at their next change or publication.

A resource block's extract runs that observer on the block alone; deal
memberships and their versions are its own recorded calls. Its fold checks that,
after the first hour, the carried state learned the same resource names; that
every run, deal and known span the extract opened in that hour continues one the
carried state observed then; that every country the extract met later was
closed; and that every key of that hour is carried. It then carries the country
scan forward from the extract's end of block. What the first hour continued keeps
its carried first hour and trade position, counts add to the carried counts, and
countries and resources the block never touched stay as carried. An army block's
fold checks that every run, composition and identity segment opened in the first
hour continues an equal carried one, and that tracks met later were closed
before the block. It extends those, appends later ones, adds the unreadable
counts and ors the availability flags; composition versions are its own recorded
calls.

Runtime country joins share the resolved link-slot map through `CountryReads`;
country-key changes refresh holders and partners without copying unchanged links.
Country-group continuation retains merged faction rosters and country/faction
read proof spans for native faction sums. Repeated unchanged hours only extend
their endpoints. Loading rebuilds the open roster keys and drops table views.

Country balance, stability, stability-part, territory, army, supply, air-force and navy S
queries answer beyond committed coverage through one transient selected-table
reader in `prepared::query::progressive`. Wide views sample each
block's keyframe and carry approximate spans separately from display values.
Windows touching at most thirteen unprepared blocks reconstruct every frame;
this is about one year, for the owner's two-second exact-zoom target. Exact left
edges read their preceding observation when needed, so preparation does not add
an identity/value boundary. Sparse state requests select distinct target blocks
and retain only requested hours. Sampled native identity spans also stay
transient. Observers and transient queries share their native per-frame extraction.
Stability parts expand selected countries to their child keys while sampling,
retaining each sampled frame's naming operands only for the reply. Their observer uses the same
production step and numeric projection with its retained supplying-read state.
Carried advisor and country-leader part identities need the character names from
the earlier supplying frame; direct reads leave these parts unknown and the exact
breakdown pending. Formula terms, clamp, remainder and non-character lines remain
available. No tail values or names are written to the store.
Fuel level/capacity, factory parts and nuclear totals/types use the same reader
and their observers' stateless samples. Factory totals retain the native count
formula; factory and nuclear Top choices rank those transient values with the
existing sweep. Territory shares its native holder join and land extraction with
that transient path, resolves map-area inputs once from recorded content, and
ranks sampled values without persisting tail ranks. Regional
breakdown hours are sampled as [dashboard-base](../dashboard-base/README.md#territory-breakdown) describes. Group sums reuse the one
member sum with transient numeric, presence and roster-proof runs. Other S
families retain their prepared coverage gate until they provide a stateless
sampler; fields requiring cross-frame state remain pending.

S replies carry their committed branch coverage. Event reads expose only that
prepared range, without computing or storing tail events. The recording reader
and the shared summary grid remain the only read and geometry mechanisms.

Balances, stability, experience, stability parts, army and supply use the shared
series accumulator for open key state, counts and publication batches, and the
same tag-to-key catalogue reader and writer. Their adapters supply keys to one
keyboard range navigator, including rank-position keys for top selections and
per-key membership spans for country selections. It preserves clipped left-edge
stops and the reached hour when no further change exists.

Every family's catalogue (`{family}-{branch}-catalogue`) is a compressed base
followed by patches of what changed
([prepared series](../dashboard-base/README.md#prepared-series)); the event feed
stamps its decoded catalogues with the identities of every range in their chains.

Observers authenticate accepted hours with `Accumulator::advance` before
observing changed, new or removed keys. Held values extend through the previous
frontier when they change, and `seal` materializes remaining active runs at
publication. `flush` seals before writing; checkpoints serialize after that
flush. Unknown values and gaps close held runs, while explicit spans and sparse
events retain their own bounds. Dense observers retain exact point semantics.
The continuation keeps authenticated frontiers and active membership for resume
and append.

Native read-derived runs use `ReadRuns::advance` on authenticated unchanged
hours to record a scalar frontier. Reads, carries and closes seal their open
keys internally; direct `runs`/`spans` consumers must call `seal` before
publication, checkpoint serialization or building a reply. Sealing visits
only open keys. A gap seals at the prior accepted frontier and closes the
carry. `IdentityTrack::advance` extends its known holder through authenticated
unchanged hours; the next `step` still breaks on an absent/replaced holder or
missing hour. Neither helper invents a read or fills a missing interval.

Country balances share one tag/link extraction with the stateless sampler. The
observer uses the accepted frame fact and shared `CountryKeys` projection to
rejoin changed identities and links. Value-only changes reuse the live holder
projection; removals compare only live holders, while retired actors remain
catalogue entries. Unchanged balances carry through the shared authenticated frontier.
Unresolved and unnamed counts still accrue for every accepted hour, including
carried inputs, without repeating the join. Unknown values and holder changes
keep their exact segment and balance-drop rules.

Territory and regional attribution use the shared frame-change fact to compare
their country and land operands only when those tables changed or were read.
Current state and province contributions refresh independently; unchanged
values advance the shared series frontier. Regional emissions use changed
indices, and open native facts are materialized at changes and publication.
Native owned-state counters share the country-history join, with runtime-only
counter lookups and deferred identity endpoints. Country operand and counter
refreshes reuse the resolved slot-map allocation while links are unchanged;
territory and regional attribution use its shared holder lookups. Diagnostic rates accrue over
accepted hours without visiting retained country histories.
Changed native counter slots select their current holders through that join,
including removals; unchanged holders do not enter the counter emission pass.

Occupation binds fixed law, equipment and define outcomes at configuration
through the shared content cache. Held occupation values and means advance
without traversing closed tracks. Supplying daily reads retain their exact
read dates and run boundaries; law eligibility uses that read's recorded world.
The land-owner catalogue uses keyed witnesses and a runtime ownership join,
rebuilt after restoration, so a changed country table does not revisit
unchanged state rows or scan earlier aliases.
Balance-only country updates stop after their operand comparison. Link refreshes
resolve the retained ownership operands without reading unchanged state rows.

Prepared political and command power queries use one accepted-frame observer for
the raw ledger, spending, balance details and government causes. It calls the
production accounting and presentation builders. Publication-hour series hold
applications, payments, changed item facts, holds and coverage reasons. A second
series section closes native days in order, holding recurring charges, supplying
lags, daily figures and native government events. The newest native day remains
a numeric tail; attribution behind the closed frontier is a sparse dated
publication event. Native totals stop at the last closed cell before merging the
tail. Queries gate every fact by its supplying hour.

Spending queries fold recurring runs over the selected portions of game months
and return the country's complete compact row list. Per-item cumulative payment
endpoints serve spending breakdowns; income and cause charts read width summaries
and exact reached days through the shared reader and navigator. Government marks
use native event clusters and exact event pages; complete lists retain supplying
read order. Government lag runs keep each native event hour as a keyboard stop,
including adjacent hours with equal lags. Exact viewports retain the
production census/cause suppression join; dense views may retain a census cluster
whose replacement cannot be proven from its summary. Continuation keeps bounded
open accounting operands in the replay checkpoint codec, counters and the current
day, and takes its previous frame from the restored replay accumulator. Between
reads the observer holds no view of the previous read beyond its country-naming,
civil-war and lexical tables and the power, government and daily tables it joins,
so the decoder changes every other table in place; a payment or cost dated at the
previous read is described from that read decoded again from the recording.

The ledger binds successful and refused content outcomes before observing a
branch. Fixed plain text resolves once per content key; recorded names and
scoped text still use their supplying frame. Runtime native joins hold the
decoded event tables rather than copies. When an hour continues the previous one,
every table they read changed by exactly its recorded rows
(`FrameChanges::rows`) and the census keys, links, dictionary and daily
publications are unchanged, only the countries those rows name are joined
again; a capture that only extends its interval to the hour joins nothing and
moves its expiry to the next hour. Rows not yet recorded as events, and daily
gains at their hour and the hour after, are observed again, as a whole-table join
would. Such an hour publishes again the event keys of only the countries whose
power or government rows changed, and the newest native day those rows date is
kept as a count of rows per day. Each country's command power holders are indexed
by the unit leader and character rows their names were read from at its last
snapshot; a changed row observes again only the countries it names, and a
changed dictionary, or a table whose changed rows are not known, every country
holding a named allocator. Otherwise the joins refresh from the whole tables
read, and the shared
country-key projection skips identity joins on balance-only updates. Only
affected countries run accounting and presentation. Unchanged frames advance authenticated scalar frontiers;
open coverage, holds and exact diagnostic rates accrue in bulk before relevant
changes, gaps and publication. Daily amounts expire at their native day boundary.
Census membership changes and accepted-frame runs preserve absent, refused and
missing intervals without carrying holds across recording gaps.
Recording-wide ledger scans are compiled only as test oracles; production
queries read the prepared series and compact native-day tail.

## Prepared party history

The party topic's production observer prepares every tag together. Popularity
uses shared series runs per tag and ideology. Ruling, naming, leader and election
changes use shared events; categorical runs reference those event ordinals.
Identity and accepted-hour unavailability runs preserve gaps. The resumable
`party-history-{branch}` range keeps open runs, last-election state and undated
name/person catalogues; dated history lives in `party-series-{branch}` batches.
Naming, portrait selection and replies use the production product code.
The shared frame fact refreshes current native joins only for changed source
tables. Unchanged hours advance one frontier; changed holders and publication
seal their open intervals. Name/person catalogues are built at publication.
Proven ruling changes also append to key `0` in `party-ruling-{branch}` shared
event batches. Each row carries its tag, ideology token and recorded naming
operands, including refusals; the common event reader pages the global stream.
Each supplying hour appends in stable tag order for direct one-row cursor pages.

Prepared `party-history` requests select state, width summary, event page or next
change. The completed branch catalogue includes ideologies and naming refusals
from every recorded hour. Ruling party charts and Stability markers share the
same events. Exact windows use the legacy signed stack; dense windows use the
shared extrema sampler. Membership-aware clusters retain exact event counts and
native ordinals. Reached pages keep the native-government suppression rule;
dense census counts remain independent of native causes. The per-tag loader and
selected-country scan compile only in tests as the parity reference. The
branch-wide scan remains available to the event-feed loader. Parity models are
"Party history values" and "Stability party markers".

Popularity extraction is shared by the observer and progressive queries and reads
only Countries, CountryLinks and PoliticalParties. Unprepared windows use the
shared keyframe tail; windows covering at most thirteen blocks read every frame
exactly. Recorded ideology tokens are discovered during that read without a
prepared catalogue. The shared series reader merges the prepared prefix and tail
and carries approximate spans into the chart hook. Reached popularity reads use
their target blocks. Duplicate-token diagnostics preserve the existing overlap
refusal, including when the duplicate is only in the unpublished tail. Mounted
summaries and states publish immediately and refresh on committed coverage changes.
Ruling, naming, leader and election facts remain prepared events; reached pages
show the shared prepared-range note. Full parity history waits for complete coverage.

## Prepared idea history

Idea history is prepared on its own (registry entry in
[prepared/pipeline/independent/quiet_politics.rs](src/prepared/pipeline/independent/quiet_politics.rs));
[dashboard-base](../dashboard-base/README.md#idea-history) describes its extract,
pages and queries.

## Prepared focus pages

The focus order is prepared on its own (registry entry in
`prepared/pipeline/independent/focus.rs`, family `focus`; its extract, fold and
queries are in `prepared_focus.rs` and `prepared_focus/`). Its blocks are read by
the fields its observation reads: Countries' tag and canonical slot,
CountryLinks' canonical slot, Focus's active and continuous focus, progress,
pause and completed count (not the tree, which no reply reads), and
CompletedFocuses' tokens. Country balances move every hour and are not read.
Focus progress moves at each country's daily read, so about one hour in ten is
read, and the block's first hour. A pre-pass worker builds each block's facts
alone: each country's stretches of consecutive observation under one identity,
its order at the stretch's first hour and at each change, and the last run of
equal progress begun in the stretch. A country's completed focuses are its own
tokens, shared while unchanged, and a change keeps only the tokens it adds, so a
block's facts hold each completion once. The family's thread folds them in block
order on the carried state in time proportional to the countries and order
changes, not to the hours. The hourly walk builds the same facts from every hour
and folds them at each block's last hour, so both publish the same pages.

A country is observed at an hour when all four tables are complete, its tag is
nonempty UTF-8, its links agree on its slot, the slot has a focus row and the
row's completed count matches whole, ordered completed rows. A tag several
countries hold is the one with the highest country id. Its order is its
completed focuses, active and continuous focus and pause, compared by their
recorded bytes (kept shared while they repeat): two tokens that are not UTF-8
are different focuses even where replies show them by the same replacement
text. Each completed focus is dated by the
first hour it was listed (an upper bound), when the country's earlier list under
the same identity is a prefix of the new one; a shrink, a reordering or another
identity starts an undated list. An order's run, its first hour
(`orderFirstHour`), continues while the country is observed at consecutive
recorded hours with the same identity and order.

Pages use the shared series/event batches with a tag-to-key catalogue. Key 0
holds the tables' named refusals (exact supplying-read reasons and truncation
counts) and their intervals. Each tag's key holds its observed runs and its
order events (identity, `orderFirstHour`, active focus, pause and its completed
focuses as a run of its completion chain); the next key holds the chain's rows,
each completed focus with its date written once; the one after holds the
progress the tag carries into each block it is observed in. Runs and events
never break where a commit falls: open runs are carried in the saved state and
read across batches, so every reply is the same however batches end.

`focus-hour` answers from the family's own coverage: the order holding at the
hour, bounded by its first and last hour (`hour`, `lastHour`), and the
progress and daily gain at the hour asked (`at`). Progress is read back from
that hour's block alone, from its first hour through the hour, on the progress
the block starts from (one block read). The daily gain is the rise since the
observation the hour before the progress run began, while the same focus stays
active. `focus-order-page` reads a page of the order's completion chain, checking
the page descriptor `focus-hour` gave; `focus-unavailable` names the tables'
refusals at the hour.

Beyond the focus prefix, `focus-hour` reads the recorded order and progress at
the requested hour. `orderFirstHour` is null and `preparing` names that field and
`completedByHour`; completion pages carry `sampled: true` and retain every
recorded token and ordinal, without dates. A cadence-bounded window supplies the
previous daily gain of the same focus when it can prove it; otherwise the gain
stays null and pending. These hours share the source's decoded frame cache.
The widget shows "Still being prepared" in date slots, disables their Jump,
and re-asks as the focus gate grows. Dated artwork uses the shown hour until
preparation supplies its completion or order date.

Titles, icons and descriptions are not prepared. `focus-art` evaluates a page of
distinct tokens (in ascending order) for a country at one recorded hour, as the
focus tree shows them (`display`: titles and icons, `text`: titles and
descriptions), with the production interpreter over the recorded content stack.
Results that read no recorded state (no scripted text, no trigger-chosen icon)
are kept per content binding, within 64 MiB (icons included; the least recently
used is evaluated again), and read no recording; a token that reads recorded
state reads that hour's frame of the focus, scripted-text and Clock tables
(shared with `focus-artwork`'s frames). Rows name their own refusals; a failed
content binding or evaluation is the reply's named failure with the count of
tokens it left without art. `focus-static-text` returns each token's static
title and description from the content.

The focus completions the event feed reads are the order's newly dated entries
at hours the country was observed the hour before. Completions first seen after
the country was not observed, resets and countries without an observation are
counted by reason (country-hours for the latter) in the completions' catalogue.
Incompatible saved focus schemas refuse only these derived pages and state;
recording-only products remain available.

## Prepared country groups

War starts and capitulations keep their native dates, which later observations can
add anywhere in the prefix. Their event inventories carry `eventsApproximate`
while preparation runs; read-dated identity changes retain their existing coverage.
Shared event badges visibly mark that distinction. Wars timelines likewise mark
unfinished capitulation history approximate.

Country groups use the product's incremental scan as the pipeline observer. One
compressed JSON state range per branch is replaced with each committed checkpoint.
It keeps membership, identity, faction-name and event histories plus their
continuation state. Equal name operands share a run across repeated reads;
lexical dictionary versions store only changed and deleted token rows. Derived
roster projections are rebuilt from new frames, and appended preparation observes
only new frames. Prepared queries read this range and use
the product's reply builder, resolving names through the recorded
content stack at query time. Widgets and faction colours share the same cached
prepared reply and pending-coverage polling.

The range is the family's continuation, saved with each commit of its
coverage, so before completion the reply is the prepared prefix's, carried on
by the keyframe of each later block (`Scan::sample_tail`, through the shared
keyframe sampler): each keyframe is observed as an hour and stands for its
block's hours as unchanged hours would, up to the next keyframe or a recorded
gap. Faction ids are recorded, so lanes continue across samples; the leader is
the member at order zero, a value of the frame. Those hours are `approximate`
at the reply's root; a block whose keyframe is not sampled stays `pending`, and
hours after the prefix are unobserved only inside recorded gaps. War starts,
capitulations and identity changes are dated diffs that need every hour: they
stay the prefix's. Widgets that follow the groups republish each such reply;
the campaign's faction colours and founders wait for the complete history.

Groups consume the shared frame fact and keep current faction naming/member
operands as runtime projections. Holder, human, faction, war and capitulation
steps update independently. `LinkTags::update_projected` shares the native tag
join with current country/link operands, so changing one source table leaves
unchanged native tables untouched. Dictionary changes or supplying reads revisit
only current templated faction names, retaining their native name-revision
boundaries. The observer driver counts this dictionary-driven interpretation
separately. Equal hours advance scalar frontiers and diagnostic
rates; open endpoints materialise at their relevant changes, gaps, checkpoints
and replies. The event publisher preserves open membership in other steps.

## Prepared aces

Aces reuse the roster observer and window builder. Every recording block has one
compressed roster reply, with its exact supplying dates, text dictionary and death
bounds. Published continuation retains each ace's identity and current run; raw
roster references restore from the shared decoder checkpoint. Interleaved branches
restore their previous roster with one selected-hour read during preparation.
Variant naming changes and successful roster reads append through shared series
events. Manifest replies use the original naming and manifest builders.

Portrait selections retain only country, name, surname, gender, pool index and
recorded pool operands, merging equal adjacent observations per ace. Missing reads
and gaps break these intervals. Portrait requests look up the requested ace and
hour in this prepared state and use the same cached portrait ladder as roster
artwork. A `view: roster` request with an hour uses these pages when reached;
beyond the prefix it uses the latest complete roster read within the daily
cadence, with firstSeen null and first-sighting/death dates preparing. Lazy
portraits use that supplying read too. The UI loads the roster and portraits
through the shared product and dated-fact queues and uses name order while dates
are pending.

Aces are prepared on their own (`products/air_aces/prepared/block.rs`; registry
entry in `prepared/pipeline/independent/air.rs`), gating `air-aces` and
`air-ace-portraits`. Their blocks are read by the fields the roster and the
variant catalogue read: every field of the roster and every roster read (daily);
the countries' keys, slots and tags; every link; the wings' names (their planes,
missions and eligibility move every hour and are not read); the wings'
equipment variants (not amounts); the active raids' wing, phase and unit and
their tooltip operands (legacy wing names); the original tags; the dictionary;
and every field of the variant tables, with every variant read (daily). The
extract keeps each read hour's frame reduced to what those reads use (countries
to their keys, wings to id and name, equipment to its variants, raids to their
wing, phase and unit, original tags alone), each reduced table shared by kept
hours where it is equal ([`raids::kept`](../dashboard-raids/src/products/raids/kept.rs)). The
fold observes each kept hour on the carried aces as hourly preparation does,
and each quiet hour on the last kept frame with that hour's read evidence and
the tables hourly preparation marks changed there: the roster is read again
at a read only when one of its tables was decoded again, so an ace that
entered between reads joins at the read hourly preparation reads it. A slot
hour whose roster read was timed on another table is quiet and still read by
its evidence. At a block's end the window is built as hourly preparation
builds it and held; each commit writes the held windows first (`publish`),
also when the aces refused the branch later in the batch, so every commit
boundary publishes the window pages hourly preparation does.

## Prepared raids

Raid episodes, tallies, coverage and read dates use the product's frame
observer and append through shared series events and runs
(`products/raids/prepared.rs`); the continuation keeps open episodes and
tracks. Raids are prepared on their own (`products/raids/prepared/block.rs`;
registry entry in `prepared/pipeline/independent/air.rs`), gating `raids`; the
tally view still waits for the identity history. Their blocks are read by the
fields the raid scan reads: the countries' keys, slots and tags; every link; the
raid countries' countries and eligible types (not their used types); each
raid's parent, phase, outcome, flight distance and preparation hours (the last
two move every hour a raid is live); every target field; the dictionary; the
provinces' states and controllers; the states' owners, controllers and stored
names; the original tags; and the reserved country's naming, which names a
target state owned by country zero. Hours with no live raid that moves are
quiet, however balances, used types or province gates move. The extract keeps
each changed hour's frame reduced to what the scan reads (countries to their
keys, raid countries to slot, country and eligible types, original tags alone,
provinces and states to the rows the targets name), each shared by kept hours
where it is equal. The fold observes each kept hour on the carried scan, with
the raid content hourly preparation binds, and each quiet hour on the last kept
frame with its read evidence and no table changed.

## Prepared military air ledger

Aircraft loss bookkeeping and applied bombing damage use the product's existing
frame observer. Preparation appends exact signed deltas through the shared series
event writer, keyed by endpoint country. Each event keeps its native application
hour independently of the supplying read that publishes it. Unknown endpoints,
unsupported callers and signed corrections retain their original meaning.
Event identities must increase from read to read; a repeated or backwards
identity refuses the family's branch. After an unread event interval the
collector may restart its identities: the first read with events after it starts
a new identity epoch when its identities restart and every event in it is dated
within or after that interval, since those events were never read before.

The replaced continuation range contains only identity and coverage spans,
warning counters, event ordinals and the series continuation. Country and leaf
catalogues preserve complete choices without retaining dated deltas. Availability
reads the saved family flag. Prepared replies gather event pages and use the
product's reply builder, including native-date range selection and ranking.
Queries mark that committed prefix with coverage and pending hours. A cold reply
has an empty coverage extent ending before the first recorded hour and no
observed intervals; the three widgets keep totals at
unreached hours pending.
Equal native source reads reuse the current country join. One authenticated
frontier carries country-presence endpoints until an identity/read boundary or
publication; balance-only country changes compare the shared country-key
projection before rebuilding the join. Event ordinals and per-event refusals
retain the same supplying-read checks.

The ledger is prepared on its own (`products/military_air_ledger/prepared/block.rs`;
registry entry in `prepared/pipeline/independent/air.rs`), gating
`military-air-ledger`. Its blocks are read by the fields its scans read: the
countries' keys, slots and tags (not balances), every link, every field of both
event tables (an equal table decoded again is a repeated ordinal) and the
dictionary. A pre-pass worker runs the ledger over the block from its first
hour with no earlier state and records its series calls after that hour; each
quiet span (event tables unchanged, so empty or a repeated ordinal) extends
presence and coverage and counts the per-hour warnings at once, and while an
identity read is refused every following hour is the extract's deadline, read
again. The fold observes the block's first hour on the carried ledger, makes
the recorded calls again under the carried keys and joins the block's spans
and counts after that hour. After the first hour the carried ledger differs
from the extract's only in the event ordinal and unread interval carried into
the block: the extract follows their first use, so the fold checks the block's
first read with events (a new epoch or its first ordinal) against the carried
state. When the carried ordinal would refuse that read, or an ordinal is
checked outside a read, the fold observes the rest of the block hour by hour.
Wartime hours mostly carry events, so the per-event work runs on workers.

## Prepared construction

Construction's immediate prefix is independent of its newest-window request
(`construction-log-manifest` with `newest: true`). A query-time look-back in
`dashboard-construction` proves all selected project starts and output chains,
extending by their recorded dates; its numeric window is cached across re-asks.
Windows carry a local `types` dictionary and `exactFrom`. Only building identities
whose first spelling the forward catalogue resolved can be published this way;
unproven windows leave the prefix shown. Prefix rows and spending work intervals
are marked approximate until the family completes.

Construction observes each accepted frame once for every country. Project starts,
queue progress, level claims, endings and output publications append as shared
series events. Queue coverage keeps held run endpoints; replay-block membership
uses project-key events. Window queries load only their project keys and those
projects' later facts, then call the construction window builder. Appends add
facts without replacing earlier window pages. Resume restores previous building
views from the shared decoder checkpoint.
Shared frame-change facts gate the state/province projections. The native level
diff supplies only changed watched keys; carried queue runs materialise at their
next read, gap or publication. Spending projection visits only changed projects.
Publication compacts closed native slots and their parallel continuation fields;
the existing catalogue keeps their stable keys and output retains known closed
handles for attribution. Projects awaiting final output remain open continuation.

Construction is prepared on its own (`prepared/pipeline/independent/economy.rs`,
with the stockpile, production, experience and fuel). Its blocks are read by the
fields the scan reads: building levels, the countries' and provinces' keys (read
at the queue reads alone), the links, every field of the daily queues and output
and every rebuild of the dictionary; each queue read hour is visited, and the
next one by the clock is the extract's deadline. The family's thread observes
each kept hour as hourly preparation does and advances each quiet span in
constant time (`Prepared::advance_quiet`: the series' accepted hours and the
scan's last hour; an incomplete building table loses each such hour for the
queues' stretches). A queue read dated off the clock (another table of its slot
read at another hour) leaves the rest of the block to hourly observation. Its
block end and flush take the branch's replay blocks and the source's naming
handles (`NameSource`, held from its first block), so names come from the same
cache at the same flush. A continuation loaded from a commit takes its building
tables and dictionary from the last frame of the block before, read alone, as
hourly preparation's stream supplies them. It saves its continuation as
`construction-continuation-<branch>`; the log, its manifest and spending answer
from its own coverage. Before it completes, the manifest is the one the prepared
prefix published (its building types and warnings so far) with the later hours
pending, and a window the prefix reached answers from its pages, its projects'
later history pending; a later window waits.

Construction spending uses the same observer and leaf grouping. Exact signed
leaf totals use shared series; project intervals, unknown stretches and content
presentation stay in the leaf catalogue. Sunbursts read only their requested
start, reached and final totals. Before construction's coverage completes,
spending answers its prepared prefix from the catalogue that prefix committed:
a requested hour after the prefix reads the total through the prefix's last
hour, and every leaf gains an unknown stretch from the first pending hour with
the reason `preparing`.
Leaf continuation updates keyed project intervals and missing-output stretches
in place. Their existing ordered reply vectors are built only at publication,
through the same leaf grouping and reply builder as the recording product.
Subjects use their product's relation/level observer and reply builder, prepared
on their own (`products/subjects/prepared.rs`). Their blocks are read by the
relations' keys, starts, ends and cancel bytes, the subjects' autonomy states and
the country keys and links; a relation's recorded end is the extract's deadline,
observed on the last hour's rows. One compressed state range per branch retains
changes, coverage spans and continuation state, and `subject-state-{branch}` the
change publisher's continuation. The frame fact refreshes only changed holder,
relation and level operands. Equal hours, and each span of hours between those
read, advance a frontier and exact diagnostic rates (as that many hourly
additions would, an overflow included); open relation and level endpoints are
sealed at changes, native ends, gaps, publication and reads. Prepared queries
read the family's own coverage and resolve level names and order through the
recorded content stack. Before it is complete they answer the saved scan, the
prefix, and each later block's keyframe as the scan observes an hour alone
(`Scan::sample_tail`): a relation reaching the prefix's end, or a sampled block
before, goes on while the next keyframe holds it with the same start, runs break
at recorded gaps, which are unobserved, and a keyframe the scan would leave
unobserved samples nothing, so its block stays pending.

Autonomy reuses its hourly subject observer and level-name reply builder. Exact
scores, thresholds, ratios and plotting progress append to shared series batches;
overlord/level/segment changes are sparse events. Its checkpoint retains only
the current native samples and open band starts, catalogue, coverage spans and
series counters. Reached amounts share one batch lookup per hour; canonical
bounds use open key state or one requested run window per key. Point assembly
performs no file reads. Categorical events are read only inside requested windows,
and keyboard navigation uses the shared series navigator. Branch-wide overlord selection uses a compact catalogue of
subjects each overlord has held. Replies contain exact reached runs and
screen-width summaries for selected holder/membership pieces. Visible lanes ask
at their actual eased viewport; hourly views draw the original curves, and
keyboard navigation reads numeric and categorical changes through the shared
span-aware reader.

Autonomy preparation retains runtime current holder/relation joins independently
of balances and scores. Unchanged hours advance the shared accumulator and coverage
frontier; changed samples preserve supplying and categorical boundaries with the
shared boundary observer. Publication seals the current continuation samples.

World tension uses its original read/contribution/threat observer and reply
builder. Shared batches retain world amounts, uncapped sums, supplying-read
hours and recorded/share contribution runs. Equal amounts join while read-hour
runs keep every supplying read reachable. The shared catalogue retains the latest
read, current amounts, first-contribution dates, sparse threat changes,
coverage spans and diagnostic counters. Generic queries use the prepared country
identities and return exact reached values and width summaries. Known zero
contributions and unknown spans remain distinct. Overview and breakdown use
their original hourly drawing and the shared signed-stack renderer for dense
samples on a common screen grid.

Tension preparation refreshes identity, aggregate and source operands independently.
Unchanged contributions and closed threat history are not revisited. Exact
supplying hours remain observable while diagnostic rates and open threat-read
endpoints advance through authenticated hours and seal before changes or publication.

Wars prepare one global stream of changed topology, native naming inputs and
capitulation evidence. Changed tables use the replay's existing prepared
checkpoint encoding inside shared events; unchanged tables carry forward.
Four exact casualty series per bilateral relation lifetime retain signed
counters independently of viewpoint. Queries replay the product's same native
grouping and app continuity walk over changes, then read only requested reached
casualty hours. The widget keeps structural lanes loaded while displayed and
hovered counters load. No viewpoint is multiplied into preparation state.

Country groups, subjects, autonomy, world tension, wars and event feed read
prepared data. Their recording loaders are test-only parity oracles. Autonomy,
world-tension and feed-only private branch walks are also test-only; shared
native extraction, per-frame steps and reply builders remain in production.
The shared subject recording scan also supplies the feed's recording reference.
Recording faction-totals, air-superiority and casualty-breakdown consumers use
the shared group and viewpoint-war references. Country balances retain a
recording route for pinned and smart-group views and the acceptance binary's
reference comparison.

## Prepared event feed

Groups, subjects, ideas, focus and special-project completions, and nuclear
stockpile increases publish their own witnessed changes as shared events
alongside their existing resumable state. One feed observer
publishes native actions, fresh-owner guarantees, existence transitions and
nuclear impacts. Exact reference/slot/holder/instance witnesses and fresh-read
spans live in the shared country-history observer.

The feed observer is prepared on its own (`prepared/pipeline/independent/event_feed.rs`,
registry name `event_feed`), with its own read of the identity history: at
every hour it observes the history, then the feed naming countries through
it, as hourly preparation does. That history is saved as
`event-feed-identities-<branch>`, never published as the identities'; a
refusal of it refuses the feed from the identities. Every hour reads an event
pack of its own, so blocks are read whole: pre-pass workers decode them and
keep each hour's frame reduced to what the feed and the history read (country
keys, owned-state counters' country and count, experience gains' country and
instance, uncancelled guarantees and that hour's relation slice reads; links,
packs and strikes as read), and the family's thread observes every hour in
order. Its continuation is `event_feed-continuation-<branch>`. The query waits
for the coverage of every family whose pages it reads (the feed's facts, the
groups, subjects, ideas, focus, stockpile and special-project changes).

The `event-choices` and `decisions` queries
([event_feed/prepared/script.rs](src/products/event_feed/prepared/script.rs))
read the feed's own facts alone and gate on that family only. Each native
fact keeps its typed operands beside its row, and the observer joins
continuing facts to their opener as it publishes, in native order within a
read. A choice continues the dispatch of the same world, native event,
instance and receiving country (the dispatch's target, the choosing command's
actor). A decision edge continues the open selection or activation of the
same world, country, decision and target. A fact it cannot join is its own
card. A decision execution keeps the power its writer attributed to it: the
mutation must match the execution's hour, country id and instance, token and
action, the country's political and command power capture must cover the hour,
and no other execution of the read may share the key. Otherwise the charge
stays unproven. The family therefore also keeps the hour's power mutations
and capture coverage. Pages step back through the merged stream of the
selected kinds by their supplying read, bound to the Live read hour.
`civil-wars` joins the `politics_state` family's rival lists with the feed's
annexations ([dashboard-politics](../dashboard-politics/README.md)).

The feed query merges these source ranges directly into backward native-date
pages. Requests read their accepted Live prefix and remain pending beyond it.
The cursor retains the Live read bound and the complete same-date tie.
Only delayed facts enter each source's `LateIndex`: sorted native/read/key/ordinal
references to existing event rows. Ordinary facts stay in their source stream;
no composed feed is stored. Catalogues cache in the opened source and invalidate
on store-key or published file-stamp changes. Existing content and row builders
supply names, artwork, refusal reasons and ordering. Non-native same-date ties
retain source insertion order; native occurrence IDs retain their own order.
Project candidates validate together against their episode references at Live.
Obsolete candidates still advance the raw cursor, so an empty page can lead to
older live rows. Nuclear availability and recorded type tokens come from the
increase stream's catalogue, including campaigns without increase rows.
Focus completions are plain events (tag, focus and hour), dated only where the
country was observed the hour before. The feed's Focus completions rows count,
by reason, completions first seen after the country was not observed (after a
gap, unread tables or absence), which have no event date; reset or reordered
lists; recorded hours with a focus table incomplete; and, per country and hour,
countries without an observation (unresolved identity, no focus row, a tag that
is empty or not UTF-8, an incomplete completed order). The page's completions
get their art at query time, each at its own hour, as an event shows a focus
(the definition's icon and full text), reading a frame only for a focus whose
art reads recorded state.

## Prepared sinkings

The retained sinking tracker observes each accepted frame once. Continuation
keeps country identity changes, unresolved attribution operands, counters and
the last successful multiset signatures and signed convoy endpoints. Resume
rebuilds table views from the pipeline's accepted decoder frame; the successful
signatures also preserve multiplicity when that frame's sinking table is unavailable.
Closed rows and dated reads leave continuation after publication. Only new
occurrences and unresolved occurrences whose country references changed pass
through attribution again. Country identities extend lazily until a relevant
reference or publication needs their endpoint.
The coordinator's shared frame facts keep row changes separate from fresh read
evidence. Equal convoy acquisitions advance supplying-read proof without visiting
the buckets again. Fleet-name projections rebuild only when the shared facts
mark their combat, detail, task-force or fleet rows changed; gaps and restore
force a fresh join. Equal active-battle inputs extend their accepted coverage
and Length without repeating participant/country joins; changed inputs or country
bindings use the native census step.

Named sinking rows are append-only shared events. The existing naming and reply
builders also produce native-date replay-window replies through `write_reply`.
Late observations and identity bridges replace only affected window ranges;
each row still carries its first supplying read and stable scan index. A small
manifest supplies class choices and naming warnings. Battle titles use the
existing geography, place-name ladder and title builder at the supplying frame,
stored as immutable indexed events and requested only for drawn rows.
The run's configuration region resolves the two NNavy retention outcomes,
including refusals, and fixed ship-class and strategic-region operands through
`CachedResult` for their recorded content binding. Branch restore reuses those
operands. Retention
updates visit only new or extended trailing breaks; naming uses the same builder
with these configured operands and only newly supplied rows.

Naval losses use the original tally collector and reply projection. Each changed
occurrence replaces only its own unresolved contribution and affects only its
holders. Resolved continuation folds into holder counters; shared series store values, completeness,
uncertainty and rank positions from the existing ranking sweep. The shared
accumulator advances once per accepted frame, receives dirty/new/removed holder
keys, and seals active values during flush. A restored observer, which does not
save its live holders, continues from the holders with an open value. Rank inputs extend at publication;
identity, month and completeness changes
update the current holders. Retained battle-result endpoints also extend lazily,
without revisiting every historical result on each unchanged read. A later identity
bridge appends holder corrections carrying the original supplying date and
calendar month. Exact values add applicable signed corrections without rewriting
earlier value batches. Uniform corrected buckets shift the shared summaries;
buckets crossing correction starts, uncertainty or sign changes rebuild from
their bounded exact runs. Dense Top-N lines use the min-rank bucket mask and
retain corrected candidates; reached values and hourly windows rerank exact
corrected candidate runs with the original sweep. The ships/convoys breakdown
samples exact component values at bucket endpoints and extrema.

Queries read requested windows, summaries and sparse holder corrections. The UI
holds playback, visible geometry and reached windows through the shared prepared
query and window hooks, with country-budget paging and dated holder bindings.
Keyboard changes use shared series navigation plus sparse correction starts.
These tallies depend on earlier retained observations and require prepared
coverage rather than a stateless keyframe sample.
The retained tables are neither lifetime counters nor counters since the first
recorded read. Ship occurrences need the multiset tracker across pruning and
duplicate entries; convoy amounts need the previous signed bucket endpoints.
Unknown holders can resolve later through identity bridges and corrections at
the original supplying date. Naval-loss exact reads, summaries and navigation
therefore retain their coverage gate, with no progressive sampler; unprepared
ranges answer pending and the widget uses the shared loading state.

## Prepared peace outcomes

Peace preparation uses the native conference and outcome frame observer, prepared
on its own (`products/peace_deals/prepared/family.rs`). Its blocks are read by
the fields the observer reads: country tags and links, state owners, province
states and victory points, war, subject, government and existence operands, the
conference roster's keys, turns and finalization, every finalizer row and the
dictionary. Kept frames hold countries, states, provinces and country counters
reduced to those fields. Two hours change the observer with no field changing:
a subject relation's recorded end, and the next scheduled roster read while the
roster holds a conference; each is a deadline, observed on the last hour's rows.
The hours between take the observer's step at once: its unobserved and
finalizer frontiers and its snapshot's hour advance, and the slot reads of an
unchanged empty roster mark the roster read. A held conference read at an
unscheduled hour, a pending comparison or a relation ending inside such a span
has the block observed hour by hour.
Finished derived deals and projected finalizer observations append to shared
series events under one branch key. Its checkpoint holds only open comparisons,
unknown intervals, identity mappings and deduplication facts; finished outcomes
leave the continuation, which is saved as `peace-state-{branch}`. Open deal
ordinals preserve the reference's ordering when finished events and retained
deals are joined after resume or append.

Queries gather the branch events and call the product's finalizer join, transfer
graph builder, action naming and complete-reply budget check. Finalizer operands
are already projected into the product's outcome types, and retained native names
survive serialization independently of the JSON reply. Peace deals and Territory
transfers use the shared pending-coverage query path and expose the "Peace deal
values" and "Territory transfer values" parity models.

Peace outcomes have no stateless progressive sampler. Conference closure,
ownership transfers and war-end evidence compare native holders and facts across
reads; delayed finalizers also join earlier observations. A single keyframe
cannot supply those dated outcomes. Event lists retain the prepared-range status;
they refuse from the family's own coverage page, and their coverage gate is the
branch's. Before that coverage is complete the event page answers the prepared
prefix (`query_through`): a conference open, or a settlement unread, at its
last hour has its `outcome` in `preparing` instead of the limitation that its
outcome was not recorded, and the hours after that hour are not unobserved.

## Prepared special projects

A request can name `hour` for Current. Before its history reaches that hour,
`current` holds running, paused and unknown-state cards from the hour's frame.
The frame is observed by `Prepared`, then its current observation passes through
`apply_interval` and the same episode/presentation reply builders as prepared
pages. Only in-progress cards enter this reply; episode starts remain observation
bounds and their tooltip and Jump action stay pending. Completed and cancelled
lists come from the prepared prefix with `pending` hours. A cold reply has an
empty history and the current frame, even before a coverage page exists.

Special-project preparation projects each accepted frame for all countries through
shared project facts, naming and scientist attribution. Scoped names evaluate the
full accepted frame in that same pass; a country's scoped name is reused,
without evaluating it, while every input its last evaluation read reads the
same (dashboard-text's evaluation reuse: the hour's change facts confirm an
input whose declared reads did not change without reading it), and check mode
evaluates it anyway. An hour that changes nothing but scoped names revisits
only the countries one of whose names changed since the hour before (another
name or refusal, or none observed then). Changed domain observations and native
identity boundaries append as shared events; progress and supplying reads use
runs. An observation event names each project's presentation and person (name,
icon, portrait) by a key whose only event publishes that value once, so a changed
project republishes its country's facts rather than every project's art; the
query reads each named value once. The supplying-read marker is stored once per
branch. Continuation retains the published values' text hashes and
key mappings, current observation hashes, open naming admission and warning
counts, without replay rows, finished episodes or presentation history.

Fixed project content binds once before observation, including named failures.
Current token/program/scientist dependencies refresh only countries whose resolved
operands change; those joins rebuild on resume. Current project/history and
holder projections refresh only on their source table's frame facts, and
history-only changes select their affected countries. One branch lexical-revision run
preserves the original warning-reset boundaries without republishing every country.
Unchanged hours advance the shared series frontier and current scoped names still
evaluate the admitted hour. Closed progress keys are visited at publication only.

The prepared query reads the selected country's change intervals and its original
roots' small identity keys, filters native holders before invoking the shared
episode step, and extends constant intervals without hourly episode expansion.
It keeps first-use presentation/person IDs, completion bounds and dated scientist
attribution. The existing complete Current and History reply contract includes
read hours derived from branch markers intersected with selected availability.
The country branch projector, dictionary-span visitor and separate scoped-name
recording pass compile only for oracle tests. The prepared query uses the shared
episode and reply builders; completion presentation and its native-hour reads
remain shared with the global completion stream. Before the family's coverage
is complete the query answers the prepared prefix, its pages being committed with
that coverage: episodes and runs end at its last hour, and the reply carries the
prefix's `coverage` and `pending`.
The widget exposes the **Special project values** parity model.

Project completions also append to one global `special-project-completions-{branch}`
events key (`0`, catalogue label `completions`). Each change keeps its stable
episode ID, native holder, project/program operands, native date, supplying hour,
presentation and exact refusals. A later exact date appends another change;
missing native hours or identities append a refused result. The catalogue holds
warnings, content availability and the shared late-fact references. The same
section stores an episode's current completion ordinal as runs under key `id + 1`;
key `0` remains the events stream. These references continue through accepted Live
hours and end when a completion is withdrawn. A feed page validates each candidate
with `Reader::value_at` (or `values_at` for the page), keeping only the matching
ordinal at Live, without reading later events. Within each supplying hour, visible
completion rows append in resolved native-tag order, preserving recognition order
for equal tags. Withdrawals still append and advance event cursors.

Refusal counters share that section under keys starting at `1 << 63`; episode
keys occupy the lower range. The catalogue's `refusal_keys` maps exact oracle
reason strings to their counter keys. One `Reader::values_at` at Live returns
branch totals, including diagnostics of withdrawn or removed completions. Zero
or absent counts omit that reason. Preparation shares the oracle's input-span
boundaries and refusal formatting, retaining cumulative scan/closed warnings,
deduplicated closed text refusals and current open completion contributions.

Preparation recognises these changes through the same episode step. Only new
changes read native-hour presentation inputs: required hours are grouped by replay
block, each selected block is read once per commit batch, and reached scoped names
reuse `NameScan` and the recorded trigger kit. Continuation keeps open episode
projections and the first completion's program selector, without raw project rows.
Feed pages read only the shared events and catalogue, without recording reads or
country-history reconstruction.

Special projects are prepared on their own
(`prepared/pipeline/independent/evaluated.rs`, registry name `projects`), and
`special-projects` queries wait only for their coverage; the event feed, which
reads the published completions when asked, refuses from their page. A
pre-pass worker reads each block by its changes
(`products::special_projects::prepared::block`) with the observer's tables and,
only where the bound content holds a project name key, or a country-prefixed
variant of one, that needs scope (`block::bound`, chosen as the run starts),
the trigger kit `triggers::RECORDED_TABLES` such a name evaluates:
every field of the tables the observer reads of a frame (projects, programs,
history, characters, portraits, the dictionary and the country links) is
watched, so each new allocation of the dictionary's view, which the published
dictionary revision counts, is an observed hour; so are the countries' keys,
the fields of the naming and trigger tables that scoped names' declared reads
read (but the few moving every hour), the row keys of the other tables, and
each completion date the project history holds. The worker keeps each
observed hour's frame, whether it is the hour's whole frame, one the block
reader built on the last read, or one it read that still carries a table
built at an earlier hour (at that built allocation, whose rows are not the
hour's), and each quiet hour's projects read and rebuilt tables. A frame is
whole only where the reader read the hour and every table it holds is that
read's own. The family folds them in order: a whole hour as the hourly walk
observes it; a built or quiet hour (on the last kept frame), or a read one
carrying a built table (its rebuilt tables taken as changed in any field),
exactly as well, since the observer reads of a frame only the tables watched
whole and the countries' keys, while every scoped name reads the same without
the frame (its inputs' declared reads unchanged,
`triggers::inputs::Inputs::advance`).
Where a scoped name cannot be told so, or a table watched whole changed or
was decoded again on a frame not whole, the rest of the block is observed
hour by hour on the
block read alone with the tables its content reads (the trigger kit only when
the content holds a project name key, or a country-prefixed variant of one,
that needs scope), as when the extract fails. They bind their content at every
block, as the walk does. Every block starts at a keyframe, whose views are new
allocations, so the views the walk binds where it resumes a branch read as
none there and the family binds none. Their pages and their continuation
(`special-projects-state-<branch>`, the walk's branch state's `projects`) are
hourly preparation's; the walk's branch state leaves them out. A measured run
times their folds and block reads (`projects.read`, one a block) apart from
their observation (`projects.observe`, one an hour; inside a fold it is not
timed apart).

## Prepared research

Research preparation uses the native supplying-read observer and visits only the
countries an accepted frame reads, changes or ends a carry for (see above).
Completed assignments and ended segments use
shared series events; the checkpoint retains only latest segments, pending
completion evidence and the countdown's previous operands. Progress, slot
availability and slot-state boundaries use shared runs. Country keys and slot
keys use the common catalogue, and each country's open projection is a compressed
reply range. Resume and append continue from the published prefix.

The research query reconstructs one country's complete reply from its events and
open projection, using the product's text, folder and modifier naming. Research
List and Columns keep complete row arrays and their existing sorting and
breakdowns. State, width summaries and navigation read the shared runs. The
Research history and Active research parity models expose their existing rows
and reached values.

Research history event pages answer the prepared prefix with later hours
pending; assignment attribution and completion recognition need earlier
observations. The empty prefix is a non-null reply before the first publication.

`series-state` for family `research` accepts `reads: [{tag, hour}]`. Each state
contains the slot reads and a presented `data` reply for the reached values.
Beyond coverage the production observer reads a bounded window of two supplying
periods: the latest owner read, its previous daily progress and every intervening
hour's carry proof. A gap, identity change, unavailable table or missed owner
read leaves the state unknown. Slots, progress and ahead-of-time operands are
exact. The countdown is exact at a supplying read; during a carry its older compressed segment phase is
still being prepared. Assignment dates and breakdown stay `preparing`. The Active
research cards appear from this state, and read history when attribution is
prepared. Their breakdown remains pending until then.

Research is prepared on its own (`prepared/pipeline/independent/evaluated.rs`,
registry name `research`, with `research::prepared::block` in
[dashboard-base](../dashboard-base/README.md)), and its queries (family kind
`research`) wait only for its coverage. A country slice is read every hour
and each read moves progress, so no hour is quiet; each country's carried
state (open assignments, last read and its continuity, progress samples, the
carry deadline of its last read) decides what a read adds, and an assignment
spans any number of blocks. The observation is two steps: an index step,
which refreshes the holders, owners and owners' read hours by slot from the
frame and the indexes of the hour before alone, and a country step, which
carries each country through the slots the hour selects. A pre-pass worker
reads each block alone and runs the index step from the block's keyframe,
keeping for every hour after the first the index values it changed, the
slots to observe again, whether the holders changed and each fresh read's
rows. A fresh read joins assignments to the inventory by the template indexes
the country's carried state holds, which the worker does not know: at a
slot's first fresh read in the block it keeps the owner's whole inventory,
and at a later one the rows of every template index the slot's reads in the
block assigned. The fold observes the block's first hour on the carried
research from the frame the worker kept; a keyframe's views are all new
allocations, so the carried indexes and the worker's rebuild every index
from that frame alike, and differ at most in the latest future read hour,
which the carried indexes keep from earlier hours. When those agree it takes
every later hour's country step from the kept index changes and rows, reading
no table, and at the block's end takes the worker's indexes. When they
disagree, or a country of a slot read with part of an inventory holds an
assignment that part does not cover, it observes the rest of the block hour
by hour from the block read alone, its indexes rebuilt from the block's first
hour. Its pages and its continuation (`research-state-<branch>`, the walk's
branch state's `research`) are hourly preparation's; the walk's branch state
leaves them out. A measured run times its folds (`research.fold`) apart from
blocks it reads itself (`research.read`).

Resource balance prepares all countries in one observer, using the same per-country
step as its test oracle. Shared pool/deal events keep independent coverage;
exact supply, demand, signed parts and native-net channels provide requested-width
summaries. Campaign maxima choose the automatic resource and stable bar scale.
Dated-alias drills join their membership-clipped summaries through the shared
chart helper; exact moments retain every active deal and its recorded terms.
The native pool sampler feeds the observer's numeric fields and the shared
progressive reader. Unprepared keyframe geometry carries approximate spans;
thirteen-block zooms and requested moments decode exactly. Deal events keep
their prepared-prefix coverage gate.
Unchanged resource frames advance authenticated frontiers without extracting
pools or rebuilding deals. The existing shared country projection retains resolved
links and holders at runtime and rebuilds after restore. Open endpoints and exact
hourly diagnostic counts materialise at a relevant change or publication; closed
deals compact once per publication and retired aliases stay out of observation.

Trade flows reuse their all-country trade step. Each actual native seller/buyer/
resource tuple has one shared-series key; no absent country pair is allocated.
Fact events preserve terms and aliases, while cumulative signed endpoints retain
amount, factory, convoy and trading-hour totals through peace and unknown reads.
Refusal series and mutually exclusive observed refusal sets retain exact selected
partial-hour counts. Queries read requested endpoints, bounded geometry and the
shared native census. Dense scales follow the D-12 per-flow envelope rule; exact
windows keep the product's simultaneous-peak sweep.
The same stateless native-flow sample supplies preparation and progressive
current charts. Exact state identity evidence overlays rough aliases through
the shared identity join. Cumulative totals, averages, coverage counts and
first/last dates require the complete prepared prefix; sparse samples never
establish those quantities.

## Prepared politics ledger

The politics ledger ([dashboard-politics-ledger](../dashboard-politics-ledger/README.md))
is prepared on its own (`prepared/pipeline/independent/ledger.rs`, registry name
`politics_ledger`), and `politics-ledger` queries wait only for its coverage.
Every captured country's interval extends to each hour, so no hour is quiet
for its observer: an hour whose captures only extend and which selects no
country takes the observer's advance path (dashboard-politics-ledger), every
other hour its whole accounting. Its blocks are not read by their changes: only
a capture's last hour tells an extended capture from a stopped one, and the
advance path reads the hour's changed rows. What an hour adds depends on the state carried from the hour
before (published event keys, the tag history, open days, coverage and hold
intervals). Its own thread therefore observes each block in order, as the
hourly walk does, on the block read alone with the tables the observer reads
(its native tables, the census and capture coverage, the naming tables, and the
trigger kit `triggers::RECORDED_TABLES` an item's text evaluates at its event
hour) and with the change facts of the block alone, which are the walk's at
every hour.

A pre-pass worker joins each block alone on the join's tables
(`politics_ledger::prepared::BlockFacts`): it keeps the join the block's first
hour leaves and certifies each later hour whose changed captures only extend
to it while nothing else joins a country. A join's state after an hour follows
from its state before and the hour's tables alone, so once the join the fold's
first hour leaves is the worker's, the worker's join is the fold's at every
later hour of the block. A certified hour that only advances captures for the
fold's own state (no row it has not yet recorded is observed again, no linked
capture is due, no day boundary, nothing dirty) then takes the advance path
without visiting its captures: each one's expiry moves at the hour, and its
witness reaches the last hour it extended to before any other join reads it.
Daily gain and cost changes with recorded deltas join only their affected
owners, including replacing their applications and wrapping component sums.
Rebuilt reads and incomplete reads whose former rows must be removed use the
whole-table join.
Every other hour, and every hour of a block whose first join is not the
worker's, is observed as the walk observes it; a block whose extract fails or
refuses is observed hour by hour. An hour presenting many countries (a day's
first hour, a block's first hour) presents them on several threads, each as
presenting it in turn makes it, once the content's power diagnostics are
reachable. Like the walk, it binds the ledger content and the recording at
every block, and where it loads a branch's saved state (at a run's start, and
after a commit that reloads) it first resumes the ledger from the frame the
walk's stream holds there: the last hour of the recording's block before, when
that block is on the same branch. Its pages and its state
(`politics-ledger-state-<branch>`, written after its pages at each commit) are
hourly preparation's. It costs, on one thread, decoding every recorded hour's
tables and the observer's work at every hour it does not advance from a
certificate.

Spending totals beyond coverage answer through the ledger prefix's last hour,
with `pending` for the requested hours after it. Before the first publication
the reply has no payment or recorded-spending coverage; it is still being
prepared, rather than a known zero or an unrecorded period. The breakdown note
names the date spent through. Spending tables use the native identity histories
of the country-groups first view without waiting for its dated event lists.

## Prepared country balances

Political and command power use shared series/event batches. Each branch/measure
section appends one binary range per commit, holding every key in a sorted
directory. Per-key blobs use delta/zigzag integers and zstd for runs, summaries
and sparse events. An event row is JSON, or bytes in a family's own form that its
queries read back raw (the Wars change inputs). Generic runs keep exact values and
breaks; equal contiguous runs join across batches only when there is no break.
Navigation derives dates from runs, including unknown boundaries. Sparse events
retain every row and duplicate in order. A small tag/measure catalogue maps choices to
keys. Holder identities are sparse measure events at each known segment start,
separate from balance drops.

Four aligned summary levels (16, 256, 4096 and 65536 hours) retain
first/last/min/max values and dates with breaks, plus exact absolute raw
contribution totals (value magnitude times covered hours). Totals use decimal
strings at the series scale and add across buckets and batches. Partial
buckets and completed prefix snapshots share the batch file; queries look up
aligned summaries directly. Hourly summaries include the original supplying runs
with unclipped bounds and breaks for matching chart drawing, extents and picks.
Dense views retain screen-sized buckets. Event indexes and
cumulative counts produce complete paged rows and screen-sized clusters.
Checkpoints retain open runs, counters and partial summaries, so resume and append
prepare only new hours. An open summary bucket that one value held through its
whole span is kept as that span (first and last hour, raw value, first break), and
only when it makes the bucket again bit for bit; saved, a span that ends the open
run is its alignment alone. Most keys hold one value through their open buckets,
so this is most of a saved branch state.

Host requests use `series-state`, `series-summary`, `series-next-change`,
`events-page` and `events-clusters` with a family selector. The shared frontend
prepared-series module handles pending coverage and cancellation. Country-balance
widgets keep exact reached values separate from chart geometry and page every
drop without limiting the reachable list.

## Prepared force series

Territory's observer also publishes the native owned-state counter in numeric
channel 8. Country-history's identity join supplies that count once per frame,
with unresolved or ambiguous counters unavailable and empty tags excluded.
The identity state retains counter refusal counts without retaining numeric history.

Faction totals join prepared territory or factory values to the prepared native
country identities and faction rosters at query time. Exact windows use the native-member
sum sweep shared with the test oracle, with one predecessor hour to
preserve segment boundaries. Dense geometry uses the shared group sum and summary
grid, stitching a holder's aliases before adding members. Known empty rosters and
proved absent holders contribute zero; unread identities and values remain unknown.
Dense Top lines retain every faction whose aligned column maximum reaches the
N-th largest column minimum, preserving brief leaders while allowing extra lines.
Reached values, origins and line endpoints are exact. Keyboard navigation compares
the totals around member changes and membership boundaries, skipping cancellations.
Member details retain the native identity census for stable colours and use the
shared summary stack. Parity models are "Faction totals" and "Faction member values".
A reached hour of factory totals under hour membership, without member parts,
that the prepared pages do not hold yet answers from that hour's frame and the
hour's before alone (`faction_totals::prepared::sampled_state`): a fresh roster
scan, identity history and factory sample of those hours feed the same sum
sweep, the hour before deciding a segment break at the hour as preparation's
runs do, so the totals, members and spans are the pages' (warnings aside). End
and Ever membership, member parts, summaries and the next change wait. The
content-bound observation cache retains reached replies by block, selection and
hour, so mounted head and hover re-asks reuse their roster and sum work.

Country factories and nuclear stockpiles are prepared on their own
(`prepared/pipeline/independent/census.rs`, registry names `factories` and
`nukes`). Each census joins one hourly table to the countries' keys and
links, so its blocks are read by those keys, every link and every field of its
table (and the name tokens, for the stockpiles): workers keep the frames of the
hours one of them changes or its table's read is due to begin or end by the
recording's cadence (never, for a table read every hour), and each hour
between takes the census's
unchanged-hour step (`advance_quiet`: the last read's counts again, the read it
dates and every series' accepted hour) in constant time. An hour between that
the census cannot step so (its read flag differs, its countries did not join,
or an incomplete stockpile read names its reason from the frame) leaves the
rest of the block to hourly observation. Continuations are
`factories-continuation-<branch>` and `nukes-continuation-<branch>`. Country
factories and stockpiles answer from their own coverage with the identity
history's. Factory faction totals wait only for the faction rosters, the
identity history and the factories; their exact replies (reached hours,
exact windows and the next change) never read territory, while dense factory
summaries join territory's member presence and wait for territory's coverage
through their window. Territory totals wait for the faction rosters, the
identity history and territory.

The sampled families are prepared on their own
(`prepared/pipeline/independent/sampled.rs`, registry names `air`, `navy`,
`stability`, `equipment_losses`, `supply`, `parties`, `trade` and
`territory`): air force and navy size, country stability and its parts,
equipment losses, supply health, the party history, trade flows and territory. Their widgets answer at open through samplers;
this is their exact history. Each observer reads tables that move nearly every
hour in war or production and carries state that decides what the next hour
adds, while an hour where nothing it reads changed already costs it almost
nothing, so no block is read by its changes alone and none is observed apart
from the block before. Each family's own thread observes every block in order
on the shared block read, with the tables its observer (and, for air and navy,
its resumption) reads, and does around each block what the hourly walk does:
air and navy bind their part content and resume their censuses from the last
frame of the recording's block before on the same branch (kept from the
family's own read of that block, or read alone again after a load), supply
binds its defines, territory binds its area data and breakdown content,
equipment losses end each block at its last hour (the branch's last hour at its
last block), and trade flows take the source's resource naming, which their
flush reads. Territory observes the owned states, which drive its own read of
the identity history, then its own observation, in each hour; an identity
refusal refuses it as coming from the identities, and its read of the history
is written at its flush, before its pages. Continuations are
`<family>-continuation-<branch>`; the party history's saved page
(`party-history-<branch>`) is its state, as in the walk, and territory's read
of the identity history is `territory-identities-<branch>`. Air force size and
navy size fold their blocks from pre-pass workers instead: a worker steps its
own census through the block and keeps the census after the first hour it
holds nothing a carried census holds from earlier hours (the first hour for
navy; for air, the first hour the block reads or forgets variant names, which
hold from the read that gave them) and each later hour's changed rows; the
family observes the hours through that one on the carried state and, when the
carried census decides the later hours as the worker's did (tracks compared as
the next hour extends them; the hour's own join flag and when each variant
name was first read left out), renumbers navy's ship classes as the carried
census numbers them, projects those rows through the content (hours apart, on
several threads), publishes them on the carried series and carries the
census's tracks and counters; otherwise it observes the rest of the block.
Air force size, navy
size, trade flows and the party history answer from their own coverage,
stability and its parts by their series kinds; supply health, equipment losses
and territory with the identity history's (`independent::JOINT`). The scoreboard, which reads them
with other families, waits for the branch's coverage and, for each column it
reads, that column's supplier's (`pipeline::product_coverage`).

Territory's observer feeds country totals, independent world totals and strategic
region attribution in one accepted frame walk. All eight measure/rule channels
use shared series batches; the existing ranking sweep supplies rank-position
runs. Composite facts retain exact regional state-share fractions and native
identity boundaries. Queries read reached hours, requested event pages or width
summaries. Group sums read exact member runs only for windows bounded by screen
width; dense geometry adds aligned member summaries. Missing member values and
unobserved membership remain unknown. The browser parity models are
"Territory values" and "Territory regional values".

The territory, regional-territory and faction-total product loaders and territory
replay walks compile only in tests as parity oracles. Production retains their
shared observers, content bindings, sums and reply builders. Shared country-history,
country-group and factory scans remain available to other production consumers.

Army size feeds its existing inventory/census step into shared exact series for
all eight measure/attribution channels. Composition changes retain dated parts.
The inventory holds the division, template and land-input rows by key, and the
census each country row's slot and count. When an hour continues the previous
one and a table changed by exactly its recorded rows (`FrameChanges::rows`), only
those rows are read: changes to fields no total or composition reads (such as
organisation) are skipped, a hit-point change edits its countries' strength alone,
and only the countries those rows name are observed again. Country changes to
fields no identity reads keep the identities without comparing every country.
Otherwise each changed table is read whole. An observed country whose totals,
coverage or composition continue from the previous hour leaves those series open
rather than writing them again.
The shared ranking sweep supplies rank positions and country ranks; top selections
read their requested positions, while dense lines use width summaries. Replay-block
windows are indexed when the source opens. Supply health feeds its existing native
classifier into three synchronized display-share series and dated complete-count events.
Supply binds the successful or refused Defines outcome before frame observation.
Unchanged inputs advance its native and series frontiers; changed inputs reuse the
native classifier. Its current country, owner and division joins rebuild after
resume and are omitted from continuation. Only current holders are visited when
values change; retired tags retain their published history without hourly work.
Its thresholds resolve through the recorded content stack; refused defines retain
their named reason. Window/hour queries reuse prepared country identities and the
products' reply builders. Whole-branch army and supply scans are test-only parity
oracles. Supply display summaries preserve exact reached counts independently
of dense geometry; short views keep their existing drawing path.

Army and supply queries join accepted values with the shared transient tail.
Army block windows also carry native battalion compositions without preparing
the tail. A window's exact read ([`progressive::tail_recorded`](../dashboard-base/src/prepared/query/progressive.rs))
carries the division inventory from hour to hour as the observer does: each
inventory table is applied unchanged when the decoder carried its rows, from the
rows its recorded delta changed, or whole, always with the decoded table's values;
identities, census, totals and metadata are read afresh each hour. The first hour,
an hour after a gap or a skipped hour, a frame read alone and an hour after a
refused inventory table are scanned whole. Supply resolves recorded Defines once when sampled land inputs need
them, and retains complete-count amounts separately from approximate share
geometry. Native identity selection follows tail-only aliases after the shared
prefix/identity join. Prepared rank positions serve the accepted prefix; sampled
tail totals rank transiently through the same native sweep, without publication.

Air force size feeds its native wing step into four total channels, prepared rank
positions, per-part summaries and dated census changes. Daily variant spellings
and open country continuations resume independently of transient census joins. The committed decoder frame
rebuilds those joins without repeating accepted histories or warning counters.
The parts view reads its viewport and exact reached hours separately; short views
keep native reply projection and drawing, while dense lines and compositions use
width summaries. Air and navy retain their product loaders and branch scans for
country part drills; the scoreboard's plane and ship visitors are test-only.

Air and navy share one stateless point census and tail adapter. Their total lines
publish approximate keyframe spans immediately and exact bounded zooms through
the shared summary path. Navy part projection captures recorded content once and
uses the native class/role reply builder; maximum HP stays independent. Air part
drills retain their prepared coverage gate because their native projection needs
the dated variant spelling catalogue retained across supplying reads. Exact
reached identities clip rough aliases through the shared slot-aware UI overlay.

Force observers consume the shared `FrameChanges` facts. Their native country joins
compare `CountryKeys` before rebuilding after balance-only country reads. Current
resolved joins and supplying operands are runtime state, omitted from continuation
and reconstructed on resume. Unchanged accepted frames advance one authenticated
frontier; changes, gaps and publication materialize the native open endpoints.
Army, factory, nuclear, supply, wing and ship outputs observe changed, new or
removed keys in `series::Accumulator`. Supplying nuclear reads still publish their
native read evidence, independently of stockpile changes.

The shared air/navy parts adapter preserves unchanged native table updates instead
of copying owned snapshots. Native dirty slots select current holders; retired
tracks are read at publication rather than visited hourly. Parts project only the
changed native runs, using current part membership to close removed or refused
values. Navy's immutable class catalogue is shared across temporary projections.
Equipment and sub-unit databases bind their successful or refused `CachedResult`
before observation. Recorded spellings and mixed wing signatures use `KeyedResults`
in the existing source/catalogue binding, including localisation language. The
native classifiers supply both the fixed parts and their exact named refusals.

## Prepared scoreboard

The scoreboard adapter composes force, factory, nuclear, balance, stability,
production, native owned-state counter and casualty values from their prepared
suppliers at one displayed hour. Its moment index reads the sorting column for
all current countries with one shared batch lookup, then uses the native ranking
comparison with exact signed amounts, current-tag ties and unknown values last
in either direction.
Top selection retains only its requested countries. Other columns are read for
the table's mounted rows. Cell bars use exact maxima over the whole selected
table, read in one batch per displayed column when bars are enabled.

A moment inside accepted coverage answers from the committed prefix and its
identities, before preparation completes, once every column it reads is
covered there by its supplier too: a family prepared on its own (territory,
air force, navy, stability, factories, stockpiles, casualties, production)
answers from its own coverage, which may be behind the branch's
(`scoreboard::prepared::gate`). Beyond either, a moment whose needed
columns are all native factory counts reads that hour's frame directly: the
country-history identity join supplies current holders, joined with the prefix
and the shared alias seed, and the stateless factory sample supplies the
all-group count for each holder, dated at the hour. The whole branch's
identities, once prepared, supply the holders instead; until then a tag
selection of the index, which names every holder its tags ever had, stays
pending. Other columns carry cumulative, carried or multi-table facts and stay
pending there.

The column registry and cell serialization remain shared with the native
scoreboard. Prepared identity intervals supply current holders and aliases;
replies contain only their displayed-hour slice. Casualty cells reuse native
sample projection for supplying dates and partial-history flags. Army's native
raw-run coverage is stored separately from known manpower, so an unknown amount
can retain its recorded read date. Territory uses the territory observer's native
country counter channel with the same ambiguity and availability rules as the
country-history join. The adapter adds no ranking history or pipeline stage. The
whole-branch scoreboard and force visitors exist only as test parity oracles.
Factory types, game-score option labels and air/navy/nuclear country part drills
still use ordinary product requests. Their loaders and required scans remain
available alongside the prepared chart adapters. Factory faction totals and
nuclear event-feed callers share those native scans.

## Prepared daily country values

The production country-stability and country-experience scans also observe
accepted preparation frames. Stability, war support, maximum command power and
army/navy/air banked experience, daily gain and net change use shared series
batches. Supplying-read boundaries, identity breaks and exact unknown reasons
remain distinct. Sparse holder/read events and a stability supplying-read series
preserve carried dates even when a formula is unknown. Experience keys include
native holders, so aliases and unrelated tag reuse remain separate. The existing
country-group identity catalogue selects their dated UI membership.

Open scans, recent experience reads and complete gain runs resume across batches
and appended recordings. Current country joins, supplying-read operands and
numeric inputs are runtime-only and rebuild after restore. Equal country keys
reuse joins across balance updates. Accepted hours authenticate shared series
frontiers; native daily reads and complete hourly XP gain runs preserve their
own cadence. The country-stability and country-experience families
answer exact state, clipped width summaries, navigation and provenance event
pages through the country-measure adapter. Exact windows retain their original
supplying runs for chart drawing. At-hour stability breakdowns use the registered
product.

The stability scan and the parts scan read their sliced tables by the hour's
recorded changes. When an hour continues the previous one and a table changed
by exactly its recorded rows (`FrameChanges::rows`), only those rows are taken:
input and command power rows by their keys, the stability input and line read
hours from SliceReads' changed rows of those two tables (no other table's rows
are visited), and stability lines grouped again only for the countries the
changed rows name. A country is observed again only when its rows differ from
the hour before's, as a comparison of the whole tables finds. Country changes
to fields no key reads keep the countries' keys without comparing every
country, and the parts scan holds the hour's SliceReads view instead of a
copy. Otherwise each changed table is read whole.

Beyond committed coverage, stability, war support and maximum command power
are read by the shared progressive reader with the observer's production step
over each frame and no carried state. Each sampled measure carries its frame's
supplying read hour; a measure the frame cannot compute has no holder, so its
native segment ends where the observer's does. A transient read proves that a
country was recorded but never that it was not. Banked experience is a daily
read held until the next: beyond coverage its sampler steps the experience scan
through the hours it is given, starting afresh at an hour that does not follow
the last, so an hour answers each bank as its last read at or before it holds
it, dated by that read (`tail_first_read`, `state_tail_recorded`). All three banks
share a compact cached first-read observation of each block for navigation.
Banked summaries answer from the exact prepared prefix, including an empty
prefix, with the remaining recorded hours pending; they decode no tail blocks.
Exact state requests share cached whole-block observations of their selected
native holders, including all banks and read hours, and clip those observations
after coverage moves. Daily gain and
change per day keep the coverage gate: they need every hourly interval of a day,
or a read exactly a day earlier, within one native stretch.

Experience is a family of its own (`experience`, `ExperienceAccumulator`, with
its own status row, refusals and flush), prepared on its own; stability stays
hourly. Its blocks are read by the countries' keys, slots and tags, the links
and every field of banked experience and the hourly gains, whose rows the scan
compares whole; each banked read hour is visited, and the next one by the clock
is the extract's deadline. A quiet span is the scan's step with nothing changed
(`Scan::advance_quiet`): complete gains hold as runs until their inputs change,
so a quiet span advances only the frontier. An observed hour visits countries
whose inputs changed, or every current country at a banked read; a country's
last day is summed from its runs, widening each signed amount before multiplying
by its hours. Missing intervals, ambiguous joins and native-instance changes
break the complete day. When an observed hour continues the previous one with
the links unchanged and its gains changed by exactly their recorded rows
(`FrameChanges::rows`), only the slots of the countries those rows name are
joined again, each from every row the links resolve to it; country changes to
no key field keep the countries' keys without comparing every country. It saves its continuation as
`experience-continuation-<branch>`; its series route by the family kind
`country-experience`.

The stability-parts scan feeds per-part runs and observed supplying-read coverage
through the same batches and query adapter. Its resumable state retains identities,
missing-line warnings and recent naming tables. Equal naming allocations share
one zstd-compressed replay checkpoint across fresh acquisition dates; the dated
lookup retains every supplying hour. Numeric stability and parts share the
current native input join. Unchanged part keys advance through shared series;
missing coverage and zero parts close their keys explicitly. Dynamic countries
retain hourly native naming for clock-dependent civil-war facts and refusals.
A catalogue retains each part's latest naming operands; the production line
namer resolves them through recorded content. The drill ranks its palette by exact
absolute contribution over the selected range, using a bounded summary. Hourly
windows recover raw parts from absolute totals and draw the legacy signed stack;
dense windows draw each sparse part's ordered bucket extrema, keeping a missing
child bucket from hiding another part's known values. Observed coverage controls
the final signed stack; absent parts are zero only inside known coverage.
Approximate spans pass through the shared signed-stack geometry and the same
clipped faded/dashed renderer as line summaries; exact geometry is unchanged.
Hover and selected hours use exact part values. Both paths expose the
"Stability part values" browser parity model. The registered stability-parts
product shares the scan and remains the history reference.

## Prepared fuel

Fuel level, capacity, completed-day accounting and current consumption share one
all-country observer. Shared frame-change facts
guard country/link/state joins; balance-only country changes use the shared key
projection. The shared series frontier carries unchanged current measures to
publication; changed current values and supplying reads emit observations. The
same accumulator supplies exact runs,
supplying-read events and chart summaries. Country and native actor bindings use
the shared catalogue. Native aliases, conflicting aliases and demand continuity
are prepared with the same observation.

The observer advances the bounded preceding-ownership proof for current tags
and the completed-day settlement clock each accepted hour. Dormant tags recover
their absent/unknown proof from the shared bounded suffix when they reappear;
closed settled countries need no hourly visit. Retained unnamed diagnostics
materialize once at publication. A fresh equal fuel acquisition retains its
supplying date without rebuilding unchanged ownership joins.
Omitted-day counts accrue once per settled supplying read; unnamed diagnostics
count distinct native identities, so neither count multiplies with held hours.
Current resolved links, holder identities, state-owner operands and raw
level/capacity operands stay runtime-only and rebuild from the restored frame.
The stateless progressive sampler and the observer use the same amount projection
and native slot join.

Fuel is prepared on its own (`prepared/pipeline/independent/economy.rs`, registry
name `fuel`, with the economy ledgers) and answers from its own coverage. Its
blocks are read by the fields its scan reads: the countries' keys (id, slot and
tag), every link, each state's owner, and every field of the fuel table and its
demand parts at their daily read; the next read by the clock is the extract's
deadline. The family's thread observes each kept hour as hourly preparation
does. Each span of hours between is stepped without a frame
(`Prepared::advance_quiet`): no identity, ownership or open read changes there,
so the scan carries each current country's ownership proof (its last day of
hours), identity track and open reads through the span in one pass, and the
current series advances its accepted hours. Settling a day at such an hour
decides, retires or ends nothing unless the hour's cutoff is a settling read's
first or last hour, and otherwise only extends each open day run by that hour,
so the days are settled at those hours and at the span's last, with the scan
carried to each first: a constant number of settlements per day, where hourly
observation settles every hour. A fuel read dated off the clock (another table
of its slot read at another hour) leaves the rest of the block to hourly
observation. It saves its continuation as `fuel-continuation-<branch>`.

The fuel day's overlap rule prefers daily-slot reads. The observer therefore
retains only the final 23 unsettled hours, the open read and the preceding kept
day in its continuation state. Settled day values enter the shared accumulator;
queries merge the bounded derived tail. Appending a daily-slot read can suppress
an earlier off-slot day across a commit without changing settled ranges or
retaining raw replay rows.

Fuel and consumption chart queries read exact reached hours and screen-width
summaries through the prepared series routes. Whole-branch scans and chart
loaders compile only as parity-test oracles. Registered detail products use their
supplying-block and 720-hour window reads; the prepared summary supplies their read bounds, actors
and segments without a branch walk.

Fuel level and capacity draw beyond prepared coverage from selected-table
keyframes and become exact within thirteen blocks. Reached raw amounts remain
independent of completed-day reads. Day accounting needs preceding ownership
and overlap decisions; consumption needs its stamped supplying read and actor
segment. Those fields and supplying-read navigation remain pending beyond
accepted coverage. A scalar sample never supplies a synthetic day or zero rate.

## Prepared economy widgets

Fuel by branch, the market and lend-lease, lend-lease balances, the production
catalogue (licences, industrial organisations and designs) and convoy
efficiency are five families prepared on their own
([dashboard-economy](../dashboard-economy/README.md)): `fuel_branches`,
`market`, `lend_lease_balance`, `production_catalogue` and `convoys`, each
observing every accepted hour on the shared block read
(`prepared/pipeline/independent/sampled.rs`) and rebuilding only where its
watched tables changed, a table was read in its slot, or a gap broke the hours.
Each saves its continuation as `<family>-continuation-<branch>` and gates its
own queries, which read through its prepared prefix (`prepared/query.rs`,
`economy`). Their replies are whole-branch histories looked up locally by the
widgets, or the market ledger's newest-first pages.

## Prepared game score

Unchanged score inputs advance the authenticated series frontier without visiting
retained country facts or keys. An hour that continues the previous one is read
from its changed rows (`FrameChanges::rows`). Each changed row names the countries
whose parts it reaches: score inputs and factories by their link's slot, an
air-base registration by its slot, a wing by the registrations listing it, an
appearance colour by the countries coloured from that row, a membership by its
slot, and a faction name by its members. Changes to fields no part reads, such
as balances, factory use or wing missions, read nothing. A naming operand change
reads the names again and computes again only the countries whose name, or an
overlord name it reaches, changed. Only countries whose facts changed are
observed, with any country sharing a native holder whose total became known or
unknown. A table that is not complete in either hour has no changed rows and is
unchanged when it holds the same. Keyframes, resumes, gaps, new or departed
countries, link or multiplier changes, other tables changed without recorded
rows, and names reaching slot zero read the whole tables; the observer then compares shared country identity operands
before rebuilding for balance changes, and reserved naming that includes its read
hour keeps its native hour-dependent path. Native warning counts accrue each
accepted hour; a count is added to the catalogue when it changes and at
publication. A changed observation updates holder stretches and closes departed
rows; publication seals the numeric endpoints through the frontier and dates
every retained hour marker at the last observed hour, so a restored continuation
equals a continuous one.
Game score shares its native frame calculator between its product loader,
background preparation and direct progressive reads. Six native fixed5 score
parts per country and the global multipliers use shared series runs. Sparse events hold dated identity, faction,
missing-value reasons and native-holder continuity across aliases. Continuation
state retains current facts, counters and open stretches; committed history stays
in append-only series batches.

The game-score series adapter answers one hour, width summaries, event
pages/clusters and navigation from prepared files. Whole-hour table/race requests
beyond accepted coverage use the shared progressive state reader to reconstruct
the target replay block, without publishing prepared files. The observer and
direct-hour closure share stateless score extraction; names and factions use the
existing recorded-content presentation resolver. Countries absent from the
prepared catalogue remain visible. Direct replies carry an approximate flag and
omit prior race continuity; accepted preparation refreshes the same table/race
shape. Replies reuse the history product's score representation and row builders.
Pending requests retain their own displayed date. Gaps, missing score parts and
replacement holders end continuity independently of name and colour changes.

## Prepared leaders

Leader preparation uses the native scan step and reply projection. Leader
choices, leaderboards and careers read prepared prefixes. A leaderboard request
with an hour beyond coverage uses the hour's frame with the same visibility and
content rules. Current skills, rank, attributes and traits are exact; battle
arrays and historical dates are withheld with the preparing fields. The table
keeps those counts null and waits on battle-count sorts. Country-filtered
requests wait for complete identity history before resolving native holders and
aliases. The shared frame-change
facts gate current native indexes and outcome reloads. Equal country id/slot/tag
projections retain the resolved holder join when only balances change; the saved
state keeps the projection the join was built from, so a restored scan retains it
exactly when a continued one would. Admission
rosters update in place when a visible leader's kind, owner or native holder changes; they never
walk the accumulated roster each hour. When an identity table (Characters,
CharacterPortraits, LexicalTokens) changes by exactly its recorded delta, a
leader whose character, portrait holder, name token and template tag token are
not among the changed rows keeps the person and visibility role it read.

Consecutive unchanged observations advance one scalar frontier. Current runs,
commands and supplying coverage seal through it on changes, gaps, publication
and open-state replies; no carry crosses missing hours. An hour is unchanged
when none of the fields the scan reads changed (`unit_leaders::reads`): every
leader field but the retired experience, the identity and trait tables, the
holders' country slots, tags and links, a formation's leader, name and kind, a
mission's leader and a task force's fleet, the results, and of the combat
details only each combat and its sides' commanders, which the scan compares
itself, since the sides' other operands move every combat hour. Task force
positions, mission fuel and ranges and country balances are not read. An hour
at which only the combats and results changed takes the battle step alone:
the careers, commands and coverage carry through it and its results and battles
are read as the leader walk reads them. A leader's visibility
evaluation in its owner's scope records the inputs it read
(`triggers::inputs`) and is reused while each reads the same value; with scoped
visibility an unchanged hour carries only when the core tables refuse as before
and every such evaluation confirms. An input whose declared dependence the
hour's change facts leave unchanged confirms without being read again, so an
hour changing only fields and rows no evaluation reads builds no trigger World. Check mode (always in tests,
`BATTLE_PLAN_REUSE_CHECK` in preparation) runs each reused evaluation anyway and counts
any change. Pending byte totals update with new facts and are recounted only at
restore and publication. The counts of identity, owner and trait views read
are runtime only, as the caches they key.

Leaders are prepared on their own
([products/unit_leaders/block.rs](../dashboard-unit-leaders/src/products/unit_leaders/block.rs);
registry entry in `prepared/pipeline/independent/leaders.rs`, family `leaders`).
The extract reads a block by the fields the scan reads and, of the shared
trigger tables, the fields their declared reads read (the row keys of the
rest). A run reads only the scan's own tables, the trigger world's core
(provinces and states) and, where the phase binds a catalogue, the shared
trigger tables leader visibility's declared reads read (`block::bound`); the
rest of the trigger kit is not read, and is taken as changed anywhere at every
hour. The content selection includes ScriptVariables entries and seed, and
country tag resolution's owned-state counts and original-tag buckets. Counter
rows keep only their key and owned-state count; unrelated counter fields do not
wake the extract. These selected tables join the shared read and its deposit,
so the family folds their reconstructed views without decoding them again.
At each hour read the extract takes what changed since the hour before
from the block reader (`BlockTables` of its changed tables; where it is read
with families watching more, it finds them from the hour before's rows), and
joins to the quiet hours (`QuietTables`) every hour whose combats changed only
in operands beside their commanders. It keeps the block's first hour as read
and every other hour reduced: hourly-moving tables by the fields leaders and
the declared reads read, reduced again only where those fields changed since
their rows were last reduced, and tables no declared read reads not at all.
The fold observes each kept hour on its kept frame with those change facts,
and each hour kept as read the same way where the run left tables out: a
verdict is read again there only when every input it recorded reads what the
whole frame holds, and evaluated there only when everything the evaluation
read is the whole frame's; otherwise, before any career changes, the rest of
the block is observed hour by hour on its whole frames, read with the whole
trigger kit, counted as `leaders.fallback`. Quiet spans carry the scan through
in constant time.
The test-only recording oracle executes the native step and every evaluation
every hour. Shared
series events hold closed value runs, trait gains, commands, battle observations
and coverage spans. Each full leader
reference has its own key. Combat outcomes and naval result source links have
shared keys in the same catalogue, which also carries each such key's latest
published value: observation restores reached keys and publication finds
changes from it, never reading the series back. Selected careers join their
latest outcomes, preserving the first supplying read and permanently unresolved
conflicts.

One compressed branch range retains current leader facts, open commands,
last-known traits and continuation counters. It contains no closed command or
battle history and no campaign-wide outcome/source maps. Person, role and trait
catalogues retain distinct presentation facts. Choice requests use the compact
native-holder and kind admission rosters without reading career events; other
requests page only the selected leaders' facts and shared observation spans,
then call the shared materializer and projection. Country selection
uses the prepared native identity history. Portraits remain separate one-block
requests for mounted rows.

Intelligence queries use prepared pages. The five Product implementations retain
their registry metadata, validation, TypeScript declarations, content keys and
offline checks. Their whole-branch loaders are test-only; production uses the
shared default refusal. Branch walks and standalone per-frame visitors live in
test-only oracle modules, alongside the native helpers used for parity. The
prepared paths share the production extraction, steps and reply builders.

The intelligence coordinator shares one current country/link projection among
placements, agency comparison, roster, decryption and espionage. CountryKeys
ignore balance-only Countries updates; meaningful identity/link changes rebuild
the join. Its identity columns, partial slot admission, aliases and link labels
are runtime state, rebuilt after restore, and test-only recording oracles use
the same extraction. The accepted frame's shared change fact separates changed
rows from successful fresh reads, including equal acquisitions.

Intelligence databases bind before observing frames. Their success and named
failure share the catalogue result policy; roster and operation pages share the
operation database. Fixed assignment classification and names use keyed content
results, while their icons retain the separate keyed read policy. Capturer names,
experience amounts and date-dependent status are evaluated from recorded inputs.

Unchanged value keys advance the shared series frontier. Fresh membership and
operation dates remain independent provenance. Native open endpoints materialise
at relevant changes, publication and checkpoints; closed episode compaction runs
at publication. Captured/cooldown roster status schedules its next native signed
day boundary, cipher slopes validate their next daily phase, and espionage still
builds each 24-hour reply. None of these clocks walks closed history per frame.

The intelligence family is prepared on its own
(`products/intelligence_prepared.rs`, registry entry
`prepared/pipeline/independent/intelligence.rs`), gating the five products. Its
extract reads a block by the fields its observers read: the countries' keys,
slots and tags; the links, the cipher operands, memberships, character
operatives and portraits, upgrade levels, the dictionary, operations, their
assignments and completions whole; each target's cryptology strength; the
agency feature bit; the agency scalars the comparison shows; and the country
naming operands. Memberships and operations are also read at every hour they
were read, which dates them even when equal. Unwatched fields (country
balances, the feature row's runtime scalars, agency upgrade progress, raid
defense) move freely. The extract keeps each changed hour's frame, its large
partially watched tables reduced to the fields read and shared while equal,
and for the hours between, which tables each rebuilt. The fold makes every
hour's observation on the carried observer: a kept hour on its frame and
change facts, an hour between on the last kept frame stamped with its hour and
no read. Decryption's next daily phase, the roster's day rounding and
espionage's daily pages therefore fire at the hours hourly preparation fires
them. Between kept hours the change facts report only an agency rebuild (the
agency scan prunes held rosters there, as hourly preparation does), and the
roster is told of the other rebuilds for the read marks it keeps; every other
observer treats a rebuild of fields it does not read as no change. A block's
espionage day pages are staged when it finishes, read through to the committed
store, and written at the family's next commit before its flush (also when it
refuses the branch later in the batch);
the espionage log reads its committed series through the same store, at the
same commit boundaries as hourly preparation.

## Prepared operative placements

The operative placement scan observes every accepted frame once for all countries.
Pair roster/name changes and coverage changes use shared append-only events;
successful membership dates use separate exact runs and events. Checkpoints retain
the latest pair values, frozen held placements, coverage and supplying read only.
Prepared range queries include the actual membership read supplying the left
edge, then use the product's reply builder. Hourly tag changes retain that read
date, while failures and gaps end the hold. The chord widget requests day windows
through shared prepared coverage polling.

## Prepared espionage log

The intelligence observer uses the production operation builder. Existing daily
metadata replies are packed in day order into one shared reply range per replay
block. Blocks that own no day starts publish empty ranges, preserving consecutive
block ordinals. A partial day crossing a block is rebuilt in its original range; resume
retains its open instances and identity/ending witnesses. The small persisted
manifest provides day discovery, coverage, types and warnings directly.

Closed episode candidates and immutable completion references use shared events.
Preparation restores them for late drains, preserving the original native
handle/owner/target/definition match. Launch discovery and delayed completions
repair affected earlier day replies during preparation. Queries read the small
manifest and the requested block, then use the existing content presentation.
Operation descriptions retain their independent one-block reader.

## Prepared operatives roster

Roster preparation uses the production per-frame scan for every country. Daily
membership and Operations provenance use shared runs. Operative value changes,
episodes and sparse capture witnesses use shared events. Checkpoints retain the
latest episode per handle, held projected values, episode counters and string
catalogues. Holder-scoped refusals and first-use string provenance preserve
manual identity selections and the existing reply dictionary order.

Requested day windows include authentic supplying reads for held aliases,
historical capture witnesses and optional departed rows. Departed rows keep their
last actual value and observation date. Country selection uses prepared identity
histories; requests never walk recording blocks.

## Prepared agency comparison

Agency preparation shares the production frame scan and reply builder. Scalar
and roster changes are shared series events; membership supplying dates are
separate runs and events. Each checkpoint keeps the latest scalar/roster rows,
country identity segments and counters. Requested day windows recover those
events and use the existing selection, ranking and content projection. Manual
choices use prepared country identities and do not scan the recording.

## Prepared decryption

The production decryption scan observes accepted preparation frames for every
decryptor/target pair. Exact stored amounts and bonus countdowns use shared series
runs. Sparse events preserve the scan's daily-step operands, attempt starts,
break observations and independently dated endpoint tags. Its checkpoint keeps
only each pair's latest run, tag stretch and break, plus the current coverage
stretch and refusal catalogue. Appended frames continue that state.

Prepared `decryption` queries accept pair identities and an inclusive hour range.
They read the shared catalogue, requested value coverage and sparse events to
rebuild the existing reply, preserving raw integers and native daily arithmetic.
Missing pair coverage and unread operands remain unknown.

## Prepared casualty values

One shared casualty observer covers every native country. Census and endpoint
joins refresh from shared frame-change facts and the native identity projection.
Equal relations advance the supplying frontier; changed relations update their
own lifetimes. A runtime endpoint adjacency finds ended lifetimes affected by a
holder change and rebuilds once on resume. Open country and pair runs materialise
at their next change, publication or gap. Static casualty text binds once through
the shared catalogue and keyed result store, including refused text. Exact
dealt, known taken and recoverable unknown totals use shared series batches, with
sparse holder and supplying-read events. Country keys distinguish native holders
of reused tags. The existing ranking sweep publishes rank-position and inverse
rank runs for the requested top selection. Gaps break coverage and unknown-loss
recovery; signed counters and incomplete recorded subtotals remain separate.

Publication retains counter operands and each track's open sample, discarding
closed sample history from the continuation state. Resume rebuilds joins from
the next accepted frame without treating that rebuild as a holder change; the
relations the last processed read supplied are those carrying the scan's
generation. The continuation keeps a digest of the census and naming inputs
in place of the naming snapshot, so a resumed scan names countries again only
where continued observation would; operand refusal reasons carry the hour
they were read. The catalogue's content warnings and the family's own content
refusals, including the unknown-attacker title's, are refreshed at each
publication.
The series adapter answers reached-hour state, width summaries, event pages and
navigation from shared readers. The Casualties widget requests one reached hour
and retains pending playback at the previous reply's own hour. Its parity model
is labelled "Casualties exact values".

Casualty values, pairs and breakdown use prepared queries. Their product loaders
and casualty-label cache are test-only parity oracles. The shared casualty
branch scan also supplies scoreboard's recorded-product loss columns.

Casualties are prepared on their own
(`products/war_casualties/prepared/block.rs`), beside the hourly walk. Their
pairs and breakdown wait for their own coverage, and those and their values
(family kind `war-casualties`) refuse from their own page. A block is read at the hours a field
the scan reads changes: every field of the war relations and country links and
the campaign, the countries' slots and tags, and the naming operands
`same_name_rows` compares. Counters move every hour of a war, so wartime hours
are kept and peacetime hours are quiet, and the per-hour work runs in the
extract, on the pre-pass workers: it runs the casualty observer on the block
alone from the block's first hour, observing each kept hour and advancing each
span of quiet hours in constant time (every series authenticates the span, the
scan's read sequence and relation frontier move to its last hour, and no
country, pair or value is visited). For each later hour it records whether the
relations were read again, the lifetimes first held, the naming snapshot when
it named again, and each changed tag (an endpoint of the relations read) with
its country's counters and unknown losses, counted both as its own scan counts
them and as they move for a country seen before. A block with a truncated
table read by some of its fields only, whose census, read or continuity
changes after its first hour, or whose quiet hours do not continue a covered
supplying read, is observed hour by hour.

The family's own thread folds the blocks in order and observes no hour but
each block's first on the carried observer. After that hour both scans hold
the same census, relations and naming snapshot, and differ only by what
earlier blocks left: each country's totals by its ended lifetimes' counters
(a difference that holds through the block), unknown-loss totals, gap
recovery flags, the read sequence and generations (an offset), country
indices, tracks, keys and pair history. For each later hour the fold moves
the carried read frontiers, holds the lifetimes first held there, samples each
changed tag's track with the extract's values carried over (an unknown total
counted on from the carried total for a country the carried scan saw, as the
extract counts it otherwise), and publishes the tags' values and
observations and labels new names with the code observing an hour runs. After
the block it takes the relations' counters, reads and generations, the
countries' totals, unknown losses and last reads, and the pair history's runs,
numbered as the carried history would meet new lifetimes and marked
incomplete as the carried scan marks their endpoints. When the carried scan
does not continue the first hour as an hourly supplying read with nothing to
join, name or recover, holds a lifetime the extract first held later, or its
pair history does not end at that hour as the extract's begins, the rest of
the block is observed hour by hour. Each block's pair window is rendered at
its end and written at the family's next commit before its flush, also when a
later block of that batch refused the branch, as hourly preparation wrote it
at the block's end; the continuation hourly preparation keeps in its branch
state is saved as `war-casualties-state-<branch>`. The breakdown also reads the wars' change
inputs, which are prepared on their own too: it waits for the casualties' and
the wars' joint coverage (`independent::JOINT`).

Country totals retain ended relation lifetimes and unknown-loss recovery across
reads, so there is no stateless progressive sampler for them. A state read past
the casualties' coverage answers at its last hour, as a read there does, with
the requested hours after it `pending` ([`prefix`](../dashboard-base/src/prepared/prefix.rs));
the widget draws those totals with the shared preparation note. Width summaries
past coverage stay pending.

Casualty pairs share that observer. Each replay block publishes unfiltered
native-lifetime pair records through the shared compressed reply helper. Queries
apply the product's native-holder and `between` selection before summing distinct
lifetimes with its existing window builder. Availability comes from accepted
block observations, including a complete empty relation census. While the
casualties' coverage is partial, a window not yet published answers from the
prefix's newest window (its reply's `window`), whose lifetime counters are the
flows at the prefix's last hour, with the hours through the requested window's
end `pending`; a manifest that has met no pair yet is marked so too. The window
manifest uses the source's existing block catalogue. Continuation retains
endpoint tags and counters, discarding closed pair runs after each block.

## Prepared casualty breakdown

`casualty-breakdown` reads the shared unfiltered casualty window and the single
typed `wars::prepared::projection`, with the selected native holders and no
casualty hydration. The existing breakdown projection and reply construction
join native lifetime counters to viewpoint memberships, preserving app IDs,
names and grouping reads before the requested window. The source's block
catalogue supplies the manifest; shared casualty coverage supplies availability
and gaps. Queries reuse the Wars change walk the source keeps for the tag and
its native holders, walking only change inputs published since the last query
for them, and read one casualty window without rescanning the recording. While
the joint coverage is partial (`query_through`), a window ending after it
answers from its newest window (the reply's `window`), whose lifetime counters
are the losses at its last hour, observation stops there, and the requested
hours after it are `pending`.

Pair continuation retains one open run per key so its original supplying
boundary survives publication. The casualty catalogue keeps endpoint presence
and uncertainty changes, independent of hourly counters, to retain earlier
supplying boundaries and the whole-branch overlapping-holder refusal. The existing holder continuity walk validates both
paths. No second casualty observer, war grouping, selected reply store or
projection cache is added.
## Prepared equipment stockpiles

One frame joins stockpiles, lines and logistics for every holder. Preparation
and its test oracle use the same frame step and reply projection. Shared
integer series retain each variant's stock, output, IC and cost operands, each
logistics type's balance, stock and available operand, independent row presence,
coverage, and supplying reads. Known absence contributes zero; an explicit
unknown row remains unknown. Gaps and replacement holders end carries.

An hour that continues the previous one with the same holders and dictionary is
observed from its changed rows (`FrameChanges::rows`). A changed or deleted
Stockpiles or LogisticsEquipment key names its holder by its country reference; a
ProductionLines change to a field a holder reads (country, kind, variant, cost,
speed) names the line's old and new holder through an index of each line's
country. Such a holder is read from its own rows, gathered by key across its
links, at that table's next read, and a holder whose variant changed group is
regrouped that hour. Every other holder's read repeats its last read's dates and
diagnostic counts, and its values carry. A holder is stepped through the hours it
was not visited in one span when it is next visited, and publication steps every
holder through the last accepted hour before sealing. Within a holder only
values that started, ended or are new to it are observed, series keys are
resolved once, and every group is totalled in one pass over the members.
Keyframes, resumes, gaps,
holder or dictionary changes, and a table becoming or ceasing to be complete read
the whole tables; so does a changed numeric table without recorded row deltas.
A whole-table hour that does not read a table leaves every holder's next read
of it a full one.

Observation-time type and domain stock, balance, need and drawability use aggregate
series. Dated classification intervals include unnamed periods and changed tokens;
the latest nonempty token classifies the whole numeric history. Queries split
membership spans by the affected intervals, use summaries for unaffected pieces,
and apply signed adjustments to affected aggregate runs. Unknown aggregates are
reconstructed from final members and unresolved completeness operands. Explicit
Included-types domain filters merge the selected types' corrected change points.
Integer extrema below 2^53 use their summary supplying date; wider buckets walk
only that bucket's runs for the exact maximum.

Naming and refusal changes are dated events. The checkpoint retains current and
last-known naming, open numeric carries, refusal identities, observed design
fingerprints and the first design/read witness. Date replies and independent
naming pages preserve canonical supplying bounds across publication and resume.
Native alias and cohort queries clip numeric coverage before composing pages.
Before the stockpile's coverage completes, a window of one hour answers from the
recording rather than null (`sample_at`): the frame step over the hour's block from
its keyframe, and over earlier blocks while the country's stock and logistics
reads by the hour are not all found within the tables' daily read cadence, so
each last read carries to the hour as preparation carries it. Its values at the
hour are exact; a design's type is its latest nonempty token by the hour, the
branch's own at its last hour. The reply carries the coverage with no hour
pending and no prepared display, whose automatic choices describe the branch.
Wider windows wait for complete coverage.
Ratios use exact integer operands before the legacy float division. Dense charts
receive screen-width group summaries; exact charts retain legacy coalescing and
walk only their supplying edges. Stockpile parity models are labelled
"Equipment stockpile exact values" and "Equipment stockpile breakdown exact values".

The ignored 5,000-country year and real-recording tests report prepared bytes,
checkpoint bytes and bytes per observed country-year. The real test also checks
the complete canonical reply against the recording oracle, reports date/display
query timings and logs changed nonempty tokens outside Git.
Stockpile preparation binds equipment outcomes before observation and uses the
shared keyed content policy for fixed token classifications. Holders are
stepped through carried frames when next visited; publication steps every holder
through the last accepted hour and seals it. Naming publishes
only changed variant IDs. Pending naming refreshes survive carried frames and
restore until the next complete native supplying read rebuilds the projection.
Numeric and group observation use current members and
removed members once, while historical catalogues remain available to queries.

The stockpile is prepared on its own. Its blocks are read by the countries'
keys, slots and tags, the links and every field of its numeric and
variant-catalogue tables; each stock, logistics and variant read hour is
visited, and the next one by the clock is the extract's deadline. Hourly
variant changes also trigger naming between the daily variant reads. A quiet span
advances the series and the catalogue's last hour in constant time
(`Prepared::advance_quiet`); holders catch up through it in one step when next
visited, with the same saved frontiers as advancing each hour. A kept hour
carries no recorded rows, so a changed numeric table reads the whole projection;
before a loaded branch's first whole-table hour has built the row index,
the rest of the block is observed hour by hour.
Its equipment content is bound before the first block it observes. It saves its
continuation as `stockpile-continuation-<branch>`, buffering JSON before compression
with the hourly branch writer's shared helper, and answers from its own
coverage.

## Prepared equipment losses

Incomplete loss prefixes mark all presented recorded hours approximate: native
sinking dates and later type tokens have no finite backward revision bound.
`PreparedLossesReply` exports land spans as `landCoverage` and preparation progress
separately. Completion removes the provisional marks.

One decoded frame feeds every native holder's existing loss ingestion. Division
census and variant naming observations are shared. Keys exist for native
countries, observed cause/design leaves and observed variant naming histories.
Cumulative signed leaf values and supplying changes use the shared series
accumulator; changes to coverage, refusals and naming keep their existing product
shapes in compressed metadata ranges. Destruction and Combat Log selections use
the same retained operands and the product's existing reply builder.

Equipment-loss requests use prepared data; the recording scan and its loader
remain test-only parity oracles. A reply's root `coverage` and `pending` are the
prepared prefix it answers, as for every prepared reply; its land coverage spans
travel as `landCoverage`. Before the losses family's coverage completes (with
the identity history prepared), each reached hour after the prefix and the
viewport end at the prefix's last hour, which the reply's prepared reads and
head and detail hours name.

Shared frame-change facts refresh current native owner joins. The country
identity projection excludes unrelated balance changes, without comparing the
whole census when the recorded delta changes no slot or key. Combat Log reads
select a raw owner's contiguous recorded-key range, including truncated reads;
no hourly log index is rebuilt when another owner's rows change or are pruned.
Amount-only destruction deltas with unchanged keys keep their owner join and
availability operands, while consuming every hour's signed amounts. Each
country's typed read key is compared before serializing its saved text.
Changed or supplying rows visit their owners; retained holder and design keys
carry through one authenticated series frontier. Open coverage and uncertainty endpoints extend
when touched or published. Equal lend-lease reads advance one supplying frontier,
and resolved current owners retain deletion witnesses across resume. Disappeared
holders leave that runtime join; returning holders rejoin from their retained
exchanges. While Countries or CountryLinks is incomplete no holder resolves, so
every retained holder is observed at each such hour and at the first hour both
are complete again. Nonempty destruction drains add their signed removals every accepted
hour, including equal drains. Pending variant operands resolve at their first real supplying
read. Static and empty naming results keep their original read dates.

Sunk-ship records use sparse country events in observation order. Each event
retains its native sinking hour and signed operands. Queries apply those native
dates through the existing points builder, including sinkings first observed
after an earlier prepared commit. Other causes use their observation hour.
Queries read requested viewport summaries, exact reached dates and palette
extrema, keeping wide integers exact. Navigation uses shared series navigation
and the sparse native ship dates. The widget requires exact reads for hover and
picks and retains pending values at their previous reply's own hour.

Every cumulative loss leaf needs retained history: hourly destruction drains,
closed or pruned Combat Log keys, convoy resets, ended lend-lease exchanges or
earlier ship observations. These fields keep their coverage gate and answer
pending outside it. The widget uses the shared preparation note; no stateless
progressive sampler is registered for them.

## Prepared production selectors

Before completion, selections mark their accepted hours approximate: later type
tokens can reclassify earlier designs, and an open supplying interval can receive
a Rocket join repair. Production spending also marks its provisional type facts.
No fixed backward window certifies those branch-wide facts. The automatic units
choice can use `typeTotals`, known totals by type before paging, until its exact
whole-history choice arrives. The first caption identifies the choice as provisional.

`prepared_production` incrementally consumes validated selected frames. Its retained
row counters keep final publications, lost-output exactness, refit attribution,
country holders and independent Rocket coverage. The recording's format is passed
to the shared supplying-read and variant/Rocket observers. Grouped replay blocks
retain successful-read markers without inventing nanosecond timings. Each
time-major endpoint header covers an inclusive interval with unchanged supplying
reads and facts. Headers are compressed JSON and omit amounts and classifications.
The existing spent series holds exact changed-leaf and classification events;
unchanged retired amounts are not republished. A read joins the dated header with
each key's latest event, retaining exact signed quantities, native holder/segment,
refit, naming and unknown operands. Removal events close absent leaves. Hash-only
deduplication and current key membership survive checkpoints and appends without
retaining another copy of the accounting quantities. Classification-event keys
share the existing catalogue's numeric namespace with spent keys. A small
separate time entry per page supports binary search without a resident campaign
index. The serializable accumulator retains current reads and counters; appended
frames continue it without rescanning the prefix. Publication keeps one stable
accumulator and classification catalogue per branch. Each country keeps one
independent Rocket endpoint, including hours without readable equipment output.
Equipment selectors resolve their Rocket binding at the equipment supplying
read; join validity retains the original complete supplying-interval rule.
Production observation uses the shared frame-change facts and resolved current
country joins. Carried hours advance one accepted frontier; changes, gaps and
publication materialise open endpoints and spent runs. Endpoint pages retain the
unnamed-holder count before their accepted span, its hourly rate and its position
in the recording warnings. Reads render the exact warning count at the requested
hour, without accruing unavailable hours or gaps. Retired country aliases keep
their existing catalogue facts and take no part in current output observation.
A lost Rocket supplying
interval repairs only its affected equipment interval, including already accepted
compressed headers, so the joined totals match the whole-branch reference.
Type-only queries read the catalogue even when the requested hour has no endpoint.
Before production's coverage completes, a selection after the prepared prefix
answers at the prefix's last hour (each source's calendar predecessor is no
later), from the catalogue that prefix committed, and carries the coverage and
`pending`.
One query's endpoint reads (`prepared_production::Endpoints`) share the catalogue,
its parsed leaf tokens and one leaf reader, and look up only the leaves they
restore: a selection restores every country's leaves at its hour, the Rocket
binding only the joined countries' Rocket leaves at each supplying read, and
Production spending only its country's leaves.

Subset queries load the current endpoint and each source's exact calendar
predecessor. They sum signed cumulative quantities before representation conversion,
preserve missing coverage and unresolved-type ambiguity, and rank nonzero known
totals with tag-order ties. Explicit country choices retain request order and
deduplicate tags; ranked selection sorts them by exact known quantity before paging. Only the requested page receives grouped values. Wide quantities
and native variant identities are decimal strings. Differential checks retain the
whole-history equipment projection as their oracle.

Production spending reads the same endpoints and classifications and calls its
existing reply builder. Presence metadata preserves a complete empty census,
its supplying date and independent Rocket absence. Endpoint segments decide
range continuity; campaign spending reasons preserve lost publications, gaps,
country changes and observation restarts independently of delivery exactness.
Already computed new/refit/combined and Rocket leaf amounts also feed one shared
series accumulator, without repeating their accounting. Shared width summaries
locate range extrema, whose raw values are read as exact integers; wide buckets
resolve float ties through their exact runs. The sunburst reads only its reached,
final and range-baseline dates and exposes Production spending values for parity.
Before production's coverage completes, a reply answers its prepared prefix:
each date after the prefix reads the prefix's last endpoint, with the catalogue
that prefix committed (its designs and type facts so far), and every run ends
with the pending hours, unknown with the reason `preparing`. A range whose
baseline lies after the prefix reads no baseline; all of it is pending.

Production is prepared on its own. Its blocks are read by the countries' keys,
slots and tags, the links and every field of every other table it reads, since
it ingests again at any change of their allocations; each output and variant
read hour is visited, and the next one by the clock is the extract's deadline. A
quiet span extends the open endpoint page and advances the spent series in
constant time (`ProductionAccumulator::advance_quiet`); without an open page,
the rest of the block is observed hour by hour. A page a kept hour completes is
published as on the coordinator, its endpoint and time writes held in order
(`HeldPages`) and written at the family's next commit, before its flush, which
publishes the open page and then writes the catalogue and spent series, as hourly
preparation's flush steps do; the store holds what publishing each page at its
frame writes. It
saves its continuation as `production-continuation-<branch>`; selection and
spending answer from its own coverage.

At a complete variant read, production's single-read catalogue reuses the
production-line input reader (`equipment::VariantReads`): only changed variant,
module, upgrade, country or token operands are rebuilt. Each tracked variant is
classified again only when those operands changed, while its name still takes
the current read's date. Carried and incomplete reads see an empty catalogue,
as the hourly frame's catalogue does. Removed definitions, gaps and unread names
keep their existing unknown-name bounds and accumulated refusal identities.
Endpoint events serialize their payload once for hashing and publication.

## Recorded histories

Preserve exact recorded values and distinguish zero, absent rows, unavailable
reads and values that cannot be computed. `read_runs` tracks known coverage;
outside it, an absent run cannot establish zero. Gaps and identity changes end
continuity. Never backfill a missing hour from a later observation.

A held daily value retains its **supplying read hour**. Use `metadata::slot_read`
with the recording's format (a table is dated at the cadence that format read it
at) and check table completeness; a held table or an unchanged value is not evidence
of another successful read. Keep read confirmations separate from numeric
changes. Carry values only over the coverage allowed by the supplying table.
Cadence belongs to [replay](../replay/README.md), not a product's private schedule.
Keep native event dates, observation dates and publication/read dates distinct;
first/last sightings establish observed bounds, not an invented exact start or end.

Tags are opaque lookup keys, not paths or a fixed spelling format. Resolve raw
country references through recorded CountryLinks. Native identity is
`(canonical slot, country_id)`; a slot or tag alone is insufficient. Shared
country-history identity runs let manual picks follow the holders recorded
under selected tags through renames and aliases, without including unrelated
later holders of an alias. An origin-hour selection binds the holder at that
hour; smart groups use identities at their membership dates. Ambiguous links,
missing hours and a replacement holder break continuity; do not infer identities
from tag-change events or bridge a gap.

Metadata's country catalogue is an initial observation, not the campaign census.
Initial selection uses that frame's LocalPlayer, including observer choice,
joined through CountryLinks; the multiplayer human roster does not identify the
recording client. A terminal partial marker is separate from branch gaps: an
unknown failed hour supplies no inferred gap. Branch metadata lists recorded
event losses (`eventLosses`) apart from gaps: the recorded events between their
two reads are incomplete, and a frame lost at either read is its own gap; the
event feed names the detached recorder beside any recorded failure of the
interval one ends. See
[replay compatibility and recovery](../replay/README.md#compatibility-and-recovery)
for readable prefixes and field availability. Refuse only dependent fields that
were not recorded when the rest of an older recording remains usable.

## Content and text

Resolve content through the recording's stack and the shared `catalogue`;
localisation, native GUI/sprite lookup and texture decoding must not acquire
parallel parsers. The [installed-content contract](../installed-content/README.md) owns content
verification and uncovered-read reporting. Keep its named, counted diagnostics
visible after lazy reads. Missing artwork leaves independent text and values
available with its reason. Native GUI templates are read once on the first
template-dependent lookup, preserving the shared registry and post-load pass;
focus text and focus images do not wait for GUI parsing. GUI read failures remain
named refusals for dependent fields while independent localized titles stay usable.

Focus text-only requests and research history resolve text independently of sprite
rendering. Focus cards request localized titles and icons without expanding
descriptions; opening their details loads the complete dated description. Focus
`iconsOnly` requests omit text evaluation and its unrequested operands, while
retaining the complete authored icon selector at the recorded hour. Other focus
requests preserve their combined presentation fields. Focus
hour reads stream selected tables only up to the requested hour and retain frames
after the replay reader verifies the block. Technology images use the separate
`technology-artwork` product.
First-use focus history scans run alongside shared content preparation. The
preparation worker is fully joined before returning or unwinding; a refused scan
keeps its own error, and pre-cancelled requests start neither task. Populated
focus contexts use the normal shared catalogue freshness check.
Selected-country focus history also retains its new order segments' first-hour
Campaign, Countries and CountryLinks views in that same bounded frame cache.
Static completion images reuse those verified inputs without another recording
read; scoped presentation still upgrades to its complete selected-table kit.
Seeding preserves sampled provenance and previously scoped entries, and retains
neither focus/completion rows nor a second list of decoded hours.
Cached hours stay available during reconstruction misses. Misses serialize and
recheck the cache after admission, keeping shared reads and cancellation local
to the requesting lease.
The shared `catalogue::CachedResult` retains each demanded typed database's
success or named failure in the existing source/recorded-stack cache. Concurrent
cold requests perform one load; cancellation leaves the entry retryable. Changed
content bindings use separate results.
`catalogue::KeyedResults<K, T>` applies the same result policy to each recorded
operand's fixed projection in an ordered map within the existing typed source or
resolved-stack slot. New spellings resolve on first sight; repeated spellings
reuse the same success or exact named refusal. Concurrent first access resolves
each key once; Interrupted remains retryable, and different keys load independently.
The catalogue's text refusal tally is shared by every consumer of the source; a
consumer whose output must not depend on the others keeps its own
([own refusals](../dashboard-text/README.md#own-refusals)). A prepared query
reply's content warnings gain the stack's current artwork-effect and
uncovered-read diagnostics where it is served (`Source::append_stack_warnings`),
as product replies gain the read set, at each place the reply keeps them: the
top-level `warnings` of most replies; `contentWarnings` of battle and division
logs, whose `warnings` are recorded battle warnings; both `contentWarnings` and
the classes reply of the sinking manifest; the wrapped reply of sinking
windows, game-score states and the scoreboard; and each country's reply in fuel
series.
Focus art binds its catalogue and
definitions once per content binding; a refused binding is each reply's named,
counted failure. Scoped interpreter state and dynamic presentation are
evaluated at the requested hour. Scripted dependencies stay
lazy until a recorded token requests them, retaining both successful and refused
loads without exposing diagnostics from unused definitions.
Focus/technology definitions and rendered sprite frames are shared within the
resolved content stack; concurrent identical images render once. The image cache
has a byte budget, without limiting content inventories or reply completeness.

Sprite images select an authored strip frame at time zero. All sprite images
run the stack's Up program through [shader-pipeline artwork](../shader-pipeline/src/artwork.rs),
including masking and active animation records; PNGs carry straight shader output
for the host to composite. Effect/compiler/device failures remain named refusals.
Trait assignment icons use registered spelling and the native GUI constructor's
fallback retry; a declared but refused sprite never takes that retry.

Use the shared localiser and plain-text helpers. Stored display text without a
proven localisation key keeps the shared recording-language guard; a literal
never becomes a key. Scoped text, triggers and descriptions use the same
recorded-hour evaluator and caller-owned event graph/variable session. Missing
operands and unsupported reached forms retain named, counted refusals, without
invented scopes, roster order or rebuilt native caches. Supported forms and
refusal handling belong beside those helpers.

Leader visibility reuses an evaluation while every input
it recorded reads the same ([evaluation reuse](../dashboard-text/README.md#evaluation-reuse)),
confirming from the hour's change facts (its `FrameChanges` in the hourly walk,
the block reader's report on its own) the inputs whose declared dependence did
not change.

Scripted text and trigger worlds read province-neighbor graphs and movement maps
only when a reached neighbor, coastal or terrain operand needs them. These reads
share success and refusal caches within the resolved content environment, and
their diagnostics join the shared catalogue after loading. Political and constant
scripted text do not prepare those geographic inputs. Effect descriptions reuse
the resolved stack's complete key-name registry without rereading its databases.

Other scripted and trigger operand databases also load only when reached, using
the environment's OnceLocks. Their readers retain the native load points,
refusals and diagnostic order. Evaluated defines and registered equipment are
shared after each consumer performs its ordinary verified content reads.

Small requested focus and technology image batches share one GPU submission and
readback. Temporary batch payloads have a byte budget; oversized or failed
batches use the individual image reader, preserving each named refusal. Rendered
images use the session caches, and GPU uploads reuse already-read decoded bytes.

Faction naming preserves complete empty bindings and successfully localised empty strings as known empty text. Missing inputs retain a null name and a named reason; a literal never becomes a localisation key.

Place-name consumers retain successful empty state/region text separately from unavailable inputs and preserve each caller's native naming ladder. Army composition treats an omitted authored sub-unit group as the known token-zero default; offline inventory does not prove that every air/naval definition reaches an Army grid. Equipment icons retain own-type, technology, family and pool fallback for a null archetype, keeping real empty equipment names distinct from that null object. Political names, descriptions and sprites use the selected definition's authored spelling after a case-alias lookup.

`content_check::check` visits products in registry order, shares preparation per
stack and calls their real loaders. Offline findings belong to that preparation,
not later live replies. Preserve producer diagnostic kinds: `Content` requires
supported content behaviour, while `Offline` identifies a recording dependency. Keep
counts/examples separate from diagnostic identity and avoid duplicating
reader-owned notes in product checks.

Country-people preparation checks its three recruitment defines through the runtime cache. Invalid or missing defines produce named refusals; manpower, exile resources and elections still require their recorded operands.

## Service and limits

Recording files use the standard filesystem reader. Metadata grows country
catalogues fallibly and propagates allocation failures. Product read-ahead uses
checked block storage and fallible worker creation; failed admission stops and
joins already-started readers.

Each loader job catches Rust unwinding across opening, country naming, metadata,
product loading and reply creation. A panic becomes that request's error and the
loader continues; synchronous source opening has the same boundary. Block
decode workers retain their own panic isolation. This boundary requires an
unwinding Rust build and cannot catch native C memory faults or process aborts.
The [recording fuzz targets](../../fuzz/README.md) exercise metadata and native
block decoding with synthetic input; the root [dependency checks](../../README.md#recording-and-dependency-checks)
cover RustSec and npm advisories.

`Service` shares sources by canonical path and revision and issues independently
releasable leases; `open_map` uses the same sources. An outdated expected revision
is refused. A new revision uses a separate source and caches. Every product reply
echoes the requesting lease, revision and key so it cannot answer another recording.

Loader jobs run off rendering in four bounded lanes, each with its own threads
([service.rs](src/service.rs)):

| Lane | Jobs | Threads |
|---|---|---|
| History | Whole-recording products, opening, map settings, preparation refreshes | A quarter of the machine's hardware threads, at least 3 |
| Interactive | `Product::INTERACTIVE` reads (one hour or one block, for hovers and labels), the history cache's size, preparation actions | 2 |
| Prepared | Every prepared query, first | 2 |
| Sampled | Prepared queries that sample recorded blocks | Half of the hardware threads, at least 3 |

A prepared query runs first on the prepared lane with block decodes refused
(`progressive::without_block_decodes`): an answer from prepared pages alone, or
from the source's frame cache, is given there at once, so it never waits behind
sampled reads. A query whose answer needs a block decode stops at that decode,
answers nothing, and runs again from the start on the sampled lane; the partial
reply assembled on the first attempt is not sent. Compact block observations and
prepared facts remain in their bounded caches. A sampled read holds one block's selected frames,
the two read ahead of it and the source's frame cache (dashboard-base), so the
sampled lane's transient recorded-frame storage grows with its thread count.

Series summaries and states, army size, trade flows, air superiority, event feed,
supply health, party history, division log manifests and air aces retain their reply data in the shared
observation cache as branch entries, separate from block observations. Each
branch and parameter selection keeps at most one current answer. The store epoch,
numeric publication identities of gates and declared suppliers, current
localisation key, content folders, cache folder, unverified-content choice and
texture decode setting bind its validity; the observation scope also binds the
recording and recorded content stack. A changed binding replaces the previous
answer. Replies also retain decoded series blobs, parsed event rows and rebuilt
catalogue/reply text, alongside the numeric prefix folds used by that request.
A changed publication offers the completed predecessor without waiting for an
in-flight reply; readers validate immutable page identities, text encodings and
bucket intervals before reuse. Only entries visited by the current ask survive.
Their allocation capacities are charged with the reply, so all selected series
share the request's retention and eviction. Branch replies share the
product's bounded LRU with block observations:
largest observed entry times live blocks, evicting the least recently used entry
across selections and viewports. Retained replies with unchanged bindings reuse
their projection; supplier refusals are checked before lookup. Live content warnings
and transport fields are added when serving each response. In-flight replies that need recording reads retain the ordinary
prepared-lane refusal and sampled-lane retry.

Prepared transports mark a re-ask of an already shown reply as a refresh. The
prepared and sampled queues prefer queued first asks, FIFO within each class.
While refreshes wait, at most eight consecutive first asks are admitted before
one refresh. Serving a refresh or finding none waiting resets the count. The
browser's connection queue uses the same bound; running work finishes normally. The mark stays with the
request when a prepared attempt moves to the sampled lane. Navigation and new
request parameters start as first asks.

Each lane uses the existing request admission budget for queued jobs.
`Lanes` owns one non-cloneable sender per lane; dispatch and preparation
refreshes borrow it, and desktop runtimes share the service. Workers share one
receiver through `Arc<Mutex<_>>`. Dropping its final owner disconnects sends and
releases queued work; dropping the sender lets the receiver drain before closing.
Cancellation releases the frontend request but its queued job keeps occupying
channel capacity until a loader physically receives it. Full lanes refuse new
work with the existing retryable capacity error; bodies and country selections
are accepted whole or refused whole. A query moving to a full sampled lane is
refused the same way. Preparation refreshes share the history-lane bound and
coalesce to at most one queued refresh per source, preserving a requested
restart. A full lane refuses refresh admission without waiting or creating a
separate retry queue. Ordinary product caches serialize their own requests; scans with
their own gates, and hour reads, do expensive work outside short cache locks.
Open/append work stays outside the global sources lock. Check cancellation during
reads, scans and serialization, and never publish a cancelled partial result.
Different products and sources can run in parallel; replies are fragmented and bounded.
Its hosts (the viewer and `dashboard-bridge`) allocate with mimalloc: the Windows heap
slows every later scan several-fold once the service holds gigabytes of caches.

Whole-branch replies persist in `reply_cache`, in the `history` folder beside the
content hash cache, so a reopened campaign answers without scanning recorded history.
An entry is keyed by the recording identity, the cache format, the build,
the product and parameters, the content folders, the unverified-content choice,
the game language and the effective texture decode settings. Held recordings
use a BLAKE3 hash of the canonical path and opened metadata revision; synthetic
in-memory recordings hash their bytes once per source without inflating blocks. A reply must depend on nothing else. Content-dependent replies
are stored and served only after the source resolves and verifies its recorded
content stack for this open; a failed resolution leaves live replies available
with their refusals. Recording-only products do not require that verification.
Replies are neither stored nor served while the source reports dependencies
outside the recorded content coverage, including explicit uncovered reads and
missing candidates. Compressed entries, streamed decoded replies and decoder
windows are bounded by the transport reply budget; oversized, corrupt or obsolete
entries are misses and can be replaced. A stored reply
keeps the catalogue's text-refusal lines of the session that computed it. The folder is
disposable; the least recently used entries go past `reply_cache::LIMIT`.
Settings measures and clears it through `DataCommand::HistoryCache`, on the interactive lane.

Measure complete product replies against `products::MAX_PRODUCT_REPLY_BYTES`
with `transport::json_len` (or the shared budget helper), including identity,
read-date and warning metadata. Refuse oversized replies explicitly; never
truncate countries, entries or runs to fit. Preflight large allocations as well
as checking final serialized size. Request envelopes share
`protocol::MAX_MESSAGE_BYTES` across the desktop host, native reader and browser
bridge. These are byte/retention budgets, not chosen country limits: size buffers
from recorded data, design for 1,500 countries and verify at 5,000 synthetic countries.

Capacity errors use `transport::TRANSIENT_ERROR_NAMES` and its shared classifier;
the generated frontend contract exports the same names. They decide no country
fact and can be retried. Other errors remain failures until an explicit retry or
content/configuration change, never successful empty data.

Opening reads only each branch's first readable block, and only the catalogue
tables, as far as its first complete country table. An unreadable block there
becomes a branch gap and the next block is tried. Metadata reports the parts the
reader skipped or found unreadable while opening, their known hour range and
bounded reasons, including readable trailer checksum failures. Damage elsewhere is
found when playback reaches it: unreadable blocks are gaps and failed tables are
unavailable. Campaign keys
use checked hexadecimal allocation. Metadata retrieval and country naming use
ordinary clones. Recorded gap reasons, partial notices and table availability
diagnostics remain exact; only generated damage diagnostics use bounded excerpts.
Failed opens return a plain damaged-recording or insufficient-memory sentence
followed by the technical reason for bug reports.

## Check a change

The [prepared slice acceptance harness](acceptance.md) runs real background
preparation and host-direct pages against the existing focus, equipment-produced,
country-measure and replay readers. It writes exact differences, query/reopen
timings, reply bytes and checkpoint read counts outside Git, and its readiness mode
times every widget and the map from open ([readiness](acceptance.md#readiness)). Its launcher isolates
process/content caches; supplied game and mod installations remain read-only.
Native viewer and frontend presentation checks remain separate.

Tests that parse installation-dependent content or authenticate modifier names explicitly initialize shared metadata from `BATTLE_PLAN_GAME_DIR`. An unset variable fails with a named setup error. Recording metadata, numeric history and transport tests that use no executable tables remain installation-independent.

Follow [AGENTS.md](../../AGENTS.md) for crate-scoped formatting, clippy and
affected synthetic tests. Exercise changed identity, gap/read-date, mod-content
and complete-reply-budget behaviour at the
required scale. Capture integration also needs the relevant fresh-save oracle
and smoke workflow in [recorder](../recorder/README.md).

Rust owns [ui/src/generated/dashboard.ts](../../ui/src/generated/dashboard.ts);
do not hand-edit it. From the repository root, generate changed types and check
that the committed contract matches. The `export-types` feature enables contract
assembly and the registry's declaration entry points; ordinary host builds omit
them. Unit tests retain the same contract checks without needing the feature.

```powershell
cargo run -q -p dashboard --features export-types --bin export-types
cargo run -q -p dashboard --features export-types --bin export-types -- --check
```

UI changes also need `npm.cmd run check:ui` and `npm.cmd run test:ui`, as described
in [ui](../../ui/README.md). Documentation-only changes need link and diff checks.

Occupied-state preparation shares one daily join with occupation trends. Each replay block publishes its standalone state window through the shared reply helpers. Compressed continuation retains the carried state values, supplying read, laws and exact refusal identities. Trends publish fixed5 resistance/compliance, composite context events and rank-position runs through the shared series accumulator. Queries read one window, one reached hour or an aligned width summary; gaps and native holder selection remain explicit.

A window preparation has not reached is answered exactly as it will be written, from its block and a look-back alone (`occupied_states::prepared::look_back_window`): a fresh observation of the block before it, then of twice as many, until it is anchored at the window's first hour, holding a supplying read or having dropped its carried state (a gap, an unreadable hour, an hour without resistance rows) as the branch's observation does there, or starting at the branch's first block. Every later value is then the branch's, since each daily read restates the occupied states and the hourly projection reads the hour's tables. Its blocks are read ahead together, and it refuses where block decodes are not allowed. It carries the laws its states hold (`laws`), which the manifest published so far may not list; it is null while one of its blocks cannot be read. The manifest answers the branch's windows with the laws and warnings of the windows published so far, with its prefix's `coverage` and `pending` (an empty extent before the first).

Occupation is prepared on its own (`prepared/pipeline/independent/evaluated.rs`, registry name `occupation`), gating `occupied-states`, `occupied-states-manifest` and the `occupation-trend` series (whose identity metadata stays preparing until the identity history is complete). Its daily reads decide law eligibility through the trigger world of the read, and its owner and controller projection follows the allocations of the views it holds, so its blocks are not read by their changes. Its own thread observes each block hour by hour, as the hourly walk does, on the block read alone with its tables (states, the daily occupation tables, the countries' keys and links, the dictionary, and the trigger kit `triggers::RECORDED_TABLES` when a bound law has a visible or available trigger) and with the change facts of the block alone. It has no pre-pass lane (`Own::laned`): no block is read on the pool for it. It binds its content at every block, holds each block's window page and manifest from the block's end and writes them at its next commit, and saves its continuation (`occupation-<branch>`) after its pages, as hourly preparation does. A measured run times its block reads (`occupation.read`) apart from its observation (`occupation.observe`).

Occupation trend reached values reuse the proved occupied-state window's
`trend.history`, clipped to the requested hour. Wider unprepared geometry samples
each block at its first supplying resistance read, using the same observer and
arithmetic; summary spans are approximate. Exact reached identity evidence
replaces sampled aliases. Branch-wide `identities`, `segments` and segment-start
metadata remain listed in `preparing` until identity preparation completes;
coverage continues following that gate. Numeric state counts and collaboration
are never taken from summary geometry. Proved windows and first-read observations
use the shared bounded observation cache, with content-bound keys.

The land-owner observer extends one recording-wide compressed state while preparation visits every branch. The existing ownership step retains one lowest-branch witness per tag and canonical slot; unchanged hours add no state. The query reads the single `land-owners` range and calls its sorted reply builder. Accepted completeness belongs to the whole recording.

The occupied-state, manifest, occupation-trend and land-owner product loaders and
their replay walks and branch caches compile only in tests as parity oracles.
Production retains the daily joins, carried state, native interpretation and reply
builders used by preparation and queries. Validation, types and offline content
checks remain available for all these products.

Navy preparation calls the native ship census step and retains its open tracks and dated class dictionary. Shared series store count/current HP, independent maximum HP, classified parts, census events and rank positions. Window queries call the native reply builder; scoreboard ship visitors retain the oracle scan.

Nuclear arsenal preparation reuses the stockpile scan and reply selection, including native signed arithmetic and origin identities. Equal counters use shared runs; every complete supplying read, including empty reads, is an append-only event. The continuation retains only open native tracks and counters.

The same changed-mixture boundary publishes positive whole-stockpile increases
to shared series Events in `nukes-increases-{branch}`, under global key `0`.
Adjacent known totals within a segment supply the signed widened difference;
the changed mixture supplies both native and read dates, with `exact=false`.
Rows retain the native holder, a null type ordinal for the whole total, and
recorded type tokens/support metadata for request-time presentation. No part-only
increase, unknown total, decrease or segment start implies an increase.

`nukes-increases-{branch}-catalogue` uses the shared catalogue writer. Its metadata
holds coalesced complete-read spans, refusal-reason spans, native unsupported-type
counts and recorded type tokens/support metadata. Every supplying census updates
availability independently of increases. Native named types remain available to
strike-only campaigns, and names resolve from recorded content at request time.

Factory preparation uses the native scan for counts and supplying reads. Shared series retain every field, compressed read provenance, value events and ranks for the three count modes over each nonempty subset of the three recorded groups. The native count and reply builders answer both prepared windows and oracle requests.
Ace preparation reuses the roster and variant observers. Changed variant versions
append directly, while roster supplying reads use shared runs. Equal roster reads
advance one supplying frontier; active read/alive bounds and portrait endpoints
materialise at changes, gaps and publication. Portrait selection retains only
changed operands per ace, with an effective endpoint for open-state replies.

Raid preparation uses the existing raid observer. Completed fact/preparation chunks
append as shared series events, while per-country launch/success tallies and
identity coverage use shared runs. Continuation retains only open episode suffixes,
country counters and compact native-reference witnesses. Prepared log and tally
queries reconstruct their requested data and share the registered reply builder;
country selection uses the prepared identity history. Catalogue availability reads
recorded evidence from continuation without a recording scan. Missing or invalid
raid content leaves the observer's content unavailable while recorded launch and
success witnesses continue to accumulate. Content loading refusals remain local
to raid queries, preserving the registered product's behaviour. Content success
and failure outcomes bind before frame observation. Shared frame-change facts
retain current identity and parent joins; equal reads advance scalar presence
and episode frontiers. Shared tally/read runs carry unchanged values, and held
preparation values preserve the exact transition from a rising to a flat segment.

Air-superiority preparation joins recorded score owners once and writes friendly,
enemy, assigned-wing and active-region values through shared series runs.
Identity/score coverage and dated theatre operands append as series events.
Distinct published side-identity/name operands retain their first-appearance
day ordinal; Choices resolves them against the final prepared country-groups
state, preserving historical alliance keys without another recording pass.
The open day remains replaceable when an append supplies a later read.
Moment and timeline adapters read requested hours and share the registered
snapshot, selection, ranking and reply builders. Moment windows require the
family's complete coverage. Choices and timelines combine the prepared daily
prefix with cached keyframes of later blocks: cells, historical side identities
and most-active row choices are visibly approximate until completion, and sampleHours
keeps the actual supplying dates. Unread blocks stay pending, with no geometry in
their pending band. A daily cell crossing coverage is wholly approximate. The
country groups come from the family's own read: side assignment uses
dated country-group memberships, leaders, wars and native identities, while
stable side keys use historical coalition anchors. A later alliance can relabel
an earlier side; a single-frame side identity is therefore approximate.
Equal inputs advance shared series frontiers. Current country/score projections
and region memberships bound changed observations without revisiting historical
keys. Distinct Choices operands update by key; unchanged days extend the current
operand's date, and final country groups still determine its published side key.
Index-only window discovery remains available before preparation.

Land battle logs, availability, rankings and courses read prepared pages.
Preparation calls the native census for each accepted frame and persists its
derived continuation for resume. Shared frame changes retain current resolved
participant joins and skip unchanged live rosters and loss inputs. One accepted
frontier defers constant native duration reads, their supplying-read events and
hourly participant diagnostics. Input changes, gaps and publication expand the
exact hourly runs; open-state replies see the same effective endpoints. Resolved
joins are runtime-only and rebuild after resume. Plain compressed JSON pages hold exact block replies and compact ranking
records with prior peaks and supplying loss reads. Courses use stable numeric
keys from the shared catalogue.
Ranking reads only records for battles in the selected windows and uses the same
country filter, measure, tie order and reply builder as the recording product.
Delayed finalizer facts repair their affected block pages. Manifests read prepared
availability. The Land battles log and biggest list retain their parity row labels.

Each branch's newest land battle window is prepared beside the forward families
(`prepared/pipeline/newest.rs`, `products/land_battles/newest.rs`), for every
branch whose last block forward preparation has neither accepted nor refused. A
look-back observes the branch's blocks from one block through the last, each
read alone (read ahead on spare cores within `scan_blocks_by`'s core budget and
reconstructed alone, as `Replay::read_block_alone` reads them), on a fresh census
configured as forward preparation's and observing with no change facts, as the
recording product does, and builds the last window's page as forward
preparation's publication of the last block does. The page is forward preparation's when the look-back observed every fact the page
carries; a look-back starting at the branch's first block would be forward
preparation's walk of the branch, so none is tried and the window is left to
forward preparation. After a recorded gap (the branch's previous block ends before
the hour before the look-back's first), forward preparation closes every battle
there, so its battles are the look-back's. Otherwise the look-back starts before
the window and every battle on the page started after its first hour, so each
battle's id, `Exact` or `By` start, counter base, runs, loss reads (the one
carried into the window included) and prior peaks are whole. Every finalizer
result read in the window covers no hour before the look-back's first, so the
battle it closes is one the look-back saw. Outside a look-back after a gap,
every loss read dated in the window names a variant whose naming the look-back
read, or follows an hour whose variant table was not complete, where every name
is forgotten. Only an available hour inside the look-back proves availability.
One look-back is tried, from the block one read of the branch's blocks by their
changes decides: the last block's by the combat and naming tables' keys and
every field of the progress, results and event coverage, then each earlier
block's, from the last back, by the combat and naming tables' keys, each block
read alone on spare cores (`scan_blocks_by`) and taken in that order until one
holds every check as far as the changes decide it. The last block alone only
after a gap; otherwise, the latest block from which each battle on the page
(every combat the last block holds, those the block before holds at its last
hour when it adjoins the window, and those present from a window result's
coverage start) is traced, through its presence in a complete, read census
across adjoining blocks, to an opening after that block's first hour (or the
block follows a gap); no result read in the window covers an earlier hour; and
each variant a progress row or result in the window names was read in a
complete variant read, or forgotten at an incomplete variant table, before the
read naming it (or the block follows a gap). A change the keys do not show (a
handle reused in another province) only moves the start earlier. The checks
above then decide; a look-back not proven publishes nothing. A start that would
be the branch's first block is not tried. The window's reply carries the
branch's content warnings, once its diagnostics are committed, as forward
preparation publishes them after its last block. The configured and catalogue
warnings are the look-back's. The variant naming refusals are those of every variant read on the branch: the look-back's
reads, and the earlier blocks' read by `Replay::read_block_changes` over the
variant catalogue's tables alone (the countries' keys and tags, every field of
the rest), each daily read of the variant table observed only when one of them
changed since the last read the block observed, and its first read always.
Each block is read on its own catalogue on spare cores (within
`dashboard_core::blocks::scan_blocks_by`'s core budget) and joined in block
order, dropping a block's first version of a variant whose operands the
variant's last joined version holds, as reading in order adds none there: that
holds every operand the catalogue reading them in order holds, and no other, in
about as many versions, and the refusals and variant warnings are of the
operands read, however often each was read. With content, the census's own text
refusals come from naming the variants its loss reads use, every one a variant
read on the branch; naming every variant read must refuse no text and use no
lazy reader, so forward preparation's naming refuses none either. A census
refusal, a block that cannot be read or unproven availability publishes
nothing, leaving the window to forward preparation. The page is written as
`land-battles-newest-<branch>`, with the bytes forward preparation writes to
`land-battles-<branch>` for that window, beside `land-battles-newest-window-<branch>`
(the window's position and hours), in a commit of its own as soon as the
look-back is proven, through the families' group commits. The same Beside job
then reads the earlier blocks' variant names on a quarter of the hardware
threads while those families prepare, and commits the branch's content
warnings as `land-battle-newest-diagnostics-<branch>` (the window's position
and hours, and the warnings); a variant naming that refuses text withholds
only those diagnostics. Until the branch's coverage is complete, the manifest
(available, with the index's windows) and that window answer from these pages
while they name the branch's current last block, the window's reply with the
branch's content warnings once they are written and, until then, none and
`warningsComplete: false`. Other windows answer their committed prefix;
rankings and courses wait for coverage. Forward preparation later publishes the same page for the window. A
refusal forward preparation records answers every query of the branch, as for
every family. Growth prepares the new last block's window again.

Each branch's newest naval battle window is prepared as another Beside job
(`products/naval_battles/newest.rs`), for every branch whose last block the
naval family has neither accepted nor refused: by its own progress while it is
prepared on its own, by the hourly walk's otherwise. A look-back observes the
branch's blocks from one block through the last, each read alone with the
battle census tables and change facts of its own, on a fresh scan, as the naval
family observes them, extending the retained results at each block's end, and
builds the last window's page as forward preparation's publication of the last
block does. The first look-back is the last block alone. A look-back starting
at the branch's first block would be forward preparation's walk of the branch,
so none is tried and the window is left to forward preparation (a single
block's branch among them). The page is forward preparation's when the earlier
blocks, read once by `Replay::read_block_changes` over the combat
keys, the result keys and every source field (each block on its own on spare
cores, joined in block order, where a combat present from a block's first hour
continues the presence the block before held at its last hour when that is
the hour before), prove absent every fact the page
carries that they could decide: no result on the page has a reference read, or
named by a source, before the look-back; every source the look-back read has an
id above, and a game hour at or after, every earlier source's, and a persistent
one names a combat no earlier persistent source named and a result no earlier
source named or block read; each source read at its first hour dates at or
after the hour observed before it; and, without a recorded gap before it, no
combat on the page opening at its first hour and none a source it read names
is open in a complete census at the hour before. The page's place names read
every recorded hour of the earlier blocks with the look-back's, and the
manifest's availability every hour's, so an unavailable branch publishes its
manifest unavailable. A failed check names the earliest earlier fact: a
result's or source's first read, or an open combat's first hour since the last
gap or incomplete census. The next look-back starts at the block holding that
hour, and from its second retry also at least doubles its length; one that
would start at the branch's first block is not tried. A block that
cannot be read publishes nothing. The page is written as
`naval-battles-newest-<branch>`, with the bytes forward preparation writes to
`naval-battles-<branch>` for that window, beside `naval-battle-newest-<branch>`
(the window's position and hours, and the branch's availability), in a commit
of its own as soon as it is proven. The land and naval windows are prepared at
once, each family's on a thread of its own beside the families prepared on
their own (`independent::Beside`), branch by branch, their block reads sharing
the core budget; neither waits for the other's, nor any family for either. Until the naval family's coverage of the branch is complete,
the manifest and that window answer from these pages while they name the
branch's current last block. Other windows answer their committed prefix;
rankings and details wait for coverage.

What each branch's newest land or naval window preparation did is reported
(`dashboard_base::prepared::newest::Report`): the blocks before the last read by
their changes, every look-back tried (its first block, the blocks it observed,
its time, and the check that failed with the hour it names), the earlier blocks
read for the land window's diagnostics, and the outcome (proven; left to forward
preparation at the branch's first block, unproven or unavailable; naming refused;
or unreadable). A naming refusal after a proven land page leaves that page
available with incomplete warnings. Measured runs count it in the family's timing
rows: `newest.land` and `newest.naval` (the family's time outside its timed parts), `.earlier`,
`.look-back` and `.naming` (time, with blocks as calls), and one row per outcome
(a call per branch). The land diagnostics row excludes its separate naming time.
While preparation runs, every accepted status carries a
row per family trying windows, `land-battles-newest` and `naval-battles-newest`:
preparing until it has tried every branch, its count the branches whose pages
and diagnostics completed of those tried, its text each branch's report on one line. The readiness probe
logs every status row's text with its state and counts.

Naval logs, availability, rankings and drawn-row details read prepared pages.
Preparation shares the native census and retained sinking multiset
observer: one retained sinking scan supplies naval details, the sinking log,
titles and loss tallies. Plain compressed JSON pages hold exact block replies and ranking records;
the shared catalogue supplies stable battle keys. Series events hold changed
fleet names and exact ship-read confirmations. Prepared named sinking
rows retain duplicates and both native and supplying dates. Drawn details read
only their battle, fleet and sinking pages; the production selection and detail
builder preserve full references and missing names. The Naval battles log and
biggest table retain their parity row labels.

Division log, availability and totals queries read prepared pages and cumulative
endpoints. Preparation uses the native per-frame appearance and removal observer.
The shared frame changes skip unchanged division rosters,
deletion witnesses and naming operands. Last-complete census and template dates
advance independently of those operands, preserving held presentation dates across
unavailable reads. Its reply builders publish exact block rows and the latest
template catalogue as plain compressed JSON replies. Totals use cumulative endpoints for
all actors/templates, individual actors/templates and their recorded pairs. The
existing country predicate supplies membership and missing-read intervals;
queries subtract endpoints without loading events. An adjacent-drain boundary
correction keeps a witnessed destruction dated before its read out of a later
native-date range. Unknown actors remain separate. Parity models are labelled
Division log for rows and Division totals for the displayed counts.

The division log is prepared on its own (`products/division_log/block.rs`;
registry entry in `prepared/pipeline/independent/windowed.rs`, name
`divisions`), gating `division-log`, `division-log-manifest` and
`division-log-totals`. Divisions move and their names are read again every
hour, so no hour is quiet for it: a pre-pass worker reads each block alone,
every hour, and runs the observer from the block's first hour. The family's
own thread observes each block's first hour on the carried scan; where that
hour read the census, the removal witnesses and the country tables completely
and built the presentation census again, the carried scan then holds what the
extract's holds, and the fold takes the extract's census, witnesses, tags,
presentation, dates, events and warnings. A division absent from the last
census is a candidate the fold logs only if the carried scan never saw it on
the branch, and presentations dated before the block's first template read
take the carried read hour. Otherwise, and for a block whose extract stopped,
the block is observed hour by hour. Each block's window and the manifest are
rendered at the block's end; the next commit writes them and feeds the block's
events to the totals in hour order and flushes them once per block, as hourly
preparation does at each block's end. A block whose publication fails commits
at once, refusing the branch at the block's last hour. The scan is saved as
`division-log-state-<branch>` and resumed where the family loads the branch,
from the frame hourly preparation's stream holds there.

The naval family is prepared on its own (registry entry in
`prepared/pipeline/independent/windowed.rs`, name `naval`), gating the naval
battle manifest, windows, rankings and details, the ships sunk manifest, log
and battle titles, and naval losses, whose queries use the naval scan's own
recorded identity spans. Its scan carries the open and retained battles, the
result and source links and the retained sinkings from every earlier hour and compares table
allocations hour to hour, so its own thread observes each block hour by hour,
as the hourly walk does, on the block read alone with its tables
(`naval_battles::PREPARED_TABLES` and the trigger kit `triggers::RECORDED_TABLES`
that a sunk ship's scripted place name evaluates) and the change facts of the
block alone. A place name is read only for a ship the sinking scan first finds,
which it does only at a slot read of the sunk ships that is the block's first,
follows a change of their rows, or follows a read whose rows hold an entry
dated after it. A pre-pass worker reads each block's sunk ships by their changes
and finds the last such hour; the family's thread reads the trigger kit with
its tables through that hour only (`Block::read_through`), and its tables alone
for the hours after it from a read of their own from the block's first hour,
handed the first read's views of the tables that read has not rebuilt since,
so each table's allocations, change facts and view comparisons are those of
one read. Detail
observation of a frame without the kit
(`naval_battles::Scan::observe_details_read`) stops preparation at a ship it
first finds there rather than title it. A block observed without a pre-pass
read reads the kit at every hour. It publishes each block at the block's end (`Scan::publish_configured`)
into pages held until its next commit (`Branch::publish_block`,
`dashboard_base::prepared::pages`), which answer its reads of what earlier
blocks of the batch wrote, its committed pages answering the rest; it commits
where the hourly walk does, writing the held pages first, and saves its scan
there as `naval-battles-state-<branch>`. It binds its sinking content once per loaded
state and resumes the scan where it loads the branch, as hourly preparation
does.

Battle window queries read the plain reply at their block ordinal. Rankings
locate their first and last block with two dated series event lookups and load
only the selected ranking records.
Land ranking intervals include the missing hours between recorded blocks, where
the census can bound an unknown ending or date a finalizer. Independent supplying
read events retain the last complete census even when the selected range is empty
or ends inside a gap. Availability probes read a small recorded-value page.

The pipeline's resume hook gives observers the restored decoder frame. Naval
result input caches are rebuilt by the census, rather than serialized. Division
continuation retains only last-complete presentation operands and deletion
signatures: an unavailable read or an interleaved branch can hide those facts
from the restored frame. Sinking continuation retains whole-entry multiset
signatures; its dated event signatures preserve naming operands after native
retention prunes an entry. No observer serializes a fake frame or checkpoint.

## Prepared ship sinking log

The naval battle observer also supplies the ship sinking log. Its retained-vector
step and country attribution are shared with the recording oracle. New sinking
occurrences append supplying-dated events with stable scan indices. Native sink
dates choose replay windows; late observations replace only the affected window
reply. Each reply uses the original class, stored-name and place resolver and
keeps its own dictionaries. Supplying reads still gate row visibility. A small
manifest holds one first occurrence per class for choices and the replay windows.
The log keeps visible, playback and reached windows, with earlier/latest browsing;
parity preparation reaches every requested window. Before the branch is complete,
these queries expose the committed prefix with coverage/pending fields; a cold
reply has no accepted coverage. The log starts at the newest covered window and
bounds its class choices to prepared history. The naval combat/result newest
proof does not prove sinking ordinals or late identity repairs, so unreached
sinking windows remain pending.

Sinking battle titles use the accepted supplying frame's geography and the
original province/state name ladder. One dated title event per stable scan index
keeps the exact result or refusal; requests page only drawn indices. Geography
notes and unique place refusals are aggregated by the original reply builders
for the requested rows. Title continuation holds only event counters.

Air and navy share the prepared parts-race observer, publication and query flow in `products/parts_race_prepared.rs`. Native bindings supply their census continuation, channel/group projection and optional hull-maximum reference. Both keep their existing series sections and native reply builders.

The intelligence, raid and division-log first views also answer before any
accepted hour, with an empty preparation extent ending immediately before the
first recorded hour, `complete: false`, and the recorded hours pending.
Intelligence replies use `preparedCoverage` because cipher `coverage` already
names capture evidence. The query gate, readiness classification and client
polling follow that separate extent, including a joint gate before its first
accepted hour. Day values and proven newest windows replace pending data as
described in their product guides; requested espionage and division windows
remain mounted progressive requests.

## Cold prepared replies

Default widget requests publish their ordinary reply objects before the first
family commit. The shared prefix extent ends immediately before the branch's
first recorded hour; its pending spans name recorded hours only. Empty lists
carry no totals or evidence of absence. Country groups and subjects can extend
that prefix with the existing cached keyframe observations. Wars can also list
the exact native groups at a requested hour while their continuity lanes are
pending. Event pages retain their requested bounds and have no continuation
until accepted events exist.

Battle manifests and log windows use committed pages when the independent newest
proof has no answer. Those pages are approximate while later terminal reads can
repair them; a proved newest window retains its exact first hour. Both metadata
and windows remain mounted progressive requests. Cumulative casualties,
construction and production retain empty or accepted prefixes, with unprepared
amounts absent or carrying the existing preparing reason.

Scoreboard follows each requested column's supplier coverage. Factory ranks
and holder identities can come from the exact moment frame; uncovered columns
have no value or maximum and are named in the root preparing list. A sort whose
supplier is pending returns an empty prefix rather than an invented ranking.
The readiness probe asks for the actual default columns and a visible cell page.

Proven newest land and naval windows publish `newest.land` and `newest.naval`
branch extents in status `families` atomically with their pages. Commit members
explicitly carry family progress, a proven newest-window extent, or work beside
them. Side-job completion and newly queued production scans merge into the
transaction's side-work metadata before its checkpoint is published. Family
progress contains only persisted fields.
Partial battle replies name both their forward gate and this window gate, so a
mounted cold reply refreshes while independent families still prepare. These extents cover
only their newest window and never extend the forward prefix. Sinking and peace
prefixes remain approximate until complete because later observations can revise
them. The experience readiness requests derive native-holder JSON keys from their
country-groups dependency, matching the widget's series selections.
The probe retains the latest dependency replies and instantiates dependent
requests again from them, including selections already resolved or absent.
A widget's exact time waits for its final dependency replies and the exact
requests built from those replies; replaced selections are no longer polled.
Production lines follows its immutable first view with the widget's separate
`exact: true` product request. Its first-view time belongs to the first reply;
its exact time waits for that whole-history follow-up, independently of family
preparation coverage. The first view and follow-up share immutable extracts of
the newest blocks through the source's bounded observation LRU. Only that overlap
is retained, sized by its live block count and observed allocation capacities;
the whole scan still folds every hour in order from its own start. Different
countries, native holders and selected tables have distinct entries.

The interactive `doctrines` product seeks one selected country/hour and only its
required tables. It returns ordered grand/subdoctrine choices, completion
history, mastery and bank, effective and unit-source daily gains, commander
contributions and stored grants/handles. Authored costs and reward benefits use
the recorded doctrine catalogue and shared description formatters; missing
runtime cost operands remain explicit diagnostics.

Doctrine reward labels use the subdoctrine/reward composite localisation key;
optional generated description keys share the authored benefit formatter. The
doctrine product reconstructs the shared scoped-text and trigger inputs used by
other description callers, including recorded flags and variables.
preparation coverage.

The country-people product supplies reached-hour manpower, mobilisation, state pool, exile and election details to the existing manpower and ruling-party views.


Offline product checks load faction goal/template and upgrade/group content through the same readers as faction details, and check country-people mobilisation defines. Selected identities and supplying simulation operands remain explicit offline findings. Sampled banked-experience replies retain their supplying read-hour marker. Subject replies mark both explicit recorded gaps and discontinuities between recorded blocks as unobserved after the prepared prefix.
## Fold CPU measurements

When `replay_frame::cpu` is enabled, independent family workers split their CPU into
`fold driver`, `fold binds`, `fold facts`, `fold frame` and `fold observe`.
Driver scopes cover a block, each kept hour, and each quiet hour or span the
family consumes. Existing family phases retain their names inside observation.
`frame changes` measures construction and tracker observations. Shared series
use `series advance`, `series seal`, `series replay` and `series flush`; replay
includes key remapping, observations, pushes, events and family callbacks under
one block scope. Direct per-key publication remains in its family's
observer/projection phase, avoiding a timer around each key.

Block publication is `fold block flush`; batch publication, flushing, saved
output and continuation are `fold publish`, `fold flush`, `fold save` and
`fold continuation`, inside the existing `commit` scope. `fold load` covers
branch restoration. `save serialize` separates the pipeline's JSON/compressed-JSON
continuation writes. Per-key reply serialization stays inside its enclosing
block/batch publication scope. The `fold-facts`,
`fold-own`, `fold-sequential` and `fold-fallback` notes remain inclusive block
measurements; they are not added
to exclusive phase CPU. Shared helpers also run in extracts and queries, so
fold breakdowns select the `family thread` role. Accounting is disabled
unless a measurement harness enables it; scopes use its ordinary enabled check.

Doctrine reward labels use the subdoctrine/reward composite localisation key;
optional generated description keys share the authored benefit formatter. The
doctrine product reconstructs the shared scoped-text and trigger inputs used by
other description callers, including recorded flags and variables.

The installed-metadata doctrine reward/content regression runs with the ordinary
suite and initializes executable metadata from `BATTLE_PLAN_GAME_DIR`. Run it on its
own with:

```powershell
cargo nextest run -p dashboard doctrine_reward_content_names_generated_prose_and_recorded_conditions
```

Rejected family batches leave their pending metadata or accepted prefix untouched; only adopted publications advance family progress. Country-group tail queries preserve both explicit recording gaps and discontinuities between recorded blocks after prefix clipping.

The interactive `naval-management` product serves the Navy Size drill at a selected country/date, using one frame and its recorded content. It does not prepare or scan a campaign history.
The existing Agency comparison widget requests `intel-details` for one selected country and hour. It shares the source frame cache and recorded content catalogue, and introduces no preparation scan or new widget. Source operands, observed ranges, network occurrences and propaganda history remain separate, with named counted explanation refusals.
The existing event feed includes accepted script events, news, chosen options and decision/mission lifecycle edges. Static event metadata resolves through the recorded content stack; conditional presentation and uncaptured scope operands are named and counted. Retained registry baselines appear only as labelled state observations, without past fire times.

Authored event-option and decision effects use the shared scoped-text/effect formatter with occurrence scopes and exact country-identity witnesses. The feed names and counts unsupported bodies/inputs. Effect execution rows remain distinct from actual terminal transitions and native duration writes; sampled state is never substituted for execution-time state.

Certified native rebuild rows show the decision roster in the existing Event feed. Membership, native lifecycle/durations, target identities, ignored flags and description counts come from the retained event reducer. Category/decision predicates share the trigger interpreter; unsupported content and mutable inputs without an exact rebuild certificate are refused by decision, predicate and operand. Prepared rows retain these same inputs. Baselines remain state, never selections or past fires.

Script-family masks describe real interval losses; none of the supported decision families is unconditionally refused. Effect invocation, actual result transitions and description-only randomness retain their separate meanings.

Event-feed preparation persists the complete retained script reducer, including owner maps, ordered members and open pass cursors. Retained-state detail rows show pending instances/due hours/scopes/originators, fired membership, delayed timers and decision containers/counters; baseline entries are read-time observations, never past fires. Final roster cost checks compare the certified fixed5 political balance with the shared interpreter cost.
The existing Agency comparison widget requests `intel-details` for one selected country and hour. It shares the source frame cache and recorded content catalogue, and introduces no preparation scan or new widget. Source operands, observed ranges, network occurrences and propaganda history remain separate, with named counted explanation refusals. Its content preparation check loads the same define stack and sub-unit database as the product, and reports source/range operands and supplying dates as recording-only inputs.

The focus-art runtime metadata and image parity test runs with the ordinary
suite and initializes executable metadata from `BATTLE_PLAN_GAME_DIR`. Its fixture
preserves unused singleton inputs as unavailable using the table registry, so new
singleton tables cannot become invalid empty observations.

## Faction histories

`faction-state` is one content-dependent prepared product in [dashboard-factions](../dashboard-factions/README.md), with influence, rules and manpower views for one recorded faction. Its independent `faction_state` family shares the block pre-pass, coalesced series, atomic continuation publication and query observation/series caches. Each channel reports known and supplying-read spans independently. Retained account association is explicit, not a current-member ranking; programme labels never use a later daily read. The selected-hour `faction-details` drill shares its typed selectors and native arithmetic, and goal/resource artwork uses the existing catalogue/resource routes.

## Prepared diplomacy state

[dashboard-diplomacy](../dashboard-diplomacy/README.md) owns `diplomacy-state`,
with the serde-tagged `opinions`, `pair`, `pacts` and `proposals` views. Its one
independent `diplomacy_state` family uses the shared block pre-pass, atomic group
commits, sparse numeric series, occurrence event pages and resumable continuation.
Availability and supplying reads stay independent for opinions, tuples and the
Daily(22) queue. Sparse owner proof supplies exact zeros on demand; gaps, aliases,
missing reads and identity changes never manufacture zero or diplomatic events.
Queries share the ordinary reply/series caches and cold prefix/pending contract.
Only an unreached opinion moment may read one cadence-bounded exact window,
explicitly marked `snapshot`; pair and inventory histories never scan the branch.

`volunteers-expeditionaries` is a prepared-only recording product in dashboard-forces, gated by the existing army family's hourly-walk coverage. Its directed land/expeditionary links and native first-match VolunteerHosts tallies share army's block pre-pass and continuation. Independent source and native-country presence proofs distinguish empty lists, unavailable counts and pending hours; repeated native occurrences remain in army aid event pages. The ordinary query observation/series caches retain its complete prefix answer. No air-volunteer classifier, cumulative dispatch total or additional heavy family is introduced.

## Prepared dockyard repairs

The `repairs` family prepares hourly used dockyards times the country's carried
production-census dockyard output as fixed5 daily IC rates. It shares the country
census extract and fold; quiet hours visit no countries. Missing repair rows,
negative counts and unavailable output stay unknown; observed zero use needs no
output row. Daily refusals withdraw output until a successful read, and recording
gaps never carry it. Native holder changes break the rate series.

`dockyard_repairs::RepairReads` reads only a requested tag's runs and integrates
inclusive hour spans with checked arithmetic; the caller divides by 24.
`origin` supplies its first known rate hour, including zero. In-use opening hours
before a valid modifiers read have no rate, not zero. The `dockyard-spending`
query joins the `production` and `repairs` families' accepted coverage.

Repair parts use the current selection at each supplying read, not an elapsed-hour
repair-event ledger. Each selected queue occurrence contributes one dockyard,
charged to the base holder even for allied recipients. Repeated occurrences stay
separate; refused selection, missing design and unused allocation remain
unattributed. Selection exceeding independent country use withdraws every part
without withdrawing the total. Zero use needs no queue walk. Selection is cached
apart from output changes; quiet steps visit neither countries nor bases.

`RepairReads::parts` returns valued parts in series-key order; `part_integrals`
shares total integration and validates the common split before treating a later
selected design's earlier hours as zero. `variants` exposes only repaired designs'
dated raw catalogue for query-time content classification. The pre-pass watches
selection operands and preserves both output and variant census read boundaries.
