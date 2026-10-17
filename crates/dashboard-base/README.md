# dashboard-base

The dashboard's shared product layer, below every product crate and the
[dashboard](../dashboard/README.md) itself: recording sources and their content,
the catalogue loader, the prepared store's shared pages, the product contract and
the shared products other products build on. Nothing here names the product
registry, the preparation pipeline or the service; the dashboard re-exports every
module at its old `dashboard::` path, and product crates name what they use
here as `dashboard_base::…`.

| Module | What it does |
|---|---|
| [source.rs](src/source.rs), [content.rs](src/content.rs), [catalogue.rs](src/catalogue.rs) | One recording revision with its metadata, resolved content and typed per-source caches; content verification and cache keys; one content catalogue per source and folders (the catalogue itself is in [dashboard-text](../dashboard-text/README.md)). |
| [prepared/](src/prepared/mod.rs) | The prepared store of one recording path and its source key, each branch's published coverage (with the gates the dashboard installs for families prepared on their own), binary series and event batches (with the recorded accumulator calls block folds make again, and the patched family catalogues), frame changes, the contract of a family prepared from blocks (its fields, block extract and fold outcome), exact group sums, progressive (prepared plus recorded tail) query helpers and the per-source cache of their decoded reads; the scheduling priority every preparation thread takes from its job (`priority`, re-exported from [job-priority](../job-priority/README.md)). |
| [prepared_chart.rs](src/prepared_chart.rs), [prepared_measure.rs](src/prepared_measure.rs), [prepared_production.rs](src/prepared_production.rs) | Aligned width summaries and event clusters; exact balance and holder observations; production observations and selections. |
| [prepared_map.rs](src/prepared_map.rs) | Sparse native map checkpoints prepared beside the products, and the reader that serves viewer playback from them. |
| [ranking.rs](src/ranking.rs) | Event-based top-N selection and clipping. |
| [read_runs.rs](src/read_runs.rs) | Read-derived value runs and known coverage. |
| [research_dto.rs](src/research_dto.rs) | The research products' reply types, shared with the frontend. |
| [products/product.rs](src/products/product.rs) | The `Product` trait, `ReplyWarnings` and the exported `reply_warnings!` macro, the shared parameter types and validators, and the reply budget check. |
| [products/](src/products/mod.rs) | The shared products: country groups and history, country measure, experience and factories, equipment, equipment icons, produced equipment and production spending, faction totals, force census, territory and its breakdown, politics content, idea history and format, character portraits and production lines; and the small shared modules several product families name: artwork (focus, technology and equipment art with its research and resource titles), research, resources, place names, country styles, battles and fixed-point runs. |
| [test_fixtures.rs](src/test_fixtures.rs) | Test sources and content options over dashboard-core's recording builders. |

## Prepared series

A series family adopts the dashboard's block pre-pass through shared recording
([prepared/series.rs](src/prepared/series.rs)). Its extract marks its accumulators `record_as`, and from
the block's second hour every call they receive (advances as spans of
consecutive hours, observations, pushes and serialized events) is recorded in
call order. A call whose outcome depends on the family's carried state rather
than on the block (a deal membership and its version, a composition version) is
recorded as the family's own call (`record_family`) and made inside
`unrecorded`. The fold observes the block's first hour itself on the carried
state, which is where continuation differs from a fresh start, numbers the
extract's first-seen key names into the carried catalogue (`number_names`), and
makes the recorded calls again on the carried accumulators (`Recording::replay`),
its own calls on its carried state. Keys are the only relabelling: after the
first hour, a value open since then is never published again. The family then
carries its scan forward from the extract's end of block, having first checked
that the carried state continued as the extract did from the first hour; when a
check fails it observes the rest of the block hour by hour with the block's own
facts.

Each prepared query reads through one store scope for its source key. A series
reader keeps every batch's range handle and hour bounds once read, so batch
searches run in memory, and keeps the directory entries and decoded blobs (up to
256) it has read. A key's directory entry is a binary search of small reads in
its batch; once a reader has searched one batch for many keys (at least 16, and
one per 256 entries), it reads that batch's whole directory at once and searches
it in memory (up to 64 MiB of directories per reader).
`Reader::latest_event_rows` reads many keys' latest events at one hour, finding
the hour's batch once.

Every family's catalogue (`{family}-{branch}-catalogue`, the JSON text of its
keys and metadata) goes through the shared writer in
[prepared/series/catalogue.rs](src/prepared/series/catalogue.rs).
Range 0 holds the compressed base text; later ranges hold patches, each holding only
what changed in the text since the ranges before it: per object or array, runs of
earlier members or elements kept, ones edited in place and new ones' text, so
reconstruction reproduces the text byte for byte. Each patch records its result's
length and hash, which every reader checks, and the writer applies each patch before
writing it. A flush whose text is unchanged writes nothing. Otherwise it writes one
patch, first merging its newest patches into it while the newest is less than twice
the merged patch's size, so each patch is at least about twice the next and a chain
holds a logarithmic number of them; once the patches would outweigh the base it
writes a new base and empties the patch ranges. Bytes written stay proportional to
what changed, and a reader reads at most about twice the base. Queries read a
catalogue's ranges through one store session; restoration after growth reads the
saved key's chain (`series::read_saved_catalogue`); the event feed stamps its
decoded catalogues with the identities of every range in their chains.

## Sampled reads

Beyond a family's committed prepared prefix, queries sample the recording itself
([prepared/query/progressive.rs](src/prepared/query/progressive.rs)): a window of
at most 13 blocks is read exactly, every hour decoded and sampled in order; a wider
one reads each block's keyframe. The exact read takes its blocks through
dashboard-core's `scan_blocks_by`, two blocks read ahead on spare cores while the
calling thread decodes and samples, block by block, so samplers see the same hours
in the same order as a serial read. `tail_recorded` also hands the sampler each
exactly read hour's recorded updates (`Recorded`), so a sampler can carry its own
state from hour to hour; a frame read alone (a keyframe, the hour before a window)
comes without them.

Each source keeps a cache of what these reads decode
([prepared/query/frames.rs](src/prepared/query/frames.rs)): a block's frames as
read for the tables asked of it, and one hour's reconstructed frame with the union
of the tables asked of it (`frame_at` and the hour before an exact window or state
read). A request missing tables reads only those: a block keeps each read beside
the earlier ones and decodes them in step, an hour joins them to its frame.
Concurrent requests for one block or hour wait for a single read. Every table is
read and reconstructed on its own, so a request receives exactly what its own
narrower read gives, with only its tables selected; an error, a gap or a keyframe
whose extra table the decoder would refuse is read directly and not kept. The cache
holds at most 512 MiB per source, counting everything an entry keeps alive: each
read's frames whole (every table's update, read or not, and the hour's metrics), the
rows they hold with every string, list and nested value each row owns at its
capacity (replay's `TableRows::heap_bytes`), and the reasons of unavailable tables.
A request that may not decode makes no entry for what the cache does not hold, and
an entry left empty when its read ends is removed. It evicts
the least recently used entry first. A reader publishes and accounts its read before
any other request may read the same block or hour.

`progressive::without_block_decodes` runs a query with every block decode refused
(a sampled read with blocks to read, an hour the cache does not hold) and says
whether one was; the data service answers prepared queries this way first. Such a
query takes what the cache holds and never waits on another request's read of a
block or hour: it is refused at once.

The `test-support` feature compiles the fixtures, test oracles (the `#[cfg(test)]`
whole-recording loaders of prepared-only products) and the test instrumentation
(read and work counters) for the tests of the crates above, so their test builds
behave exactly as this crate's own. Only dev-dependencies enable it.

## Production lines first view

`production-lines` answers from one scan of every hour of the branch, in order
on one thread ([products/production_lines.rs](src/products/production_lines.rs)),
so its cost grows with the recording. A request without `exact` is answered from
the same scan over only the branch's newest blocks: the fewest holding one detail
window of hours, never more than four (the whole scan where they are all of the
branch). A block holds at most one keyframe interval, so this read does not grow
with the recording. The reply's `approximate` spans cover the branch from its
start through the first read among those blocks, where the lines listed are taken
as begun and their appearances are counted from. Further first-view requests
reuse the held first view. A request with `exact` scans the whole branch, or
answers from the held scan. Each reply is a function of its request alone, as the
history reply cache that keeps replies between sessions requires. Detail windows
and breakdowns always read the whole scan.

A whole scan is read once at a time: a request finding the scan of its country
being read waits for it, and takes it over if it fails. A country's whole scan
can be kept between sessions (`saved_scan`, `write_saved`, in the recording's
own row encoding for its modifier rows, each shared country row found in one
lookup) with what it was read with: the branch's last block, whether icon
content was loaded, whether executable metadata was configured, a digest of the
icon rule's visual inputs the recorded content's hash does not cover (declared
and refused sprites, graphic pools, geography, DLC names) and every texture
path the rule asked about with its answer. A whole-scan request that holds no
scan reads a kept one, read-only, and answers from it only where it is the scan
it would make (`hold_saved`, `pictures_hold`): a deleted or added texture, or
changed sprites, refuse it. The countries a request read whole wait in their
source's unkept set (`read_whole`, a set) until the source's next preparation
keeps them; a request writes nothing.

The scan repeats no world-wide work a read did not change. A variant read builds
a definition's operands again only where its row, modules or upgrades, or the
countries' keys, links, appearance or lexical tokens changed since the read
before (`VariantCatalogue::observe_built`, which records the versions `observe`
does). The input reader lives in `equipment::VariantReads`, shared with prepared
production's single-read catalogue. Production reuses equal classifications and
name strings while dating each selected name at the current supplying read;
non-read and incomplete hours still have an empty single-read catalogue.
Definitions are stored only where the table changed, and the icon rule's
country operands are built only at a read with variants to picture. Blocks are
read and decoded on spare cores (`extract_block`, three ahead), each hour reduced
to what the fold reads of it, with its frame kept only where the fold reads rows
(a variant read, a read of the country's lines).

## Country experience

The country-experience scan keeps complete hourly gains as runs of equal signed
amounts within a native instance. Changed inputs and banked reads visit their
countries; other countries' open runs hold through the scan's frontier. Daily
reads sum the last 24 complete intervals with wide arithmetic. Missing or
ambiguous gains, instance changes and identity gaps break that day. Quiet spans
advance the frontier without visiting countries. The saved runs and recent
banked reads resume with the observer; runtime joins rebuild from the first frame.

## Country identity history

The country identity history is prepared on its own. Its extract
([products/country_history/prepared.rs](src/products/country_history/prepared.rs)) reads a block by the fields the
history reads: the countries' keys, slots and tags; every decode of the links
(the join and the native names are held for one links allocation); only the
row keys of the counter rows (whose join counts unresolved and repeated rows)
and of the experience gains (country and native instance). Balances,
owned-state counts and gain amounts change every hour without making an hour
observed. Its history of the block alone observes each changed hour as hourly
preparation observes owned states, and steps each quiet span without its rows
(`HistoryScan::advance_quiet`) from the hour's read evidence alone: whether all
three witness tables (countries, links, gains) were read. An hour that did not
read them seals and closes the native witnesses; the next read opens them
again from the held names. Counter counts moving in a quiet hour change no
retained history: the holders hourly observation visits there are sealed at the
same hours by the next observation or seal. When an hour that did not read the
countries leaves the held names stale, the extract's deadline is the next hour,
observed on the last frame's rows until a read hour names them again. The
worker keeps the observed hours' frames (countries reduced to their keys,
counter rows to their country and owned-state count) and the quiet spans; the
fold observes each kept hour on the carried history and steps each span, and
observes the rest of the block hour by hour when the carried history does not
hold the inputs the worker's held before a span. Each commit writes
`country-identities-<branch>` whole, sealed, as hourly preparation's flush
does; it is also the history's continuation.

The shared country-history observer also retains native holders, aliases, identity
segments and complete-read coverage. Each commit replaces one branch identity
state page; cached table joins rebuild from the next accepted frame on resume.
Prepared identity lookup (`prepared_branch_histories`, [products/country_history.rs](src/products/country_history.rs)) returns the same
identity-only histories as the shared recording walk once the identities cover
the branch through this recording's last branch hour (a page completed for a
shorter recording of the same store does not count), by their own coverage,
ahead of the hourly walk (or by the branch's
in a store the hourly walk published alone), and remains pending before then; a
refused history answers every reader with its refusal. The identity gate of
naval losses, the occupation trend, the operatives roster, the raid tally and
the event feed therefore waits for the identities alone, not for every family. A
partial reader (`progressive::histories`) takes the history through its own
coverage's last hour (`HistoryScan::finish_through`), as a history sealed
there holds it. Its retained histories grow with identity and coverage changes.

## Idea history

Idea history is prepared on its own ([products/idea_history/prepared.rs](src/products/idea_history/prepared.rs),
registered in the dashboard's
[independent/quiet_politics.rs](../dashboard/src/prepared/pipeline/independent/quiet_politics.rs)) with the
production branch-wide idea observer. Its blocks are read by the fields the
observer and its change publisher read: country keys and links, held ideas and
appointed advisors, the characters' naming and template fields, every spelling
and modifier definition (packed together whenever either changes), the
appearance fields a holder is named by, and the civil-war lists, enemy
vectors, war operands and owned state counts a new holding's naming reads.
Kept frames hold countries, characters and country counters reduced to those
fields. Each hour read is observed as hourly preparation observes it; the
hours between visit no holder, so they advance the frontier at once, keep an
hour without its inputs unobserved and date missing character-template
refusals at their last hour. One `idea-history-{branch}` range holds its
zstd-compressed JSON state and `idea-state-{branch}` its change publisher's
continuation, both replaced at each of its commits. Resume reloads the same
observer; append observes only the accepted tail. The frame fact updates
changed holdings through the same native scan. Open endpoints materialize at a relevant
change, publication or reply. Missing character-template refusals retain their
hour-dependent text without rebuilding holdings. Stored spans, identity naming and refusal sets grow with recorded
changes, rather than elapsed hours.
Idea classification content is bound at configuration; the shared result cache
retains successful and refused loads, and fixed token classifications use the
shared keyed result cache. Only new additions contribute classification counts.

Prepared `idea-history` requests use `{branchId, tag}` and the production topic
reply builder. The observer retains both identity-start naming for topic replies
and span-start naming for event-feed, plus country-scoped refusal counts.
Queries wait for the family's own complete coverage and refuse from its own
coverage page. The shared idea-history UI hook uses
the ordinary preparation status, cancellation and retry handling; idea timelines
and national-spirit widgets retain their existing reply shapes and presentation.
The legacy product and event-feed branch scan remain available as references.
Idea holdings, acquisition/removal bounds, advisor seats and recorded renames
depend on preceding observations; they have no stateless progressive sampler.

## Territory breakdown

A reached regional territory breakdown hour
([products/territory_breakdown.rs](src/products/territory_breakdown.rs)) runs the
regional observer's step over that hour's frame alone, read directly with its
country totals: region series the frame does not reach are absent rather than
zero, and its observed spans name the columns the hour proves. Other regional
views keep their coverage gate.

## Prepared map checkpoints

[prepared_map.rs](src/prepared_map.rs) prepares the map's native inputs and shared
authored-trigger tables. Independent native checkpoints retain decoded rows,
partial censuses, unavailable fields and supplying hours every 720 accepted
updates. Each existing Store range uses zstd9 with a checksum. Hourly updates
remain in the recording; no prepared update pages or packet seeds are written.
`Builder::accept_sparse` accepts an hour after its caller admitted the whole
source block. It reads rows only when that hour writes a checkpoint, using the
keyframe sections at a block's first hour and an ordinary selected prefix read
otherwise. Other hours advance the same progress without reading map rows.
Checkpoint positions follow accepted frame counts through gaps and short blocks,
so sparse preparation writes exactly the hourly observer's checkpoint bytes.
The walk's selected `map.resume` state is replaced independently for its
preparation observers. Progress keeps checkpoint ordinals per branch and the
accepted source cursor. Appended preparation restores that native resume state
with the same selection. `decode_resume` validates a shared page using its stored
selection; a family restoring its own baseline checks its inputs and hour before
using it, and reads its baseline from the recording when they are absent.

The shared [Reader](src/prepared_map/reader.rs) serves viewer playback and
acceptance. It binary-searches checkpoint headers, restores the nearest native
state and calls the ordinary selected recording reader through the target hour.
Source parsing starts at each source block's keyframe; native application skips
the frames preceding the checkpoint cursor. Selected table decoding uses half
the available processors, at least one, with the existing below-normal worker
priority. Consecutive steps mutate the native decoder in place. A step needing a
source load reads the rest of that accepted block once; later steps retain it.
Only one parsed source block remains between requests. Source admission precedes
passed-hour callbacks, and reads never extend beyond accepted branch progress.
No recording format, directory, index or setting is added.
Changed gaps become interval records with reasons in original occurrence order,
including repeats and overlaps. Readers binary-search gap records on disk.

Prepared map selection retains the Operatives map's mission, operation, agency-buffer, strategic-operative and stability inputs alongside the shared map and trigger operands.
