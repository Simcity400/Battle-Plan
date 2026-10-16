# dashboard

Native widget data: opened recordings, typed products, the data service and the
host/viewer protocol. This crate reads recordings and their content stack; it
neither reads live game memory nor draws map pixels. See the
[architecture](../../docs/dashboard.md), [desktop host](../desktop/README.md) and
[widget framework](../../ui/src/widgets/README.md) for the surrounding flow.

The host owns a bounded background preparation queue. Published library recordings
are prepared without being opened, and opening a source also starts preparation
independently of the opening request; Status, Start, Cancel and Clear
commands operate on that recording through its current source binding. Prepared
queries use the same host service and cancellation boundary as ordinary products,
and read only their requested pages. The reply echoes source, revision and product.
Clear cancels writers across opened revisions of the path before deleting the
prepared files; existing open range readers retain their files.

`Service::configure_content` verifies the selected game folder and prepares the process-wide executable tables before reading game settings or exposing content options. Content-dependent consumers share the decoded registry, equipment and rule metadata. Built-in variable names come from the installation's shipped documentation. Recording-only source opening, metadata and numeric histories remain independent of these tables.

## Layout

| Module | What it does |
|---|---|
| [dashboard-text](../dashboard-text/README.md) | The text interpreter, name rules, content catalogue and recording metadata helpers, re-exported here at their old paths. |
| [dashboard-base](../dashboard-base/README.md) | Sources, content, the catalogue loader, the prepared store's shared pages, series, charts, group sums and measures, ranking, the product contract and the shared products including country history, re-exported here at their old paths. |
| [dashboard-core](../dashboard-core/README.md) | Host/viewer protocol, data transport, recording metadata, host configuration, map settings and whole-branch block scans, re-exported here at their old paths. |
| Product crates | One crate per heavy product or product family, above dashboard-base, re-exported here at `products::<name>`: [dashboard-air-aces](../dashboard-air-aces/README.md), [dashboard-air-superiority](../dashboard-air-superiority/README.md), [dashboard-construction](../dashboard-construction/README.md), [dashboard-equipment-losses](../dashboard-equipment-losses/README.md), [dashboard-equipment-stockpile](../dashboard-equipment-stockpile/README.md), [dashboard-forces](../dashboard-forces/README.md), [dashboard-intelligence](../dashboard-intelligence/README.md), [dashboard-land-battles](../dashboard-land-battles/README.md), [dashboard-military-air-ledger](../dashboard-military-air-ledger/README.md), [dashboard-naval](../dashboard-naval/README.md), [dashboard-politics-ledger](../dashboard-politics-ledger/README.md), [dashboard-raids](../dashboard-raids/README.md), [dashboard-resource-balance](../dashboard-resource-balance/README.md), [dashboard-unit-leaders](../dashboard-unit-leaders/README.md). |
| [prepared_focus.rs](src/prepared_focus.rs) | Sequential focus observations, production presentation and immutable requested-page reads. |
| [products/](src/products/mod.rs) | Product registry and shared domain helpers; each product's behaviour lives beside its code or in its widget README. |
| [content_check.rs](src/content_check.rs) | Offline preparation and diagnostics for every registered product. |
| [service.rs](src/service.rs), [service/](src/service/) | Source leases, loader jobs, request admission and the bounded response pump. |
| [reply_cache.rs](src/reply_cache.rs) | Whole-branch product replies kept on disk between sessions. |
| [prepared_stability.rs](src/prepared_stability.rs) | Production stability and experience scans feeding shared series and supplying-read events. |
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

`CONTENT_DEPENDENT` is true when any reply field reads localisation, artwork or
content definitions. The generated `CONTENT_PRODUCTS` list controls frontend
content-cache invalidation. Service, transport and host route by key; widgets
and their data helpers choose products. The shared serialization boundary adds
current uncovered-content-read warnings through `ReplyWarnings`, including for
cached replies. Preserve independent known values when a name or image refuses.

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

See [peace-deals.rs](src/products/peace_deals.rs) and the
[Territory transfers guide](../../ui/src/widgets/territory-transfers/README.md).

## Background preparation

Background preparation defaults on. The optional `background_preparation` choice
in the host's `content.json` (see [config.rs](../dashboard-core/src/config.rs)) persists the global
preference while preserving the content folders. Settings can disable automatic admission; explicit
Prepare and Resume remain available and previously prepared files stay readable.
The desktop scans the shared recording library at startup and while idle, admitting
unprepared revisions in most-recently-modified order through the same queue.
Unchanged and busy replies end admission retries. Recording revisions, completed
workers, changed preferences/content bindings and game transitions permit a new
attempt; timer ticks alone do not submit further preparation requests.
It detects completed sessions through the recorder's atomic recording publication.
A running game suspends automatic admission and cancels automatic jobs at the
existing block boundary. After game exit, accepted prefixes resume from their
unaccepted tails, including interrupted opened recordings outside the library.
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

The pipeline reconstructs accepted selected frames once and feeds focus,
production, country balances, country identities, groups, stability, experience,
stability parts, army size, air force size, supply health, territory, regional territory, occupation, land owners, game score, leader careers, battle and division logs, sinking logs,
naval losses and map checkpoints together. The naval observer owns one retained
sinking scan shared by naval details, the sinking log, titles and loss tallies.
One replay worker reconstructs a block sequentially through a channel retaining
one queued frame. For each observed frame, scoped worker lanes borrow disjoint
numeric accumulators while the coordinator runs content-dependent observations
and stages writes in their original order. Country groups, its dependent air
superiority observer and idea changes stay on the coordinator for recorded-content
resolution, alongside focus completions and the feed-only observer. Subject changes
stay with the subject scan on one lane. Territory, country identities and supply
share coordinator ownership. Every lane finishes before the next frame or any block finalization,
publication, budget inspection, resume or branch change. No accumulator/history
is duplicated, and every lane shares one frame's existing table allocations.
One runtime frame fact separates allocation changes from sampled dates and the
current table/slot read evidence. Preparation computes it once before dispatch
and shares it with all lanes and the coordinator. Weak stamps retain no previous
rows; branch changes, missing hours and observer restores invalidate the stamps.
Families adopting the fact still advance their own clocks and read owner-specific
SliceReads from the frame; unchanged allocation alone never proves no fresh read.
Lane count is available parallelism minus the decoder and coordinator, capped by
the number of independent groups; two or fewer available processors use serial
preparation. Scoped lanes close and join on cancellation, errors or unwinding.
Cancelled frames never move the accepted cursor or decoder resume state.

Focus display uses the production interpreter, evaluating the recorded player's initial country first
within each frame. Content inputs come from the recorded stack's physical files,
including installed mod archives and the selected language settings.
Their size/mtime stamps are validated once when source/content options bind,
then retained in source state. Prepared queries use this binding and the current
small store manifest without restatting installed content. Publication validates all
inputs before committing; the store keeps their stamp list in a separate immutable file.

Publication batches end at a branch boundary, a 64 MiB byte budget, ten seconds
of work or the recording end. Progress and cancellation are checked per block.
Domain data, branch accumulators, decoder resume state, accepted coverage and the
common cursor share one manifest commit. Saved hash-backed continuation
maps and sets serialize in key order, including tuple-key pair arrays and nested
holder sets. A source-tree test rejects unreviewed hash-backed serialized fields.
Country-history tags register supply actors
in stable tag order. Thus equal observation/commit plans produce equal section
payloads across fresh processes and serial or lockstep observation. Cancellation or an I/O refusal leaves
the last committed prefix readable. Resume starts at its next source block;
growth compares the saved index, session, branch and gap metadata and prepares
only new blocks. Changed index metadata starts a replacement. The current batch
is buffered; saved accumulators retain only continuation state.

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

Published coverage is branch-scoped and distinct from recorded missing hours and
an unprepared tail. Numeric focus can be ready while unavailable display fields
retain named, counted reasons. Progress reports accepted block counts, committed coverage hours per branch and active
payload bytes, without inferred rates or timer progress. Clear cancels and waits
for writers to close, removes names immediately, and lets held file readers keep
their existing files. Stale or unsupported prepared schemas restart preparation from the recording;
recording readers remain available.

The shared country-history observer also retains native holders, aliases, identity
segments and complete-read coverage. Each commit replaces one branch identity
state page; cached table joins rebuild from the next accepted frame on resume.
Prepared identity lookup returns the same identity-only histories as the shared
recording walk once the branch is covered, and remains pending before then.
Its retained histories grow with identity and coverage changes.
Peace preparation refreshes current native ownership, war, subject, government
and existence operands only with their supplying table changes. Country changes
resolve those retained operands again; snapshot dates share immutable field maps.
Subject end hours remain native clock work. Open-card positions compact on
membership changes, and finalizer country-name proof spans materialise at changed
holders, finalizer reads and publication. Missing-input spans share an accepted
frontier and seal at changes, gaps, publication and reply construction.

Research preparation retains current ordered country operands and supplying-row
joins, refreshing each join only when its source table changes. A deadline-ordered
current-holder schedule closes carry exactly at the native stagger deadline.
Accepted unchanged hours advance one frontier; supplying reads, deadlines, gaps
and publication materialise the existing native coverage and slot endpoints.

Stockpile choices keep ordered native ID/classification witnesses and the latest
logistics representative per type, with keyed static group evidence for unresolved
types and required available amounts. Late lexical names move their witness to
its new type, and the previous type elects its next highest ID directly.
Supplying reads combine those facts with only current numeric operands through
the same evaluator used for corrections. Equal native tables advance supplying
dates and diagnostic counts from their current read evidence without repeating
row joins or visiting retained numeric histories. Retired IDs keep their native
absence semantics without repeating their classification or group projection.

Trade preparation retains current exchange operands in recorded order and
resolves them through the shared country lookup only when their source operands
change. Sparse current flows and refusal groups feed their existing shared series.
Active trade and known-hour integrals advance hourly; retired cumulative endpoints
materialise once at publication, including across missing observations.

Resource preparation keeps current native pool and exchange operands at runtime.
Each projection refreshes only when its own table changes; holder changes resolve
the retained operands through the shared country lookup. Resource definitions
refresh only with their definition or lexical inputs. Unchanged countries keep
one accepted frontier, materialised at their next change or publication.

Runtime country joins share the resolved link-slot map through `CountryReads`;
country-key changes refresh holders and partners without copying unchanged links.
Country-group continuation retains merged faction rosters and country/faction
read proof spans for native faction sums. Repeated unchanged hours only extend
their endpoints. Loading rebuilds the open roster keys and drops table views.

Country balance, stability-part, territory, army, supply, air-force and navy S
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
ranks sampled values without persisting tail ranks. Group sums reuse the one
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
day, and takes its previous frame from the restored replay accumulator.

The ledger binds successful and refused content outcomes before observing a
branch. Fixed plain text resolves once per content key; recorded names and
scoped text still use their supplying frame. Runtime native joins refresh from
the shared frame-change flags, and the shared country-key projection skips
identity joins on balance-only updates. Only affected countries run accounting
and presentation. Unchanged frames advance authenticated scalar frontiers;
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

The production branch-wide idea observer receives each accepted decoded frame.
One `idea-history-{branch}` range holds its zstd-compressed JSON state and is
replaced with the common preparation commit. Resume reloads the same observer;
append observes only the accepted tail. The shared frame fact updates changed
holdings through the same native scan. Open endpoints materialize at a relevant
change, publication or reply. Missing character-template refusals retain their
hour-dependent text without rebuilding holdings. Stored spans, identity naming and refusal sets grow with recorded
changes, rather than elapsed hours.
Idea classification content is bound at configuration; the shared result cache
retains successful and refused loads, and fixed token classifications use the
shared keyed result cache. Only new additions contribute classification counts.

Prepared `idea-history` requests use `{branchId, tag}` and the production topic
reply builder. The observer retains both identity-start naming for topic replies
and span-start naming for event-feed, plus country-scoped refusal counts.
Queries wait for complete branch coverage. The shared idea-history UI hook uses
the ordinary preparation status, cancellation and retry handling; idea timelines
and national-spirit widgets retain their existing reply shapes and presentation.
The legacy product and event-feed branch scan remain available as references.
Idea holdings, acquisition/removal bounds, advisor seats and recorded renames
depend on preceding observations; they have no stateless progressive sampler.

## Prepared focus pages

Focus preparation accepts verified decoded frames in order. Its resumable state
keeps the latest ordered completion list and native identity per tag; progress
is retained in full on disk. The production focus interpreter evaluates dated
titles, icons and descriptions from the same accepted frame with the recorded
content catalogue. Display and description projections occupy separate pages. Consecutive equal order, identity, progress
and presentation results share an interval; missing reads and gaps leave no
coverage. Hour metadata retains native slot/country identity and the known daily
progress gain. Display and text-only description results use separate pages,
preserving each production caller's projection and refusal semantics. Missing
required tables retain their exact named supplying-read reasons or truncation
counts in independent branch intervals, without refusing numeric recovery. Completed orders and presentation results use row pages, preserving
all ordinals and repeated tokens. Nonempty UTF-8 country tags and focus tokens
have no preparation length limit. Incompatible saved focus schemas refuse only
these derived pages and accumulators; recording-only products remain available.

Focus uses the shared series/event batches with a small tag-to-key catalogue.
One country event holds its changed order, display, descriptions and warnings;
progress and daily gain use separate exact runs, so progress changes reuse the
event. Table refusals use a branch key. Static text stays in one file per observed
token; selected text files open together after one recording check and manifest read.
Focus reader setup opens its country catalogue and first series file together,
including the published series count. A page validates its supplying hour and
descriptor in the same reader that returns its rows.
The pipeline's shared frame fact invalidates changed focus inputs. Unchanged
shared tables advance one authenticated series frontier without visiting countries,
interpreting text or writing files. Publication seals the active run endpoints;
progress-only changes reuse the order and presentation. Cached content refusals
still count every affected country/hour. Static presentation is cached per
content, token and mode, including
country sprite overrides. A tree button's declared or refused ID sprite override
stops conditional definition-icon evaluation and allows static reuse. Scripted
text and reached trigger-selected icons retain hourly production evaluation.
Current native joins refresh each changed source independently. Dynamic calls
retain their token/row positions; unchanged results reuse the completed order
and full presentation, while a changed result writes the same country event.
All country/mode calls within an accepted frame lazily share immutable country
names and trigger indexes. Each text expansion still gets fresh caller scopes,
token/Clock random seeds, counters and effect sessions. The focus query adapter
reads the latest event and dated runs, then returns the requested row page with
the existing reply shapes.
Publication flushes the shared batch before saving accumulators; resume reads
only the appended tail and preserves interval boundaries.
The focus-hour reply keeps its accepted-hour gate: completion confirming dates,
order-segment starts and daily gain depend on preceding observations, and dated
presentation comes from the production interpreter. Raw progress alone is native,
but cannot supply that reply independently.

The ignored `real_recording_focus_frame_counts_timings_and_exact_output` test
uses the production frame loop to compare full hourly state with per-call scoped
inputs and the native tree-selection reuse rule. It records per-hour observation,
order/progress, extension, dynamic/static token, refusal, interpreter and scoped
input counts and timings. Dynamic token names and classifier reasons appear only
in its JSON-lines report outside Git. Set `BATTLE_PLAN_GAME_DIR`, `BATTLE_PLAN_USER_DIR`,
`BATTLE_PLAN_FOCUS_PROFILE_RECORDING` and an absolute `BATTLE_PLAN_FOCUS_PROFILE_DIR`; set
`LOCALAPPDATA` and `APPDATA` to that private directory's `localappdata` and
`appdata` children. The recording and installation remain read-only.

## Prepared country groups

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
artwork. The UI loads prepared manifests, windows and portraits through the shared
product and dated-fact queues.

## Prepared military air ledger

Aircraft loss bookkeeping and applied bombing damage use the product's existing
frame observer. Preparation appends exact signed deltas through the shared series
event writer, keyed by endpoint country. Each event keeps its native application
hour independently of the supplying read that publishes it. Unknown endpoints,
unsupported callers and signed corrections retain their original meaning.

The replaced continuation range contains only identity and coverage spans,
warning counters, event ordinals and the series continuation. Country and leaf
catalogues preserve complete choices without retaining dated deltas. Availability
reads the saved family flag. Prepared replies gather event pages and use the
product's reply builder, including native-date range selection and ranking.
Equal native source reads reuse the current country join. One authenticated
frontier carries country-presence endpoints until an identity/read boundary or
publication; balance-only country changes compare the shared country-key
projection before rebuilding the join. Event ordinals and per-event refusals
retain the same supplying-read checks.

## Prepared construction

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

Construction spending uses the same observer and leaf grouping. Exact signed
leaf totals use shared series; project intervals, unknown stretches and content
presentation stay in the leaf catalogue. Sunbursts read only their requested
start, reached and final totals.
Leaf continuation updates keyed project intervals and missing-output stretches
in place. Their existing ordered reply vectors are built only at publication,
through the same leaf grouping and reply builder as the recording product.
Subjects use their product's relation/level observer and reply builder. One
compressed state range per branch retains changes, coverage spans and continuation
state. The shared frame fact refreshes only changed holder, relation and level
operands. Equal hours advance a frontier and exact diagnostic rates; open relation
and level endpoints are sealed at changes, native ends, gaps, publication and reads.
Prepared queries resolve
level names and order through the recorded content stack.

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
keys, and seals active values during flush. Rank inputs extend at publication;
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

Peace preparation uses the native conference and outcome frame observer.
Finished derived deals and projected finalizer observations append to shared
series events under one branch key. Its checkpoint holds only open comparisons,
unknown intervals, identity mappings and deduplication facts; finished outcomes
leave the continuation. Open deal ordinals preserve the reference's ordering
when finished events and retained deals are joined after resume or append.

Queries gather the branch events and call the product's finalizer join, transfer
graph builder, action naming and complete-reply budget check. Finalizer operands
are already projected into the product's outcome types, and retained native names
survive serialization independently of the JSON reply. Peace deals and Territory
transfers use the shared pending-coverage query path and expose the "Peace deal
values" and "Territory transfer values" parity models.

Peace outcomes have no stateless progressive sampler. Conference closure,
ownership transfers and war-end evidence compare native holders and facts across
reads; delayed finalizers also join earlier observations. A single keyframe
cannot supply those dated outcomes. Event lists retain the prepared-range status
and their existing coverage gate.

## Prepared special projects

Special-project preparation projects each accepted frame for all countries through
shared project facts, naming and scientist attribution. Scoped names evaluate the
full accepted frame in that same pass. Changed domain observations and native
identity boundaries append as shared events; progress and supplying reads use
runs. The supplying-read marker is stored once per branch. Continuation retains
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
remain shared with the global completion stream.
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

## Prepared research

Research preparation uses the native supplying-read observer and visits all
countries in each accepted frame. Completed assignments and ended segments use
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

Research has no time-series chart or stateless progressive sampler. Assignment
attribution and completion recognition need previous observations; slot state
keeps its segment's first hour, and carried progress depends on that assignment
and its supplying-read continuity. Requests beyond prepared coverage return
pending through the shared query path, including complete history event requests.
The widgets show the shared preparation state until that history is available.
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

## Prepared country balances

Political and command power use shared series/event batches. Each branch/measure
section appends one binary range per commit, holding every key in a sorted
directory. Per-key blobs use delta/zigzag integers and zstd for runs, summaries
and sparse JSON events. Generic runs keep exact values and breaks; equal
contiguous runs join across batches only when there is no break. Navigation
derives dates from runs, including unknown boundaries. Sparse events retain every
JSON row and duplicate in order. A small tag/measure catalogue maps choices to
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
prepare only new hours.

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
the tail. Supply resolves recorded Defines once when sampled land inputs need
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

Open scans, recent experience reads and hourly gain queues resume across batches
and appended recordings. Current country joins, supplying-read operands and
numeric inputs are runtime-only and rebuild after restore. Equal country keys
reuse joins across balance updates. Accepted hours authenticate shared series
frontiers; native daily reads and complete hourly XP gain queues preserve their
own cadence. The country-stability and country-experience families
answer exact state, clipped width summaries, navigation and provenance event
pages through the country-measure adapter. Exact windows retain their original
supplying runs for chart drawing. At-hour stability breakdowns use the registered
product.

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

## Prepared game score

Unchanged score inputs advance the authenticated series frontier without visiting
retained country facts or keys. Native warning counts still accrue each accepted
hour. A changed observation updates holder stretches and closes departed rows;
publication seals the numeric endpoints through the frontier.
The native observer compares shared country identity operands before rebuilding
for balance changes; reserved naming that includes its read hour keeps its native
hour-dependent path.
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
choices, leaderboards and careers read prepared data. The shared frame-change
facts gate current native indexes and outcome reloads. Equal country id/slot/tag
projections retain the resolved holder join when only balances change. Admission
rosters update in place when a visible leader's kind, owner or native holder changes; they never
walk the accumulated roster each hour.

Consecutive unchanged observations advance one scalar frontier. Current runs,
commands and supplying coverage seal through it on changes, gaps, publication
and open-state replies; no carry crosses missing hours. Scoped visibility with
configured content still runs its hourly interpreter step. Pending byte totals
update with new facts and are recounted only at restore and publication.
The test-only recording oracle executes the native step every hour. Shared
series events hold closed value runs, trait gains, commands, battle observations
and coverage spans. Each full leader
reference has its own key. Combat outcomes and naval result source links have
shared keys in the same catalogue; selected careers join their latest outcomes,
preserving the first supplying read and permanently unresolved conflicts.

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
the next accepted frame without treating that rebuild as a holder change.
The series adapter answers reached-hour state, width summaries, event pages and
navigation from shared readers. The Casualties widget requests one reached hour
and retains pending playback at the previous reply's own hour. Its parity model
is labelled "Casualties exact values".

Casualty values, pairs and breakdown use prepared queries. Their product loaders
and casualty-label cache are test-only parity oracles. The shared casualty
branch scan also supplies scoreboard's recorded-product loss columns.

Country totals retain ended relation lifetimes and unknown-loss recovery across
reads. They keep their accepted-coverage gate: unprepared hours answer pending,
and the widget shows the shared preparation note while holding earlier values
at their own hour. There is no stateless progressive sampler for these totals.

Casualty pairs share that observer. Each replay block publishes unfiltered
native-lifetime pair records through the shared compressed reply helper. Queries
apply the product's native-holder and `between` selection before summing distinct
lifetimes with its existing window builder. Availability comes from accepted
block observations, including a complete empty relation census. The window
manifest uses the source's existing block catalogue. Continuation retains
endpoint tags and counters, discarding closed pair runs after each block.

## Prepared casualty breakdown

`casualty-breakdown` reads the shared unfiltered casualty window and the single
typed `wars::prepared::projection`, with the selected native holders and no
casualty hydration. The existing breakdown projection and reply construction
join native lifetime counters to viewpoint memberships, preserving app IDs,
names and grouping reads before the requested window. The source's block
catalogue supplies the manifest; shared casualty coverage supplies availability
and gaps. Queries reuse the Wars structural change walk and read one casualty
window without rescanning the recording.

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
Ratios use exact integer operands before the legacy float division. Dense charts
receive screen-width group summaries; exact charts retain legacy coalescing and
walk only their supplying edges. Stockpile parity models are labelled
"Equipment stockpile exact values" and "Equipment stockpile breakdown exact values".

The ignored 5,000-country year and real-recording tests report prepared bytes,
checkpoint bytes and bytes per observed country-year. The real test also checks
the complete canonical reply against the recording oracle, reports date/display
query timings and logs changed nonempty tokens outside Git.
Stockpile preparation binds equipment outcomes before observation and uses the
shared keyed content policy for fixed token classifications. Carried frames
advance native read/holder frontiers; publication seals them. Naming publishes
only changed variant IDs. Pending naming refreshes survive carried frames and
restore until the next complete native supplying read rebuilds the projection.
Numeric and group observation use current members and
removed members once, while historical catalogues remain available to queries.

## Prepared equipment losses

One decoded frame feeds every native holder's existing loss ingestion. Division
census and variant naming observations are shared. Keys exist for native
countries, observed cause/design leaves and observed variant naming histories.
Cumulative signed leaf values and supplying changes use the shared series
accumulator; changes to coverage, refusals and naming keep their existing product
shapes in compressed metadata ranges. Destruction and Combat Log selections use
the same retained operands and the product's existing reply builder.

Equipment-loss requests use prepared data; the recording scan and its loader
remain test-only parity oracles.

Shared frame-change facts refresh current native owner joins. The country
identity projection excludes unrelated balance changes. Changed or supplying
rows visit their owners; retained holder and design keys carry through one
authenticated series frontier. Open coverage and uncertainty endpoints extend
when touched or published. Equal lend-lease reads advance one supplying frontier,
and resolved current owners retain deletion witnesses across resume. Disappeared
holders leave that runtime join; returning holders rejoin from their retained
exchanges. Nonempty destruction drains add their signed removals every accepted
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
Focus preparation binds its catalogue and
definitions before observing frames; refused bindings still contribute each
affected country/hour's existing refusal count. Scoped interpreter state and
dynamic presentation remain at their recorded hour. Scripted dependencies stay
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

Loader jobs run off rendering in two bounded lanes. Each lane uses the existing
request admission budget for queued jobs. Cancellation releases the frontend
request but its queued job keeps occupying channel capacity until a loader
physically receives it. Full lanes refuse new work with the existing retryable
capacity error; bodies and country selections are accepted whole or refused whole.
Preparation refreshes share the history-lane bound and coalesce to at most one
queued refresh per source, preserving a requested restart. A full lane refuses
refresh admission without waiting or creating a separate retry queue.

`Product::INTERACTIVE` reads (one hour
or one block, for hovers and labels) have their own threads, so a whole-branch scan
never delays them. Ordinary product caches serialize their own requests; scans with
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
timings, reply bytes and checkpoint read counts outside Git. Its launcher isolates
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
that the committed contract matches:

```powershell
cargo run -q --profile test -p dashboard --bin export-types
cargo run -q --profile test -p dashboard --bin export-types -- --check
```

UI changes also need `npm.cmd run check:ui` and `npm.cmd run test:ui`, as described
in [ui](../../ui/README.md). Documentation-only changes need link and diff checks.

Occupied-state preparation shares one daily join with occupation trends. Each replay block publishes its standalone state window through the shared reply helpers. Compressed continuation retains the carried state values, supplying read, laws and exact refusal identities. Trends publish fixed5 resistance/compliance, composite context events and rank-position runs through the shared series accumulator. Queries read one window, one reached hour or an aligned width summary; gaps and native holder selection remain explicit.

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
snapshot, selection, ranking and reply builders. Regional comparisons wait for
complete branch preparation, including zoomed windows: side assignment uses
dated country-group memberships, leaders, wars and native identities, while
stable side keys use historical coalition anchors. A later alliance can relabel
an earlier side, so one frame cannot supply a progressive regional comparison.
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

Naval logs, availability, rankings and drawn-row details read prepared pages.
Preparation shares the native census and retained sinking multiset
observer. Plain compressed JSON pages hold exact block replies and ranking records;
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
parity preparation reaches every requested window.

Sinking battle titles use the accepted supplying frame's geography and the
original province/state name ladder. One dated title event per stable scan index
keeps the exact result or refusal; requests page only drawn indices. Geography
notes and unique place refusals are aggregated by the original reply builders
for the requested rows. Title continuation holds only event counters.

Air and navy share the prepared parts-race observer, publication and query flow in `products/parts_race_prepared.rs`. Native bindings supply their census continuation, channel/group projection and optional hull-maximum reference. Both keep their existing series sections and native reply builders.
